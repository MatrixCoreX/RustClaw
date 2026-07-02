use super::*;

#[cfg(test)]
#[path = "ask_pipeline_boundary_preflight_tests.rs"]
mod tests;

pub(super) fn defer_locator_binding_to_agent_loop(route_result: &mut crate::RouteResult) {
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.locator_hint.clear();
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route_result.output_contract.requires_content_evidence = false;
}

pub(super) fn boundary_safety_preflight(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    pre_loop_clarify_candidates: &mut Vec<&'static str>,
    route_result: &mut crate::RouteResult,
) {
    if deictic_memory_only_route_should_force_clarify(
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        defer_locator_binding_to_agent_loop(route_result);
        push_pre_loop_clarify_candidate(pre_loop_clarify_candidates, "deictic_memory_only");
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "deictic_memory_only_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
    if unbound_model_context_target_route_should_force_clarify(
        state,
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        push_pre_loop_clarify_candidate(
            pre_loop_clarify_candidates,
            "unbound_model_context_target",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "unbound_model_context_target_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
    if bare_topic_model_supplied_locator_route_should_force_clarify(
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        defer_locator_binding_to_agent_loop(route_result);
        push_pre_loop_clarify_candidate(
            pre_loop_clarify_candidates,
            "bare_topic_model_supplied_locator",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "bare_topic_model_supplied_locator_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
    if implicit_workspace_file_locator_route_should_force_clarify(
        state,
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        defer_locator_binding_to_agent_loop(route_result);
        push_pre_loop_clarify_candidate(
            pre_loop_clarify_candidates,
            "implicit_workspace_file_locator",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "implicit_workspace_file_locator_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
}

pub(super) fn boundary_post_binding_locator_preflight(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    pre_loop_clarify_candidates: &mut Vec<&'static str>,
    route_result: &mut crate::RouteResult,
) {
    if model_completed_workspace_file_locator_hint_should_force_clarify(
        state,
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        defer_locator_binding_to_agent_loop(route_result);
        push_pre_loop_clarify_candidate(
            pre_loop_clarify_candidates,
            "model_completed_workspace_file_locator",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "model_completed_workspace_file_locator_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
    if inferred_missing_workspace_locator_hint_should_force_clarify(
        state,
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        defer_locator_binding_to_agent_loop(route_result);
        push_pre_loop_clarify_candidate(
            pre_loop_clarify_candidates,
            "inferred_missing_workspace_locator",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "inferred_missing_workspace_locator_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
    if active_anchor_file_delivery_without_structured_reference_should_force_clarify(
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        defer_locator_binding_to_agent_loop(route_result);
        push_pre_loop_clarify_candidate(
            pre_loop_clarify_candidates,
            "active_anchor_file_delivery_without_structured_reference",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "active_anchor_file_delivery_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
    if bare_topic_model_supplied_locator_route_should_force_clarify(
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        defer_locator_binding_to_agent_loop(route_result);
        push_pre_loop_clarify_candidate(
            pre_loop_clarify_candidates,
            "bare_topic_model_supplied_locator",
        );
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "bare_topic_model_supplied_locator_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
}

pub(super) fn boundary_context_locator_preflight(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    resolved_prompt: &str,
    recent_execution_context: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    pre_loop_clarify_candidates: &mut Vec<&'static str>,
    route_result: &mut crate::RouteResult,
) {
    if background_only_locator_route_should_force_clarify(
        state,
        prompt,
        resolved_prompt,
        recent_execution_context,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        defer_locator_binding_to_agent_loop(route_result);
        push_pre_loop_clarify_candidate(pre_loop_clarify_candidates, "background_only_locator");
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "background_only_locator_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
    append_runtime_status_capability_context(route_result, turn_analysis);
    if locatorless_observation_route_should_force_clarify(
        state,
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        let before_gate_kind = route_result.gate_kind();
        push_pre_loop_clarify_candidate(pre_loop_clarify_candidates, "locatorless_observation");
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "locatorless_observation_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
    if unbound_targeted_evidence_route_should_force_clarify(
        prompt,
        route_result,
        session_snapshot,
        recent_execution_context,
    ) {
        let before_gate_kind = route_result.gate_kind();
        defer_locator_binding_to_agent_loop(route_result);
        push_pre_loop_clarify_candidate(pre_loop_clarify_candidates, "unbound_targeted_evidence");
        log_route_guard_record(
            task,
            "worker_locator_guard",
            "unbound_targeted_evidence_deferred_to_agent_loop",
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
}
