pub(super) fn prebind_runtime_status_scalar_path_to_current_workspace(
    route_result: &mut crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if !route_result.is_execute_gate()
        || route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarPathOnly
        || route_result.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route_result.output_contract.locator_hint.trim().is_empty()
    {
        return false;
    }
    let runtime_status_kind = turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .and_then(|patch| patch.get("runtime_status_query"))
        .and_then(serde_json::Value::as_object)
        .and_then(|query| query.get("kind"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim);
    let structured_cwd_query = runtime_status_kind.is_some_and(|kind| {
        matches!(
            kind,
            "current_working_directory" | "current_process_cwd" | "process_cwd"
        )
    });
    let status_query_scalar_path = turn_analysis.is_some_and(|analysis| {
        analysis.turn_type == Some(crate::intent_router::TurnType::StatusQuery)
    });
    if !structured_cwd_query
        && !status_query_scalar_path
        && active_ordered_entries_without_structured_ref(session_snapshot, turn_analysis)
    {
        super::append_route_reason(
            route_result,
            "scalar_path_only_missing_ordered_entry_ref_not_bound_to_current_workspace",
        );
        return false;
    }
    if !structured_cwd_query
        && !status_query_scalar_path
        && active_task_scalar_path_without_locator_should_not_bind(turn_analysis)
    {
        super::append_route_reason(
            route_result,
            "scalar_path_only_active_task_update_not_bound_to_current_workspace",
        );
        return false;
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route_result.output_contract.locator_hint.clear();
    let reason = if structured_cwd_query || status_query_scalar_path {
        "runtime_status_scalar_path_bound_to_current_workspace"
    } else {
        "scalar_path_only_without_locator_bound_to_current_workspace"
    };
    super::append_route_reason(route_result, reason);
    true
}

fn active_task_scalar_path_without_locator_should_not_bind(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(analysis) = turn_analysis else {
        return false;
    };
    let active_task_turn = matches!(
        analysis.turn_type,
        Some(
            crate::intent_router::TurnType::TaskAppend
                | crate::intent_router::TurnType::TaskCorrect
                | crate::intent_router::TurnType::TaskReplace
                | crate::intent_router::TurnType::TaskScopeUpdate
        )
    );
    let active_task_policy = matches!(
        analysis.target_task_policy,
        Some(
            crate::intent_router::TargetTaskPolicy::ReuseActive
                | crate::intent_router::TargetTaskPolicy::ReplaceActive
                | crate::intent_router::TargetTaskPolicy::PauseAndQueue
        )
    );
    active_task_turn || active_task_policy
}

fn active_ordered_entries_without_structured_ref(
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let has_active_ordered_entries = session_snapshot
        .active_followup_frame
        .as_ref()
        .is_some_and(|frame| !frame.ordered_entries.is_empty());
    if !has_active_ordered_entries {
        return false;
    }
    !turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(|state_patch| {
            state_patch.get("ordered_entry_ref").is_some()
                || state_patch.get("ordered_entry_reference").is_some()
        })
}

pub(super) fn promote_locatorless_status_query_to_service_status(
    state: &crate::AppState,
    prompt: &str,
    route_result: &mut crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(turn_analysis) = turn_analysis else {
        return false;
    };
    let system_health_selector =
        route_or_turn_has_system_health_selector(route_result, Some(turn_analysis));
    if turn_analysis.turn_type != Some(crate::intent_router::TurnType::StatusQuery)
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
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::RawCommandOutput
        )
    {
        return false;
    }
    let promotable_gate = route_result.is_execute_gate()
        || (route_result.needs_clarify
            && (route_result.clarify_question.trim().is_empty() || system_health_selector));
    if !promotable_gate {
        return false;
    }
    if prompt_is_bare_request_fragment(prompt) {
        return false;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && super::route_reason_has_marker(
            route_result,
            "command_payload_requires_raw_output_execution",
        )
    {
        return false;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && turn_analysis_has_runtime_status_query(turn_analysis)
        && !system_health_selector
    {
        return false;
    }
    if route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar
        && super::route_reason_has_marker(
            route_result,
            "execution_recipe_scalar_runtime_tool_observation",
        )
        && !system_health_selector
    {
        return false;
    }
    if super::raw_command_output_has_explicit_command(state, prompt) {
        return false;
    }

    route_result.needs_clarify = false;
    route_result.clarify_question.clear();
    route_result.set_execute_gate();
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    if system_health_selector
        && route_result
            .output_contract
            .self_extension
            .structured_field_selector
            .is_none()
    {
        route_result
            .output_contract
            .self_extension
            .structured_field_selector = Some("system_health.*".to_string());
    }
    super::append_route_reason(
        route_result,
        if system_health_selector {
            "system_health_selector_promoted_to_service_status"
        } else {
            "locatorless_status_query_promoted_to_service_status"
        },
    );
    true
}

fn prompt_is_bare_request_fragment(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    !trimmed.is_empty()
        && !trimmed.contains('\n')
        && trimmed.split_whitespace().count() <= 1
        && trimmed.chars().any(|ch| ch.is_alphanumeric())
}

pub(super) fn promote_locatorless_scalar_status_query_to_runtime_info(
    route_result: &mut crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(turn_analysis) = turn_analysis else {
        return false;
    };
    if turn_analysis.turn_type != Some(crate::intent_router::TurnType::StatusQuery)
        || !route_result.is_execute_gate()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || route_result.output_contract.delivery_required
        || route_result.wants_file_delivery
        || !route_result.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    route_result.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_result.output_contract.locator_hint.clear();
    super::append_route_reason(
        route_result,
        "execution_recipe_scalar_runtime_tool_observation",
    );
    true
}

pub(super) fn turn_analysis_has_runtime_status_query(
    turn_analysis: &crate::intent_router::TurnAnalysis,
) -> bool {
    turn_analysis
        .state_patch
        .as_ref()
        .and_then(|patch| patch.get("runtime_status_query"))
        .and_then(serde_json::Value::as_object)
        .and_then(|query| query.get("kind"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .is_some_and(|kind| !kind.is_empty())
}

pub(super) fn route_or_turn_has_system_health_selector(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    route_result
        .output_contract
        .self_extension
        .structured_field_selector
        .as_deref()
        .is_some_and(system_health_selector)
        || turn_analysis.is_some_and(turn_analysis_has_system_health_selector)
}

pub(super) fn turn_analysis_has_system_health_selector(
    turn_analysis: &crate::intent_router::TurnAnalysis,
) -> bool {
    turn_analysis
        .state_patch
        .as_ref()
        .and_then(structured_field_selector_from_state_patch)
        .is_some_and(|selector| system_health_selector(&selector))
}

fn structured_field_selector_from_state_patch(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if key == "structured_field_selector"
                    || key == "field_selector"
                    || key == "field_path"
                    || key == "key_path"
                {
                    if let Some(selector) = value.as_str().map(str::trim).filter(|s| !s.is_empty())
                    {
                        return Some(selector.to_string());
                    }
                }
                if let Some(selector) = structured_field_selector_from_state_patch(value) {
                    return Some(selector);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(structured_field_selector_from_state_patch),
        _ => None,
    }
}

fn system_health_selector(selector: &str) -> bool {
    let selector = selector.trim();
    selector == "system_health"
        || selector == "system_health.*"
        || selector
            .strip_prefix("system_health.")
            .is_some_and(|suffix| !suffix.trim().is_empty())
}

#[cfg(test)]
#[path = "ask_pipeline_runtime_status_tests.rs"]
mod tests;
