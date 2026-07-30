use serde_json::json;

use super::*;

#[test]
fn dangerous_remove_file_missing_path_blocks_without_default_target() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("delete it"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "remove_file".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
}

#[test]
fn dangerous_fs_basic_remove_path_missing_path_blocks_without_default_target() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("remove that path"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({ "action": "remove_path" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
}
