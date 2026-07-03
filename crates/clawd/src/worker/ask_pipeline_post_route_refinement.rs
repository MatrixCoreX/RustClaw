use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundaryClarifyCandidate {
    MissingPathScopedLocator,
    FuzzyLocatorCandidates,
    FileDeliveryCurrentRequestLocator,
    UnresolvedFileDeliveryRequiresLocator,
}

impl BoundaryClarifyCandidate {
    fn observation_token(self) -> &'static str {
        match self {
            Self::MissingPathScopedLocator => "post_route_missing_path_scoped_locator",
            Self::FuzzyLocatorCandidates => "post_route_fuzzy_locator_candidates",
            Self::FileDeliveryCurrentRequestLocator => {
                "post_route_file_delivery_current_request_locator"
            }
            Self::UnresolvedFileDeliveryRequiresLocator => {
                "post_route_unresolved_file_delivery_requires_locator"
            }
        }
    }

    fn gate_reason_code(self) -> &'static str {
        match self {
            Self::MissingPathScopedLocator => {
                "post_route_missing_path_scoped_locator_deferred_to_agent_loop"
            }
            Self::FuzzyLocatorCandidates => {
                "post_route_fuzzy_locator_candidates_deferred_to_agent_loop"
            }
            Self::FileDeliveryCurrentRequestLocator => {
                "post_route_file_delivery_current_request_locator_deferred_to_agent_loop"
            }
            Self::UnresolvedFileDeliveryRequiresLocator => {
                "post_route_unresolved_file_delivery_deferred_to_agent_loop"
            }
        }
    }

    fn requires_file_delivery_contract(self) -> bool {
        matches!(self, Self::UnresolvedFileDeliveryRequiresLocator)
    }
}

pub(super) fn apply_post_route_refinements(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    pre_loop_clarify_candidates: &mut Vec<&'static str>,
    post_route: &mut crate::post_route_policy::PostRoutePolicyResult,
) {
    if refine_unresolved_file_delivery_boundary_contract(prompt, post_route) {
        tracing::info!(
            "{} worker_once: ask file_delivery_boundary_contract_refined_for_agent_loop task_id={}",
            crate::highlight_tag("routing"),
            task.task_id
        );
    }
    if auto_locator_scalar_file_without_current_locator_should_defer_to_agent_loop(
        state,
        prompt,
        &post_route.execution_route_result,
        post_route.auto_locator_path.as_deref(),
    ) {
        post_route
            .execution_route_result
            .output_contract
            .locator_kind = crate::OutputLocatorKind::None;
        post_route
            .execution_route_result
            .output_contract
            .locator_hint
            .clear();
        post_route.auto_locator_path = None;
        post_route.auto_locator_hint = None;
        post_route.auto_locator_resolved_direct = false;
        post_route.missing_locator_for_path_scoped_content = true;
        push_pre_loop_clarify_candidate(
            pre_loop_clarify_candidates,
            "auto_locator_scalar_file_without_current_locator",
        );
        post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_auto_locator_scalar_file_deferred_to_agent_loop",
            crate::post_route_policy::PostRoutePolicyOutcome::RefineContract,
        );
    }
    if post_route.missing_locator_for_path_scoped_content
        && !route_reason_has_marker(
            &post_route.execution_route_result,
            "directory_file_delivery_requires_structured_selection",
        )
        && path_scoped_locator_guard_can_defer_to_prompt_targets(
            prompt,
            &post_route.execution_route_result,
        )
    {
        post_route.missing_locator_for_path_scoped_content = false;
        post_route.execution_route_result.needs_clarify = false;
        post_route.execution_route_result.clarify_question.clear();
        let finalize = crate::post_route_policy::content_evidence_execution_finalize_style(
            &post_route.execution_route_result.output_contract,
            false,
        )
        .unwrap_or(crate::ActFinalizeStyle::ChatWrapped);
        post_route
            .execution_route_result
            .set_planner_execute_finalize(finalize);
        post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_locator_guard_deferred_to_prompt_targets",
            crate::post_route_policy::PostRoutePolicyOutcome::BoundaryReady,
        );
        append_route_reason(
            &mut post_route.execution_route_result,
            "locator_guard_deferred_to_prompt_targets",
        );
    }
    if defer_config_contract_default_main_config_after_locator_policy(state, prompt, post_route) {
        tracing::info!(
            "{} worker_once: ask config_contract_default_main_config_deferred task_id={}",
            crate::highlight_tag("routing"),
            task.task_id
        );
    }
    if super::ask_prepare::repair_structural_file_delivery_resolution_for_turn(
        &mut post_route.execution_route_result,
        session_snapshot,
        turn_analysis,
    ) && !post_route.execution_route_result.needs_clarify
    {
        let target = post_route
            .execution_route_result
            .output_contract
            .locator_hint
            .trim()
            .to_string();
        if !target.is_empty() {
            post_route.auto_locator_path = Some(target);
            post_route.auto_locator_resolved_direct = true;
            post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
                "post_route_structural_file_delivery_bound_target",
                crate::post_route_policy::PostRoutePolicyOutcome::BoundaryReady,
            );
        }
    }
    auto_locator_binding::bind_structured_field_read_to_auto_locator(post_route);
    if route_reason_has_marker(
        &post_route.execution_route_result,
        "directory_file_delivery_requires_structured_selection",
    ) && !route_has_structured_list_selector(&post_route.execution_route_result)
    {
        post_route.execution_route_result.needs_clarify = false;
        post_route.execution_route_result.clarify_question.clear();
        post_route
            .execution_route_result
            .output_contract
            .locator_kind = crate::OutputLocatorKind::None;
        post_route
            .execution_route_result
            .output_contract
            .locator_hint
            .clear();
        post_route.auto_locator_path = None;
        post_route.auto_locator_hint = None;
        post_route.auto_locator_resolved_direct = false;
        post_route.missing_locator_for_path_scoped_content = true;
        push_pre_loop_clarify_candidate(
            pre_loop_clarify_candidates,
            "directory_file_delivery_requires_structured_selection",
        );
        post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_directory_file_delivery_deferred_to_agent_loop",
            crate::post_route_policy::PostRoutePolicyOutcome::RefineContract,
        );
    }
    defer_boundary_clarify_to_agent_loop(pre_loop_clarify_candidates, post_route);
}

