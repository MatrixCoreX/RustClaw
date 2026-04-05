use std::path::{Path, PathBuf};

use crate::AppState;

pub(crate) fn has_concrete_locator_hint(text: &str) -> bool {
    if has_explicit_path_or_url_locator(text) {
        return true;
    }
    text.split_whitespace()
        .map(trim_locator_token)
        .any(|token| looks_like_filename_locator(&token))
}

#[derive(Debug)]
pub(crate) enum LocatorAutoResolution {
    Direct(String),
    Fuzzy(Vec<String>),
}

pub(crate) fn try_resolve_implicit_locator_path(
    state: &AppState,
    raw_text: &str,
    resolved_text: &str,
    locator_kind: crate::OutputLocatorKind,
    _context_hint: Option<&str>,
) -> Option<LocatorAutoResolution> {
    let query_text = format!("{raw_text}\n{resolved_text}");
    let keywords = extract_locator_keywords(&query_text);
    let filename_tokens = extract_filename_like_tokens(&query_text);
    if matches!(locator_kind, crate::OutputLocatorKind::CurrentWorkspace) {
        let workspace_root = state.default_locator_search_dir.clone();
        if workspace_root.is_dir() {
            if let Some(path) = try_resolve_direct_child_locator(&workspace_root, &keywords) {
                return Some(LocatorAutoResolution::Direct(path));
            }
            if let Some(resolved) = try_resolve_implicit_locator_path_in_roots(
                std::slice::from_ref(&workspace_root),
                &keywords,
                &filename_tokens,
                state.locator_scan_max_depth,
                state.locator_scan_max_files,
            ) {
                return Some(resolved);
            }
        }
        return Some(LocatorAutoResolution::Direct(resolve_workspace_root_path(
            &state.workspace_root,
        )));
    }
    let mut roots = vec![state.default_locator_search_dir.clone()];
    let system_root = PathBuf::from("/");
    if state.default_locator_search_dir != system_root && system_root.is_dir() {
        roots.push(system_root);
    }
    try_resolve_implicit_locator_path_in_roots(
        &roots,
        &keywords,
        &filename_tokens,
        state.locator_scan_max_depth,
        state.locator_scan_max_files,
    )
}

fn resolve_workspace_root_path(workspace_root: &Path) -> String {
    workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf())
        .display()
        .to_string()
}

fn try_resolve_implicit_locator_path_in_roots(
    roots: &[PathBuf],
    keywords: &[String],
    filename_tokens: &[String],
    max_depth: usize,
    max_files: usize,
) -> Option<LocatorAutoResolution> {
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let files = collect_files_for_locator_scan(root, max_depth, max_files);
        if files.is_empty() {
            if let Some(path) = try_resolve_direct_child_locator(root, keywords) {
                return Some(LocatorAutoResolution::Direct(path));
            }
            continue;
        }
        for token in filename_tokens {
            let mut ci_matches = collect_case_insensitive_filename_matches(&files, token);
            ci_matches.sort();
            ci_matches.dedup();
            if ci_matches.len() == 1 {
                return Some(LocatorAutoResolution::Direct(
                    ci_matches.into_iter().next().unwrap_or_default(),
                ));
            }
            if ci_matches.len() > 1 {
                if let Some(preferred) =
                    prefer_direct_child_filename_match(root, &ci_matches, token)
                {
                    return Some(LocatorAutoResolution::Direct(preferred));
                }
            }
            if ci_matches.len() > 1 {
                return Some(LocatorAutoResolution::Fuzzy(
                    ci_matches.into_iter().take(3).collect(),
                ));
            }
        }
        if !filename_tokens.is_empty() {
            continue;
        }
        if let Some(path) = try_resolve_direct_child_locator(root, keywords) {
            return Some(LocatorAutoResolution::Direct(path));
        }
    }
    None
}

fn prefer_direct_child_filename_match(
    root: &Path,
    matches: &[String],
    token: &str,
) -> Option<String> {
    let want_stem_match = !token.contains('.');
    let mut direct_hits = matches
        .iter()
        .filter_map(|raw| {
            let path = PathBuf::from(raw);
            let parent = path.parent()?;
            if parent != root {
                return None;
            }
            let file_name = path.file_name()?.to_string_lossy().to_string();
            if file_name.eq_ignore_ascii_case(token) {
                return Some(raw.clone());
            }
            if want_stem_match
                && path
                    .file_stem()
                    .map(|v| v.to_string_lossy().eq_ignore_ascii_case(token))
                    .unwrap_or(false)
            {
                return Some(raw.clone());
            }
            None
        })
        .collect::<Vec<_>>();
    direct_hits.sort();
    direct_hits.dedup();
    (direct_hits.len() == 1).then(|| direct_hits.into_iter().next().unwrap_or_default())
}

