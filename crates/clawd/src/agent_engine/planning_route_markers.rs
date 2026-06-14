use crate::RouteResult;

pub(in crate::agent_engine) fn route_reason_has_structural_marker(
    route: &RouteResult,
    marker: &str,
) -> bool {
    route.route_reason.split(';').map(str::trim).any(|part| {
        part == marker
            || part
                .rsplit_once(':')
                .is_some_and(|(_, suffix)| suffix.trim() == marker)
    })
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
