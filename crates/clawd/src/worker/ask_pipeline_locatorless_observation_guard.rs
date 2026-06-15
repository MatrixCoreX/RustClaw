use super::*;

#[cfg(test)]
#[path = "ask_pipeline_locatorless_observation_guard_tests.rs"]
mod tests;

pub(super) fn semantic_kind_can_execute_without_locator(kind: crate::OutputSemanticKind) -> bool {
    matches!(
        kind,
        crate::OutputSemanticKind::RawCommandOutput
            | crate::OutputSemanticKind::CommandOutputSummary
            | crate::OutputSemanticKind::ExecutionFailedStep
            | crate::OutputSemanticKind::ServiceStatus
            | crate::OutputSemanticKind::HiddenEntriesCheck
            | crate::OutputSemanticKind::WorkspaceProjectSummary
            | crate::OutputSemanticKind::RecentScalarEqualityCheck
            | crate::OutputSemanticKind::GitCommitSubject
            | crate::OutputSemanticKind::GitRepositoryState
            | crate::OutputSemanticKind::RssNewsFetch
            | crate::OutputSemanticKind::WebSearchSummary
            | crate::OutputSemanticKind::WeatherQuery
            | crate::OutputSemanticKind::MarketQuote
            | crate::OutputSemanticKind::ImageUnderstanding
            | crate::OutputSemanticKind::PublishingPreview
            | crate::OutputSemanticKind::PackageManagerDetection
            | crate::OutputSemanticKind::ToolDiscovery
            | crate::OutputSemanticKind::DockerPs
            | crate::OutputSemanticKind::DockerImages
            | crate::OutputSemanticKind::DockerLogs
            | crate::OutputSemanticKind::DockerContainerLifecycle
    )
}

pub(super) fn promote_locatorless_scalar_child_metadata_to_quantity_comparison(
    state: &AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(prompt);
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || surface.token_count <= 1
        || surface.has_explicit_path_or_url()
        || surface.has_filename_candidates()
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        )
        || !matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::RawCommandOutput
        )
        || (route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::RawCommandOutput
            && route_reason_has_marker(
                route_result,
                "command_payload_requires_raw_output_execution",
            ))
        || raw_command_output_has_explicit_command(state, prompt)
    {
        return false;
    }
    let Some(path) = current_request_resolves_workspace_child_locator(state, prompt) else {
        return false;
    };
    let resolved_path = std::path::Path::new(&path);
    if resolved_path.is_file()
        && route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
    {
        return false;
    }
    if resolved_path.is_file()
        && route_result.output_contract.semantic_kind == crate::OutputSemanticKind::None
        && route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
    {
        return false;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = path;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route_result.output_contract.requires_content_evidence = true;
    route_result.set_planner_execute_finalize(
        crate::post_route_policy::content_evidence_execution_finalize_style(
            &route_result.output_contract,
            false,
        )
        .unwrap_or(crate::ActFinalizeStyle::ChatWrapped),
    );
    append_route_reason(
        route_result,
        "locatorless_scalar_child_metadata_promoted_to_quantity_comparison",
    );
    true
}

pub(super) fn promote_locatorless_git_capability_to_repository_state(
    route_result: &mut crate::RouteResult,
) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
        || !matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::CommandOutputSummary
        )
        || !route_mentions_git_capability(route_result)
    {
        return false;
    }

    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::GitRepositoryState;
    append_route_reason(
        route_result,
        "locatorless_git_capability_promoted_to_git_repository_state",
    );
    true
}

fn route_mentions_git_capability(route_result: &crate::RouteResult) -> bool {
    ascii_token_present(&route_result.resolved_intent, "git")
        || ascii_token_present(&route_result.route_reason, "git")
}

fn ascii_token_present(text: &str, token: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .any(|candidate| candidate.eq_ignore_ascii_case(token))
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
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
            | crate::OutputSemanticKind::CommandOutputSummary
            | crate::OutputSemanticKind::ExecutionFailedStep
    ) && (raw_command_output_has_explicit_command(state, prompt)
        || route_reason_has_marker(
            route_result,
            "explicit_command_requires_command_output_summary_execution",
        )
        || route_reason_has_marker(
            route_result,
            "command_payload_requires_raw_output_execution",
        )
        || route_reason_has_marker(
            route_result,
            "command_payload_requires_command_output_summary_execution",
        ))
}

pub(super) fn scalar_raw_runtime_observation_can_plan(route_result: &crate::RouteResult) -> bool {
    if route_result.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        || route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let args = serde_json::json!({
        "action": "runtime_status",
        "kind": "current_user",
    });
    crate::contract_matrix::action_policy_for_output_contract(
        Some(&route_result.output_contract),
        "system_basic",
        &args,
    )
    .is_some_and(|policy| policy.is_allowed())
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
    let raw_command_without_input = route_result.output_contract.semantic_kind
        == crate::OutputSemanticKind::RawCommandOutput
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
        || (has_structural_locator_surface && !raw_command_without_input)
        || has_self_contained_payload
        || state_patch_allows_deictic_locator_guard_bypass(turn_analysis)
        || (has_authoritative_deictic_anchor && !raw_command_without_input)
        || has_structured_session_anchor
    {
        return false;
    }
    if semantic_kind_can_execute_without_locator(route_result.output_contract.semantic_kind) {
        if route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::CommandOutputSummary
        {
            return false;
        }
        if raw_command_output_without_locator_can_plan_via_contract(state, prompt, route_result) {
            return false;
        }
        if runtime_status_query_route_can_plan_without_locator(turn_analysis, route_result) {
            return false;
        }
        if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
            && route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar
            && (turn_analysis.is_some_and(turn_analysis_has_runtime_status_query)
                || route_reason_has_marker(
                    route_result,
                    "execution_recipe_scalar_runtime_tool_observation",
                )
                || scalar_raw_runtime_observation_can_plan(route_result))
        {
            return false;
        }
        return route_result.output_contract.semantic_kind
            == crate::OutputSemanticKind::RawCommandOutput
            && !raw_command_output_has_explicit_command(state, prompt)
            && (!route_reason_has_marker(
                route_result,
                "command_payload_requires_raw_output_execution",
            ) || raw_command_without_input);
    }
    if command_observation_route_has_runtime_evidence(state, prompt, route_result) {
        return false;
    }

    true
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
