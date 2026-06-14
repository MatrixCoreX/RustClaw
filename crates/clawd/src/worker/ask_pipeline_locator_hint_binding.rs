use super::*;

#[cfg(test)]
#[path = "ask_pipeline_locator_hint_binding_tests.rs"]
mod tests;

pub(super) fn resolved_prompt_existing_workspace_locator(
    state: &AppState,
    resolved_prompt: &str,
) -> Option<String> {
    crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(
        resolved_prompt,
    )
    .into_iter()
    .filter(|locator| matches!(locator.locator_kind, crate::OutputLocatorKind::Path))
    .filter_map(|locator| resolve_existing_workspace_locator_hint(state, &locator.locator_hint))
    .next()
}

pub(super) fn resolve_existing_workspace_locator_hint(
    state: &AppState,
    locator_hint: &str,
) -> Option<String> {
    let hint = locator_hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return None;
    }
    let path = std::path::Path::new(hint);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    };
    if !path.exists() {
        return None;
    }
    let canonical_path = path.canonicalize().unwrap_or(path);
    let canonical_root = state
        .skill_rt
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.skill_rt.workspace_root.clone());
    canonical_path
        .starts_with(canonical_root)
        .then(|| canonical_path.display().to_string())
}

pub(super) fn resolve_direct_child_stem_workspace_locator_hint(
    state: &AppState,
    locator_hint: &str,
) -> Option<String> {
    let hint = locator_hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return None;
    }
    let hint_path = std::path::Path::new(hint);
    let file_name = locator_component_token(hint_path.file_name()?.to_str()?)?;
    if file_name.contains('.') {
        return None;
    }
    let parent = hint_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                state.skill_rt.workspace_root.join(path)
            }
        })
        .unwrap_or_else(|| state.skill_rt.workspace_root.clone());
    let canonical_root = state
        .skill_rt
        .workspace_root
        .canonicalize()
        .unwrap_or_else(|_| state.skill_rt.workspace_root.clone());
    let canonical_parent = parent.canonicalize().ok()?;
    if !canonical_parent.starts_with(&canonical_root) {
        return None;
    }
    let mut matches = std::fs::read_dir(&canonical_parent)
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
                .is_some_and(|stem| stem.eq_ignore_ascii_case(&file_name))
                .then_some(path)
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return None;
    }
    let path = matches.pop()?;
    let canonical_path = path.canonicalize().unwrap_or(path);
    canonical_path
        .starts_with(canonical_root)
        .then(|| canonical_path.display().to_string())
}

pub(super) fn locator_hint_token_present_in_prompt(prompt: &str, locator_hint: &str) -> bool {
    let hint_tokens = locator_hint_match_tokens(locator_hint);
    if hint_tokens.is_empty() {
        return false;
    }
    structural_locator_token_candidates(prompt)
        .into_iter()
        .any(|token| {
            hint_tokens
                .iter()
                .any(|hint_token| token.eq_ignore_ascii_case(hint_token))
        })
}

pub(super) fn locator_hint_token_ambiguous_in_workspace(
    state: &AppState,
    locator_hint: &str,
) -> bool {
    let hint_tokens = locator_hint_match_tokens(locator_hint);
    if hint_tokens.is_empty() {
        return false;
    }
    let mut roots = Vec::new();
    push_unique_canonical_locator_root(&mut roots, state.skill_rt.workspace_root.clone());
    push_unique_canonical_locator_root(
        &mut roots,
        state.skill_rt.default_locator_search_dir.clone(),
    );

    let mut matches = Vec::new();
    for root in roots {
        collect_locator_token_matches(
            &root,
            &hint_tokens,
            state.skill_rt.locator_scan_max_depth,
            state.skill_rt.locator_scan_max_files,
            &mut matches,
        );
        matches.sort();
        matches.dedup();
        if matches.len() > 1 {
            return true;
        }
    }
    false
}

fn push_unique_canonical_locator_root(
    roots: &mut Vec<std::path::PathBuf>,
    root: std::path::PathBuf,
) {
    let canonical = root.canonicalize().unwrap_or(root);
    if !roots.iter().any(|existing| existing == &canonical) {
        roots.push(canonical);
    }
}