fn defer_boundary_clarify_to_agent_loop(
    pre_loop_clarify_candidates: &mut Vec<&'static str>,
    post_route: &mut crate::post_route_policy::PostRoutePolicyResult,
) {
    let route_has_unresolved_file_delivery_marker = route_reason_has_marker(
        &post_route.execution_route_result,
        "unresolved_file_delivery_requires_locator",
    ) || route_reason_has_marker(
        &post_route.execution_route_result,
        "unresolved_file_delivery_requires_clarify",
    );
    let route_has_workspace_root_delivery_reject = route_reason_has_marker(
        &post_route.execution_route_result,
        "direct_file_delivery_workspace_root_locator_rejected",
    );
    let candidate = match (
        post_route.gate_record.owner_layer,
        post_route.gate_record.reason_code,
    ) {
        ("boundary_locator_gate", "post_route_missing_path_scoped_locator") => {
            BoundaryClarifyCandidate::MissingPathScopedLocator
        }
        ("boundary_locator_gate", "post_route_fuzzy_locator_candidates") => {
            BoundaryClarifyCandidate::FuzzyLocatorCandidates
        }
        (
            "boundary_delivery_gate",
            "post_route_file_delivery_current_request_locator_deferred_to_loop",
        ) => BoundaryClarifyCandidate::FileDeliveryCurrentRequestLocator,
        (
            "agent_loop_boundary_defer",
            "post_route_unresolved_file_delivery_deferred_to_agent_loop",
        ) => BoundaryClarifyCandidate::UnresolvedFileDeliveryRequiresLocator,
        _ if route_has_unresolved_file_delivery_marker
            && !route_has_workspace_root_delivery_reject =>
        {
            BoundaryClarifyCandidate::UnresolvedFileDeliveryRequiresLocator
        }
        _ => return,
    };

    post_route.execution_route_result.needs_clarify = false;
    post_route.execution_route_result.clarify_question.clear();
    post_route
        .execution_route_result
        .set_planner_execute_finalize(crate::ActFinalizeStyle::ChatWrapped);
    push_pre_loop_clarify_candidate(pre_loop_clarify_candidates, candidate.observation_token());
    if candidate.requires_file_delivery_contract() {
        post_route.execution_route_result.wants_file_delivery = true;
        post_route
            .execution_route_result
            .output_contract
            .delivery_required = true;
        post_route
            .execution_route_result
            .output_contract
            .delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        post_route
            .execution_route_result
            .output_contract
            .response_shape = crate::OutputResponseShape::FileToken;
        post_route
            .execution_route_result
            .output_contract
            .requires_content_evidence = true;
        append_route_reason(
            &mut post_route.execution_route_result,
            "unresolved_file_delivery_deferred_to_agent_loop",
        );
    }
    post_route.gate_record = crate::post_route_policy::PostRouteGateRecord::with_owner(
        "agent_loop_boundary_defer",
        candidate.gate_reason_code(),
        crate::post_route_policy::PostRoutePolicyOutcome::RefineContract,
    );
}
