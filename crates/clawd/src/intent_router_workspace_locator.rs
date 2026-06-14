use std::path::{Path, PathBuf};

pub(super) fn workspace_direct_child_stem_locator_from_text(
    text: &str,
    workspace_root: &Path,
) -> Option<String> {
    let root = workspace_root.canonicalize().ok()?;
    let mut selected: Option<PathBuf> = None;
    for token in bare_stem_tokens(text) {
        let Some(path) = resolve_direct_child_stem(&root, token) else {
            continue;
        };
        match &selected {
            None => selected = Some(path),
            Some(existing) if existing == &path => {}
            Some(_) => return None,
        }
    }
    selected.map(|path| path.display().to_string())
}

fn bare_stem_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split_whitespace()
        .flat_map(|token| token.split(is_stem_delimiter))
        .map(trim_stem_token)
        .filter(|token| looks_like_bare_stem_token(token))
}

fn is_stem_delimiter(ch: char) -> bool {
    matches!(
        ch,
        ',' | '，'
            | '。'
            | ';'
            | '；'
            | ':'
            | '：'
            | '?'
            | '？'
            | '!'
            | '！'
            | '('
            | ')'
            | '（'
            | '）'
            | '['
            | ']'
            | '【'
            | '】'
            | '<'
            | '>'
            | '《'
            | '》'
            | '"'
            | '\''
            | '`'
    )
}

fn trim_stem_token(value: &str) -> &str {
    value.trim_matches(|ch: char| ch.is_whitespace() || is_stem_delimiter(ch))
}

fn looks_like_bare_stem_token(token: &str) -> bool {
    let len = token.chars().count();
    (2..=64).contains(&len)
        && token.is_ascii()
        && !token.contains('.')
        && !token.contains('/')
        && !token.contains('\\')
        && !token.contains("://")
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

fn resolve_direct_child_stem(root: &Path, token: &str) -> Option<PathBuf> {
    let mut matches = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_file() {
                return None;
            }
            let path = entry.path();
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(token))
                .then_some(path)
        })
        .collect::<Vec<_>>();
    matches.sort();
    if matches.len() == 1 {
        matches.pop()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time before unix epoch")
                .as_nanos();
            path.push(format!(
                "clawd_intent_router_workspace_locator_{prefix}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn binds_unique_workspace_direct_child_stem() {
        let root = TempDirGuard::new("unique_stem");
        fs::write(root.path.join("README.md"), "# demo").expect("write readme");

        let resolved = workspace_direct_child_stem_locator_from_text(
            "Read README and summarize it",
            &root.path,
        )
        .expect("resolved stem");

        assert!(resolved.ends_with("README.md"));
    }

    #[test]
    fn rejects_multiple_distinct_matching_stems() {
        let root = TempDirGuard::new("multiple_stems");
        fs::write(root.path.join("README.md"), "# demo").expect("write readme");
        fs::write(root.path.join("AGENTS.md"), "# agents").expect("write agents");

        let resolved =
            workspace_direct_child_stem_locator_from_text("Read README and AGENTS", &root.path);

        assert!(resolved.is_none());
    }
}
