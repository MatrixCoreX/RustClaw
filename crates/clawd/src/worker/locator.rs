use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::AppState;

pub(crate) fn has_concrete_locator_hint(text: &str) -> bool {
    if has_explicit_path_or_url_locator(text) {
        return true;
    }
    text.split_whitespace()
        .flat_map(locator_token_segments)
        .any(|token| looks_like_filename_locator(&token))
}

pub(crate) fn has_explicit_path_or_url_locator_hint(text: &str) -> bool {
    has_explicit_path_or_url_locator(text)
}

#[cfg(test)]
pub(crate) fn has_multiple_explicit_local_path_locators(text: &str) -> bool {
    unique_explicit_local_path_token_count(&extract_explicit_path_like_tokens(text)) >= 2
}

pub(crate) fn has_multiple_distinct_explicit_local_path_locators(
    state: &AppState,
    text: &str,
    context_hint: Option<&str>,
) -> bool {
    explicit_tokens_resolve_to_multiple_distinct_local_paths(
        &state.skill_rt.workspace_root,
        &state.skill_rt.default_locator_search_dir,
        &extract_explicit_locator_tokens_for_distinct_resolution(text),
        context_hint,
        state.skill_rt.locator_scan_max_depth,
        state.skill_rt.locator_scan_max_files,
    )
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
    let explicit_tokens = extract_explicit_path_like_tokens(&query_text);
    if explicit_tokens_resolve_to_multiple_distinct_local_paths(
        &state.skill_rt.workspace_root,
        &state.skill_rt.default_locator_search_dir,
        &extract_explicit_locator_tokens_for_distinct_resolution(&query_text),
        context_hint,
        state.skill_rt.locator_scan_max_depth,
        state.skill_rt.locator_scan_max_files,
    ) {
        return None;
    }
    if let Some(explicit_path) = resolve_context_aware_explicit_locator_path_from_text(
        &state.skill_rt.workspace_root,
        &state.skill_rt.default_locator_search_dir,
        &explicit_tokens,
        context_hint,
    ) {
        return Some(LocatorAutoResolution::Direct(explicit_path));
    }
    let relative_file_tokens = relative_explicit_file_tokens(&explicit_tokens);
    if !relative_file_tokens.is_empty() {
        let roots = context_relative_locator_roots(
            &state.skill_rt.workspace_root,
            &state.skill_rt.default_locator_search_dir,
            context_hint,
        );
        if let Some(resolved) = try_resolve_relative_explicit_suffix_tokens_in_roots(
            &roots,
            &relative_file_tokens,
            state.skill_rt.locator_scan_max_depth,
            state.skill_rt.locator_scan_max_files,
        ) {
            return Some(resolved);
        }
        return None;
    }
    if has_unresolved_explicit_local_locator_tokens(&explicit_tokens) {
        return None;
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
    if let Some(resolved) = try_resolve_implicit_locator_path_in_roots(
        &roots,
        &keywords,
        &filename_tokens,
        state.skill_rt.locator_scan_max_depth,
        state.skill_rt.locator_scan_max_files,
    ) {
        return Some(resolved);
    }
    try_resolve_deep_database_filename_token_in_roots(
        &roots,
        &filename_tokens,
        state.skill_rt.locator_scan_max_depth,
        state.skill_rt.locator_scan_max_files,
    )
}

pub(crate) fn try_resolve_workspace_child_locator_from_text(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    text: &str,
) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || has_explicit_path_or_url_locator(trimmed) {
        return None;
    }
    let keywords = extract_locator_keywords(trimmed);
    if keywords.is_empty() {
        return None;
    }
    let filename_tokens = extract_filename_like_tokens(trimmed);
    let resolved = resolve_explicit_workspace_child_target(
        workspace_root,
        default_locator_search_dir,
        &keywords,
        &filename_tokens,
    )?;
    let LocatorAutoResolution::Direct(path) = resolved else {
        return None;
    };
    let path_buf = PathBuf::from(&path);
    if normalize_workspace_child_candidate(&path_buf)
        == normalize_workspace_child_candidate(workspace_root)
    {
        return None;
    }
    if is_hidden_vcs_control_path(&path_buf) {
        return None;
    }
    Some(path)
}

fn normalize_workspace_child_candidate(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn is_hidden_vcs_control_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| matches!(name, ".git" | ".hg" | ".svn"))
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

