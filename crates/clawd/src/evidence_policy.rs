//! Evidence-policy facade for planner/verifier/finalizer boundaries.
//!
//! The older `contract_matrix` module still owns the backing parser and policy
//! tables while the runtime migrates terminology. New production call sites
//! should prefer this facade so evidence policy stays framed as validation and
//! prompt context, not ordinary semantic routing authority.

pub(crate) use crate::contract_matrix::{
    arg_policy_decision_for_route, capability_ref_action_policy_for_route,
    capability_ref_action_refs_for_route, compact_prompt_line_for_route,
    final_answer_shape_for_output_contract, final_answer_shape_for_route, fnv1a_hex,
    ActionPolicyDecision, ActionRef, ArgPolicyDecision, FinalAnswerShape, FinalAnswerShapeClass,
};
