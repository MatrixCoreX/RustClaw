pub(super) fn append_runtime_status_capability_context(
    route_result: &mut crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if !route_result.output_contract.requires_content_evidence
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
    if route_has_runtime_status_capability_context(route_result) {
        return false;
    }
    let Some(turn_analysis) = turn_analysis else {
        return false;
    };
    let capability = if route_or_turn_has_system_health_selector(route_result, Some(turn_analysis))
    {
        Some(SYSTEM_HEALTH_CHECK_CAPABILITY_REF)
    } else if turn_analysis_has_runtime_status_query(turn_analysis) {
        Some(SYSTEM_RUNTIME_STATUS_CAPABILITY_REF)
    } else {
        None
    };
    if let Some(capability) = capability {
        super::append_route_reason(route_result, capability);
        true
    } else {
        false
    }
}

fn route_has_runtime_status_capability_context(route_result: &crate::RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_action_name(
        route_result,
        &["system"],
        &["runtime_status", "health_check"],
    ) || super::route_reason_has_marker(route_result, LEGACY_HEALTH_CHECK_CAPABILITY_REF)
}

const SYSTEM_HEALTH_CHECK_CAPABILITY_REF: &str =
    concat!("capability_ref=", "system", ".", "health_check");
const SYSTEM_RUNTIME_STATUS_CAPABILITY_REF: &str =
    concat!("capability_ref=", "system", ".", "runtime_status");
const LEGACY_HEALTH_CHECK_CAPABILITY_REF: &str = concat!("capability_ref=", "health_check");

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
