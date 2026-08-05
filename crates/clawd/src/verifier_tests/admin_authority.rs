use super::*;

#[test]
fn unrestricted_admin_can_execute_enabled_non_planner_skill() {
    let state = test_state();
    state.seed_test_auth_identity("rk-verifier-admin", "admin");
    let identity = crate::resolve_auth_identity_by_key(&state, "rk-verifier-admin")
        .expect("resolve admin")
        .expect("admin identity");
    let mut payload = json!({"text": "run enabled internal skill"});
    crate::task_execution_policy::stamp_authenticated_submission_policy(
        &mut payload,
        Some(&identity),
        Some("ui"),
        None,
    )
    .expect("stamp admin policy");
    let mut task = test_task();
    task.user_key = Some("rk-verifier-admin".to_string());
    task.channel = "ui".to_string();
    task.payload_json = payload.to_string();

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "admin_hidden_probe".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::SkillNotVisible)));
}
