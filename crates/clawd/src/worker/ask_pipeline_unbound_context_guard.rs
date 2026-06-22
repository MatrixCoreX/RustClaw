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

pub(super) fn deictic_memory_only_route_should_force_clarify(
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

pub(super) fn unbound_model_context_target_route_should_force_clarify(
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
        || command_observation_route_has_runtime_evidence(state, prompt, route_result)
        || current_request_has_structural_locator_surface_for_route(state, prompt, route_result)
        || current_request_has_self_contained_structured_payload(prompt)
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
        || active_session_has_structured_observation_anchor(session_snapshot)
    {
        return false;
    }
    if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route_result.output_contract.semantic_kind == crate::OutputSemanticKind::None
    {
        return false;
    }
    if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::DirectoryPurposeSummary
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
    if semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        && !(route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::RawCommandOutput
            && !raw_command_output_has_explicit_command(state, prompt)
            && !route_reason_has_marker(
                route_result,
                "execution_recipe_scalar_runtime_tool_observation",
            ))
    {
        return false;
    }
    route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
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
    route_contains_machine_token(route_result, "task_control")
}

pub(super) fn restore_explicit_extension_assess_gap_to_command_summary(
    route_result: &mut crate::RouteResult,
) -> bool {
    if !(route_result.is_execute_gate() || route_result.is_clarify_gate())
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !route_requests_extension_assess_gap(route_result)
    {
        return false;
    }

    route_result.needs_clarify = false;
    route_result.clarify_question.clear();
    route_result.set_execute_gate();
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::CommandOutputSummary;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.locator_hint.clear();
    if route_result.output_contract.response_shape == crate::OutputResponseShape::Strict {
        route_result.output_contract.response_shape = crate::OutputResponseShape::Free;
    }
    append_route_reason(
        route_result,
        "extension_assess_gap_contract_restored_to_command_output_summary",
    );
    true
}

fn route_contains_machine_token(route_result: &crate::RouteResult, token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    [
        route_result.resolved_intent.as_str(),
        route_result.route_reason.as_str(),
    ]
    .into_iter()
    .any(|text| machine_token_present(text, token))
}

fn route_requests_extension_assess_gap(route_result: &crate::RouteResult) -> bool {
    route_contains_machine_token(route_result, "extension.assess_gap")
        || (route_contains_machine_token(route_result, "extension_manager")
            && route_contains_machine_token(route_result, "assess_gap"))
}

fn machine_token_present(text: &str, token: &str) -> bool {
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
    {
        return false;
    }
    if matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::HiddenEntriesCheck
    ) {
        return true;
    }
    !route_introduces_unmentioned_distinctive_context_target_except_workspace_root(
        prompt,
        route_result,
        workspace_root,
    ) && matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::DirectoryEntryGroups
            | crate::OutputSemanticKind::RecentArtifactsJudgment
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
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || raw_command_output_has_explicit_command(state, prompt)
    {
        return false;
    }
    let has_command_payload_marker = route_reason_has_marker(
        route_result,
        "command_payload_requires_raw_output_execution",
    ) || route_reason_has_marker(
        route_result,
        "command_payload_requires_command_output_summary_execution",
    );
    if !has_command_payload_marker
        || (!current_request_has_self_contained_structured_payload(prompt)
            && !raw_command_request_has_structural_input_locator(state, prompt))
    {
        return false;
    }
    crate::contract_matrix::final_answer_shape_for_output_contract(&route_result.output_contract)
        .is_some_and(|shape| shape.allows_model_language())
}

pub(super) fn runtime_status_query_route_can_plan_without_locator(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    route_result: &crate::RouteResult,
) -> bool {
    route_result.is_execute_gate()
        && route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
        && turn_analysis.is_some_and(turn_analysis_has_runtime_status_query)
        && crate::contract_matrix::final_answer_shape_for_output_contract(
            &route_result.output_contract,
        )
        .is_some_and(|shape| shape.allows_model_language())
}

fn current_workspace_route_can_skip_unbound_context_guard(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && !is_bare_topic_only_prompt(prompt)
        && super::semantic_kind_can_bind_workspace_child_locator(
            route_result.output_contract.semantic_kind,
        )
}

pub(super) fn unbound_targeted_evidence_route_should_force_clarify(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    recent_execution_context: &str,
) -> bool {
    if current_workspace_target_binding_should_force_clarify(prompt, route_result, session_snapshot)
        || current_workspace_unmentioned_locator_hint_should_force_clarify(
            prompt,
            route_result,
            session_snapshot,
        )
    {
        return true;
    }
    if current_request_has_concrete_locator_surface(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || session_has_authoritative_deictic_anchor(prompt, route_result, session_snapshot)
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
        && !semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
    {
        if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
            && route_result.output_contract.semantic_kind
                == crate::OutputSemanticKind::DirectoryPurposeSummary
        {
            return false;
        }
        if route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
            && route_result.output_contract.semantic_kind == crate::OutputSemanticKind::None
            && !route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
        {
            return false;
        }
        return true;
    }
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarCount
    ) || matches!(
        route_result.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::DirectoryLookup
            | crate::OutputDeliveryIntent::DirectoryBatchFiles
    )
}

