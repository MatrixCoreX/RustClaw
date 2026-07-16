use super::*;

#[test]
fn external_workspace_scope_persisted_target_allows_followup_validation_plan() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result(false).output_contract),
            request_text: Some("继续修外部工作区里的项目，并验证通过。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({
                    "command": "cargo check",
                    "_clawd_validation": {
                        "profile": "code_change",
                        "validator_type": "build",
                        "validated_target": "external_workspace"
                    }
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState {
                kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                target_scope:
                    crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
                phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
                inspect_first: true,
                validation_required: true,
                max_repairs: 2,
                saw_inspect: true,
                saw_mutation: true,
                saw_external_target: true,
                ..Default::default()
            },
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.issues.iter().all(|issue| {
        !matches!(
            issue.kind,
            VerifyIssueKind::RecipeTargetScopeRequired
                | VerifyIssueKind::RecipeValidationAfterMutateRequired
                | VerifyIssueKind::RecipeInspectBeforeMutateRequired
        )
    }));
}

#[test]
fn greenfield_scope_persisted_creation_allows_followup_validation_plan() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result(false).output_contract),
            request_text: Some("继续验证刚创建的新项目。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({
                    "command": "cargo check -p clawd",
                    "_clawd_validation": {
                        "profile": "code_change",
                        "validator_type": "build",
                        "validated_target": "greenfield_project"
                    }
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState {
                kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
                phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
                inspect_first: true,
                validation_required: true,
                max_repairs: 2,
                saw_inspect: true,
                saw_mutation: true,
                saw_greenfield_creation: true,
                ..Default::default()
            },
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.issues.iter().all(|issue| {
        !matches!(
            issue.kind,
            VerifyIssueKind::RecipeTargetScopeRequired
                | VerifyIssueKind::RecipeValidationAfterMutateRequired
                | VerifyIssueKind::RecipeInspectBeforeMutateRequired
        )
    }));
}
