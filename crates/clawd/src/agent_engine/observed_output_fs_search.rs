use super::*;

pub(super) fn fs_search_find_name_results(
    value: &serde_json::Value,
) -> Option<(Vec<String>, usize, Option<String>)> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if !action.eq_ignore_ascii_case("find_name") {
        return None;
    }
    let results = value
        .get("results")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let count = value
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(results.len() as u64) as usize;
    let pattern = value
        .get("pattern")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);
    Some((results, count, pattern))
}

pub(super) fn fs_search_find_ext_results(
    value: &serde_json::Value,
) -> Option<(Vec<String>, usize, String)> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if !action.eq_ignore_ascii_case("find_ext") {
        return None;
    }
    let results = value
        .get("results")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let count = value
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(results.len() as u64) as usize;
    let ext = value
        .get("ext")
        .or_else(|| value.get("extension"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();
    Some((results, count, ext))
}

fn fs_search_grep_text_results(
    value: &serde_json::Value,
) -> Option<(Vec<(String, u64, String)>, usize, String)> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if !action.eq_ignore_ascii_case("grep_text") {
        return None;
    }
    let query = value
        .get("query")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();
    let matches = value
        .get("matches")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let obj = item.as_object()?;
                    let path = obj
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|v| !v.is_empty())?
                        .to_string();
                    let line = obj.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
                    let text = obj
                        .get("text")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|v| !v.is_empty())?
                        .to_string();
                    Some((path, line, text))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let match_count = value
        .get("match_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(matches.len() as u64) as usize;
    Some((matches, match_count, query))
}

fn fs_search_grep_text_name_results(value: &serde_json::Value) -> Option<(Vec<String>, usize)> {
    let action = value.get("action").and_then(|v| v.as_str())?;
    if !action.eq_ignore_ascii_case("grep_text") {
        return None;
    }
    let results = value
        .get("name_results")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let count = value
        .get("name_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(results.len() as u64) as usize;
    Some((results, count))
}

pub(super) fn fs_search_grep_text_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let (matches, match_count, query) = fs_search_grep_text_results(value)?;
    let file_count = value
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();
    let patterns = value
        .get("patterns")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let (name_results, name_count) =
        fs_search_grep_text_name_results(value).unwrap_or((Vec::new(), 0));
    let mut lines = vec![format!(
        "grep_text query={query} file_count={file_count} match_count={match_count}"
    )];
    if !patterns.is_empty() {
        lines.push(format!("file_patterns={}", patterns.join(", ")));
    }
    if name_count > 0 && !name_results.is_empty() {
        lines.push(format!("name_count={name_count}"));
        lines.extend(
            name_results
                .into_iter()
                .take(16)
                .map(|path| format!("name_match path={path}")),
        );
    }
    if matches.is_empty() {
        lines.push("matches: none".to_string());
    } else {
        lines.extend(
            matches
                .into_iter()
                .take(16)
                .map(|(path, line, text)| format!("match path={path} line={line} text={text}")),
        );
    }
    Some(lines.join("\n"))
}

pub(super) fn fs_search_find_name_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let (results, count, pattern) = fs_search_find_name_results(value)?;
    let mut lines = vec![format!("find_name count={count}")];
    if let Some(root) = value
        .get("root")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        lines.push(format!("root={root}"));
    }
    if let Some(pattern) = pattern.as_deref().and_then(|value| {
        normalized_find_name_pattern(Some(value)).filter(|value| !value.trim().is_empty())
    }) {
        lines.push(format!("pattern={pattern}"));
    }
    if results.is_empty() {
        lines.push("matches: none".to_string());
    } else {
        lines.extend(
            results
                .into_iter()
                .take(64)
                .enumerate()
                .map(|(idx, path)| format!("result.{}.path={path}", idx + 1)),
        );
    }
    Some(lines.join("\n"))
}

fn path_matches_find_name_pattern(path: &str, pattern: &str) -> bool {
    let path = Path::new(path);
    let Some(file_name) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    if file_name.eq_ignore_ascii_case(pattern) {
        return true;
    }
    if pattern.contains('.') {
        return false;
    }
    path.file_stem()
        .and_then(|v| v.to_str())
        .map(|stem| stem.eq_ignore_ascii_case(pattern))
        .unwrap_or(false)
}

