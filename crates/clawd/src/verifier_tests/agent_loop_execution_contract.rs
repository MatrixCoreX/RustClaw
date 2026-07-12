use super::*;

#[test]
fn agent_loop_execution_boundary_does_not_require_legacy_output_contract() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result(false);
    route.route_reason =
        "inline_structured_payload_context_execute; executable_contract_preserved_for_agent_loop"
            .to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({"action": "stat_paths", "path": "README.md"}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractMissing)));
}
