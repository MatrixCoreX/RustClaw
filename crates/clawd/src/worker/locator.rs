use std::path::{Path, PathBuf};

use crate::AppState;

pub(crate) fn has_concrete_locator_hint(text: &str) -> bool {
    if has_explicit_path_or_url_locator(text)
        || crate::delivery_utils::has_concrete_locator_input(text)
    {
        return true;
    }
    text.split_whitespace()
        .map(trim_locator_token)
        .any(|token| {
            looks_like_filename_locator(&token) || looks_like_bare_filename_stem_locator(&token)
        })
}

pub(crate) fn has_explicit_path_or_url_locator_hint(text: &str) -> bool {
    has_explicit_path_or_url_locator(text)
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
    context_hint: Option<&str>,
) -> Option<LocatorAutoResolution> {
    let query_text = format!("{raw_text}\n{resolved_text}");
    if let Some(explicit_path) = resolve_explicit_locator_path_from_text(
        &state.skill_rt.workspace_root,
        &state.skill_rt.default_locator_search_dir,
        &query_text,
    ) {
        return Some(LocatorAutoResolution::Direct(explicit_path));
    }
    let keywords = extract_locator_keywords(&query_text);
    let filename_tokens = extract_filename_like_tokens(&query_text);
    if matches!(locator_kind, crate::OutputLocatorKind::CurrentWorkspace) {
        if let Some(resolved) = resolve_current_workspace_target(
            &state.skill_rt.workspace_root,
            &keywords,
            &filename_tokens,
            state.skill_rt.locator_scan_max_depth,
            state.skill_rt.locator_scan_max_files,
        ) {
            return Some(resolved);
        }
        return Some(LocatorAutoResolution::Direct(resolve_workspace_root_path(
            &state.skill_rt.workspace_root,
        )));
    }
    if let Some(resolved) = resolve_explicit_workspace_child_target(
        &state.skill_rt.workspace_root,
        &state.skill_rt.default_locator_search_dir,
        &keywords,
        &filename_tokens,
    ) {
        return Some(resolved);
    }
    if let Some(resolved) = resolve_direct_child_filename_token_target(
        &state.skill_rt.workspace_root,
        &state.skill_rt.default_locator_search_dir,
        &filename_tokens,
        state.skill_rt.locator_scan_max_files,
    ) {
        return Some(resolved);
    }
    let roots = implicit_locator_search_roots(
        &state.skill_rt.workspace_root,
        &state.skill_rt.default_locator_search_dir,
        context_hint,
    );
    try_resolve_implicit_locator_path_in_roots(
        &roots,
        &keywords,
        &filename_tokens,
        state.skill_rt.locator_scan_max_depth,
        state.skill_rt.locator_scan_max_files,
    )
}

fn resolve_current_workspace_target(
    workspace_root: &Path,
    keywords: &[String],
    filename_tokens: &[String],
    max_depth: usize,
    max_files: usize,
) -> Option<LocatorAutoResolution> {
    if !workspace_root.is_dir() {
        return None;
    }
    if let Some(path) = try_resolve_direct_child_locator(workspace_root, keywords) {
        let direct_path = PathBuf::from(&path);
        if direct_child_path_compatible_with_filename_tokens(&direct_path, filename_tokens) {
            return Some(LocatorAutoResolution::Direct(path));
        }
    }
    try_resolve_implicit_locator_path_in_roots(
        &[workspace_root.to_path_buf()],
        keywords,
        filename_tokens,
        max_depth,
        max_files,
    )
}

