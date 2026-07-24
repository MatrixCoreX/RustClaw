//! Evidence-policy facade for planner/verifier/finalizer boundaries.
//!
//! The older `contract_matrix` module still owns the backing parser and policy
//! tables while the runtime migrates terminology. New production call sites
//! should prefer this facade so evidence policy stays framed as validation and
//! prompt context, not ordinary semantic routing authority.

pub(crate) use crate::contract_matrix::{
    action_policy_for_output_contract, action_trace_for_output_contract,
    compact_prompt_line_for_output_contract, final_answer_shape_for_output_contract, fnv1a_hex,
    required_evidence_for_output_contract, runtime_contract_snapshot_for_output_contract,
    trace_snapshot_for_output_contract, ActionPolicyDecision, ActionRef, EvidenceExpression,
    FailureAttribution, FinalAnswerShape, FinalAnswerShapeClass,
};

pub(crate) use crate::task_contract::{
    TaskDeliveryShape as EvidenceDeliveryShape, TaskOperation as EvidenceOperation,
    TaskTargetObject as EvidenceTargetObject,
};

pub(crate) fn evidence_expression_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> Option<EvidenceExpression> {
    if output_contract.requests_exact_structured_fields() {
        if let Some(fields) = output_contract
            .selection
            .structured_field_selector
            .as_deref()
            .and_then(crate::machine_selector::exact_machine_field_selector)
        {
            return Some(EvidenceExpression {
                all_of: fields,
                ..EvidenceExpression::default()
            });
        }
    }
    crate::contract_matrix::bundled_contract_matrix()
        .and_then(|matrix| matrix.match_output_contract(output_contract))
        .map(|matched| matched.evidence_expression())
}

#[allow(dead_code)]
pub(crate) fn delivery_shape_for_output_contract(
    route: &crate::IntentOutputContract,
) -> EvidenceDeliveryShape {
    crate::task_contract::delivery_shape_for_output_contract(route)
}

pub(crate) fn operation_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> EvidenceOperation {
    crate::task_contract::operation_for_output_contract(output_contract)
}

pub(crate) fn required_evidence_fields_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> Vec<String> {
    crate::task_contract::required_evidence_fields_for_output_contract(output_contract)
}

pub(crate) fn target_object_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> EvidenceTargetObject {
    crate::task_contract::target_object_for_output_contract(output_contract)
}
