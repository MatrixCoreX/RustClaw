use crate::{OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape, OutputSemanticKind};

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
        && !matches!(
            route_result.output_contract.semantic_kind,
            OutputSemanticKind::GeneratedFileDelivery
                | OutputSemanticKind::GeneratedFilePathReport
                | OutputSemanticKind::FilesystemMutationResult
        )
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
        && route_result.output_contract.semantic_kind == OutputSemanticKind::GeneratedFileDelivery
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
        && route_result.output_contract.semantic_kind == OutputSemanticKind::ScalarPathOnly
        && !route_result.output_contract.delivery_required
        && route_result.output_contract.locator_kind == OutputLocatorKind::Path
        && route_result.output_contract.locator_hint.trim().is_empty()
}