fn resolve_explicit_workspace_child_target(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    keywords: &[String],
    filename_tokens: &[String],
) -> Option<LocatorAutoResolution> {
    let direct = if !filename_tokens.is_empty() {
        try_resolve_implicit_direct_child_locator(workspace_root, keywords).or_else(|| {
            (workspace_root != default_locator_search_dir)
                .then(|| {
                    try_resolve_implicit_direct_child_locator(default_locator_search_dir, keywords)
                })
                .flatten()
        })
    } else {
        try_resolve_implicit_direct_child_locator(default_locator_search_dir, keywords).or_else(
            || {
                (workspace_root != default_locator_search_dir)
                    .then(|| try_resolve_implicit_direct_child_locator(workspace_root, keywords))
                    .flatten()
            },
        )
    }?;
    let direct_path = PathBuf::from(&direct);
    if !direct_child_path_compatible_with_filename_tokens(&direct_path, filename_tokens) {
        return None;
    }
    Some(LocatorAutoResolution::Direct(direct))
}

fn resolve_direct_child_filename_token_target(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    filename_tokens: &[String],
    max_files: usize,
) -> Option<LocatorAutoResolution> {
    if filename_tokens.is_empty() {
        return None;
    }
    let mut roots = vec![workspace_root.to_path_buf()];
    if workspace_root != default_locator_search_dir {
        roots.push(default_locator_search_dir.to_path_buf());
    }
    try_resolve_implicit_locator_path_in_roots(&roots, &[], filename_tokens, 0, max_files)
}

fn resolve_workspace_root_path(workspace_root: &Path) -> String {
    workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf())
        .display()
        .to_string()
}

fn normalize_implicit_locator_root(candidate: &Path) -> Option<PathBuf> {
    let normalized = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());
    (!is_system_root(&normalized)).then_some(normalized)
}

fn is_system_root(path: &Path) -> bool {
    path == Path::new("/")
}

fn find_matching_root_index(roots: &[PathBuf], candidate: &Path) -> Option<usize> {
    let Some(normalized) = normalize_implicit_locator_root(candidate) else {
        return None;
    };
    roots
        .iter()
        .position(|root| root.canonicalize().unwrap_or_else(|_| root.clone()) == normalized)
}

fn append_implicit_root_if_missing(roots: &mut Vec<PathBuf>, candidate: PathBuf) {
    let Some(normalized) = normalize_implicit_locator_root(&candidate) else {
        return;
    };
    if find_matching_root_index(roots, &normalized).is_none() {
        roots.push(normalized);
    }
}

fn prepend_implicit_root_if_missing(roots: &mut Vec<PathBuf>, candidate: PathBuf) {
    let Some(normalized) = normalize_implicit_locator_root(&candidate) else {
        return;
    };
    if let Some(idx) = roots
        .iter()
        .position(|root| root.canonicalize().unwrap_or_else(|_| root.clone()) == normalized)
    {
        if idx != 0 {
            let existing = roots.remove(idx);
            roots.insert(0, existing);
        }
        return;
    }
    roots.insert(0, normalized);
}

fn implicit_locator_search_roots(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    context_hint: Option<&str>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(context_root) =
        resolve_contextual_locator_root(workspace_root, default_locator_search_dir, context_hint)
    {
        prepend_implicit_root_if_missing(&mut roots, context_root);
    }
    append_implicit_root_if_missing(&mut roots, default_locator_search_dir.to_path_buf());
    if workspace_root != default_locator_search_dir {
        append_implicit_root_if_missing(&mut roots, workspace_root.to_path_buf());
    }
    roots
}

