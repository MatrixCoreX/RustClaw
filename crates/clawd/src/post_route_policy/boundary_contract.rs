use crate::{
    ActFinalizeStyle, IntentOutputContract, OutputLocatorKind, OutputResponseShape, RouteResult,
};

fn route_reason_has_marker(route_result: &RouteResult, marker: &str) -> bool {
    route_result
        .route_reason
        .split(';')
        .any(|part| part.trim() == marker)
}

pub(crate) fn content_evidence_execution_finalize_style(
    contract: &IntentOutputContract,
    needs_clarify: bool,
) -> Option<ActFinalizeStyle> {
    if needs_clarify || !contract.requires_content_evidence {
        return None;
    }
    if matches!(contract.locator_kind, OutputLocatorKind::None)
        && !contract.delivery_required
        && !matches!(
            contract.response_shape,
            OutputResponseShape::Scalar | OutputResponseShape::FileToken
        )
    {
        return None;
    }
    if let Some(style) = contract_matrix_finalize_style(contract) {
        return Some(style);
    }
    if matches!(
        contract.response_shape,
        OutputResponseShape::Scalar | OutputResponseShape::FileToken
    ) {
        Some(ActFinalizeStyle::Plain)
    } else {
        Some(ActFinalizeStyle::ChatWrapped)
    }
}

fn contract_matrix_finalize_style(contract: &IntentOutputContract) -> Option<ActFinalizeStyle> {
    let shape = crate::contract_matrix::final_answer_shape_for_output_contract(contract)?;
    match shape.class() {
        crate::contract_matrix::FinalAnswerShapeClass::DeliveryArtifact
        | crate::contract_matrix::FinalAnswerShapeClass::ScalarValue
        | crate::contract_matrix::FinalAnswerShapeClass::SinglePath
        | crate::contract_matrix::FinalAnswerShapeClass::StrictList
        | crate::contract_matrix::FinalAnswerShapeClass::Table => Some(ActFinalizeStyle::Plain),
        crate::contract_matrix::FinalAnswerShapeClass::Freeform
        | crate::contract_matrix::FinalAnswerShapeClass::GroundedSummary
        | crate::contract_matrix::FinalAnswerShapeClass::Verdict => {
            Some(ActFinalizeStyle::ChatWrapped)
        }
    }
}

pub(super) fn should_force_content_evidence_for_path_bound_chat_wrapped_execution(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    if route_result.output_contract.delivery_required
        || !route_result.ask_mode.finalize_chat_wrapped()
        || !matches!(
            route_result.output_contract.response_shape,
            OutputResponseShape::Free | OutputResponseShape::OneSentence
        )
    {
        return false;
    }

    match route_result.output_contract.locator_kind {
        OutputLocatorKind::Path | OutputLocatorKind::CurrentWorkspace => {
            direct_locator_path.is_some_and(super::boundary_locator::path_is_existing_directory)
        }
        _ => false,
    }
}

pub(super) fn should_clear_scalar_count_marker_for_non_scalar_contract(
    route_result: &RouteResult,
) -> bool {
    route_reason_has_marker(route_result, "scalar_count")
        && !scalar_count_contract_allows_count_shape(&route_result.output_contract)
}

fn scalar_count_contract_allows_count_shape(contract: &IntentOutputContract) -> bool {
    matches!(
        contract.response_shape,
        OutputResponseShape::Scalar | OutputResponseShape::OneSentence
    ) || (contract.response_shape == OutputResponseShape::Strict
        && contract.exact_sentence_count == Some(1))
}

pub(super) fn should_clear_scalar_path_marker_without_locator_binding(
    route_result: &RouteResult,
) -> bool {
    if !route_reason_has_marker(route_result, "scalar_path_only")
        || route_result.output_contract.response_shape != OutputResponseShape::Scalar
        || route_result.output_contract.delivery_required
    {
        return false;
    }
    route_result.output_contract.locator_kind == OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
}
