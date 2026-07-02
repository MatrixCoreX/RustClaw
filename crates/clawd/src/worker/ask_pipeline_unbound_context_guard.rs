use super::*;

#[cfg(test)]
#[path = "ask_pipeline_unbound_context_guard_tests.rs"]
mod tests;

pub(super) fn execute_route_without_input_locator_should_plan(
    route_result: &crate::RouteResult,
) -> bool {
    route_result.is_execute_gate()
        && route_result.output_contract.requires_content_evidence
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
        && !route_result.wants_file_delivery
        && !route_result.output_contract.delivery_required
        && !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
}

pub(super) fn deictic_memory_only_route_should_defer_to_agent_loop(
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if execute_route_without_input_locator_should_plan(route_result) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if !state_patch_requires_deictic_locator_clarify(turn_analysis)
        || surface.has_concrete_locator_hint()
        || surface.has_delivery_token_reference()
    {
        return false;
    }
    if state_patch_allows_deictic_locator_guard_bypass(turn_analysis) {
        return false;
    }
    if route_locator_hint_matches_active_ordered_entry(route_result, session_snapshot) {
        return false;
    }
    if session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot) {
        return false;
    }
    if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && !active_observed_facts_have_bound_target(session_snapshot)
    {
        return false;
    }
    route_result.is_execute_gate()
        || route_result.output_contract.requires_content_evidence
        || route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
}

pub(super) fn unbound_model_context_target_route_should_defer_to_agent_loop(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || raw_command_output_without_locator_can_plan_via_contract(state, prompt, route_result)
        || runtime_status_query_route_can_plan_without_locator(turn_analysis, route_result)
        || task_control_route_can_plan_without_locator(route_result)
        || capability_route_can_plan_without_locator(route_result)
        || command_observation_route_has_runtime_evidence(state, prompt, route_result)
        || current_request_has_structural_locator_surface_for_route(state, prompt, route_result)
        || current_request_has_self_contained_structured_payload(prompt)
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || active_session_has_structured_observation_anchor(session_snapshot)
    {
        return false;
    }
    if current_workspace_listing_search_route_can_skip_unbound_context_guard(
        prompt,
        route_result,
        &state.skill_rt.workspace_root,
    ) {
        return false;
    }
    if current_workspace_route_can_skip_unbound_context_guard(prompt, route_result)
        && !route_introduces_unmentioned_distinctive_context_target_except_workspace_root(
            prompt,
            route_result,
            &state.skill_rt.workspace_root,
        )
    {
        return false;
    }
    if route_can_execute_without_locator(route_result)
        && !command_marker_without_input_needs_context_guard(state, prompt, route_result)
    {
        return false;
    }
    route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
}

fn capability_route_can_plan_without_locator(route_result: &crate::RouteResult) -> bool {
    if !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    super::route_has_capability_ref_machine_signal(route_result)
}

pub(super) fn task_control_route_can_plan_without_locator(
    route_result: &crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    super::route_reason_has_capability_ref_prefix(route_result, "task_control.")
}

fn route_contains_machine_token(route_result: &crate::RouteResult, token: &str) -> bool {
    machine_token_present(route_result.route_reason.as_str(), token.trim())
}

fn machine_token_present(text: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
        .any(|part| part == token || part.starts_with(&format!("{token}.")))
}

fn current_workspace_listing_search_route_can_skip_unbound_context_guard(
    prompt: &str,
    route_result: &crate::RouteResult,
    workspace_root: &std::path::Path,
) -> bool {
    if route_result.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
    {
        return false;
    }
    !route_introduces_unmentioned_distinctive_context_target_except_workspace_root(
        prompt,
        route_result,
        workspace_root,
    )
}

pub(super) fn raw_command_output_without_locator_can_plan_via_contract(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar
        || !command_payload_observation_marker_present(route_result)
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || raw_command_output_has_explicit_command(state, prompt)
    {
        return false;
    }
    if !current_request_has_self_contained_structured_payload(prompt)
        && !raw_command_request_has_structural_input_locator(state, prompt)
    {
        return false;
    }
    crate::evidence_policy::final_answer_shape_for_route(route_result)
        .is_some_and(|shape| shape.allows_model_language())
}