fn is_direct_child_relative_match(path: &str) -> bool {
    let path = Path::new(path);
    match path.parent().and_then(|parent| parent.to_str()) {
        None => true,
        Some("") | Some(".") => true,
        Some(_) => false,
    }
}

pub(super) fn preferred_fs_search_exact_match(results: &[String], pattern: &str) -> Option<String> {
    let mut exact_matches = results
        .iter()
        .filter(|path| path_matches_find_name_pattern(path, pattern))
        .cloned()
        .collect::<Vec<_>>();
    exact_matches.sort();
    exact_matches.dedup();
    let mut direct_child_matches = exact_matches
        .iter()
        .filter(|path| is_direct_child_relative_match(path))
        .cloned()
        .collect::<Vec<_>>();
    direct_child_matches.sort();
    direct_child_matches.dedup();
    if direct_child_matches.len() == 1 {
        return direct_child_matches.into_iter().next();
    }
    (exact_matches.len() == 1).then(|| exact_matches.into_iter().next().unwrap_or_default())
}

fn rank_fs_search_candidates(results: &[String], pattern: &str) -> Vec<String> {
    let pattern_norm = pattern.trim().to_lowercase();
    let mut ranked = results
        .iter()
        .cloned()
        .map(|path| {
            let path_buf = Path::new(&path);
            let file_name = path_buf
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or_default()
                .to_string();
            let file_name_norm = file_name.to_lowercase();
            let stem_norm = path_buf
                .file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or_default()
                .to_lowercase();
            let score = if stem_norm == pattern_norm {
                500
            } else if stem_norm.starts_with(&pattern_norm) {
                400
            } else if stem_norm.contains(&pattern_norm) {
                300
            } else if file_name_norm.starts_with(&pattern_norm) {
                200
            } else if file_name_norm.contains(&pattern_norm) {
                100
            } else {
                0
            };
            (score, file_name.len(), path)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    ranked.dedup_by(|a, b| a.2 == b.2);
    ranked
        .into_iter()
        .take(3)
        .map(|(_, _, path)| path)
        .collect()
}

pub(super) fn normalized_find_name_pattern(pattern: Option<&str>) -> Option<String> {
    let pattern = pattern?.trim();
    if pattern.is_empty() {
        return None;
    }
    let path = Path::new(pattern);
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some(pattern.to_string()))
}

pub(super) fn fs_search_scalar_candidate(
    _state: Option<&AppState>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
    _prefer_english: bool,
) -> Option<String> {
    let (mut results, count, pattern) = fs_search_find_name_results(value)?;
    if count == 0 || results.is_empty() {
        return Some(fs_search_no_match_machine_answer(
            "find_name",
            pattern.as_deref().map(|pattern| ("pattern", pattern)),
        ));
    }
    if results.len() > 1 {
        if let Some(locator_ext) = locator_hint.and_then(path_extension_hint) {
            let filtered = results
                .iter()
                .filter(|path| path_has_extension(path, &locator_ext))
                .cloned()
                .collect::<Vec<_>>();
            if !filtered.is_empty() {
                results = filtered;
            }
        }
    }
    if results.len() == 1 {
        let root = value
            .get("root")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|root| !root.is_empty());
        if prefer_full_path {
            let resolved_path = Path::new(&results[0])
                .is_absolute()
                .then(|| canonical_existing_path(Path::new(&results[0])))
                .or_else(|| {
                    root.and_then(|root| {
                        let candidate = Path::new(root).join(&results[0]);
                        candidate
                            .exists()
                            .then(|| canonical_existing_path(&candidate))
                    })
                })
                .or_else(|| resolve_listing_entry_full_path(&results[0], auto_locator_path))
                .unwrap_or_else(|| results[0].clone());
            return Some(resolved_path);
        }
        return Some(results[0].clone());
    }
    let pattern = normalized_find_name_pattern(pattern.as_deref())
        .or_else(|| normalized_find_name_pattern(locator_hint))?;
    let preferred = preferred_fs_search_exact_match(&results, &pattern)?;
    if !prefer_full_path {
        return Some(preferred);
    }
    let root = value
        .get("root")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|root| !root.is_empty());
    Path::new(&preferred)
        .is_absolute()
        .then(|| canonical_existing_path(Path::new(&preferred)))
        .or_else(|| {
            root.and_then(|root| {
                let candidate = Path::new(root).join(&preferred);
                candidate
                    .exists()
                    .then(|| canonical_existing_path(&candidate))
            })
        })
        .or_else(|| resolve_listing_entry_full_path(&preferred, auto_locator_path))
        .or_else(|| Some(preferred))
}

