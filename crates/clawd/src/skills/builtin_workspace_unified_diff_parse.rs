use serde_json::json;

pub(super) struct ParsedPatch {
    pub files: Vec<ParsedFile>,
}

pub(super) struct ParsedFile {
    pub path: String,
    pub old_missing: bool,
    pub new_missing: bool,
    pub hunks: Vec<Hunk>,
    pub additions: u64,
    pub deletions: u64,
}

pub(super) struct Hunk {
    pub old_start: usize,
    pub new_start: usize,
    pub lines: Vec<HunkLine>,
}

pub(super) struct HunkLine {
    pub kind: LineKind,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LineKind {
    Context,
    Remove,
    Add,
}

pub(super) fn parse_patch(patch: &str) -> Result<ParsedPatch, String> {
    let lines = patch.split_terminator('\n').collect::<Vec<_>>();
    let mut files = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        reject_unsupported_metadata(lines[index])?;
        if !lines[index].starts_with("--- ") {
            index += 1;
            continue;
        }
        let old_path = parse_header_path(&lines[index][4..])?;
        index += 1;
        let new_header = lines.get(index).ok_or_else(invalid_patch)?;
        if !new_header.starts_with("+++ ") {
            return Err(invalid_patch());
        }
        let new_path = parse_header_path(&new_header[4..])?;
        index += 1;
        let old_missing = old_path.is_none();
        let new_missing = new_path.is_none();
        if old_missing && new_missing {
            return Err(invalid_patch());
        }
        let path = match (&old_path, &new_path) {
            (Some(old), Some(new)) if old != new => {
                return Err(super::diff_error(
                    "rename_not_supported",
                    "workspace.patch.rename_not_supported",
                    json!({"old_path": old, "new_path": new}),
                ));
            }
            (Some(path), _) | (_, Some(path)) => path.clone(),
            _ => unreachable!(),
        };
        if files.iter().any(|file: &ParsedFile| file.path == path) {
            return Err(super::diff_error(
                "duplicate_patch_path",
                "workspace.patch.duplicate_path",
                json!({"path": path}),
            ));
        }

        let mut hunks = Vec::new();
        let mut additions = 0;
        let mut deletions = 0;
        while index < lines.len() {
            reject_unsupported_metadata(lines[index])?;
            if lines[index].starts_with("diff --git ") || lines[index].starts_with("--- ") {
                break;
            }
            if !lines[index].starts_with("@@ ") {
                index += 1;
                continue;
            }
            let (hunk, next) = parse_hunk(&lines, index)?;
            additions += hunk
                .lines
                .iter()
                .filter(|line| line.kind == LineKind::Add)
                .count() as u64;
            deletions += hunk
                .lines
                .iter()
                .filter(|line| line.kind == LineKind::Remove)
                .count() as u64;
            hunks.push(hunk);
            index = next;
        }
        if hunks.is_empty() {
            return Err(invalid_patch());
        }
        files.push(ParsedFile {
            path,
            old_missing,
            new_missing,
            hunks,
            additions,
            deletions,
        });
    }
    if files.is_empty() {
        return Err(invalid_patch());
    }
    Ok(ParsedPatch { files })
}

fn parse_hunk(lines: &[&str], start: usize) -> Result<(Hunk, usize), String> {
    let header = lines[start]
        .strip_prefix("@@ ")
        .and_then(|value| value.split_once(" @@"))
        .map(|(ranges, _)| ranges)
        .ok_or_else(invalid_patch)?;
    let mut ranges = header.split_whitespace();
    let (old_start, old_count) = parse_range(ranges.next(), '-')?;
    let (new_start, new_count) = parse_range(ranges.next(), '+')?;
    if ranges.next().is_some() {
        return Err(invalid_patch());
    }

    let mut parsed = Vec::<HunkLine>::new();
    let mut index = start + 1;
    while index < lines.len() {
        let line = lines[index];
        if line.starts_with("@@ ") || line.starts_with("diff --git ") || line.starts_with("--- ") {
            break;
        }
        if line.trim_end_matches('\r') == "\\ No newline at end of file" {
            let previous = parsed.last_mut().ok_or_else(invalid_patch)?;
            previous.text.pop();
            index += 1;
            continue;
        }
        let (kind, text) = match line.as_bytes().first() {
            Some(b' ') => (LineKind::Context, &line[1..]),
            Some(b'-') => (LineKind::Remove, &line[1..]),
            Some(b'+') => (LineKind::Add, &line[1..]),
            _ => break,
        };
        parsed.push(HunkLine {
            kind,
            text: format!("{text}\n"),
        });
        index += 1;
    }
    let observed_old = parsed
        .iter()
        .filter(|line| line.kind != LineKind::Add)
        .count();
    let observed_new = parsed
        .iter()
        .filter(|line| line.kind != LineKind::Remove)
        .count();
    if observed_old != old_count || observed_new != new_count {
        return Err(super::diff_error(
            "invalid_hunk_counts",
            "workspace.patch.invalid_hunk_counts",
            json!({
                "old_count": old_count,
                "observed_old_count": observed_old,
                "new_count": new_count,
                "observed_new_count": observed_new,
            }),
        ));
    }
    Ok((
        Hunk {
            old_start,
            new_start,
            lines: parsed,
        },
        index,
    ))
}

fn parse_range(value: Option<&str>, prefix: char) -> Result<(usize, usize), String> {
    let value = value
        .and_then(|value| value.strip_prefix(prefix))
        .ok_or_else(invalid_patch)?;
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    let start = start.parse::<usize>().map_err(|_| invalid_patch())?;
    let count = count.parse::<usize>().map_err(|_| invalid_patch())?;
    Ok((start, count))
}

fn parse_header_path(value: &str) -> Result<Option<String>, String> {
    let value = value.split('\t').next().unwrap_or(value).trim();
    if value == "/dev/null" {
        return Ok(None);
    }
    if value.is_empty() || value.starts_with('"') || value.ends_with('"') {
        return Err(super::diff_error(
            "unsupported_patch_path_encoding",
            "workspace.patch.unsupported_path_encoding",
            json!({"path": value}),
        ));
    }
    Ok(Some(
        value
            .strip_prefix("a/")
            .or_else(|| value.strip_prefix("b/"))
            .unwrap_or(value)
            .to_string(),
    ))
}

fn reject_unsupported_metadata(line: &str) -> Result<(), String> {
    if line.starts_with("new file mode 120000")
        || line.starts_with("deleted file mode 120000")
        || line.starts_with("old mode 120000")
        || line.starts_with("new mode 120000")
    {
        return Err(super::diff_error(
            "symlink_path_denied",
            "workspace.patch.symlink_denied",
            json!({"metadata": line}),
        ));
    }
    let unsafe_metadata = line == "GIT binary patch"
        || line.starts_with("Binary files ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
        || line.starts_with("copy from ")
        || line.starts_with("copy to ")
        || line.starts_with("old mode ")
        || line.starts_with("new mode ")
        || (line.starts_with("new file mode ") && !line.starts_with("new file mode 100644"));
    if unsafe_metadata {
        return Err(super::diff_error(
            "unsupported_patch_feature",
            "workspace.patch.unsupported_feature",
            json!({"metadata": line}),
        ));
    }
    Ok(())
}

fn invalid_patch() -> String {
    super::diff_error(
        "invalid_patch",
        "workspace.patch.invalid",
        json!({"engine": "pure_rust"}),
    )
}