fn try_resolve_deep_database_filename_token_in_roots(
    roots: &[PathBuf],
    filename_tokens: &[String],
    max_depth: usize,
    max_files: usize,
) -> Option<LocatorAutoResolution> {
    let database_tokens = filename_tokens
        .iter()
        .filter(|token| looks_like_database_filename_token(token))
        .collect::<Vec<_>>();
    if database_tokens.len() != 1 {
        return None;
    }
    let token = database_tokens[0];
    let scan_depth = max_depth.max(6).min(16);
    let entry_budget = deep_database_filename_scan_entry_budget(max_files);
    let mut matches = Vec::new();
    for root in roots {
        for rendered in
            collect_deep_database_filename_matches(root, token, scan_depth, entry_budget)
        {
            if !matches.iter().any(|existing| existing == &rendered) {
                matches.push(rendered);
            }
        }
    }
    matches.sort();
    matches.dedup();
    if matches.len() == 1 {
        return Some(LocatorAutoResolution::Direct(
            matches.into_iter().next().unwrap_or_default(),
        ));
    }
    if matches.len() > 1 {
        return Some(LocatorAutoResolution::Fuzzy(
            matches.into_iter().take(3).collect(),
        ));
    }
    None
}

fn looks_like_database_filename_token(token: &str) -> bool {
    if !looks_like_filename_locator(token) {
        return false;
    }
    let lowered = token.to_ascii_lowercase();
    lowered.ends_with(".db") || lowered.ends_with(".sqlite") || lowered.ends_with(".sqlite3")
}

fn deep_database_filename_scan_entry_budget(max_files: usize) -> usize {
    max_files.max(5_000).saturating_mul(4).min(50_000)
}

fn collect_deep_database_filename_matches(
    root: &Path,
    token: &str,
    max_depth: usize,
    max_entries: usize,
) -> Vec<String> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen_entries = 0usize;
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let mut entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect::<Vec<_>>(),
            Err(_) => continue,
        };
        entries.sort();
        let mut child_dirs = Vec::new();
        for path in entries {
            seen_entries += 1;
            if seen_entries > max_entries.max(1) {
                return out;
            }
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let file_type = meta.file_type();
            if file_type.is_dir() {
                if depth < max_depth && !skip_deep_database_filename_scan_dir(&path) {
                    child_dirs.push((path, depth + 1));
                }
                continue;
            }
            if !(file_type.is_file() || (file_type.is_symlink() && path.is_file())) {
                continue;
            }
            if !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(token))
            {
                continue;
            }
            let canonical = path.canonicalize().unwrap_or(path);
            let rendered = canonical.display().to_string();
            if !out.iter().any(|existing| existing == &rendered) {
                out.push(rendered);
            }
        }
        for child in child_dirs {
            queue.push_back(child);
        }
    }
    out
}

fn skip_deep_database_filename_scan_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(name, ".git" | "node_modules" | "target")
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
        return directory_contains_filename_token(direct_path, filename_tokens);
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

fn directory_contains_filename_token(dir: &Path, filename_tokens: &[String]) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let Some(file_name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            return false;
        };
        filename_tokens
            .iter()
            .any(|token| file_name.eq_ignore_ascii_case(token))
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
        .trim_end_matches('.')
        .to_string()
}

fn locator_token_segments(token: &str) -> Vec<String> {
    token
        .split(|ch: char| matches!(ch, ',' | '，' | '。' | ';' | '；' | '、' | ':' | '：'))
        .map(trim_locator_token)
        .filter(|part| !part.is_empty())
        .collect()
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
    if looks_like_protocol_field_selector_path(token) {
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

fn looks_like_protocol_field_selector_path(token: &str) -> bool {
    let trimmed = token.trim();
    if !trimmed.contains('/') || trimmed.contains('\\') {
        return false;
    }
    if trimmed.starts_with('/')
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || trimmed.starts_with("~/")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.contains("://")
    {
        return false;
    }
    let mut count = 0usize;
    for segment in trimmed.split('/') {
        let segment = trim_locator_token(segment);
        if segment.is_empty() || !protocol_field_selector_segment(&segment) {
            return false;
        }
        count += 1;
    }
    count >= 2
}

fn protocol_field_selector_segment(segment: &str) -> bool {
    let canonical = segment
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<String>();
    matches!(
        canonical.as_str(),
        "args"
            | "checkpointid"
            | "context"
            | "decision"
            | "errorcode"
            | "errortext"
            | "extra"
            | "issuecode"
            | "issuecodes"
            | "messagekey"
            | "reasoncode"
            | "repairenvelope"
            | "repairsignal"
            | "requestid"
            | "status"
            | "statuscode"
            | "text"
    )
}

fn extract_explicit_path_like_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split_whitespace() {
        let token = trim_locator_token(raw);
        push_explicit_path_token_candidate(&token, &mut out);
        if let Some((_, suffix)) = token.rsplit_once('=') {
            push_explicit_path_token_candidate(&trim_locator_token(suffix), &mut out);
        }
        if let Some((_, suffix)) = token.rsplit_once(':') {
            push_explicit_path_token_candidate(&trim_locator_token(suffix), &mut out);
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn extract_explicit_locator_tokens_for_distinct_resolution(text: &str) -> Vec<String> {
    let mut out = extract_explicit_path_like_tokens(text);
    for token in extract_filename_like_tokens(text) {
        if out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&token))
        {
            continue;
        }
        out.push(token);
        if out.len() >= 12 {
            break;
        }
    }
    for token in extract_bare_workspace_child_locator_candidate_tokens(text) {
        if out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&token))
        {
            continue;
        }
        out.push(token);
        if out.len() >= 24 {
            break;
        }
    }
    out
}

