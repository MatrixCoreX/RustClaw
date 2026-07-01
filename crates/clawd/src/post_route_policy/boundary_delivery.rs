use crate::{OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape};

fn route_reason_has_marker(route_result: &crate::RouteResult, marker: &str) -> bool {
    route_result
        .route_reason
        .split(';')
        .any(|part| part.trim() == marker)
}

fn generated_or_mutating_delivery_uses_runtime_target(route_result: &crate::RouteResult) -> bool {
    route_reason_has_marker(route_result, "generated_file_delivery")
        || route_reason_has_marker(route_result, "generated_file_path_report")
        || route_reason_has_marker(route_result, "filesystem_mutation_result")
        || route_reason_has_marker(
            route_result,
            "generated_file_delivery_allows_runtime_target",
        )
}

pub(super) fn existing_file_delivery_can_try_locator_hint(
    route_result: &crate::RouteResult,
) -> bool {
    route_result.wants_file_delivery
        && route_result.output_contract.delivery_required
        && route_result.output_contract.response_shape == OutputResponseShape::FileToken
        && route_result.output_contract.delivery_intent == OutputDeliveryIntent::FileSingle
        && route_result.output_contract.requires_content_evidence
        && matches!(
            route_result.output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        )
        && !route_result.output_contract.locator_hint.trim().is_empty()
        && !generated_or_mutating_delivery_uses_runtime_target(route_result)
}

pub(super) fn file_delivery_can_materialize_target_without_existing_locator(
    route_result: &crate::RouteResult,
) -> bool {
    // New-file delivery may choose a filename during planning; an empty locator
    // hint is not necessarily a missing existing-file target.
    route_result.is_execute_gate()
        && !route_result.needs_clarify
        && route_result.wants_file_delivery
        && route_result.output_contract.delivery_required
        && route_result.output_contract.response_shape == OutputResponseShape::FileToken
        && route_result.output_contract.delivery_intent == OutputDeliveryIntent::FileSingle
        && route_result.output_contract.requires_content_evidence
        && generated_or_mutating_delivery_uses_runtime_target(route_result)
        && matches!(
            route_result.output_contract.locator_kind,
            OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
        )
        && route_result.output_contract.locator_hint.trim().is_empty()
}

pub(super) fn scalar_path_output_can_be_observed_without_input_locator(
    route_result: &crate::RouteResult,
) -> bool {
    route_result.is_execute_gate()
        && !route_result.needs_clarify
        && route_result.output_contract.response_shape == OutputResponseShape::Scalar
        && route_reason_has_marker(route_result, "scalar_path_only")
        && !route_result.output_contract.delivery_required
        && route_result.output_contract.locator_kind == OutputLocatorKind::Path
        && route_result.output_contract.locator_hint.trim().is_empty()
}
