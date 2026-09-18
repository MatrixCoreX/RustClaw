use super::*;

#[test]
fn safe_make_dir_missing_path_defaults_under_workspace_without_confirmation() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("帮我创建一个文件夹"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "make_dir".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved);
    assert!(!result.needs_confirmation);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
    let path = result.approved_steps[0]
        .args
        .get("path")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    assert!(path.starts_with(state.skill_rt.workspace_root.to_string_lossy().as_ref()));
    assert!(path.contains("agent-created-dir-taskveri"));
}

#[test]
fn safe_write_file_relative_path_anchors_under_workspace_without_confirmation() {
    let state = test_state();
    let task = test_task();
    let filename = format!("agent-runtime-autonomy-{}.txt", uuid::Uuid::new_v4());
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("把结果写到文件"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "write_file".to_string(),
                args: json!({ "path": filename, "content": "ok" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved);
    assert!(!result.needs_confirmation);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    let path = result.approved_steps[0]
        .args
        .get("path")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    assert!(path.starts_with(state.skill_rt.workspace_root.to_string_lossy().as_ref()));
    assert!(path.ends_with(".txt"));
}

#[test]
fn runtime_owned_write_target_is_not_treated_as_safe_autonomous_creation() {
    let state = test_state();
    let task = test_task();
    let path = state
        .skill_rt
        .workspace_root
        .join(claw_core::workspace_state::WORKSPACE_STATE_DIR_NAME)
        .join("generated")
        .join(format!("result-{}.txt", uuid::Uuid::new_v4()));
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("把结果写到文件"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "write_file".to_string(),
                args: json!({ "path": path, "content": "ok" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(result.needs_confirmation, "issues: {:?}", result.issues);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}