fn extract_bare_workspace_child_locator_candidate_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split_whitespace() {
        for token in locator_token_segments(raw) {
            if !looks_like_bare_workspace_child_locator_token(&token)
                || out
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&token))
            {
                continue;
            }
            out.push(token);
            if out.len() >= 16 {
                return out;
            }
        }
    }
    out
}

fn push_explicit_path_token_candidate(token: &str, out: &mut Vec<String>) {
    if looks_like_explicit_path_or_url_token(token) && !out.iter().any(|v| v == token) {
        out.push(token.to_string());
    }
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

fn is_relative_explicit_locator_token(token: &str) -> bool {
    looks_like_explicit_path_or_url_token(token)
        && !token.starts_with('/')
        && !token.starts_with("~/")
        && !token.starts_with("http://")
        && !token.starts_with("https://")
        && !token.contains(":\\")
}

fn has_unresolved_explicit_local_locator_tokens(explicit_tokens: &[String]) -> bool {
    explicit_tokens
        .iter()
        .any(|token| is_local_explicit_locator_token(token))
}

#[cfg(test)]
fn unique_explicit_local_path_token_count(explicit_tokens: &[String]) -> usize {
    let mut unique = Vec::new();
    for token in explicit_tokens {
        if !is_local_explicit_locator_token(token) {
            continue;
        }
        if unique
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(token))
        {
            continue;
        }
        unique.push(token.clone());
        if unique.len() >= 2 {
            return unique.len();
        }
    }
    unique.len()
}

fn explicit_tokens_resolve_to_multiple_distinct_local_paths(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    explicit_tokens: &[String],
    context_hint: Option<&str>,
    max_depth: usize,
    max_files: usize,
) -> bool {
    let mut unique = Vec::new();
    for token in explicit_tokens {
        if !is_local_locator_token_for_distinct_resolution(token) {
            continue;
        }
        let paths = resolve_context_aware_local_locator_token_candidates_for_distinct_resolution(
            workspace_root,
            default_locator_search_dir,
            token,
            context_hint,
            max_depth,
            max_files,
        );
        for path in paths {
            if unique.iter().any(|existing: &String| existing == &path) {
                continue;
            }
            unique.push(path);
            if unique.len() >= 2 {
                return true;
            }
        }
    }
    false
}

fn is_local_locator_token_for_distinct_resolution(token: &str) -> bool {
    is_local_explicit_locator_token(token)
        || looks_like_filename_locator(token)
        || looks_like_bare_workspace_child_locator_token(token)
}

fn is_local_explicit_locator_token(token: &str) -> bool {
    looks_like_explicit_path_or_url_token(token)
        && !token.starts_with("http://")
        && !token.starts_with("https://")
}

fn context_relative_locator_roots(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    context_hint: Option<&str>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let workspace_root = normalize_implicit_locator_root(workspace_root);
    if let Some(context_root) = resolve_contextual_locator_root(
        workspace_root.as_deref().unwrap_or(Path::new("/")),
        default_locator_search_dir,
        context_hint,
    )
    .and_then(|path| normalize_implicit_locator_root(&path))
    {
        let mut current = Some(context_root);
        for _ in 0..8 {
            let Some(root) = current.take() else {
                break;
            };
            if let Some(workspace_root) = workspace_root.as_ref() {
                if !root.starts_with(workspace_root) {
                    break;
                }
            }
            append_implicit_root_if_missing(&mut roots, root.clone());
            if workspace_root
                .as_ref()
                .is_some_and(|workspace| &root == workspace)
            {
                break;
            }
            current = root.parent().map(Path::to_path_buf);
        }
    }
    append_implicit_root_if_missing(&mut roots, default_locator_search_dir.to_path_buf());
    if workspace_root
        .as_ref()
        .is_none_or(|workspace| workspace != default_locator_search_dir)
    {
        append_implicit_root_if_missing(
            &mut roots,
            workspace_root.unwrap_or_else(|| {
                default_locator_search_dir
                    .canonicalize()
                    .unwrap_or_else(|_| default_locator_search_dir.to_path_buf())
            }),
        );
    }
    roots
}

