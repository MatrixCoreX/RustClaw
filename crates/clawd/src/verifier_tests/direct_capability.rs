use super::*;

#[test]
fn direct_workspace_diff_resolves_and_remains_confirmation_exempt() {
    let state = registry_confirmation::workspace_registry_state();
    let task = test_task();
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "workspace.diff",
        json!({"checkpoint_id": "checkpoint_1"}),
    );

    assert_eq!(plan.steps[0].action_type, "call_tool");
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(plan.steps[0].args["action"], "diff");
    assert_eq!(plan.steps[1].action_type, "synthesize_answer");

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation, "issues: {:?}", result.issues);
}

#[test]
fn direct_workspace_rewind_resolves_but_requires_one_shot_confirmation() {
    let state = registry_confirmation::workspace_registry_state();
    let task = test_task();
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "workspace.revert_checkpoint",
        json!({"checkpoint_id": "checkpoint_1"}),
    );

    assert_eq!(plan.steps[0].action_type, "call_tool");
    assert_eq!(plan.steps[0].skill, "fs_basic");
    assert_eq!(plan.steps[0].args["action"], "rewind");

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
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
    assert_eq!(
        result.permission_decision["decision"],
        crate::policy_decision::PolicyDecision::RequireConfirmation.as_token()
    );
}