fn resolve_contextual_locator_root(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    context_hint: Option<&str>,
) -> Option<PathBuf> {
    let context_text = context_hint
        .map(str::trim)
        .filter(|text| !text.is_empty() && *text != "<none>")?;
    if let Some(path) = resolve_explicit_locator_path_from_text(
        workspace_root,
        default_locator_search_dir,
        context_text,
    ) {
        let path_buf = PathBuf::from(path);
        return if path_buf.is_dir() {
            Some(path_buf)
        } else {
            path_buf.parent().map(Path::to_path_buf)
        };
    }
    let keywords = extract_locator_keywords(context_text);
    if keywords.is_empty() {
        return None;
    }
    let direct =
        try_resolve_implicit_direct_child_locator(default_locator_search_dir, &keywords)
            .or_else(|| try_resolve_implicit_direct_child_locator(workspace_root, &keywords))?;
    let path_buf = PathBuf::from(direct);
    if path_buf.is_dir() {
        Some(path_buf)
    } else {
        path_buf.parent().map(Path::to_path_buf)
    }
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
        for token in filename_tokens {
            match crate::delivery_utils::scan_filename_matches_with_limit(
                root, token, max_depth, max_files,
            ) {
                crate::delivery_utils::FilenameScanResult::Found(path) => {
                    return Some(LocatorAutoResolution::Direct(path.display().to_string()));
                }
                crate::delivery_utils::FilenameScanResult::Candidates(paths) => {
                    let candidates = paths
                        .into_iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>();
                    if let Some(preferred) =
                        prefer_direct_child_filename_match(root, &candidates, token)
                    {
                        return Some(LocatorAutoResolution::Direct(preferred));
                    }
                    if !candidates.is_empty() {
                        return Some(LocatorAutoResolution::Fuzzy(candidates));
                    }
                }
                crate::delivery_utils::FilenameScanResult::NotFound
                | crate::delivery_utils::FilenameScanResult::TooManyEntries => {}
            }
        }
        let files = collect_files_for_locator_scan(root, max_depth, max_files);
        if files.is_empty() {
            if let Some(path) = try_resolve_implicit_direct_child_locator(root, keywords) {
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
        if let Some(path) = try_resolve_implicit_direct_child_locator(root, keywords) {
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
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut direct_hits = matches
        .iter()
        .filter_map(|raw| {
            let path = PathBuf::from(raw);
            let parent = path.parent()?.to_path_buf();
            let normalized_parent = parent.canonicalize().unwrap_or(parent);
            if normalized_parent != canonical_root {
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

fn direct_child_path_compatible_with_filename_tokens(
    direct_path: &Path,
    filename_tokens: &[String],
) -> bool {
    if filename_tokens.is_empty() {
        return true;
    }
    if direct_path.is_dir() {
        return false;
    }
    let Some(file_name) = direct_path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    filename_tokens.iter().any(|token| {
        if file_name.eq_ignore_ascii_case(token) {
            return true;
        }
        if token.contains('.') {
            return false;
        }
        direct_path
            .file_stem()
            .and_then(|v| v.to_str())
            .is_some_and(|stem| stem.eq_ignore_ascii_case(token))
    })
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
        .any(|token| looks_like_explicit_path_or_url_token(&token))
}

fn looks_like_explicit_path_or_url_token(token: &str) -> bool {
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
        || (token.contains('/') && !token.contains("://"))
}

fn extract_explicit_path_like_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split_whitespace() {
        let token = trim_locator_token(raw);
        if looks_like_explicit_path_or_url_token(&token) && !out.iter().any(|v| v == &token) {
            out.push(token);
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn expand_home_prefixed_path(token: &str) -> Option<PathBuf> {
    let suffix = token.strip_prefix("~/")?;
    let home = std::env::var_os("HOME")?;
    let mut out = PathBuf::from(home);
    out.push(suffix);
    Some(out)
}

fn resolve_explicit_locator_path_token(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    token: &str,
) -> Option<String> {
    if token.starts_with("http://") || token.starts_with("https://") {
        return None;
    }
    let raw_path = if let Some(expanded) = expand_home_prefixed_path(token) {
        expanded
    } else {
        PathBuf::from(token)
    };
    let mut candidates = Vec::new();
    if raw_path.is_absolute() {
        candidates.push(raw_path);
    } else {
        candidates.push(workspace_root.join(&raw_path));
        if default_locator_search_dir != workspace_root {
            candidates.push(default_locator_search_dir.join(&raw_path));
        }
    }
    for candidate in candidates {
        if let Some(canonical) = if candidate.is_file() || candidate.is_dir() {
            Some(candidate.canonicalize().unwrap_or(candidate))
        } else {
            crate::delivery_utils::resolve_existing_path_under_root_case_insensitive(
                Path::new("/"),
                &candidate.display().to_string(),
            )
        } {
            return Some(canonical.display().to_string());
        }
    }
    None
}

fn resolve_explicit_locator_path_from_text(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    text: &str,
) -> Option<String> {
    for token in extract_explicit_path_like_tokens(text) {
        if let Some(path) =
            resolve_explicit_locator_path_token(workspace_root, default_locator_search_dir, &token)
        {
            return Some(path);
        }
    }
    None
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

fn try_resolve_implicit_direct_child_locator(
    search_root: &Path,
    keywords: &[String],
) -> Option<String> {
    let Some(normalized_root) = normalize_implicit_locator_root(search_root) else {
        return None;
    };
    try_resolve_direct_child_locator(&normalized_root, keywords)
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
    use std::path::{Path, PathBuf};
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
        assert!(super::has_concrete_locator_hint(
            "read scripts/nl_tests/fixtures/device_local/package.json"
        ));
        assert!(super::has_concrete_locator_hint("send Cargo.toml"));
        assert!(super::has_concrete_locator_hint(
            "open https://example.com/file.txt"
        ));
        assert!(super::has_concrete_locator_hint(
            "发 document 目录下最后一个"
        ));
        assert!(super::has_concrete_locator_hint(
            "列出 logs 目录下前 5 个文件名"
        ));
    }

    #[test]
    fn concrete_locator_hint_rejects_deictic_without_locator() {
        assert!(!super::has_concrete_locator_hint(
            "读一下那个开头并用一句话总结"
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
    fn resolve_explicit_relative_locator_path_from_text_prefers_workspace_root() {
        let root = TempDirGuard::new("relative_explicit_path");
        let nested = root.path.join("fixtures").join("device_local");
        fs::create_dir_all(&nested).expect("create nested dirs");
        let file = nested.join("package.json");
        fs::write(&file, "{}").expect("write fixture");
        let out = super::resolve_explicit_locator_path_from_text(
            &root.path,
            &root.path,
            "去 fixtures/device_local/package.json 里找 name",
        );
        assert_eq!(
            out.as_deref(),
            Some(
                file.canonicalize()
                    .expect("canonical file")
                    .to_string_lossy()
                    .as_ref()
            )
        );
    }

    #[test]
    fn resolve_explicit_relative_locator_path_from_text_supports_case_mismatch() {
        let root = TempDirGuard::new("relative_explicit_path_case_mismatch");
        let nested = root.path.join("Fixtures").join("Device_Local");
        fs::create_dir_all(&nested).expect("create nested dirs");
        let file = nested.join("Package.JSON");
        fs::write(&file, "{}").expect("write fixture");
        let out = super::resolve_explicit_locator_path_from_text(
            &root.path,
            &root.path,
            "去 fixtures/device_local/package.json 里找 name",
        );
        assert_eq!(
            out.as_deref(),
            Some(
                file.canonicalize()
                    .expect("canonical file")
                    .to_string_lossy()
                    .as_ref()
            )
        );
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
    fn implicit_locator_search_roots_use_context_then_default_then_workspace() {
        let workspace = TempDirGuard::new("layered_roots_workspace");
        let default_root = workspace.path.join("fixtures");
        let logs = workspace.path.join("logs");
        fs::create_dir_all(&default_root).expect("create default root");
        fs::create_dir_all(&logs).expect("create logs");
        fs::write(logs.join("act_plan.log"), "ok\n").expect("write act plan");

        let roots = super::implicit_locator_search_roots(
            &workspace.path,
            &default_root,
            Some("刚才列的是 logs/act_plan.log"),
        );

        assert_eq!(roots.len(), 3, "unexpected roots: {roots:?}");
        assert_eq!(
            roots[0],
            logs.canonicalize().expect("canonical logs"),
            "context root should come first"
        );
        assert_eq!(
            roots[1],
            default_root.canonicalize().expect("canonical default root"),
            "default root should be second"
        );
        assert_eq!(
            roots[2],
            workspace.path.canonicalize().expect("canonical workspace"),
            "workspace root should remain as last fallback"
        );
        assert!(roots.iter().all(|root| root != Path::new("/")));
    }

    #[test]
    fn implicit_locator_search_roots_skip_system_root_default() {
        let workspace = TempDirGuard::new("skip_system_root_default");
        let roots = super::implicit_locator_search_roots(&workspace.path, Path::new("/"), None);
        assert_eq!(roots.len(), 1, "unexpected roots: {roots:?}");
        assert_eq!(
            roots[0],
            workspace.path.canonicalize().expect("canonical workspace"),
        );
    }

    #[test]
    fn implicit_locator_falls_back_to_workspace_root_when_default_root_misses() {
        let workspace = TempDirGuard::new("workspace_root_hit");
        let default_root = workspace.path.join("fixtures");
        fs::create_dir_all(&default_root).expect("create default root");
        let target = workspace.path.join("Cargo.toml");
        fs::write(&target, "[package]\nname='demo'\n").expect("write Cargo.toml");
        let roots = super::implicit_locator_search_roots(&workspace.path, &default_root, None);
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
    fn direct_child_filename_precheck_beats_context_root_for_unique_workspace_readme() {
        let workspace = TempDirGuard::new("direct_child_precheck_workspace");
        let fixture_dir = workspace.path.join("fixtures/device_local");
        fs::create_dir_all(&fixture_dir).expect("create fixture dir");
        fs::write(workspace.path.join("README.md"), "# root\n").expect("write root README");
        fs::write(fixture_dir.join("README.md"), "# fixture\n").expect("write fixture README");
        let mut state = crate::AppState::test_default_with_fixture_provider();
        state.skill_rt.workspace_root = workspace.path.clone();
        state.skill_rt.default_locator_search_dir = fixture_dir.clone();
        let out = super::try_resolve_implicit_locator_path(
            &state,
            "README.md",
            "README.md",
            crate::OutputLocatorKind::Filename,
            Some(fixture_dir.to_string_lossy().as_ref()),
        );
        match out {
            Some(super::LocatorAutoResolution::Direct(path)) => {
                assert!(
                    path.ends_with("/README.md"),
                    "unexpected direct path: {path}"
                );
                assert!(
                    !path.contains("/fixtures/device_local/README.md"),
                    "context root unexpectedly won: {path}"
                );
            }
            other => panic!("expected direct workspace README match, got {other:?}"),
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
    fn explicit_workspace_child_does_not_bind_filename_fragment_to_existing_file() {
        let root = TempDirGuard::new("explicit_workspace_child_missing_filename_fragment");
        fs::write(root.path.join("rustclaw"), "binary").expect("write rustclaw");
        let query = "把 definitely_missing_named_file_rustclaw_001.txt 发给我";
        let keywords = super::extract_locator_keywords(query);
        let filename_tokens = super::extract_filename_like_tokens(query);
        let out = super::resolve_explicit_workspace_child_target(
            &root.path,
            &root.path,
            &keywords,
            &filename_tokens,
        );
        assert!(
            out.is_none(),
            "unexpected explicit child resolution: {out:?}"
        );
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

    #[test]
    fn current_workspace_locator_prefers_direct_child_target_when_present() {
        let root = TempDirGuard::new("workspace_direct_child");
        let logs = root.path.join("logs");
        fs::create_dir_all(&logs).expect("create logs");
        fs::write(logs.join("act_plan.log"), "ok\n").expect("write act_plan");

        let out =
            super::resolve_current_workspace_target(&root.path, &["logs".to_string()], &[], 2, 100);

        match out {
            Some(super::LocatorAutoResolution::Direct(path)) => {
                assert_eq!(
                    path,
                    logs.canonicalize()
                        .expect("canonical logs")
                        .display()
                        .to_string()
                );
            }
            other => panic!("expected direct logs path, got {other:?}"),
        }
    }

    #[test]
    fn explicit_workspace_child_directory_beats_context_root() {
        let root = TempDirGuard::new("explicit_child_beats_context");
        let workspace_logs = root.path.join("logs");
        let fixture_logs = root.path.join("fixtures").join("logs");
        fs::create_dir_all(&workspace_logs).expect("create workspace logs");
        fs::create_dir_all(&fixture_logs).expect("create fixture logs");
        fs::write(workspace_logs.join("clawd.log"), "ok\n").expect("write workspace log");
        fs::write(fixture_logs.join("app.log"), "ok\n").expect("write fixture log");

        let out = super::resolve_explicit_workspace_child_target(
            &root.path,
            &root.path,
            &["logs".to_string()],
            &[],
        );

        match out {
            Some(super::LocatorAutoResolution::Direct(path)) => {
                assert_eq!(
                    path,
                    workspace_logs
                        .canonicalize()
                        .expect("canonical workspace logs")
                        .display()
                        .to_string()
                );
            }
            other => panic!("expected workspace logs path, got {other:?}"),
        }
    }

    #[test]
    fn contextual_locator_root_prefers_recent_directory_scope() {
        let root = TempDirGuard::new("context_root");
        let logs = root.path.join("logs");
        fs::create_dir_all(&logs).expect("create logs");
        fs::write(logs.join("act_plan.log"), "ok\n").expect("write act_plan");

        let out = super::resolve_contextual_locator_root(
            &root.path,
            &root.path,
            Some("### RECENT_EXECUTION_EVENTS\n- request=先列出 logs 目录下前 5 个文件名 result=act_plan.log"),
        );

        assert_eq!(
            out.map(|v| v.canonicalize().unwrap_or(v).display().to_string()),
            Some(
                logs.canonicalize()
                    .expect("canonical logs")
                    .display()
                    .to_string()
            )
        );
    }

    #[test]
    fn implicit_locator_uses_contextual_directory_root_for_followup_file() {
        let root = TempDirGuard::new("contextual_followup");
        let logs = root.path.join("logs");
        fs::create_dir_all(&logs).expect("create logs");
        fs::write(logs.join("act_plan.log"), "line1\nline2\n").expect("write act_plan");
        fs::write(root.path.join("README.md"), "hello").expect("write readme");

        let roots = vec![logs.clone(), root.path.clone()];
        let out = super::try_resolve_implicit_locator_path_in_roots(
            &roots,
            &["读取".to_string(), "act_plan.log".to_string()],
            &["act_plan.log".to_string()],
            3,
            128,
        );

        match out {
            Some(super::LocatorAutoResolution::Direct(path)) => {
                assert_eq!(
                    PathBuf::from(path)
                        .canonicalize()
                        .expect("canonical resolved follow-up"),
                    logs.join("act_plan.log")
                        .canonicalize()
                        .expect("canonical act_plan")
                );
            }
            other => panic!("expected direct follow-up file resolution, got {other:?}"),
        }
    }

    #[test]
    fn implicit_filename_locator_finds_nested_target_before_deep_noise_exhausts_budget() {
        let root = TempDirGuard::new("filename_nested_target");
        let alpha = root.path.join("alpha");
        let scripts = root.path.join("scripts");
        fs::create_dir_all(&alpha).expect("create alpha");
        fs::create_dir_all(&scripts).expect("create scripts");
        for idx in 0..8 {
            fs::write(alpha.join(format!("noise_{idx}.txt")), "x\n").expect("write noise");
        }
        let target = scripts.join("restart_clawd_latest.sh");
        fs::write(&target, "#!/bin/sh\necho ok\n").expect("write target");

        let out = super::try_resolve_implicit_locator_path_in_roots(
            &[root.path.clone()],
            &["restart_clawd_latest.sh".to_string()],
            &["restart_clawd_latest.sh".to_string()],
            2,
            10,
        );

        match out {
            Some(super::LocatorAutoResolution::Direct(path)) => {
                assert_eq!(
                    PathBuf::from(path)
                        .canonicalize()
                        .expect("canonical resolved target")
                        .display()
                        .to_string(),
                    target
                        .canonicalize()
                        .expect("canonical target")
                        .display()
                        .to_string()
                );
            }
            other => panic!("expected direct nested filename match, got {other:?}"),
        }
    }
}