fn resolve_relative_explicit_locator_path_token_in_roots(
    token: &str,
    roots: &[PathBuf],
) -> Option<String> {
    for root in roots {
        let candidate = root.join(token);
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

fn resolve_context_aware_explicit_locator_path_from_text(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    explicit_tokens: &[String],
    context_hint: Option<&str>,
) -> Option<String> {
    for token in explicit_tokens {
        if let Some(path) = resolve_context_aware_explicit_locator_path_token(
            workspace_root,
            default_locator_search_dir,
            token,
            context_hint,
        ) {
            return Some(path);
        }
    }
    None
}

fn resolve_context_aware_explicit_locator_path_token(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    token: &str,
    context_hint: Option<&str>,
) -> Option<String> {
    if is_relative_explicit_locator_token(token) {
        let roots = context_relative_locator_roots(
            workspace_root,
            default_locator_search_dir,
            context_hint,
        );
        if let Some(path) = resolve_relative_explicit_locator_path_token_in_roots(token, &roots) {
            return Some(path);
        }
    }
    resolve_explicit_locator_path_token(workspace_root, default_locator_search_dir, token)
}

fn resolve_context_aware_local_locator_token_candidates_for_distinct_resolution(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    token: &str,
    context_hint: Option<&str>,
    max_depth: usize,
    max_files: usize,
) -> Vec<String> {
    if is_local_explicit_locator_token(token) {
        return resolve_context_aware_explicit_locator_path_token(
            workspace_root,
            default_locator_search_dir,
            token,
            context_hint,
        )
        .into_iter()
        .collect();
    }
    if !looks_like_filename_locator(token) {
        if looks_like_bare_workspace_child_locator_token(token) {
            return resolve_context_aware_bare_workspace_child_locator_candidates(
                workspace_root,
                default_locator_search_dir,
                token,
                context_hint,
            );
        }
        return Vec::new();
    }
    let roots =
        context_relative_locator_roots(workspace_root, default_locator_search_dir, context_hint);
    let filename_tokens = vec![token.to_string()];
    match try_resolve_implicit_locator_path_in_roots(
        &roots,
        &[],
        &filename_tokens,
        max_depth,
        max_files,
    ) {
        Some(LocatorAutoResolution::Direct(path)) => vec![path],
        Some(LocatorAutoResolution::Fuzzy(candidates)) => candidates,
        None => Vec::new(),
    }
}

fn resolve_context_aware_bare_workspace_child_locator_candidates(
    workspace_root: &Path,
    default_locator_search_dir: &Path,
    token: &str,
    context_hint: Option<&str>,
) -> Vec<String> {
    let roots =
        context_relative_locator_roots(workspace_root, default_locator_search_dir, context_hint);
    let mut out = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let direct = root.join(token);
        let resolved = if direct.is_file() || direct.is_dir() {
            Some(direct.canonicalize().unwrap_or(direct))
        } else {
            crate::delivery_utils::resolve_existing_path_under_root_case_insensitive(&root, token)
        };
        let Some(path) = resolved else {
            continue;
        };
        let rendered = path.display().to_string();
        if !out.iter().any(|existing| existing == &rendered) {
            out.push(rendered);
        }
    }
    out
}

fn relative_explicit_file_tokens(explicit_tokens: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for token in explicit_tokens {
        if !is_relative_explicit_locator_token(token) {
            continue;
        }
        let Some(file_name) = Path::new(token).file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        if !looks_like_filename_locator(file_name) {
            continue;
        }
        if !out.iter().any(|v| v == token) {
            out.push(token.clone());
        }
    }
    out
}

fn normalized_path_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect()
}

fn normalized_relative_token_components(token: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for component in Path::new(token).components() {
        match component {
            std::path::Component::Normal(value) => out.push(value.to_string_lossy().to_lowercase()),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.is_empty()).then_some(out)
}

fn path_ends_with_relative_token(path: &Path, token: &str) -> bool {
    let Some(suffix) = normalized_relative_token_components(token) else {
        return false;
    };
    let full = normalized_path_components(path);
    full.len() >= suffix.len() && full[full.len() - suffix.len()..] == suffix
}

fn relative_suffix_scan_depth(max_depth: usize, tokens: &[String]) -> usize {
    let extra = tokens
        .iter()
        .filter_map(|token| normalized_relative_token_components(token))
        .map(|components| components.len())
        .max()
        .unwrap_or(0)
        .saturating_add(4);
    max_depth.saturating_add(extra).min(24)
}

fn skip_relative_suffix_scan_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(name, ".git" | "node_modules" | "target")
}

