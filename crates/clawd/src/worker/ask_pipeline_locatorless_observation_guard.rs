use super::*;

#[cfg(test)]
#[path = "ask_pipeline_locatorless_observation_guard_tests.rs"]
mod tests;

pub(super) fn route_can_execute_without_locator(route_result: &crate::RouteResult) -> bool {
    super::route_has_capability_ref_machine_signal(route_result)
        || command_observation_marker_present(route_result)
}

pub(super) fn raw_command_output_has_explicit_command(state: &AppState, prompt: &str) -> bool {
    crate::agent_engine::explicit_command_segment_for_policy(
        &state.policy.command_intent,
        prompt.trim(),
    )
    .is_some()
}

pub(super) fn command_observation_route_has_runtime_evidence(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    raw_command_output_has_explicit_command(state, prompt)
        || command_observation_marker_present(route_result)
}

pub(super) fn command_observation_marker_present(route_result: &crate::RouteResult) -> bool {
    route_reason_has_marker(
        route_result,
        "explicit_command_requires_command_output_summary_execution",
    ) || route_reason_has_marker(
        route_result,
        "command_payload_requires_raw_output_execution",
    ) || route_reason_has_marker(
        route_result,
        "command_payload_requires_command_output_summary_execution",
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

pub(super) fn locatorless_observation_route_should_force_clarify(
    state: &AppState,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    let has_self_contained_payload = current_request_has_self_contained_structured_payload(prompt);
    let has_raw_command_input_locator =
        raw_command_request_has_structural_input_locator(state, prompt);
    let has_structural_locator_surface =
        current_request_has_structural_locator_surface_for_route(state, prompt, route_result);
    let command_payload_without_input = command_payload_observation_marker_present(route_result)
        && !raw_command_output_has_explicit_command(state, prompt)
        && !has_self_contained_payload
        && !has_raw_command_input_locator;
    let has_structured_session_anchor =
        active_session_has_structured_observation_anchor(session_snapshot);
    let has_authoritative_deictic_anchor =
        session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot);
    if !route_result.is_execute_gate()
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || (has_structural_locator_surface && !command_payload_without_input)
        || has_self_contained_payload
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || capability_route_can_plan_without_locator(route_result)
        || runtime_status_query_route_can_plan_without_locator(turn_analysis, route_result)
        || (has_authoritative_deictic_anchor && !command_payload_without_input)
        || has_structured_session_anchor
    {
        return false;
    }
    let scalar_runtime_observation_can_plan = route_result.output_contract.response_shape
        == crate::OutputResponseShape::Scalar
        && (turn_analysis.is_some_and(turn_analysis_has_runtime_status_query)
            || route_reason_has_marker(
                route_result,
                "execution_recipe_scalar_runtime_tool_observation",
            ));
    if scalar_runtime_observation_can_plan {
        return false;
    }
    if command_payload_without_input {
        return true;
    }
    if command_observation_route_has_runtime_evidence(state, prompt, route_result) {
        return false;
    }
    true
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

pub(super) fn raw_command_request_has_structural_input_locator(
    _state: &AppState,
    prompt: &str,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    surface.has_explicit_path_or_url()
        || surface.has_delivery_token_reference()
        || surface.is_structural_locator_only_reply()
}

pub(super) fn current_request_has_self_contained_structured_payload(prompt: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    surface.inline_json_shape.is_some()
        || crate::intent::surface_signals::inline_csv_record_block(prompt).is_some()
}