fn trim_locator_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | ','
                    | '.'
                    | '，'
                    | '。'
                    | ':'
                    | '：'
                    | ';'
                    | '；'
                    | ')'
                    | '('
                    | ']'
                    | '['
                    | '）'
                    | '（'
                    | '】'
                    | '【'
                    | '>'
                    | '<'
                    | '》'
                    | '《'
            )
        })
        .to_string()
}

fn has_explicit_path_or_url_locator(text: &str) -> bool {
    text.split_whitespace()
        .map(trim_locator_token)
        .any(|token| {
            if token.is_empty() {
                return false;
            }
            token.starts_with('/')
                || token.starts_with("./")
                || token.starts_with("../")
                || token.starts_with("~/")
                || token.starts_with("http://")
                || token.starts_with("https://")
                || token.contains(":\\")
        })
}

fn looks_like_filename_locator(token: &str) -> bool {
    if token.is_empty()
        || token.contains('/')
        || token.contains('\\')
        || token.starts_with("http://")
        || token.starts_with("https://")
    {
        return false;
    }
    let Some((base, ext)) = token.rsplit_once('.') else {
        return false;
    };
    if base.is_empty() || ext.is_empty() {
        return false;
    }
    ext.chars().all(|ch| ch.is_ascii_alphanumeric()) && ext.len() <= 12
}

fn looks_like_bare_filename_stem_locator(token: &str) -> bool {
    if token.is_empty()
        || token.contains('/')
        || token.contains('\\')
        || token.contains('.')
        || token.starts_with("http://")
        || token.starts_with("https://")
    {
        return false;
    }
    if token.chars().count() < 2 {
        return false;
    }
    if !token
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return false;
    }
    token.chars().any(|ch| ch.is_ascii_uppercase())
}

