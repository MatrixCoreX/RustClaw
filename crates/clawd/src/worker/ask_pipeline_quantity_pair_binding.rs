use super::*;

#[cfg(test)]
#[path = "ask_pipeline_quantity_pair_binding_tests.rs"]
mod tests;

pub(super) fn current_request_quantity_pair_evidence(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
) -> Option<(String, String)> {
    if !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
    {
        return None;
    }
    let quantity_comparison_marker = quantity_comparison_machine_signal(route_result);
    let recent_scalar_comparison_marker = recent_scalar_comparison_machine_signal(route_result);
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if ((quantity_comparison_marker || recent_scalar_comparison_marker)
        && surface.has_structured_target_refinement())
        || (quantity_comparison_marker
            && current_request_has_multiple_structured_config_locators(prompt))
    {
        return None;
    }
    if !quantity_comparison_marker
        && !recent_scalar_comparison_marker
        && (route_has_single_existing_locator_hint(state, route_result)
            || route_has_existing_file_locator_hint(state, route_result)
            || route_has_single_file_locator_hint_shape(route_result))
    {
        return None;
    }
    if !quantity_comparison_marker && prompt_surface_contains_archive_locator_pair(prompt) {
        return None;
    }
    if !quantity_comparison_marker && !route_result.needs_clarify && !route_result.is_execute_gate()
    {
        return None;
    }
    if recent_scalar_comparison_marker {
        workspace_path_pair_from_current_request(state, prompt)
            .filter(|(left, right)| {
                std::path::Path::new(left).is_dir() && std::path::Path::new(right).is_dir()
            })
            .or_else(|| workspace_directory_pair_from_current_request(state, prompt, false))
    } else if quantity_comparison_marker {
        workspace_path_pair_from_current_request(state, prompt).or_else(|| {
            if route_has_single_existing_locator_hint(state, route_result) {
                None
            } else {
                workspace_directory_pair_from_current_request(state, prompt, false)
            }
        })
    } else {
        workspace_directory_pair_from_current_request(state, prompt, true)
    }
}

fn quantity_comparison_machine_signal(route_result: &crate::RouteResult) -> bool {
    route_reason_has_marker(route_result, "quantity_comparison")
        || route_reason_has_marker(route_result, "quantity_compare")
}

fn recent_scalar_comparison_machine_signal(route_result: &crate::RouteResult) -> bool {
    route_reason_has_marker(route_result, "recent_scalar_equality_check")
        || route_reason_has_marker(
            route_result,
            "structured_field_pair_requires_scalar_equality_check",
        )
}

fn current_request_has_multiple_structured_config_locators(prompt: &str) -> bool {
    let mut locators = Vec::new();
    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(prompt)
    {
        let target = locator.locator_hint.trim();
        let has_structured_config_extension = std::path::Path::new(target)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .is_some_and(|ext| matches!(ext.as_str(), "json" | "toml" | "yaml" | "yml"));
        if has_structured_config_extension
            && !locators
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(target))
        {
            locators.push(target.to_string());
        }
    }
    locators.len() >= 2
}

fn prompt_surface_contains_archive_locator_pair(prompt: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    let Some((left, right)) = surface.locator_target_pair.as_ref() else {
        return false;
    };
    supported_archive_locator_path(left) ^ supported_archive_locator_path(right)
}

fn supported_archive_locator_path(path: &str) -> bool {
    let path = path.trim().to_ascii_lowercase();
    path.ends_with(".zip") || path.ends_with(".tar.gz") || path.ends_with(".tgz")
}

fn route_has_single_existing_locator_hint(
    state: &AppState,
    route_result: &crate::RouteResult,
) -> bool {
    let locators = crate::task_contract::target_locators_for_route(route_result);
    if locators.len() > 1 {
        return false;
    }
    let hint = route_result.output_contract.locator_hint.trim();
    !hint.is_empty() && resolve_existing_workspace_locator_hint(state, hint).is_some()
}

fn route_has_existing_file_locator_hint(
    state: &AppState,
    route_result: &crate::RouteResult,
) -> bool {
    let hint = route_result.output_contract.locator_hint.trim();
    if hint.is_empty() {
        return false;
    }
    resolve_existing_workspace_locator_hint(state, hint)
        .map(std::path::PathBuf::from)
        .is_some_and(|path| path.is_file())
}

fn route_has_single_file_locator_hint_shape(route_result: &crate::RouteResult) -> bool {
    if !matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
    ) {
        return false;
    }
    let hint = route_result.output_contract.locator_hint.trim();
    !hint.is_empty()
        && std::path::Path::new(hint)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| std::path::Path::new(name).extension().is_some())
}

