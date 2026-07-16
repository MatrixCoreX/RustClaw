//! Evidence-policy facade for planner/verifier/finalizer boundaries.
//!
//! The older `contract_matrix` module still owns the backing parser and policy
//! tables while the runtime migrates terminology. New production call sites
//! should prefer this facade so evidence policy stays framed as validation and
//! prompt context, not ordinary semantic routing authority.

pub(crate) use crate::contract_matrix::{
    action_matches_policy_tokens, action_trace_for_route, capability_ref_action_policy_for_route,
    capability_ref_action_refs_for_route, capability_ref_replacement_action_policy_for_route,
    compact_prompt_line_for_route, contract_trace_action_key_for_route,
    final_answer_shape_for_output_contract, final_answer_shape_for_route, fnv1a_hex,
    required_evidence_for_output_contract, runtime_contract_snapshot_for_route,
    trace_snapshot_for_route, ActionPolicyDecision, ActionRef, EvidenceExpression,
    FailureAttribution, FinalAnswerShape, FinalAnswerShapeClass,
};

#[cfg(test)]
pub(crate) use crate::contract_matrix::action_policy_for_output_contract;

pub(crate) use crate::task_contract::{
    TaskDeliveryShape as EvidenceDeliveryShape, TaskOperation as EvidenceOperation,
    TaskTargetObject as EvidenceTargetObject,
};

pub(crate) fn evidence_policy_context_prompt_line_for_route(route: &crate::RouteResult) -> String {
    crate::task_contract::evidence_policy_context_prompt_line_for_route(route)
}

pub(crate) fn evidence_expression_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> Option<EvidenceExpression> {
    crate::contract_matrix::bundled_contract_matrix()
        .and_then(|matrix| matrix.match_output_contract(output_contract))
        .map(|matched| matched.evidence_expression())
}

#[allow(dead_code)]
pub(crate) fn delivery_shape_for_route(route: &crate::RouteResult) -> EvidenceDeliveryShape {
    crate::task_contract::delivery_shape_for_route(route)
}

pub(crate) fn missing_parameters_for_route(route: &crate::RouteResult) -> Vec<String> {
    crate::task_contract::missing_parameters_for_route(route)
}

pub(crate) fn operation_for_route(route: &crate::RouteResult) -> EvidenceOperation {
    crate::task_contract::operation_for_route(route)
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

pub(crate) fn required_evidence_fields_for_route(route: &crate::RouteResult) -> Vec<String> {
    crate::task_contract::required_evidence_fields_for_route(route)
}

#[allow(dead_code)]
pub(crate) fn target_object_for_route(route: &crate::RouteResult) -> EvidenceTargetObject {
    crate::task_contract::target_object_for_route(route)
}

pub(crate) fn target_object_for_output_contract(
    output_contract: &crate::IntentOutputContract,
) -> EvidenceTargetObject {
    crate::task_contract::target_object_for_output_contract(output_contract)
}

pub(crate) fn target_locators_for_route(route: &crate::RouteResult) -> Vec<String> {
    crate::task_contract::target_locators_for_route(route)
}
