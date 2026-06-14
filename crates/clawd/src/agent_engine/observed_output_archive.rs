use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ArchiveListEntry {
    pub(super) name: String,
    pub(super) size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ArchiveListSummary {
    pub(super) archive: Option<String>,
    pub(super) entries: Vec<ArchiveListEntry>,
}

pub(super) fn archive_basic_path_value_from_body(body: &str, labels: &[&str]) -> Option<String> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body.trim()) {
        for label in labels {
            if let Some(path) = value
                .get(*label)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| archive_basic_observed_path_candidate(value))
            {
                return Some(path.to_string());
            }
        }
    }
    for token in body.split_whitespace() {
        let token = token.trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '。' | '，'
            )
        });
        let Some((key, rhs)) = token.split_once('=') else {
            continue;
        };
        if !labels
            .iter()
            .any(|label| key.trim().eq_ignore_ascii_case(label))
        {
            continue;
        }
        let rhs = rhs.trim();
        if archive_basic_observed_path_candidate(rhs) {
            return Some(rhs.to_string());
        }
    }
    None
}

pub(super) fn archive_basic_observed_path_candidate(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 4096
        && !value.contains(|ch| matches!(ch, '\n' | '\r' | '\0'))
        && !value.contains("://")
        && (value.starts_with('/')
            || value.starts_with("./")
            || value.starts_with("../")
            || value.contains('/'))
}

pub(super) fn archive_list_summary_from_body(body: &str) -> Option<ArchiveListSummary> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| archive_list_summary_from_value(&value))
        .or_else(|| archive_list_summary_from_raw_output(body, None))
}

pub(super) fn archive_read_direct_answer_candidate(body: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    if value.get("action").and_then(|value| value.as_str()) != Some("read") {
        return None;
    }
    value
        .get("content")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .map(ToString::to_string)
}

pub(super) fn archive_list_summary_from_value(
    value: &serde_json::Value,
) -> Option<ArchiveListSummary> {
    if value.get("action").and_then(|v| v.as_str())? != "list" {
        return None;
    }
    let archive = value
        .get("archive")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);
    if let Some(entries) = archive_entries_from_value_array(value.get("entries")) {
        if !entries.is_empty() {
            return Some(ArchiveListSummary { archive, entries });
        }
    }
    value
        .get("output")
        .and_then(|v| v.as_str())
        .and_then(|output| archive_list_summary_from_raw_output(output, archive))
}

pub(super) fn archive_entries_from_value_array(
    value: Option<&serde_json::Value>,
) -> Option<Vec<ArchiveListEntry>> {
    let entries = value?.as_array()?;
    Some(
        entries
            .iter()
            .filter_map(|entry| {
                let name = entry
                    .get("name")
                    .or_else(|| entry.get("path"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty())?;
                let size_bytes = entry.get("size_bytes").and_then(|v| v.as_u64());
                Some(ArchiveListEntry {
                    name: name.to_string(),
                    size_bytes,
                })
            })
            .collect(),
    )
}

pub(super) fn archive_list_summary_from_raw_output(
    output: &str,
    archive_hint: Option<String>,
) -> Option<ArchiveListSummary> {
    let archive = archive_hint.or_else(|| archive_path_from_listing_header(output));
    let mut entries = output
        .lines()
        .filter_map(parse_zip_listing_entry_line)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        entries = parse_plain_archive_listing_entries(output);
    }
    if entries.is_empty() {
        return None;
    }
    Some(ArchiveListSummary { archive, entries })
}

pub(super) fn archive_path_from_listing_header(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Archive:")
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(ToString::to_string)
    })
}

pub(super) fn parse_zip_listing_entry_line(line: &str) -> Option<ArchiveListEntry> {
    static ZIP_ENTRY_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let regex = ZIP_ENTRY_RE.get_or_init(|| {
        regex::Regex::new(r"^\s*(\d+)\s+\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}\s+(.+?)\s*$")
            .expect("valid zip listing entry regex")
    });
    let captures = regex.captures(line)?;
    let size_bytes = captures.get(1)?.as_str().parse::<u64>().ok();
    let name = captures.get(2)?.as_str().trim();
    (!name.is_empty()).then(|| ArchiveListEntry {
        name: name.to_string(),
        size_bytes,
    })
}