fn command_marker_without_input_needs_context_guard(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    command_observation_marker_present(route_result)
        && !raw_command_output_has_explicit_command(state, prompt)
        && !current_request_has_self_contained_structured_payload(prompt)
        && !raw_command_request_has_structural_input_locator(state, prompt)
        && !route_reason_has_marker(
            route_result,
            "execution_recipe_scalar_runtime_tool_observation",
        )
}

fn command_payload_observation_marker_present(route_result: &crate::RouteResult) -> bool {
    route_reason_has_marker(
        route_result,
        "command_payload_requires_raw_output_execution",
    ) || route_reason_has_marker(
        route_result,
        "command_payload_requires_command_output_summary_execution",
    )
}

pub(super) fn runtime_status_query_route_can_plan_without_locator(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    route_result: &crate::RouteResult,
) -> bool {
    let Some(turn_analysis) = turn_analysis else {
        return false;
    };
    route_result.is_execute_gate()
        && route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
        && (turn_analysis.turn_type == Some(crate::intent_router::TurnType::StatusQuery)
            || turn_analysis_has_runtime_status_query(turn_analysis))
        && crate::evidence_policy::final_answer_shape_for_route(route_result)
            .is_some_and(|shape| shape.allows_model_language())
}

fn current_workspace_route_can_skip_unbound_context_guard(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route_result.output_contract.locator_hint.trim().is_empty()
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && !is_bare_topic_only_prompt(prompt)
}

pub(super) fn unbound_targeted_evidence_route_should_defer_to_agent_loop(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    recent_execution_context: &str,
) -> bool {
    if current_workspace_target_binding_should_defer_to_agent_loop(
        prompt,
        route_result,
        session_snapshot,
    ) || current_workspace_unmentioned_locator_hint_should_defer_to_agent_loop(
        prompt,
        route_result,
        session_snapshot,
    ) {
        return true;
    }
    if current_request_has_concrete_locator_surface(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || active_session_has_bound_target(session_snapshot)
        || capability_route_can_plan_without_locator(route_result)
        || route_result.needs_clarify
        || route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    if recent_artifacts_judgment_can_use_recent_execution_context(
        route_result,
        recent_execution_context,
    ) {
        return false;
    }
    if route_result.output_contract.requires_content_evidence
        && !route_can_execute_without_locator(route_result)
    {
        return true;
    }
    route_requires_target_locator_machine_signal(route_result)
        || matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::DirectoryLookup
                | crate::OutputDeliveryIntent::DirectoryBatchFiles
        )
}

fn current_workspace_unmentioned_locator_hint_should_defer_to_agent_loop(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if current_workspace_untrusted_deictic_locator_hint_should_defer_to_agent_loop(
        prompt,
        route_result,
    ) {
        return true;
    }
    if current_workspace_locator_hint_in_resolved_intent_only_should_defer_to_agent_loop(
        prompt,
        route_result,
    ) {
        return true;
    }
    if route_result.needs_clarify
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
        || route_result.output_contract.locator_hint.trim().is_empty()
        || !route_result.output_contract.requires_content_evidence
        || current_request_has_concrete_locator_surface(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || current_workspace_scope_marker_allows_scalar_count_root_hint(route_result)
        || current_workspace_scope_observation_can_execute_without_locator(prompt, route_result)
    {
        return false;
    }
    route_requires_target_locator_machine_signal(route_result)
        || matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::DirectoryLookup
                | crate::OutputDeliveryIntent::DirectoryBatchFiles
        )
}

fn current_workspace_target_binding_should_defer_to_agent_loop(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if route_result.needs_clarify
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
        || !route_result.output_contract.requires_content_evidence
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || current_request_has_concrete_locator_surface(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || route_can_execute_without_locator(route_result)
        || current_workspace_scope_observation_can_execute_without_locator(prompt, route_result)
    {
        return false;
    }
    current_workspace_empty_scope_needs_target_guard(route_result)
        || matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::DirectoryLookup
                | crate::OutputDeliveryIntent::DirectoryBatchFiles
        )
}

