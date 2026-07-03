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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundaryPreflightDeferral {
    DeicticMemoryOnly,
    UnboundModelContextTarget,
    BareTopicModelSuppliedLocator,
    ImplicitWorkspaceFileLocator,
    ModelCompletedWorkspaceFileLocator,
    InferredMissingWorkspaceLocator,
    ActiveAnchorFileDeliveryWithoutStructuredReference,
    BackgroundOnlyLocator,
    LocatorlessObservation,
    UnboundTargetedEvidence,
}

impl BoundaryPreflightDeferral {
    fn observation_token(self) -> &'static str {
        match self {
            Self::DeicticMemoryOnly => "deictic_memory_only",
            Self::UnboundModelContextTarget => "unbound_model_context_target",
            Self::BareTopicModelSuppliedLocator => "bare_topic_model_supplied_locator",
            Self::ImplicitWorkspaceFileLocator => "implicit_workspace_file_locator",
            Self::ModelCompletedWorkspaceFileLocator => "model_completed_workspace_file_locator",
            Self::InferredMissingWorkspaceLocator => "inferred_missing_workspace_locator",
            Self::ActiveAnchorFileDeliveryWithoutStructuredReference => {
                "active_anchor_file_delivery_without_structured_reference"
            }
            Self::BackgroundOnlyLocator => "background_only_locator",
            Self::LocatorlessObservation => "locatorless_observation",
            Self::UnboundTargetedEvidence => "unbound_targeted_evidence",
        }
    }

    fn reason_code(self) -> &'static str {
        match self {
            Self::DeicticMemoryOnly => "deictic_memory_only_deferred_to_agent_loop",
            Self::UnboundModelContextTarget => {
                "unbound_model_context_target_deferred_to_agent_loop"
            }
            Self::BareTopicModelSuppliedLocator => {
                "bare_topic_model_supplied_locator_deferred_to_agent_loop"
            }
            Self::ImplicitWorkspaceFileLocator => {
                "implicit_workspace_file_locator_deferred_to_agent_loop"
            }
            Self::ModelCompletedWorkspaceFileLocator => {
                "model_completed_workspace_file_locator_deferred_to_agent_loop"
            }
            Self::InferredMissingWorkspaceLocator => {
                "inferred_missing_workspace_locator_deferred_to_agent_loop"
            }
            Self::ActiveAnchorFileDeliveryWithoutStructuredReference => {
                "active_anchor_file_delivery_deferred_to_agent_loop"
            }
            Self::BackgroundOnlyLocator => "background_only_locator_deferred_to_agent_loop",
            Self::LocatorlessObservation => "locatorless_observation_deferred_to_agent_loop",
            Self::UnboundTargetedEvidence => "unbound_targeted_evidence_deferred_to_agent_loop",
        }
    }

    fn clears_locator_binding(self) -> bool {
        !matches!(
            self,
            Self::UnboundModelContextTarget | Self::LocatorlessObservation
        )
    }

    fn record(
        self,
        task: &crate::ClaimedTask,
        pre_loop_clarify_candidates: &mut Vec<&'static str>,
        route_result: &mut crate::RouteResult,
    ) {
        let before_gate_kind = route_result.gate_kind();
        if self.clears_locator_binding() {
            defer_locator_binding_to_agent_loop(route_result);
        }
        push_pre_loop_clarify_candidate(pre_loop_clarify_candidates, self.observation_token());
        log_route_guard_record(
            task,
            "worker_locator_guard",
            self.reason_code(),
            "deferred",
            before_gate_kind,
            route_result,
        );
    }
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
    if deictic_memory_only_route_should_defer_to_agent_loop(
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        BoundaryPreflightDeferral::DeicticMemoryOnly.record(
            task,
            pre_loop_clarify_candidates,
            route_result,
        );
    }
    if unbound_model_context_target_route_should_defer_to_agent_loop(
        state,
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        BoundaryPreflightDeferral::UnboundModelContextTarget.record(
            task,
            pre_loop_clarify_candidates,
            route_result,
        );
    }
    if bare_topic_model_supplied_locator_route_should_defer_to_agent_loop(
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        BoundaryPreflightDeferral::BareTopicModelSuppliedLocator.record(
            task,
            pre_loop_clarify_candidates,
            route_result,
        );
    }
    if implicit_workspace_file_locator_route_should_defer_to_agent_loop(
        state,
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        BoundaryPreflightDeferral::ImplicitWorkspaceFileLocator.record(
            task,
            pre_loop_clarify_candidates,
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
    if model_completed_workspace_file_locator_hint_should_defer_to_agent_loop(
        state,
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        BoundaryPreflightDeferral::ModelCompletedWorkspaceFileLocator.record(
            task,
            pre_loop_clarify_candidates,
            route_result,
        );
    }
    if inferred_missing_workspace_locator_hint_should_defer_to_agent_loop(
        state,
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        BoundaryPreflightDeferral::InferredMissingWorkspaceLocator.record(
            task,
            pre_loop_clarify_candidates,
            route_result,
        );
    }
    if active_anchor_file_delivery_without_structured_reference_should_defer_to_agent_loop(
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        BoundaryPreflightDeferral::ActiveAnchorFileDeliveryWithoutStructuredReference.record(
            task,
            pre_loop_clarify_candidates,
            route_result,
        );
    }
    if bare_topic_model_supplied_locator_route_should_defer_to_agent_loop(
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        BoundaryPreflightDeferral::BareTopicModelSuppliedLocator.record(
            task,
            pre_loop_clarify_candidates,
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
    if background_only_locator_route_should_defer_to_agent_loop(
        state,
        prompt,
        resolved_prompt,
        recent_execution_context,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        BoundaryPreflightDeferral::BackgroundOnlyLocator.record(
            task,
            pre_loop_clarify_candidates,
            route_result,
        );
    }
    append_runtime_status_capability_context(route_result, turn_analysis);
    if locatorless_observation_route_should_defer_to_agent_loop(
        state,
        prompt,
        route_result,
        turn_analysis,
        session_snapshot,
    ) {
        BoundaryPreflightDeferral::LocatorlessObservation.record(
            task,
            pre_loop_clarify_candidates,
            route_result,
        );
    }
    if unbound_targeted_evidence_route_should_defer_to_agent_loop(
        prompt,
        route_result,
        session_snapshot,
        recent_execution_context,
    ) {
        BoundaryPreflightDeferral::UnboundTargetedEvidence.record(
            task,
            pre_loop_clarify_candidates,
            route_result,
        );
    }
}