fn path_extension_hint(path: &str) -> Option<String> {
    Path::new(path.trim())
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
}

fn path_has_extension(path: &str, expected_ext: &str) -> bool {
    Path::new(path.trim())
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| ext == expected_ext)
}

pub(super) fn fs_search_direct_answer_candidate(
    state: Option<&AppState>,
    value: &serde_json::Value,
    locator_hint: Option<&str>,
    prefer_english: bool,
    allow_multi_result_list: bool,
    prefer_path_only: bool,
) -> Option<String> {
    if let Some(answer) = fs_search_grep_text_direct_answer_candidate(
        state,
        value,
        prefer_english,
        allow_multi_result_list,
        prefer_path_only,
    ) {
        return Some(answer);
    }
    if let Some((results, count, ext)) = fs_search_find_ext_results(value) {
        if count == 0 || results.is_empty() {
            return Some(fs_search_no_match_machine_answer(
                "find_ext",
                Some(("ext", ext.as_str())),
            ));
        }
        return Some(results.join("\n"));
    }
    let (results, count, pattern) = fs_search_find_name_results(value)?;
    if count == 0 || results.is_empty() {
        return Some(fs_search_no_match_machine_answer(
            "find_name",
            pattern.as_deref().map(|pattern| ("pattern", pattern)),
        ));
    }
    if results.len() == 1 {
        if prefer_path_only {
            return Some(results[0].clone());
        }
        return Some(path_fact_machine_answer(
            Some(&results[0]),
            true,
            None,
            None,
            Some("fs_search.find_name"),
        ));
    }
    if let Some(pattern) = normalized_find_name_pattern(pattern.as_deref())
        .or_else(|| normalized_find_name_pattern(locator_hint))
    {
        let ranked = rank_fs_search_candidates(&results, &pattern);
        if allow_multi_result_list && prefer_path_only && !ranked.is_empty() {
            return Some(ranked.join("\n"));
        }
        if let Some(preferred) = preferred_fs_search_exact_match(&results, &pattern) {
            if prefer_path_only {
                return Some(preferred);
            }
            return Some(path_fact_machine_answer(
                Some(&preferred),
                true,
                None,
                None,
                Some("fs_search.find_name"),
            ));
        }
        if !ranked.is_empty() {
            return allow_multi_result_list.then(|| ranked.join("\n"));
        }
    }
    let matches = results.into_iter().take(3).collect::<Vec<_>>().join("\n");
    allow_multi_result_list.then_some(matches)
}

fn fs_search_no_match_machine_answer(action: &str, selector: Option<(&str, &str)>) -> String {
    let mut lines = vec![
        "message_key=clawd.msg.fs_search.observed".to_string(),
        "reason_code=fs_search_no_match".to_string(),
        format!("action={action}"),
        "matched=false".to_string(),
    ];
    if let Some((key, value)) = selector {
        push_fs_search_machine_line(&mut lines, key, value);
    }
    lines.join("\n")
}

fn push_fs_search_machine_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let value = crate::truncate_for_agent_trace(
        &crate::visible_text::sanitize_user_visible_text(value).replace('\n', " "),
    );
    lines.push(format!("{key}={value}"));
}

