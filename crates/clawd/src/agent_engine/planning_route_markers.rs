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

pub(in crate::agent_engine) fn route_has_unresolved_clarify_or_locator_marker(
    route: &RouteResult,
) -> bool {
    if route.needs_clarify {
        return true;
    }
    let has_unresolved_machine_token = route.route_reason.split(';').map(str::trim).any(|part| {
        part.starts_with("clarify_reason_code:missing_")
            || part.contains("needs_clarify=true")
            || part.contains("missing_locator")
    });
    has_unresolved_machine_token
        || [
            "locator_required_for_path_scoped_content",
            "deictic_bare_locator_requires_clarify",
            "deictic_memory_only_requires_clarify",
            "unbound_existing_file_delivery_requires_clarify",
            "unbound_targeted_evidence_requires_clarify",
            "locatorless_observation_requires_clarify",
        ]
        .iter()
        .any(|marker| route_reason_has_structural_marker(route, marker))
}
