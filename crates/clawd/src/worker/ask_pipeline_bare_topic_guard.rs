use super::*;

#[cfg(test)]
#[path = "ask_pipeline_bare_topic_guard_tests.rs"]
mod tests;

pub(super) fn bare_topic_memory_expansion_route_should_force_clarify(
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if !is_bare_topic_only_prompt(prompt)
        || !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || !command_observation_marker_present(route_result)
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || active_session_has_structured_observation_anchor(session_snapshot)
    {
        return false;
    }
    route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
}

pub(super) fn bare_topic_model_supplied_locator_route_should_force_clarify(
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_reason_has_marker(route_result, "active_clarify_locator_reply_execute") {
        return false;
    }
    if turn_analysis.is_some_and(|analysis| {
        matches!(
            analysis.target_task_policy,
            Some(crate::intent_router::TargetTaskPolicy::ReuseActive)
        )
    }) && active_session_has_structured_observation_anchor(session_snapshot)
    {
        return false;
    }
    if !is_bare_topic_only_prompt(prompt)
        || current_request_has_concrete_locator_surface(prompt)
        || (!route_result.needs_clarify && !route_result.is_execute_gate())
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
        )
        || route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    true
}

pub(super) fn is_bare_topic_only_prompt(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    if trimmed.is_empty()
        || trimmed.split_whitespace().count() != 1
        || trimmed.contains(['/', '\\', '.', ':'])
        || !trimmed
            .chars()
            .any(|ch| ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&ch))
        || !trimmed.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || ('\u{4e00}'..='\u{9fff}').contains(&ch)
                || matches!(ch, '-' | '_')
        })
    {
        return false;
    }
    let signal_chars = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count();
    if trimmed.is_ascii() {
        return signal_chars <= 32;
    }
    signal_chars <= 4
}

pub(super) fn route_introduces_unmentioned_distinctive_context_target(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    let mut text = String::new();
    text.push_str(&route_result.resolved_intent);
    text.push('\n');
    text.push_str(&route_result.clarify_question);
    distinctive_context_tokens(&text)
        .into_iter()
        .any(|token| !distinctive_token_present_in_request(prompt, &token))
}

pub(super) fn route_introduces_unmentioned_distinctive_context_target_except_workspace_root(
    prompt: &str,
    route_result: &crate::RouteResult,
    workspace_root: &std::path::Path,
) -> bool {
    let mut text = String::new();
    text.push_str(&route_result.resolved_intent);
    text.push('\n');
    text.push_str(&route_result.clarify_question);
    distinctive_context_tokens(&text).into_iter().any(|token| {
        distinctive_token_relevant_for_workspace_scope_guard(&token)
            && !distinctive_token_names_workspace_root(&token, workspace_root)
            && !distinctive_token_present_in_request(prompt, &token)
    })
}

fn distinctive_token_relevant_for_workspace_scope_guard(token: &str) -> bool {
    token
        .chars()
        .any(|ch| ch.is_ascii_alphanumeric() || ch.is_ascii_digit())
        || token.contains(['.', ':'])
}

fn distinctive_token_names_workspace_root(token: &str, workspace_root: &std::path::Path) -> bool {
    let normalized_token = normalize_locator_identity_token(token);
    if normalized_token.is_empty() {
        return false;
    }
    if locator_hint_names_workspace_root(workspace_root, &normalized_token) {
        return true;
    }
    let canonical_root = normalize_workspace_locator_path(workspace_root);
    let normalized_root = normalize_locator_identity_token(&canonical_root.display().to_string());
    normalized_token == normalized_root
        || normalized_token == normalized_root.trim_start_matches('/')
}

fn distinctive_context_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| {
        !(ch.is_ascii_alphanumeric()
            || ('\u{4e00}'..='\u{9fff}').contains(&ch)
            || matches!(ch, '_' | '-' | '/' | '.' | ':'))
    })
    .map(|token| token.trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '.' | ':')))
    .filter(|token| distinctive_context_token(token))
    .map(ToOwned::to_owned)
    .collect()
}

fn distinctive_context_token(token: &str) -> bool {
    let signal_chars = token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count();
    let has_identifier_separator = token.contains(['_', '/', '.', ':']);
    let ascii_digit_identifier = token.is_ascii()
        && token.chars().all(|ch| ch.is_ascii_alphanumeric())
        && token.chars().any(|ch| ch.is_ascii_digit());
    let has_digit = token.chars().any(|ch| ch.is_ascii_digit());
    (signal_chars >= 4 && has_identifier_separator)
        || (signal_chars >= 8 && has_digit && ascii_digit_identifier)
}

fn distinctive_token_present_in_request(request: &str, token: &str) -> bool {
    let request = request.to_ascii_lowercase();
    let token = token.to_ascii_lowercase();
    if request.contains(&token) {
        return true;
    }
    token
        .split(['_', '-', '/', '.', ':'])
        .filter(|part| part.len() >= 3)
        .any(|part| request.contains(part))
}

pub(super) fn bare_topic_clarify_question_should_drop_context_target(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    is_bare_topic_only_prompt(prompt)
        && route_result.needs_clarify
        && route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
}