fn collect_relative_suffix_candidate_matches(
    root: &Path,
    token: &str,
    max_depth: usize,
    max_dirs: usize,
) -> Vec<String> {
    if !root.is_dir() || normalized_relative_token_components(token).is_none() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen_dirs = 0usize;
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        seen_dirs += 1;
        if seen_dirs > max_dirs.max(1) {
            break;
        }
        let candidate = dir.join(token);
        if candidate.is_file() && path_ends_with_relative_token(&candidate, token) {
            let canonical = candidate.canonicalize().unwrap_or(candidate);
            let rendered = canonical.display().to_string();
            if !out.iter().any(|value| value == &rendered) {
                out.push(rendered);
            }
        }
        if depth >= max_depth {
            continue;
        }
        let mut entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir() && !skip_relative_suffix_scan_dir(path))
                .collect::<Vec<_>>(),
            Err(_) => continue,
        };
        entries.sort();
        for child in entries {
            queue.push_back((child, depth + 1));
        }
    }
    out
}

fn try_resolve_relative_explicit_suffix_tokens_in_roots(
    roots: &[PathBuf],
    tokens: &[String],
    max_depth: usize,
    max_files: usize,
) -> Option<LocatorAutoResolution> {
    let scan_depth = relative_suffix_scan_depth(max_depth, tokens);
    let max_dirs = max_files.max(1).saturating_mul(4);
    for token in tokens {
        let mut matches = Vec::new();
        for root in roots {
            for rendered in
                collect_relative_suffix_candidate_matches(root, token, scan_depth, max_dirs)
            {
                if !matches.iter().any(|value| value == &rendered) {
                    matches.push(rendered);
                }
            }
        }
        matches.sort();
        matches.dedup();
        if matches.len() == 1 {
            return Some(LocatorAutoResolution::Direct(
                matches.into_iter().next().unwrap_or_default(),
            ));
        }
        if matches.len() > 1 {
            return Some(LocatorAutoResolution::Fuzzy(
                matches.into_iter().take(3).collect(),
            ));
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
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
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

fn looks_like_bare_workspace_child_locator_token(token: &str) -> bool {
    let token = token.trim();
    token.len() >= 2
        && token.len() <= 80
        && !token.contains('/')
        && !token.contains('\\')
        && !token.contains('.')
        && !token.starts_with("http://")
        && !token.starts_with("https://")
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
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

    let mut exact_matches = Vec::new();
    let mut stem_matches = Vec::new();
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
            exact_matches.push(canonical.display().to_string());
            continue;
        }
        if path.is_file() {
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let normalized_stem = normalize_locator_match_text(stem);
            if !normalized_stem.is_empty()
                && normalized_keywords
                    .iter()
                    .any(|kw| !kw.contains('.') && kw == &normalized_stem)
            {
                let canonical = path.canonicalize().unwrap_or(path);
                stem_matches.push(canonical.display().to_string());
            }
        }
    }
    exact_matches.sort();
    exact_matches.dedup();
    if exact_matches.len() == 1 {
        return exact_matches.into_iter().next();
    }
    if !exact_matches.is_empty() {
        return None;
    }
    stem_matches.sort();
    stem_matches.dedup();
    if stem_matches.len() > 1 {
        if let Some(preferred) =
            prefer_canonical_readme_markdown_match(&stem_matches, &normalized_keywords)
        {
            return Some(preferred);
        }
    }
    (stem_matches.len() == 1).then(|| stem_matches.into_iter().next().unwrap_or_default())
}

fn prefer_canonical_readme_markdown_match(
    matches: &[String],
    normalized_keywords: &[String],
) -> Option<String> {
    if !normalized_keywords.iter().any(|kw| kw == "readme") {
        return None;
    }
    let mut candidates = matches
        .iter()
        .filter(|raw| {
            Path::new(raw)
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("README.md"))
        })
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    (candidates.len() == 1).then(|| candidates.into_iter().next().unwrap_or_default())
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
        for token in locator_token_segments(raw) {
            if looks_like_filename_locator(&token) && !out.iter().any(|v| v == &token) {
                out.push(token);
            }
            if out.len() >= 8 {
                break;
            }
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
#[path = "locator_tests.rs"]
mod tests;
