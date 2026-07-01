use crate::{FirstLayerDecision, IntentOutputContract};

#[derive(Debug, Clone)]
pub(crate) struct RouteTraceRecord {
    pub(crate) owner_layer: &'static str,
    pub(crate) reason_code: &'static str,
    pub(crate) outcome: &'static str,
    pub(crate) route_trace_decision: FirstLayerDecision,
    pub(crate) needs_clarify: bool,
    pub(crate) output_contract_ref: String,
    pub(crate) repair_codes: Vec<String>,
    pub(crate) repair_classes: Vec<String>,
}

fn route_trace_output_contract_ref(contract: &IntentOutputContract) -> String {
    format!(
        "shape={};semantic={};locator={};delivery_required={};content_evidence={}",
        contract.response_shape.as_str(),
        contract.semantic_kind.as_str(),
        contract.locator_kind.as_str(),
        contract.delivery_required,
        contract.requires_content_evidence
    )
}

fn route_trace_reason_code(
    route_trace_decision: FirstLayerDecision,
    needs_clarify: bool,
) -> &'static str {
    if needs_clarify && route_trace_decision == FirstLayerDecision::Clarify {
        return "route_trace_clarify_required";
    }
    match route_trace_decision {
        FirstLayerDecision::DirectAnswer => "direct_answer_trace_inferred",
        FirstLayerDecision::PlannerExecute => "planner_execute_trace_inferred",
        FirstLayerDecision::Clarify => "clarify_trace_inferred",
    }
}

fn route_trace_outcome(
    route_trace_decision: FirstLayerDecision,
    needs_clarify: bool,
) -> &'static str {
    if needs_clarify && route_trace_decision == FirstLayerDecision::Clarify {
        "blocked"
    } else {
        "allowed"
    }
}

pub(crate) fn route_trace_record(
    route_trace_decision: FirstLayerDecision,
    needs_clarify: bool,
    output_contract: &IntentOutputContract,
    repair_codes: Vec<String>,
) -> RouteTraceRecord {
    let repair_classes = classify_route_trace_repair_codes(&repair_codes);
    RouteTraceRecord {
        owner_layer: "intent_normalizer_route_trace",
        reason_code: route_trace_reason_code(route_trace_decision, needs_clarify),
        outcome: route_trace_outcome(route_trace_decision, needs_clarify),
        route_trace_decision,
        needs_clarify,
        output_contract_ref: route_trace_output_contract_ref(output_contract),
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

fn classify_route_trace_repair_codes(codes: &[String]) -> Vec<String> {
    let mut classes = Vec::new();
    for code in codes {
        push_unique_repair_code(&mut classes, route_trace_repair_code_class(code));
    }
    if classes.is_empty() {
        classes.push("none".to_string());
    }
    classes
}

fn route_trace_repair_code_class(code: &str) -> &'static str {
    match code {
        "execution_signal_promoted_by_output_contract"
        | "execution_recipe_command_payload"
        | "execution_recipe_enum"
        | "execution_recipe_health_check_observation"
        | "execution_recipe_package_detect_manager_capability"
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
        "direct_answer_decision_overridden_by_executable_contract"
        | "executable_contract_preserved_for_agent_loop" => "contract_execution_signal",
        "archive_unpack_missing_archive_locator_requires_clarify"
        | "current_turn_anchor_overrides_contextual_target"
        | "current_turn_locator_overrides_contextual_path"
        | "missing_active_task_reuse_requires_clarify"
        | "output_contract_locator_kind_normalized"
        | "workspace_scope_patch_locator_hint" => "machine_locator_repair",
        "execution_recipe_untrusted_text_ignored"
        | "output_contract_unknown_semantic_ignored"
        | "unbound_workspace_generic_content_requires_clarify" => "boundary_safety_repair",
        _ => "machine_repair_unclassified",
    }
}

#[cfg(test)]
mod tests {
    use super::route_trace_record;
    use crate::{
        FirstLayerDecision, IntentOutputContract, OutputLocatorKind, OutputResponseShape,
        OutputSemanticKind,
    };

    #[test]
    fn route_trace_record_classifies_machine_trace_decisions() {
        let contract = IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            semantic_kind: OutputSemanticKind::FilePaths,
            locator_kind: OutputLocatorKind::Path,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        };

        let execute = route_trace_record(
            FirstLayerDecision::PlannerExecute,
            false,
            &contract,
            vec!["direct_answer_decision_overridden_by_executable_contract".to_string()],
        );
        assert_eq!(execute.reason_code, "planner_execute_trace_inferred");
        assert_eq!(execute.outcome, "allowed");
        assert_eq!(execute.owner_layer, "intent_normalizer_route_trace");
        assert!(execute.output_contract_ref.contains("semantic=file_paths"));
        assert_eq!(
            execute.repair_codes,
            vec!["direct_answer_decision_overridden_by_executable_contract"]
        );
        assert_eq!(execute.repair_classes, vec!["contract_execution_signal"]);

        let allowed = route_trace_record(
            FirstLayerDecision::DirectAnswer,
            false,
            &IntentOutputContract::default(),
            Vec::new(),
        );
        assert_eq!(allowed.reason_code, "direct_answer_trace_inferred");
        assert_eq!(allowed.outcome, "allowed");
        assert_eq!(allowed.repair_classes, vec!["none"]);

        let mixed = route_trace_record(
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

        let clarify = route_trace_record(
            FirstLayerDecision::Clarify,
            true,
            &IntentOutputContract::default(),
            Vec::new(),
        );
        assert_eq!(clarify.reason_code, "route_trace_clarify_required");
        assert_eq!(clarify.outcome, "blocked");
    }
}
