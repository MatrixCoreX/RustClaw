use super::*;

#[cfg(test)]
#[path = "ask_pipeline_quantity_pair_binding_tests.rs"]
mod tests;

pub(super) fn prebind_quantity_compare_directory_pair_from_current_request(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
    {
        return false;
    }
    let semantic_quantity_comparison =
        route_result.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison;
    let semantic_recent_scalar_comparison = route_result.output_contract.semantic_kind
        == crate::OutputSemanticKind::RecentScalarEqualityCheck;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if ((semantic_quantity_comparison || semantic_recent_scalar_comparison)
        && surface.has_structured_target_refinement())
        || (semantic_quantity_comparison
            && current_request_has_multiple_structured_config_locators(prompt))
    {
        return false;
    }
    if !semantic_quantity_comparison
        && !semantic_recent_scalar_comparison
        && route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
    {
        return false;
    }
    if !semantic_quantity_comparison && prompt_surface_contains_archive_locator_pair(prompt) {
        return false;
    }
    if !semantic_quantity_comparison
        && !route_result.needs_clarify
        && !route_result.is_execute_gate()
    {
        return false;
    }
    let path_pair = if semantic_recent_scalar_comparison {
        workspace_path_pair_from_current_request(state, prompt)
            .filter(|(left, right)| {
                std::path::Path::new(left).is_dir() && std::path::Path::new(right).is_dir()
            })
            .or_else(|| workspace_directory_pair_from_current_request(state, prompt, false))
    } else if semantic_quantity_comparison {
        workspace_path_pair_from_current_request(state, prompt).or_else(|| {
            if route_has_single_existing_locator_hint(state, route_result) {
                None
            } else {
                workspace_directory_pair_from_current_request(
                    state,
                    prompt,
                    !semantic_quantity_comparison,
                )
            }
        })
    } else {
        workspace_directory_pair_from_current_request(state, prompt, !semantic_quantity_comparison)
    };
    let Some((left, right)) = path_pair else {
        return false;
    };
    if semantic_recent_scalar_comparison {
        route_result.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
        route_result.output_contract.response_shape = crate::OutputResponseShape::Strict;
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = format!("{left} | {right}");
    route_result.output_contract.requires_content_evidence = true;
    route_result.needs_clarify = false;
    route_result.clarify_question.clear();
    route_result.set_planner_execute_finalize(
        crate::post_route_policy::content_evidence_execution_finalize_style(
            &route_result.output_contract,
            false,
        )
        .unwrap_or(crate::ActFinalizeStyle::ChatWrapped),
    );
    append_route_reason(
        route_result,
        if semantic_quantity_comparison {
            "quantity_compare_path_pair_prebound_from_current_request"
        } else if semantic_recent_scalar_comparison {
            "recent_scalar_directory_pair_promoted_to_quantity_comparison"
        } else {
            "directory_pair_prebound_from_current_request"
        },
    );
    true
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

pub(super) fn route_has_single_existing_directory_locator_hint(
    state: &AppState,
    route_result: &crate::RouteResult,
) -> bool {
    let locators = crate::task_contract::target_locators_for_route(route_result);
    if locators.len() > 1 {
        return false;
    }
    let hint = route_result.output_contract.locator_hint.trim();
    resolve_existing_workspace_locator_hint(state, hint)
        .as_deref()
        .is_some_and(|path| std::path::Path::new(path).is_dir())
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