pub(super) fn parse_plain_archive_listing_entries(output: &str) -> Vec<ArchiveListEntry> {
    if output
        .lines()
        .any(|line| line.trim_start().starts_with("Archive:"))
    {
        return Vec::new();
    }
    if output.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("adding:")
            || line.starts_with("updating:")
            || line.starts_with("freshening:")
    }) {
        return Vec::new();
    }
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != "exit=0")
        .filter(|line| !line.starts_with("tar:"))
        .filter(|line| !line.starts_with("zip warning:"))
        .filter(|line| !line.chars().all(|ch| ch == '-'))
        .map(|name| ArchiveListEntry {
            name: name.to_string(),
            size_bytes: None,
        })
        .collect()
}

pub(super) fn archive_entry_display(entry: &ArchiveListEntry, prefer_english: bool) -> String {
    match entry.size_bytes {
        Some(size) if prefer_english => format!("{} ({size} bytes)", entry.name),
        Some(size) => format!("{}（{size} 字节）", entry.name),
        None => entry.name.clone(),
    }
}

pub(super) fn archive_list_summary_direct_answer(
    state: Option<&AppState>,
    summary: &ArchiveListSummary,
    prefer_english: bool,
) -> Option<String> {
    if summary.entries.is_empty() {
        return None;
    }
    let shown = summary
        .entries
        .iter()
        .take(8)
        .map(|entry| archive_entry_display(entry, prefer_english))
        .collect::<Vec<_>>();
    if shown.is_empty() {
        return None;
    }
    let omitted = summary.entries.len().saturating_sub(shown.len());
    let separator = if prefer_english { ", " } else { "、" };
    let entries = shown.join(separator);
    let count = summary.entries.len().to_string();
    let count_label = if prefer_english {
        if summary.entries.len() == 1 {
            "1 entry".to_string()
        } else {
            format!("{} entries", summary.entries.len())
        }
    } else {
        format!("{} 个条目", summary.entries.len())
    };
    let more = if omitted == 0 {
        String::new()
    } else if prefer_english {
        format!(", plus {omitted} more")
    } else {
        format!("，另有 {omitted} 个未列出")
    };
    Some(observed_t_with_vars(
        state,
        "clawd.msg.archive_list_summary",
        "压缩包包含 {count_label}：{entries}{more}。",
        "The archive contains {count_label}: {entries}{more}.",
        prefer_english,
        &[
            ("count", &count),
            ("count_label", &count_label),
            ("entries", &entries),
            ("more", &more),
        ],
    ))
}

pub(super) fn archive_entry_existence_direct_answer(
    state: Option<&AppState>,
    route: &crate::RouteResult,
    request_text: Option<&str>,
    summary: &ArchiveListSummary,
    archive_hint: Option<&str>,
    prefer_english: bool,
) -> Option<String> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ExistenceWithPath {
        return None;
    }
    let archive_path = archive_hint
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or(summary.archive.as_deref())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty()).then_some(hint)
        });
    let target = archive_entry_target_for_observed_route(route, request_text, archive_path)?;
    let exists = archive_list_contains_requested_entry(summary, &target)?;
    let vars = [("entry", target.as_str())];
    Some(if exists {
        observed_t_with_vars(
            state,
            "clawd.msg.archive_entry_exists",
            "压缩包中存在 {entry}。",
            "Yes, {entry} exists in the archive.",
            prefer_english,
            &vars,
        )
    } else {
        observed_t_with_vars(
            state,
            "clawd.msg.archive_entry_missing",
            "压缩包中不存在 {entry}。",
            "No, {entry} does not exist in the archive.",
            prefer_english,
            &vars,
        )
    })
}