fn current_workspace_scope_marker_allows_scalar_count_root_hint(
    route_result: &crate::RouteResult,
) -> bool {
    authoritative_current_workspace_locator_signal(route_result)
        && route_result.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount)
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && !route_result.output_contract.locator_hint.trim().is_empty()
        && route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::OneSentence
        )
}

fn current_workspace_untrusted_deictic_locator_hint_should_defer_to_agent_loop(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    surface.has_deictic_reference()
        && !current_request_has_concrete_locator_surface(prompt)
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && !route_result.output_contract.locator_hint.trim().is_empty()
        && route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && !authoritative_current_workspace_locator_hint_signal(route_result)
        && !route_can_execute_without_locator(route_result)
}

fn current_workspace_locator_hint_in_resolved_intent_only_should_defer_to_agent_loop(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    let locator_hint = route_result.output_contract.locator_hint.trim();
    !locator_hint.is_empty()
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && !authoritative_current_workspace_locator_hint_signal(route_result)
        && !current_prompt_has_explicit_locator_matching_hint(prompt, locator_hint)
        && current_prompt_has_explicit_locator_matching_hint(
            &route_result.resolved_intent,
            locator_hint,
        )
}

fn current_prompt_has_explicit_locator_matching_hint(prompt: &str, locator_hint: &str) -> bool {
    crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(prompt)
        .into_iter()
        .any(|locator| explicit_locator_matches_hint(&locator.locator_hint, locator_hint))
}

fn explicit_locator_matches_hint(candidate: &str, locator_hint: &str) -> bool {
    let candidate = candidate.trim();
    let locator_hint = locator_hint.trim();
    if candidate.is_empty() || locator_hint.is_empty() {
        return false;
    }
    if candidate == locator_hint {
        return true;
    }
    let candidate_path = std::path::Path::new(candidate);
    let hint_path = std::path::Path::new(locator_hint);
    if !candidate_path.is_absolute() && !hint_path.is_absolute() {
        return false;
    }
    let candidate_path = candidate_path
        .canonicalize()
        .unwrap_or_else(|_| candidate_path.to_path_buf());
    let hint_path = hint_path
        .canonicalize()
        .unwrap_or_else(|_| hint_path.to_path_buf());
    candidate_path == hint_path
}

fn authoritative_current_workspace_locator_signal(route_result: &crate::RouteResult) -> bool {
    route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        || route_reason_has_marker(route_result, "current_workspace_scope_from_current_request")
        || super::route_has_capability_ref_machine_signal(route_result)
}

fn current_workspace_scope_has_machine_signal(route_result: &crate::RouteResult) -> bool {
    authoritative_current_workspace_locator_signal(route_result)
}

fn authoritative_current_workspace_locator_hint_signal(route_result: &crate::RouteResult) -> bool {
    route_reason_has_marker(route_result, "current_workspace_scope_from_current_request")
        || super::route_has_capability_ref_machine_signal(route_result)
}

fn current_workspace_empty_scope_needs_target_guard(route_result: &crate::RouteResult) -> bool {
    route_result.output_contract.locator_hint.trim().is_empty()
        && route_requires_target_locator_machine_signal(route_result)
}

fn route_requires_target_locator_machine_signal(route_result: &crate::RouteResult) -> bool {
    route_contains_machine_token(route_result, "target_locator_required")
        || route_contains_machine_token(route_result, "missing_target_locator")
        || route_contains_machine_token(route_result, "current_workspace_target_required")
        || route_contains_machine_token(route_result, "unbound_target_requires_clarify")
}

fn current_workspace_scope_observation_can_execute_without_locator(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    if route_result.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || is_bare_topic_only_prompt(prompt)
    {
        return false;
    }
    if route_introduces_unmentioned_distinctive_context_target(prompt, route_result) {
        return false;
    }
    if current_workspace_empty_scope_needs_target_guard(route_result) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if surface.has_deictic_reference()
        && !current_request_has_concrete_locator_surface(prompt)
        && !current_workspace_scope_has_machine_signal(route_result)
    {
        return false;
    }
    true
}
