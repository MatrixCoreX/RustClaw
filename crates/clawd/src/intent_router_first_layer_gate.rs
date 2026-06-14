use crate::{FirstLayerDecision, IntentOutputContract};

#[derive(Debug, Clone)]
pub(crate) struct FirstLayerDecisionGateRecord {
    pub(crate) owner_layer: &'static str,
    pub(crate) reason_code: &'static str,
    pub(crate) outcome: &'static str,
    pub(crate) source_decision: Option<FirstLayerDecision>,
    pub(crate) final_decision: FirstLayerDecision,
    pub(crate) needs_clarify: bool,
    pub(crate) output_contract_ref: String,
    pub(crate) repair_codes: Vec<String>,
}

fn first_layer_output_contract_ref(contract: &IntentOutputContract) -> String {
    format!(
        "shape={};semantic={};locator={};delivery_required={};content_evidence={}",
        contract.response_shape.as_str(),
        contract.semantic_kind.as_str(),
        contract.locator_kind.as_str(),
        contract.delivery_required,
        contract.requires_content_evidence
    )
}

fn first_layer_gate_reason_code(
    source_decision: Option<FirstLayerDecision>,
    final_decision: FirstLayerDecision,
    needs_clarify: bool,
) -> &'static str {
    if needs_clarify && final_decision == FirstLayerDecision::Clarify {
        return "first_layer_clarify_required";
    }
    match (source_decision, final_decision) {
        (None, FirstLayerDecision::DirectAnswer) => "direct_answer_preflight_inferred",
        (None, FirstLayerDecision::PlannerExecute) => "planner_execute_preflight_inferred",
        (None, FirstLayerDecision::Clarify) => "clarify_preflight_inferred",
        (Some(FirstLayerDecision::DirectAnswer), FirstLayerDecision::DirectAnswer) => {
            "direct_answer_preflight_allowed"
        }
        (Some(FirstLayerDecision::DirectAnswer), FirstLayerDecision::PlannerExecute) => {
            "direct_answer_preflight_promoted_for_execution_contract"
        }
        (Some(FirstLayerDecision::DirectAnswer), FirstLayerDecision::Clarify) => {
            "direct_answer_preflight_blocked_for_clarify"
        }
        (Some(FirstLayerDecision::PlannerExecute), FirstLayerDecision::DirectAnswer)
        | (Some(FirstLayerDecision::Clarify), FirstLayerDecision::DirectAnswer) => {
            "direct_answer_preflight_downgraded_to_direct_answer"
        }
        (Some(FirstLayerDecision::PlannerExecute), FirstLayerDecision::PlannerExecute) => {
            "planner_execute_preflight_allowed"
        }
        (Some(FirstLayerDecision::Clarify), FirstLayerDecision::Clarify) => {
            "clarify_preflight_allowed"
        }
        (Some(_), FirstLayerDecision::PlannerExecute) => "planner_execute_preflight_repaired",
        (Some(_), FirstLayerDecision::Clarify) => "clarify_preflight_repaired",
    }
}

fn first_layer_gate_outcome(
    source_decision: Option<FirstLayerDecision>,
    final_decision: FirstLayerDecision,
    needs_clarify: bool,
) -> &'static str {
    if needs_clarify && final_decision == FirstLayerDecision::Clarify {
        "blocked"
    } else if source_decision.is_some_and(|source| source != final_decision) {
        "repaired"
    } else {
        "allowed"
    }
}

pub(crate) fn first_layer_decision_gate_record(
    source_decision: Option<FirstLayerDecision>,
    final_decision: FirstLayerDecision,
    needs_clarify: bool,
    output_contract: &IntentOutputContract,
    repair_codes: Vec<String>,
) -> FirstLayerDecisionGateRecord {
    FirstLayerDecisionGateRecord {
        owner_layer: "intent_normalizer_first_layer_gate",
        reason_code: first_layer_gate_reason_code(source_decision, final_decision, needs_clarify),
        outcome: first_layer_gate_outcome(source_decision, final_decision, needs_clarify),
        source_decision,
        final_decision,
        needs_clarify,
        output_contract_ref: first_layer_output_contract_ref(output_contract),
        repair_codes,
    }
}

pub(crate) fn push_unique_repair_code(codes: &mut Vec<String>, code: &str) {
    let code = code.trim();
    if !code.is_empty() && !codes.iter().any(|existing| existing == code) {
        codes.push(code.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::first_layer_decision_gate_record;
    use crate::{
        FirstLayerDecision, IntentOutputContract, OutputLocatorKind, OutputResponseShape,
        OutputSemanticKind,
    };

    #[test]
    fn first_layer_gate_record_classifies_direct_answer_preflight_transitions() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            semantic_kind: OutputSemanticKind::FilePaths,
            locator_kind: OutputLocatorKind::Path,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };

        let promoted = first_layer_decision_gate_record(
            Some(FirstLayerDecision::DirectAnswer),
            FirstLayerDecision::PlannerExecute,
            false,
            &contract,
            vec!["direct_answer_decision_overridden_by_executable_contract".to_string()],
        );
        assert_eq!(
            promoted.reason_code,
            "direct_answer_preflight_promoted_for_execution_contract"
        );
        assert_eq!(promoted.outcome, "repaired");
        assert_eq!(promoted.owner_layer, "intent_normalizer_first_layer_gate");
        assert!(promoted.output_contract_ref.contains("semantic=file_paths"));
        assert_eq!(
            promoted.repair_codes,
            vec!["direct_answer_decision_overridden_by_executable_contract"]
        );

        let allowed = first_layer_decision_gate_record(
            Some(FirstLayerDecision::DirectAnswer),
            FirstLayerDecision::DirectAnswer,
            false,
            &IntentOutputContract::default(),
            Vec::new(),
        );
        assert_eq!(allowed.reason_code, "direct_answer_preflight_allowed");
        assert_eq!(allowed.outcome, "allowed");
    }
}