pub(super) fn archive_entry_target_for_observed_route(
    route: &crate::RouteResult,
    request_text: Option<&str>,
    archive_path: Option<&str>,
) -> Option<String> {
    let mut path_candidates = Vec::new();
    let mut filename_candidates = Vec::new();
    for text in request_text
        .into_iter()
        .chain(std::iter::once(route.resolved_intent.as_str()))
    {
        for locator in
            crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        {
            push_archive_entry_observed_candidate(
                &mut path_candidates,
                &mut filename_candidates,
                &locator.locator_hint,
                archive_path,
            );
        }
        for filename in crate::delivery_utils::extract_filename_candidates(text) {
            push_archive_entry_observed_candidate(
                &mut path_candidates,
                &mut filename_candidates,
                &filename,
                archive_path,
            );
        }
    }
    path_candidates
        .into_iter()
        .next()
        .or_else(|| filename_candidates.into_iter().next())
}

pub(super) fn push_archive_entry_observed_candidate(
    path_candidates: &mut Vec<String>,
    filename_candidates: &mut Vec<String>,
    candidate: &str,
    archive_path: Option<&str>,
) {
    let Some(candidate) = normalize_archive_entry_observed_candidate(candidate, archive_path)
    else {
        return;
    };
    let target = if candidate.contains('/') || candidate.contains('\\') {
        path_candidates
    } else {
        filename_candidates
    };
    if !target
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        target.push(candidate);
    }
}

pub(super) fn normalize_archive_entry_observed_candidate(
    candidate: &str,
    archive_path: Option<&str>,
) -> Option<String> {
    let trimmed = candidate.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    });
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains("://")
        || Path::new(trimmed).is_absolute()
        || archive_candidate_has_supported_extension(trimmed)
        || archive_path.is_some_and(|path| archive_candidate_matches_archive(trimmed, path))
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(trimmed)
    {
        return None;
    }
    if !trimmed.contains('/')
        && !trimmed.contains('\\')
        && !archive_entry_observed_candidate_has_extension(trimmed)
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub(super) fn archive_candidate_has_supported_extension(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".tgz")
}

pub(super) fn archive_candidate_matches_archive(candidate: &str, archive_path: &str) -> bool {
    let candidate_norm = candidate.replace('\\', "/");
    let archive_norm = archive_path.trim().replace('\\', "/");
    if candidate_norm.eq_ignore_ascii_case(&archive_norm) {
        return true;
    }
    let archive_name = archive_norm
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(archive_norm.as_str());
    candidate_norm.eq_ignore_ascii_case(archive_name)
}

pub(super) fn archive_entry_observed_candidate_has_extension(candidate: &str) -> bool {
    let basename = candidate
        .rsplit(|ch| ch == '/' || ch == '\\')
        .next()
        .unwrap_or(candidate);
    let Some((stem, ext)) = basename.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=16).contains(&ext.len())
        && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
}

pub(super) fn normalize_archive_entry_name(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

pub(super) fn archive_list_contains_requested_entry(
    summary: &ArchiveListSummary,
    target: &str,
) -> Option<bool> {
    let target_norm = normalize_archive_entry_name(target);
    if target_norm.is_empty() {
        return None;
    }
    if summary
        .entries
        .iter()
        .any(|entry| normalize_archive_entry_name(&entry.name).eq_ignore_ascii_case(&target_norm))
    {
        return Some(true);
    }
    if target_norm.contains('/') {
        return Some(false);
    }
    let basename_matches = summary
        .entries
        .iter()
        .filter(|entry| {
            normalize_archive_entry_name(&entry.name)
                .rsplit('/')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case(&target_norm))
        })
        .take(2)
        .count();
    match basename_matches {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

pub(super) fn archive_list_summary_observed_candidate(
    summary: &ArchiveListSummary,
) -> Option<String> {
    if summary.entries.is_empty() {
        return None;
    }
    let archive = summary.archive.as_deref().unwrap_or("-");
    let mut lines = vec![format!(
        "archive_basic action=list archive={archive} total_entries={}",
        summary.entries.len()
    )];
    for entry in summary.entries.iter().take(32) {
        match entry.size_bytes {
            Some(size_bytes) => {
                lines.push(format!("entry name={} size_bytes={size_bytes}", entry.name))
            }
            None => lines.push(format!("entry name={}", entry.name)),
        }
    }
    if summary.entries.len() > 32 {
        lines.push(format!("entries_omitted={}", summary.entries.len() - 32));
    }
    Some(lines.join("\n"))
}

pub(super) fn answer_is_raw_archive_listing_passthrough(answer: &str) -> bool {
    let trimmed = answer.trim();
    if trimmed.is_empty() || archive_list_summary_from_raw_output(trimmed, None).is_none() {
        return false;
    }
    trimmed
        .lines()
        .map(str::trim_start)
        .any(|line| line.starts_with("Archive:") || line.starts_with("Length"))
}

pub(super) fn latest_archive_list_summary(loop_state: &LoopState) -> Option<ArchiveListSummary> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "archive_basic")
        .filter_map(|step| step.output.as_deref())
        .find_map(archive_list_summary_from_body)
}