fn collect_files_for_locator_scan(root: &Path, max_depth: usize, max_files: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let mut entries = match std::fs::read_dir(&dir) {
            Ok(iter) => iter
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect::<Vec<_>>(),
            Err(_) => continue,
        };
        entries.sort();
        for path in entries {
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let file_type = meta.file_type();
            if file_type.is_dir() {
                out.push(path.clone());
                if out.len() >= max_files {
                    return out;
                }
                if depth < max_depth {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if file_type.is_file() || (file_type.is_symlink() && path.is_file()) {
                out.push(path);
                if out.len() >= max_files {
                    return out;
                }
                continue;
            }
        }
    }
    out
}

fn push_locator_keyword(token: &str, acc: &mut Vec<String>) {
    let lowered = token.trim().to_ascii_lowercase();
    if lowered.chars().count() < 2 {
        return;
    }
    if lowered.chars().all(|ch| ch.is_ascii_digit()) {
        return;
    }
    if !acc.iter().any(|v| v == &lowered) {
        acc.push(lowered.clone());
    }
    if acc.len() >= 12 {
        return;
    }
    for part in lowered.split(|ch: char| matches!(ch, '.' | '_' | '-')) {
        let trimmed = part.trim();
        if trimmed.is_empty() || trimmed == lowered || trimmed.chars().count() < 2 {
            continue;
        }
        if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        if !acc.iter().any(|v| v == trimmed) {
            acc.push(trimmed.to_string());
            if acc.len() >= 12 {
                break;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocatorKeywordClass {
    AsciiLike,
    Cjk,
}

fn classify_locator_keyword_char(ch: char) -> Option<LocatorKeywordClass> {
    let is_cjk = ('\u{4E00}'..='\u{9FFF}').contains(&ch);
    if is_cjk {
        return Some(LocatorKeywordClass::Cjk);
    }
    if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
        return Some(LocatorKeywordClass::AsciiLike);
    }
    None
}

fn extract_locator_keywords(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_class: Option<LocatorKeywordClass> = None;
    for ch in text.chars() {
        if let Some(next_class) = classify_locator_keyword_char(ch) {
            if cur_class.is_some() && cur_class != Some(next_class) && !cur.is_empty() {
                push_locator_keyword(&cur, &mut out);
                cur.clear();
                if out.len() >= 12 {
                    break;
                }
            }
            cur_class = Some(next_class);
            cur.push(ch.to_ascii_lowercase());
        } else if !cur.is_empty() {
            push_locator_keyword(&cur, &mut out);
            cur.clear();
            cur_class = None;
        } else {
            cur_class = None;
        }
        if out.len() >= 12 {
            break;
        }
    }
    if !cur.is_empty() && out.len() < 12 {
        push_locator_keyword(&cur, &mut out);
    }
    out
}

fn normalize_locator_match_text(token: &str) -> String {
    trim_locator_token(token)
        .chars()
        .map(|ch| match ch {
            '／' | '＼' => '/',
            '－' => '-',
            '＿' => '_',
            '．' => '.',
            '（' => '(',
            '）' => ')',
            '【' => '[',
            '】' => ']',
            '｛' => '{',
            '｝' => '}',
            '　' => ' ',
            _ => ch,
        })
        .collect::<String>()
        .to_lowercase()
}

fn try_resolve_direct_child_locator(search_root: &Path, keywords: &[String]) -> Option<String> {
    if !search_root.is_dir() || keywords.is_empty() {
        return None;
    }
    let normalized_keywords = keywords
        .iter()
        .map(|kw| normalize_locator_match_text(kw))
        .filter(|kw| !kw.is_empty())
        .collect::<Vec<_>>();
    if normalized_keywords.is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    let iter = std::fs::read_dir(search_root).ok()?;
    for entry in iter.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .map(|v| v.to_string_lossy().to_string())
            .unwrap_or_default();
        let normalized_name = normalize_locator_match_text(&name);
        if normalized_name.is_empty() {
            continue;
        }
        if normalized_keywords.iter().any(|kw| kw == &normalized_name) {
            let canonical = path.canonicalize().unwrap_or(path);
            matches.push(canonical.display().to_string());
        }
    }
    matches.sort();
    matches.dedup();
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

fn extract_filename_like_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split_whitespace() {
        let token = trim_locator_token(raw);
        if (looks_like_filename_locator(&token) || looks_like_bare_filename_stem_locator(&token))
            && !out.iter().any(|v| v == &token)
        {
            out.push(token);
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn collect_case_insensitive_filename_matches(files: &[PathBuf], token: &str) -> Vec<String> {
    let use_stem_match = !token.contains('.');
    files
        .iter()
        .filter(|path| {
            let Some(file_name) = path.file_name().map(|v| v.to_string_lossy().to_string()) else {
                return false;
            };
            if file_name.eq_ignore_ascii_case(token) {
                return true;
            }
            if !use_stem_match {
                return false;
            }
            path.file_stem()
                .map(|v| v.to_string_lossy().eq_ignore_ascii_case(token))
                .unwrap_or(false)
        })
        .map(|path| path.display().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
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
                "clawd_worker_locator_{prefix}_{}_{}",
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
    fn concrete_locator_hint_detects_explicit_path_and_filename() {
        assert!(super::has_concrete_locator_hint(
            "read /home/guagua/test/README.md and summarize"
        ));
        assert!(super::has_concrete_locator_hint("send Cargo.toml"));
        assert!(super::has_concrete_locator_hint(
            "open https://example.com/file.txt"
        ));
    }

    #[test]
    fn concrete_locator_hint_rejects_deictic_without_locator() {
        assert!(!super::has_concrete_locator_hint(
            "读一下那个 README 开头并用一句话总结"
        ));
        assert!(!super::has_concrete_locator_hint("that config please"));
    }

    #[test]
    fn filename_like_tokens_extracts_expected_items() {
        let tokens = super::extract_filename_like_tokens(
            "please open Config.toml and README.md plus README",
        );
        assert!(tokens.iter().any(|v| v == "Config.toml"));
        assert!(tokens.iter().any(|v| v == "README.md"));
        assert!(tokens.iter().any(|v| v == "README"));
    }

    #[test]
    fn locator_keywords_split_mixed_script_tokens() {
        let out = super::extract_locator_keywords("请在 doc目录 里找 App_Config.toml");
        assert!(out.iter().any(|v| v == "doc"));
        assert!(out.iter().any(|v| v == "目录"));
        assert!(out.iter().any(|v| v == "app_config.toml"));
    }

    #[test]
    fn direct_child_locator_prefers_project_root_exact_name() {
        let root = TempDirGuard::new("direct_child");
        fs::create_dir_all(root.path.join("scripts")).expect("create scripts");
        fs::create_dir_all(root.path.join("nested/scripts")).expect("create nested/scripts");
        let keywords = vec![
            "查看一下".to_string(),
            "scripts".to_string(),
            "目录下面文件".to_string(),
        ];

        let out = super::try_resolve_direct_child_locator(&root.path, &keywords);
        let expected = root
            .path
            .join("scripts")
            .canonicalize()
            .expect("canonical scripts")
            .display()
            .to_string();
        assert_eq!(out, Some(expected));
    }

    #[test]
    fn ci_filename_match_supports_bare_readme_stem() {
        let root = TempDirGuard::new("readme_stem");
        fs::write(root.path.join("README.MD"), "# title\n").expect("write README.MD");
        let files = super::collect_files_for_locator_scan(&root.path, 0, 50);
        let out = super::collect_case_insensitive_filename_matches(&files, "README");
        assert_eq!(out.len(), 1);
        assert!(out[0].ends_with("README.MD"));
    }

    #[test]
    fn ci_filename_match_returns_multiple_for_same_stem() {
        let root = TempDirGuard::new("readme_stem_multi");
        fs::write(root.path.join("README.MD"), "# title\n").expect("write README.MD");
        fs::write(root.path.join("README.txt"), "title\n").expect("write README.txt");
        let files = super::collect_files_for_locator_scan(&root.path, 0, 50);
        let out = super::collect_case_insensitive_filename_matches(&files, "README");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn implicit_locator_falls_back_to_system_root_when_default_root_misses() {
        let default_root = TempDirGuard::new("default_root_miss");
        let system_root = TempDirGuard::new("system_root_hit");
        let target = system_root.path.join("Cargo.toml");
        fs::write(&target, "[package]\nname='demo'\n").expect("write Cargo.toml");
        let roots = vec![default_root.path.clone(), system_root.path.clone()];
        let out = super::try_resolve_implicit_locator_path_in_roots(
            &roots,
            &["cargo.toml".to_string()],
            &["Cargo.toml".to_string()],
            0,
            50,
        );
        match out {
            Some(super::LocatorAutoResolution::Direct(path)) => {
                assert!(path.ends_with("Cargo.toml"));
            }
            other => panic!("expected direct match, got {other:?}"),
        }
    }

    #[test]
    fn implicit_locator_prefers_unique_direct_child_match_over_nested_stem_matches() {
        let root = TempDirGuard::new("prefer_direct_child_stem");
        fs::write(root.path.join("README.md"), "# root\n").expect("write root README");
        fs::create_dir_all(root.path.join("UI")).expect("create UI");
        fs::write(root.path.join("UI/README.md"), "# ui\n").expect("write UI README");
        let roots = vec![root.path.clone()];
        let out = super::try_resolve_implicit_locator_path_in_roots(
            &roots,
            &["readme".to_string()],
            &["README".to_string()],
            2,
            100,
        );
        match out {
            Some(super::LocatorAutoResolution::Direct(path)) => {
                assert!(
                    path.ends_with("/README.md"),
                    "unexpected direct path: {path}"
                );
                assert!(
                    !path.contains("/UI/README.md"),
                    "unexpected nested match: {path}"
                );
            }
            other => panic!("expected direct root README match, got {other:?}"),
        }
    }

    #[test]
    fn implicit_locator_does_not_promote_keyword_fragment_over_missing_filename() {
        let root = TempDirGuard::new("missing_filename_keyword_fragment");
        fs::write(root.path.join("rustclaw"), "binary").expect("write rustclaw");
        let roots = vec![root.path.clone()];
        let out = super::try_resolve_implicit_locator_path_in_roots(
            &roots,
            &[
                "把".to_string(),
                "definitely_missing_named_file_rustclaw_001.txt".to_string(),
                "发给我".to_string(),
            ],
            &["definitely_missing_named_file_rustclaw_001.txt".to_string()],
            0,
            50,
        );
        assert!(out.is_none(), "unexpected locator resolution: {out:?}");
    }

    #[test]
    fn current_workspace_locator_binds_to_workspace_root() {
        let root = TempDirGuard::new("workspace_scope");
        let out = super::resolve_workspace_root_path(&root.path);
        assert_eq!(
            Some(out),
            Some(
                root.path
                    .canonicalize()
                    .expect("canonical root")
                    .display()
                    .to_string()
            )
        );
    }
}