fn workspace_path_pair_from_current_request(
    state: &AppState,
    prompt: &str,
) -> Option<(String, String)> {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if let Some((left, right)) = surface.locator_target_pair.as_ref() {
        let left = resolve_existing_workspace_locator_hint(state, left)?;
        let right = resolve_existing_workspace_locator_hint(state, right)?;
        return (!left.eq_ignore_ascii_case(&right)).then_some((left, right));
    }
    workspace_existing_locator_pair_from_prompt_tokens(state, prompt)
}

fn workspace_existing_locator_pair_from_prompt_tokens(
    state: &AppState,
    prompt: &str,
) -> Option<(String, String)> {
    let mut out = Vec::new();
    for token in prompt
        .split_whitespace()
        .flat_map(split_structural_locator_token)
    {
        let token = token
            .trim_matches(|ch: char| {
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
            })
            .trim();
        if token.len() < 2
            || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
        {
            continue;
        }
        let Some(path) = resolve_existing_workspace_locator_hint(state, token) else {
            continue;
        };
        if out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&path))
        {
            continue;
        }
        out.push(path);
        if out.len() > 2 {
            return None;
        }
    }
    (out.len() == 2).then(|| (out.remove(0), out.remove(0)))
}

fn split_structural_locator_token(token: &str) -> impl Iterator<Item = &str> {
    token.split(|ch: char| {
        matches!(
            ch,
            ',' | '，'
                | '。'
                | ';'
                | '；'
                | ':'
                | '：'
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
    })
}

pub(super) fn workspace_directory_pair_from_current_request(
    state: &AppState,
    prompt: &str,
    require_strong_locator_tokens: bool,
) -> Option<(String, String)> {
    let mut out = Vec::new();
    for token in structural_locator_token_candidates(prompt) {
        if require_strong_locator_tokens && !strong_structural_locator_token(&token) {
            continue;
        }
        let Some(path) = resolve_unique_directory_basename_under(
            &state.skill_rt.workspace_root,
            &token,
            directory_pair_locator_scan_limit(state),
        ) else {
            continue;
        };
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&path))
        {
            out.push(path);
        }
        if out.len() >= 2 {
            break;
        }
    }
    (out.len() == 2).then(|| (out.remove(0), out.remove(0)))
}

fn directory_pair_locator_scan_limit(state: &AppState) -> usize {
    state.skill_rt.locator_scan_max_files.max(50_000)
}

fn strong_structural_locator_token(token: &str) -> bool {
    token.contains(['_', '-', '.']) || token.chars().any(|ch| ch.is_ascii_digit())
}

pub(super) fn structural_locator_token_candidates(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
            cur.push(ch.to_ascii_lowercase());
        } else if !cur.is_empty() {
            push_structural_locator_token(&cur, &mut out);
            cur.clear();
            if out.len() >= 16 {
                break;
            }
        }
    }
    if !cur.is_empty() && out.len() < 16 {
        push_structural_locator_token(&cur, &mut out);
    }
    out
}

fn push_structural_locator_token(token: &str, out: &mut Vec<String>) {
    let token = token
        .trim_matches(|ch: char| matches!(ch, '.' | '"' | '\'' | '`'))
        .trim();
    if token.len() < 2
        || token.contains('/')
        || token.contains('\\')
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(token)
        || out.iter().any(|existing| existing == token)
    {
        return;
    }
    out.push(token.to_string());
}

fn resolve_unique_directory_basename_under(
    workspace_root: &std::path::Path,
    name: &str,
    max_visits: usize,
) -> Option<String> {
    if !workspace_root.is_dir() || name.trim().is_empty() {
        return None;
    }
    let mut stack = vec![workspace_root.to_path_buf()];
    let mut matches = Vec::new();
    let mut visits = 0usize;
    while let Some(dir) = stack.pop() {
        visits = visits.saturating_add(1);
        if visits > max_visits {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                file_type.is_dir().then(|| entry.path())
            })
            .collect::<Vec<_>>();
        children.sort();
        for child in children.into_iter().rev() {
            if child
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|file_name| file_name.eq_ignore_ascii_case(name))
            {
                let canonical = child.canonicalize().unwrap_or(child.clone());
                matches.push(canonical.display().to_string());
                if matches.len() > 1 {
                    return None;
                }
            }
            stack.push(child);
        }
    }
    matches.pop()
}
