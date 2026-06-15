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
    pub(crate) repair_classes: Vec<String>,
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
    let repair_classes = classify_first_layer_repair_codes(&repair_codes);
    FirstLayerDecisionGateRecord {
        owner_layer: "intent_normalizer_first_layer_gate",
        reason_code: first_layer_gate_reason_code(source_decision, final_decision, needs_clarify),
        outcome: first_layer_gate_outcome(source_decision, final_decision, needs_clarify),
        source_decision,
        final_decision,
        needs_clarify,
        output_contract_ref: first_layer_output_contract_ref(output_contract),
        repair_codes,
        repair_classes,
    }
}

pub(crate) fn push_unique_repair_code(codes: &mut Vec<String>, code: &str) {
    let code = code.trim();
    if !code.is_empty() && !codes.iter().any(|existing| existing == code) {
        codes.push(code.to_string());
    }
}

fn classify_first_layer_repair_codes(codes: &[String]) -> Vec<String> {
    let mut classes = Vec::new();
    for code in codes {
        push_unique_repair_code(&mut classes, first_layer_repair_code_class(code));
    }
    if classes.is_empty() {
        classes.push("none".to_string());
    }
    classes
}

fn first_layer_repair_code_class(code: &str) -> &'static str {
    match code {
        "decision_promoted_by_output_contract"
        | "execution_recipe_command_payload"
        | "execution_recipe_enum"
        | "execution_recipe_health_check_observation"
        | "execution_recipe_package_manager_detection"
        | "execution_recipe_scalar_runtime_tool_observation"
        | "execution_recipe_service_status_observation"
        | "execution_recipe_structured_read_observation"
        | "file_delivery_contract_repaired"
        | "output_contract_delivery_intent_normalized"
        | "output_contract_response_shape_normalized"
        | "output_contract_semantic_kind_normalized"
        | "raw_output_explicit_locator_contract_repaired"
        | "target_task_policy_enum_normalized"
        | "turn_type_enum_normalized" => "schema_normalization",
        "answer_candidate_path_requires_evidence"
        | "archive_unpack_missing_archive_locator_requires_clarify"
        | "current_turn_anchor_overrides_contextual_target"
        | "current_turn_locator_overrides_contextual_path"
        | "missing_active_task_reuse_requires_clarify"
        | "output_contract_locator_kind_normalized"
        | "workspace_scope_patch_locator_hint" => "machine_locator_repair",
        "execution_recipe_untrusted_text_ignored"
        | "output_contract_unknown_semantic_ignored"
        | "unbound_workspace_generic_content_requires_clarify" => "boundary_safety_repair",
        _ => "legacy_semantic_reroute",
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
        assert_eq!(promoted.repair_classes, vec!["legacy_semantic_reroute"]);

        let allowed = first_layer_decision_gate_record(
            Some(FirstLayerDecision::DirectAnswer),
            FirstLayerDecision::DirectAnswer,
            false,
            &IntentOutputContract::default(),
            Vec::new(),
        );
        assert_eq!(allowed.reason_code, "direct_answer_preflight_allowed");
        assert_eq!(allowed.outcome, "allowed");
        assert_eq!(allowed.repair_classes, vec!["none"]);

        let mixed = first_layer_decision_gate_record(
            Some(FirstLayerDecision::PlannerExecute),
            FirstLayerDecision::PlannerExecute,
            false,
            &contract,
            vec![
                "output_contract_response_shape_normalized".to_string(),
                "output_contract_locator_kind_normalized".to_string(),
                "execution_recipe_untrusted_text_ignored".to_string(),
            ],
        );
        assert_eq!(
            mixed.repair_classes,
            vec![
                "schema_normalization",
                "machine_locator_repair",
                "boundary_safety_repair"
            ]
        );
    }
}