pub(super) fn archive_list_raw_passthrough_replacement(
    answer: &str,
    state: &AppState,
    loop_state: &LoopState,
    request_language_hint: &str,
) -> Option<String> {
    if !answer_is_raw_archive_listing_passthrough(answer) {
        return None;
    }
    let summary = latest_archive_list_summary(loop_state)?;
    archive_list_summary_direct_answer(
        Some(state),
        &summary,
        observed_request_prefers_english_template(Some(state), request_language_hint),
    )
}

pub(super) fn archive_basic_observed_candidate(value: &serde_json::Value) -> Option<String> {
    if let Some(summary) = archive_list_summary_from_value(value) {
        return archive_list_summary_observed_candidate(&summary);
    }
    let action = value.get("action").and_then(|v| v.as_str())?.trim();
    if action.is_empty() {
        return None;
    }
    let archive = value
        .get("archive")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("-");
    let output = value
        .get("output")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("ok");
    Some(format!(
        "archive_basic action={action} archive={archive}\n{output}"
    ))
}

pub(super) fn archive_unpack_direct_answer_candidate(
    route: Option<&crate::RouteResult>,
    body: &str,
    prefer_english: bool,
) -> Option<String> {
    let route = route?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ArchiveUnpack {
        return None;
    }
    let dest =
        archive_basic_path_value_from_body(body, &["dest", "dest_path", "destination", "path"])?;
    let members = archive_unpack_members_from_body(body, &dest);
    if !members.is_empty() {
        let joined = if prefer_english {
            members.join(", ")
        } else {
            members.join("、")
        };
        return if prefer_english {
            Some(format!("Unpacked to {dest}; extracted {joined}."))
        } else {
            Some(format!("已解压到 {dest}，包含 {joined}。"))
        };
    }
    if prefer_english {
        Some(format!("Unpacked to {dest}."))
    } else {
        Some(format!("已解压到 {dest}。"))
    }
}

pub(super) fn archive_unpack_members_from_body(body: &str, dest: &str) -> Vec<String> {
    let dest_path = Path::new(dest);
    let mut members = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        let Some((prefix, raw_path)) = line.split_once(':') else {
            continue;
        };
        if !matches!(
            prefix.trim().to_ascii_lowercase().as_str(),
            "inflating" | "extracting" | "creating"
        ) {
            continue;
        }
        let raw_path = raw_path.trim();
        if raw_path.is_empty() {
            continue;
        }
        let path = Path::new(raw_path);
        let member = path
            .strip_prefix(dest_path)
            .ok()
            .and_then(|relative| relative.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::trim)
            })
            .filter(|value| !value.is_empty());
        let Some(member) = member else {
            continue;
        };
        let member = member.trim_matches('/').to_string();
        if member.is_empty() || members.iter().any(|existing| existing == &member) {
            continue;
        }
        members.push(member);
        if members.len() >= 5 {
            break;
        }
    }
    members
}
