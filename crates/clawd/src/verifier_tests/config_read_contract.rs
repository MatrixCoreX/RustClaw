use super::*;

#[test]
fn missing_input_field_is_rejected() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("machine input is intentionally incomplete"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "config_basic".to_string(),
                args: json!({
                    "action": "read_field",
                    "path": "configs/config.toml",
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)
            && issue.detail.contains("`field_path`")
    }));
}

#[test]
fn verifier_preserves_planner_field_choice() {
    let state = test_state();
    let task = test_task();
    let mut output_contract = route_result();
    output_contract.response_shape = crate::OutputResponseShape::Strict;
    output_contract.requires_content_evidence = true;
    output_contract.selection.structured_field_selector = Some("field_path,value".to_string());
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&output_contract),
            request_text: Some("unrelated.machine.token must not replace planner input"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "config_basic".to_string(),
                args: json!({
                    "action": "read_field",
                    "path": "configs/config.toml",
                    "field_path": "llm.selected_vendor",
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert_eq!(
        result.approved_steps[0].args.get("field_path"),
        Some(&json!("llm.selected_vendor"))
    );
    assert_eq!(
        result.approved_steps[0].args.get("path"),
        Some(&json!("configs/config.toml"))
    );
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractPolicyViolation)));
}
