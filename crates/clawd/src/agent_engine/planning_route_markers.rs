use crate::RouteResult;

pub(in crate::agent_engine) fn route_reason_has_structural_marker(
    route: &RouteResult,
    marker: &str,
) -> bool {
    route.has_route_reason_machine_marker(marker)
}

pub(in crate::agent_engine) fn route_allows_structured_candidate_read_target_repair(
    route: &RouteResult,
) -> bool {
    [
        "single_path_field_extraction_semantic_kind_none_is_valid",
        "structured_field_selector_requires_scalar_value",
        "structured_keys_scalar_response_requires_field_value",
        "structured_identifier_presence_requires_content_evidence",
    ]
    .iter()
    .any(|marker| route_reason_has_structural_marker(route, marker))
}

pub(in crate::agent_engine) fn route_has_unresolved_clarify_or_locator_marker(
    route: &RouteResult,
) -> bool {
    if route.needs_clarify {
        return true;
    }
    let has_unresolved_machine_token = crate::RouteReasonMarkers::new(&route.route_reason)
        .any_part(|part| {
            part.starts_with("clarify_reason_code:missing_")
                || part.contains("needs_clarify=true")
                || part.contains("missing_locator")
        });
    has_unresolved_machine_token
        || ["locator_required_for_path_scoped_content"]
            .iter()
            .any(|marker| route_reason_has_structural_marker(route, marker))
}