fn fs_search_grep_text_direct_answer_candidate(
    _state: Option<&AppState>,
    value: &serde_json::Value,
    _prefer_english: bool,
    allow_multi_result_list: bool,
    prefer_path_only: bool,
) -> Option<String> {
    let (matches, match_count, _query) = fs_search_grep_text_results(value)?;
    if match_count == 0 || matches.is_empty() {
        if let Some((name_results, name_count)) = fs_search_grep_text_name_results(value) {
            if name_count > 0 && !name_results.is_empty() {
                if name_results.len() == 1 {
                    return name_results.into_iter().next();
                }
                return allow_multi_result_list.then(|| {
                    name_results
                        .into_iter()
                        .take(16)
                        .collect::<Vec<_>>()
                        .join("\n")
                });
            }
        }
        return Some(fs_search_no_match_machine_answer("grep_text", None));
    }
    if allow_multi_result_list && !prefer_path_only {
        return Some(
            matches
                .into_iter()
                .take(16)
                .map(|(_, line, text)| {
                    if line > 0 {
                        format!("{line}: {text}")
                    } else {
                        text
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    let mut paths = Vec::new();
    for (path, _, _) in matches {
        if !paths.iter().any(|seen| seen == &path) {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return None;
    }
    if paths.len() == 1 {
        return paths.into_iter().next();
    }
    allow_multi_result_list.then(|| paths.into_iter().take(3).collect::<Vec<_>>().join("\n"))
}

pub(super) fn fs_search_content_presence_direct_answer_candidate(
    _state: Option<&AppState>,
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
    _prefer_english: bool,
) -> Option<String> {
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::ContentPresenceCheck,
    ) {
        return None;
    }
    let (matches, match_count, query) = fs_search_grep_text_results(value)?;
    if match_count == 0 || matches.is_empty() {
        if let Some((name_results, name_count)) = fs_search_grep_text_name_results(value) {
            if name_count > 0 && !name_results.is_empty() {
                let path_text = name_results
                    .into_iter()
                    .take(16)
                    .collect::<Vec<_>>()
                    .join("\n");
                return Some(path_text);
            }
        }
        let path = value
            .get("root")
            .or_else(|| value.get("path"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(route.locator_hint.trim());
        return Some(fs_search_content_presence_machine_answer(
            &query,
            path,
            None,
            &[],
        ));
    }
    let mut paths = Vec::new();
    let mut first_match: Option<(String, u64, String)> = None;
    for (path, line, text) in matches {
        if first_match.is_none() {
            first_match = Some((path.clone(), line, text));
        }
        if !paths.iter().any(|seen| seen == &path) {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return None;
    }
    let path_list = paths.into_iter().take(8).collect::<Vec<_>>();
    if let Some((path, line, text)) = first_match {
        let location = if line > 0 {
            format!("{path}:{line}")
        } else {
            path
        };
        return Some(fs_search_content_presence_machine_answer(
            &query,
            "",
            Some((line, &location, &text)),
            &path_list,
        ));
    }
    Some(fs_search_content_presence_machine_answer(
        &query, "", None, &path_list,
    ))
}

fn fs_search_content_presence_machine_answer(
    query: &str,
    path: &str,
    match_info: Option<(u64, &str, &str)>,
    paths: &[String],
) -> String {
    let mut lines = vec![
        "message_key=clawd.msg.content_presence.observed".to_string(),
        "reason_code=content_presence_observed".to_string(),
        format!("contains={}", match_info.is_some() || !paths.is_empty()),
    ];
    push_content_presence_machine_line(&mut lines, "query", query);
    if !path.trim().is_empty() {
        push_content_presence_machine_line(&mut lines, "path", path);
    }
    if let Some((line, location, matched_text)) = match_info {
        lines.push(format!("line={line}"));
        push_content_presence_machine_line(&mut lines, "location", location);
        push_content_presence_machine_line(&mut lines, "matched_text", matched_text);
    }
    for (idx, path) in paths.iter().enumerate() {
        push_content_presence_machine_line(&mut lines, &format!("path.{}", idx + 1), path);
    }
    lines.join("\n")
}

fn push_content_presence_machine_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let value = crate::truncate_for_agent_trace(
        &crate::visible_text::sanitize_user_visible_text(value).replace('\n', " "),
    );
    lines.push(format!("{key}={value}"));
}

fn normalized_scope_text(value: &str) -> Option<String> {
    let normalized = value
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn locator_scope_candidates(locator_hint: &str) -> Vec<String> {
    let locator_hint = locator_hint.trim();
    if locator_hint.is_empty() {
        return Vec::new();
    }
    let path = Path::new(locator_hint);
    let scoped_path = if path.extension().is_some() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    let mut candidates = Vec::new();
    if let Some(scope) = normalized_scope_text(&scoped_path.to_string_lossy()) {
        candidates.push(scope);
    }
    if let Some(name) = scoped_path
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(normalized_scope_text)
    {
        candidates.push(name);
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn path_is_inside_locator_scope(path: &str, locator_hint: &str) -> bool {
    let Some(path) = normalized_scope_text(path) else {
        return false;
    };
    locator_scope_candidates(locator_hint)
        .into_iter()
        .any(|scope| {
            path == scope
                || path.starts_with(&format!("{scope}/"))
                || path.ends_with(&format!("/{scope}"))
                || path.contains(&format!("/{scope}/"))
        })
}

fn pathish_filter_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let push_token = |raw: &str, tokens: &mut Vec<String>| {
        let token = raw
            .trim_matches(|ch: char| ch == '.' || ch == '-' || ch == '_')
            .to_ascii_lowercase();
        if token.len() >= 2 && !tokens.iter().any(|seen| seen == &token) {
            tokens.push(token.clone());
        }
        for part in token.split(['.', '-', '_']) {
            let part = part.trim();
            if part.len() >= 3 && !tokens.iter().any(|seen| seen == part) {
                tokens.push(part.to_string());
            }
        }
    };
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
            current.push(ch);
        } else if !current.is_empty() {
            push_token(&current, &mut tokens);
            current.clear();
        }
    }
    if !current.is_empty() {
        push_token(&current, &mut tokens);
    }
    tokens
}

fn result_extensions(results: &[String]) -> Vec<String> {
    let mut exts = results
        .iter()
        .filter_map(|path| Path::new(path).extension().and_then(|ext| ext.to_str()))
        .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
        .collect::<Vec<_>>();
    exts.sort();
    exts.dedup();
    exts
}

fn structured_extension_hints(
    pattern: Option<&str>,
    locator_hint: &str,
    results: &[String],
) -> Vec<String> {
    let available_exts = result_extensions(results);
    if available_exts.is_empty() {
        return Vec::new();
    }
    let mut tokens = Vec::new();
    if let Some(pattern) = pattern {
        tokens.extend(pathish_filter_tokens(pattern));
    }
    tokens.extend(pathish_filter_tokens(locator_hint));
    tokens
        .into_iter()
        .filter(|token| available_exts.iter().any(|ext| ext == token))
        .collect::<Vec<_>>()
}

fn path_contains_filter_token(path: &str, token: &str) -> bool {
    let path = path.to_ascii_lowercase();
    let file_name = Path::new(&path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let stem = Path::new(&path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    path.contains(token) || file_name.contains(token) || stem.contains(token)
}

fn structured_fs_search_score(path: &str, tokens: &[String]) -> usize {
    tokens
        .iter()
        .filter(|token| {
            token.len() >= 3
                && !token.chars().all(|ch| ch.is_ascii_digit())
                && path_contains_filter_token(path, token)
        })
        .map(|token| token.len())
        .sum()
}

pub(super) fn fs_search_route_filtered_listing_candidate(
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
    allow_multi_result_list: bool,
) -> Option<String> {
    if !super::output_route_policy::route_contract_marker_is_any(
        route,
        &[
            crate::OutputSemanticKind::FilePaths,
            crate::OutputSemanticKind::FileNames,
            crate::OutputSemanticKind::ScalarPathOnly,
        ],
    ) {
        if !super::output_route_policy::route_contract_marker_is(
            route,
            crate::OutputSemanticKind::ExistenceWithPath,
        ) || !route_prefers_plain_fs_search_paths(route)
        {
            return None;
        }
    }
    let (mut results, count, pattern) = fs_search_find_name_results(value)?;
    if count == 0 || results.is_empty() {
        return None;
    }
    if results.len() == 1 {
        return Some(results[0].clone());
    }

    let locator_hint = route.locator_hint.trim();
    if !locator_hint.is_empty() {
        let scoped = results
            .iter()
            .filter(|path| path_is_inside_locator_scope(path, locator_hint))
            .cloned()
            .collect::<Vec<_>>();
        if !scoped.is_empty() && scoped.len() < results.len() {
            results = scoped;
        }
    }

    let normalized_pattern = normalized_find_name_pattern(pattern.as_deref());
    let ext_hints =
        structured_extension_hints(normalized_pattern.as_deref(), locator_hint, &results);
    if !ext_hints.is_empty() {
        let ext_filtered = results
            .iter()
            .filter(|path| {
                Path::new(path)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
                    .is_some_and(|ext| ext_hints.iter().any(|hint| hint == &ext))
            })
            .cloned()
            .collect::<Vec<_>>();
        if !ext_filtered.is_empty() {
            results = ext_filtered;
        }
    }

    let mut tokens = Vec::new();
    tokens.extend(pathish_filter_tokens(locator_hint));
    if let Some(pattern) = normalized_pattern.as_deref() {
        tokens.extend(pathish_filter_tokens(pattern));
    }
    tokens.extend(ext_hints);
    tokens.sort();
    tokens.dedup();

    if tokens.is_empty() {
        return allow_multi_result_list.then(|| {
            results
                .into_iter()
                .take(fs_search_result_list_limit(route))
                .collect::<Vec<_>>()
                .join("\n")
        });
    }
    let mut scored = results
        .iter()
        .cloned()
        .map(|path| {
            let score = structured_fs_search_score(&path, &tokens);
            (score, path)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let top_score = scored.first().map(|(score, _)| *score).unwrap_or_default();
    if top_score == 0 {
        return allow_multi_result_list.then(|| {
            results
                .into_iter()
                .take(fs_search_result_list_limit(route))
                .collect::<Vec<_>>()
                .join("\n")
        });
    }
    let second_score = scored
        .iter()
        .find_map(|(score, _)| (*score < top_score).then_some(*score))
        .unwrap_or_default();
    let decisive_single_candidate =
        top_score >= 8 && (second_score == 0 || top_score >= second_score.saturating_mul(2));
    let mut filtered = scored
        .into_iter()
        .filter(|(score, _)| *score == top_score)
        .map(|(_, path)| path)
        .collect::<Vec<_>>();
    filtered.sort();
    filtered.dedup();
    if filtered.len() == 1 {
        if allow_multi_result_list && !decisive_single_candidate && results.len() > 1 {
            return Some(
                results
                    .into_iter()
                    .take(fs_search_result_list_limit(route))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        return filtered.into_iter().next();
    }
    allow_multi_result_list.then(|| filtered.join("\n"))
}

fn fs_search_result_list_limit(route: &crate::IntentOutputContract) -> usize {
    if super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::FilePaths,
    ) {
        5
    } else {
        3
    }
}

fn route_prefers_absolute_fs_search_file_paths(route: &crate::IntentOutputContract) -> bool {
    super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::FilePaths,
    ) && route.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route.locator_hint.trim().is_empty()
}

pub(super) fn absolutize_fs_search_answer_paths(
    state: Option<&AppState>,
    route: Option<&crate::IntentOutputContract>,
    value: &serde_json::Value,
    answer: String,
    prefer_full_path: bool,
) -> String {
    if !prefer_full_path || !route.is_some_and(route_prefers_absolute_fs_search_file_paths) {
        return answer;
    }
    let Some(state) = state else {
        return answer;
    };
    let lines = answer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|path| absolutize_fs_search_result_path(state, value, path))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        answer
    } else {
        lines.join("\n")
    }
}

fn absolutize_fs_search_result_path(
    state: &AppState,
    value: &serde_json::Value,
    path: &str,
) -> String {
    let path = path.trim();
    let path_obj = Path::new(path);
    if path_obj.is_absolute() {
        return canonical_existing_path(path_obj);
    }
    let workspace_root = &state.skill_rt.workspace_root;
    let workspace_candidate = workspace_root.join(path);
    if workspace_candidate.exists() {
        return canonical_existing_path(&workspace_candidate);
    }
    let root = value
        .get("root")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|root| !root.is_empty() && *root != ".");
    if let Some(root) = root {
        let root_path = Path::new(root);
        let base = if root_path.is_absolute() {
            root_path.to_path_buf()
        } else {
            workspace_root.join(root_path)
        };
        let rooted_candidate = base.join(path);
        if rooted_candidate.exists() {
            return canonical_existing_path(&rooted_candidate);
        }
        return rooted_candidate.display().to_string();
    }
    workspace_candidate.display().to_string()
}

fn parent_directory_listing_from_paths(paths: &[String]) -> Option<String> {
    let mut dirs = Vec::new();
    for path in paths {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        let parent = Path::new(path)
            .parent()
            .map(|parent| {
                let display = parent.to_string_lossy().trim().to_string();
                if display.is_empty() {
                    ".".to_string()
                } else {
                    display
                }
            })
            .unwrap_or_else(|| ".".to_string());
        if !dirs.iter().any(|seen| seen == &parent) {
            dirs.push(parent);
        }
    }
    (!dirs.is_empty()).then(|| dirs.join("\n"))
}

pub(super) fn fs_search_contract_listing_candidate(
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
) -> Option<String> {
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::DirectoryNames,
    ) {
        return None;
    }
    let (results, count, _ext) = fs_search_find_ext_results(value)?;
    if count == 0 || results.is_empty() {
        return None;
    }
    parent_directory_listing_from_paths(&results)
}