fn current_workspace_unmentioned_locator_hint_should_force_clarify(
    prompt: &str,
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
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
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarCount
    ) || matches!(
        route_result.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::DirectoryLookup
            | crate::OutputDeliveryIntent::DirectoryBatchFiles
    )
}

fn current_workspace_target_binding_should_force_clarify(
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
        || semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind)
        || route_reason_has_marker(
            route_result,
            "compound_file_paths_plus_content_summary_contract_repaired",
        )
        || current_workspace_scope_observation_can_execute_without_locator(prompt, route_result)
    {
        return false;
    }
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarCount
            | crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ContentExcerptWithSummary
    ) || matches!(
        route_result.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::DirectoryLookup
            | crate::OutputDeliveryIntent::DirectoryBatchFiles
    )
}

fn current_workspace_scope_marker_allows_scalar_count_root_hint(
    route_result: &crate::RouteResult,
) -> bool {
    route_reason_has_marker(route_result, "current_workspace_scope_from_current_request")
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && !route_result.output_contract.locator_hint.trim().is_empty()
        && route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount
        && matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::OneSentence
        )
}

fn current_workspace_scope_observation_can_execute_without_locator(
    prompt: &str,
    route_result: &crate::RouteResult,
) -> bool {
    if route_introduces_unmentioned_distinctive_context_target(prompt, route_result) {
        return false;
    }
    if route_result.output_contract.locator_hint.trim().is_empty()
        && matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
        )
    {
        return false;
    }
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
            | crate::OutputSemanticKind::ScalarCount
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::DirectoryEntryGroups
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::WorkspaceProjectSummary
            | crate::OutputSemanticKind::DirectoryPurposeSummary
            | crate::OutputSemanticKind::HiddenEntriesCheck
            | crate::OutputSemanticKind::RecentScalarEqualityCheck
    )
}

pub(super) fn promote_broad_current_workspace_content_summary_to_directory_purpose(
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
        || !matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
        )
        || current_request_has_concrete_locator_surface(prompt)
        || current_request_has_self_contained_structured_payload(prompt)
        || route_introduces_unmentioned_distinctive_context_target(prompt, route_result)
        || route_reason_has_marker(
            route_result,
            "compound_file_paths_plus_content_summary_contract_repaired",
        )
    {
        return false;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    append_route_reason(
        route_result,
        "broad_current_workspace_content_summary_repaired_to_directory_purpose_summary",
    );
    true
}

pub(super) fn repair_directory_purpose_command_summary_contract(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::CurrentWorkspace
        )
        || route_result.output_contract.semantic_kind
            != crate::OutputSemanticKind::CommandOutputSummary
        || command_observation_route_has_runtime_evidence(state, prompt, route_result)
        || route_requests_extension_assess_gap(route_result)
        || !route_reason_has_directory_purpose_repair_token(route_result)
    {
        return false;
    }

    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    if route_result.output_contract.response_shape == crate::OutputResponseShape::Strict {
        route_result.output_contract.response_shape = crate::OutputResponseShape::Free;
    }
    append_route_reason(
        route_result,
        "command_summary_repaired_to_directory_purpose_summary",
    );
    true
}

pub(super) fn repair_directory_purpose_quantity_comparison_contract(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::CurrentWorkspace
        )
        || route_result.output_contract.semantic_kind
            != crate::OutputSemanticKind::QuantityComparison
        || command_observation_route_has_runtime_evidence(state, prompt, route_result)
        || !route_reason_has_marker_prefix(route_result, "llm_semantic_contract_repair")
        || !route_has_single_existing_directory_locator_hint(state, route_result)
    {
        return false;
    }

    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    if route_result.output_contract.response_shape == crate::OutputResponseShape::Strict {
        route_result.output_contract.response_shape = crate::OutputResponseShape::Free;
    }
    append_route_reason(
        route_result,
        "quantity_comparison_repaired_to_directory_purpose_summary",
    );
    true
}

fn route_reason_has_directory_purpose_repair_token(route_result: &crate::RouteResult) -> bool {
    route_result.route_reason.split(';').any(|part| {
        let part = part.trim();
        part.starts_with("llm_semantic_contract_repair:")
            && part.contains("directory_purpose")
            && (part.contains("summary") || part.contains("synthesis"))
    })
}
