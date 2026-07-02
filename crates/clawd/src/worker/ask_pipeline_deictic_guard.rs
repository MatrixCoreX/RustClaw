use super::*;

#[cfg(test)]
#[path = "ask_pipeline_deictic_guard_tests.rs"]
mod tests;

pub(super) fn deictic_bare_locator_should_defer_to_agent_loop(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    if state_patch_allows_deictic_locator_guard_bypass(turn_analysis) {
        return false;
    }
    if !state_patch_requires_deictic_locator_clarify(turn_analysis) {
        return false;
    }
    if route_locator_hint_matches_active_ordered_entry(route_result, session_snapshot) {
        return false;
    }
    let locator_hint = route_result.output_contract.locator_hint.trim();
    let locator_hint_is_inferred_relative_path = locator_hint_is_relative_path_like(locator_hint);
    (!crate::worker::has_explicit_path_or_url_locator_hint(locator_hint)
        || locator_hint_is_inferred_relative_path)
        && route_result.output_contract.requires_content_evidence
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::CurrentWorkspace
                | crate::OutputLocatorKind::Filename
        )
}

pub(super) fn route_locator_hint_matches_active_ordered_entry(
    route_result: &crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
) -> bool {
    let locator_hint = route_result.output_contract.locator_hint.trim();
    if locator_hint.is_empty() {
        return false;
    }
    let Some(frame) = session_snapshot.active_followup_frame.as_ref() else {
        return false;
    };
    frame.ordered_entries.iter().any(|entry| {
        text_mentions_locator_identity(locator_hint, entry)
            || text_mentions_locator_identity(entry, locator_hint)
    })
}

pub(super) fn state_patch_allows_deictic_locator_guard_bypass(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(state_patch) = turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()) else {
        return false;
    };
    if state_patch
        .get("current_result_ref")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    state_patch
        .get("deictic_reference")
        .and_then(serde_json::Value::as_object)
        .and_then(|obj| obj.get("target"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|target| {
            matches!(
                target,
                "current_action_result" | "current_turn_locator" | "comparison_result"
            )
        })
}

pub(super) fn state_patch_requires_deictic_locator_clarify(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .and_then(|state_patch| state_patch.get("deictic_reference"))
        .and_then(serde_json::Value::as_object)
        .and_then(|obj| obj.get("target"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|target| {
            matches!(
                target,
                "unresolved_prior_object" | "missing_locator" | "ambiguous_locator"
            )
        })
}

fn locator_hint_is_relative_path_like(locator_hint: &str) -> bool {
    let hint = locator_hint.trim();
    !hint.is_empty()
        && !hint.starts_with('/')
        && !hint.starts_with("~/")
        && !hint.starts_with("http://")
        && !hint.starts_with("https://")
        && !hint.contains(":\\")
        && (hint.contains('/') || hint.contains('\\'))
}

#[cfg(test)]
pub(super) fn deictic_missing_locator_reason_code(
    route_result: &crate::RouteResult,
) -> &'static str {
    if route_reason_has_marker(route_result, "target_locator_required")
        || route_reason_has_marker(route_result, "missing_count_target")
    {
        return "missing_count_target";
    }
    if matches!(
        route_result.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
            | crate::OutputDeliveryIntent::DirectoryLookup
            | crate::OutputDeliveryIntent::DirectoryBatchFiles
    ) {
        return "missing_delivery_locator";
    }
    if route_reason_has_marker(route_result, "service_status")
        || route_reason_has_marker(route_result, "missing_service_target")
    {
        return "missing_service_target";
    }
    if route_reason_has_marker(route_result, "search_locator_required")
        || route_reason_has_marker(route_result, "missing_search_locator")
    {
        return "missing_search_locator";
    }
    if route_result.output_contract.requires_content_evidence {
        return "missing_read_target";
    }
    "missing_target"
}