fn collect_locator_token_matches(
    root: &std::path::Path,
    hint_tokens: &[String],
    max_depth: usize,
    max_files: usize,
    out: &mut Vec<String>,
) {
    if !root.is_dir() {
        return;
    }
    let mut scanned = 0usize;
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if depth < max_depth {
                    queue.push_back((path, depth + 1));
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            scanned += 1;
            if locator_path_matches_any_hint_token(&path, hint_tokens) {
                out.push(path.canonicalize().unwrap_or(path).display().to_string());
                if out.len() > 1 {
                    return;
                }
            }
            if scanned >= max_files {
                return;
            }
        }
    }
}

fn locator_path_matches_any_hint_token(path: &std::path::Path, hint_tokens: &[String]) -> bool {
    let file_name = path.file_name().and_then(|value| value.to_str());
    let file_stem = path.file_stem().and_then(|value| value.to_str());
    hint_tokens.iter().any(|token| {
        file_name.is_some_and(|name| name.eq_ignore_ascii_case(token))
            || (!token.contains('.')
                && file_stem.is_some_and(|stem| stem.eq_ignore_ascii_case(token)))
    })
}

fn locator_hint_match_tokens(locator_hint: &str) -> Vec<String> {
    let hint = locator_hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return Vec::new();
    }
    let path = std::path::Path::new(hint);
    let mut out = Vec::new();
    if let Some(file_name) = path
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(locator_component_token)
    {
        push_unique_locator_hint_match_token(&mut out, file_name);
    }
    if let Some(stem) = path
        .file_stem()
        .and_then(|value| value.to_str())
        .and_then(locator_component_token)
    {
        push_unique_locator_hint_match_token(&mut out, stem);
    }
    out
}

fn push_unique_locator_hint_match_token(out: &mut Vec<String>, token: String) {
    if !out
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&token))
    {
        out.push(token);
    }
}

pub(super) fn locator_component_token(value: &str) -> Option<String> {
    let token = single_component_locator_hint(value)?;
    if token.len() < 2
        || token.chars().any(char::is_whitespace)
        || !token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return None;
    }
    Some(token)
}

pub(super) fn prebind_workspace_root_locator_from_resolved_prompt(
    state: &AppState,
    resolved_prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    let can_repair_clarify = route_result.needs_clarify && route_result.is_clarify_gate();
    if (!route_result.is_execute_gate() && !can_repair_clarify)
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        || !super::semantic_kind_can_bind_workspace_child_locator(
            route_result.output_contract.semantic_kind,
        )
    {
        return false;
    }
    if !text_contains_workspace_root_locator(resolved_prompt, &state.skill_rt.workspace_root)
        && !text_contains_workspace_root_locator(
            &route_result.resolved_intent,
            &state.skill_rt.workspace_root,
        )
    {
        return false;
    }

    let locator_hint = state.skill_rt.workspace_root.display().to_string();
    if can_repair_clarify {
        promote_clarify_observation_to_execute_with_locator(
            route_result,
            crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint,
            "workspace_root_locator_prebound_from_resolved_prompt",
        )
    } else {
        route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
        route_result.output_contract.locator_hint = locator_hint;
        append_route_reason(
            route_result,
            "workspace_root_locator_prebound_from_resolved_prompt",
        );
        true
    }
}

pub(super) fn text_contains_workspace_root_locator(
    text: &str,
    workspace_root: &std::path::Path,
) -> bool {
    crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        .into_iter()
        .any(|locator| {
            matches!(locator.locator_kind, crate::OutputLocatorKind::Path)
                && locator_path_points_to_workspace_root(&locator.locator_hint, workspace_root)
        })
}

fn locator_path_points_to_workspace_root(
    locator_hint: &str,
    workspace_root: &std::path::Path,
) -> bool {
    let hint = locator_hint.trim();
    if hint.is_empty() || hint.contains('\n') {
        return false;
    }
    let candidate = std::path::Path::new(hint);
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    let candidate = candidate.canonicalize().unwrap_or(candidate);
    let workspace_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    candidate == workspace_root
}
