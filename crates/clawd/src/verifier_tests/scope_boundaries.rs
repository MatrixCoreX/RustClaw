use super::*;

#[test]
fn current_repo_scope_rejects_external_target_plan() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("修复当前仓库里的 clawd 入口逻辑，不要动仓库外项目。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "write_file".to_string(),
                args: json!({ "path": "/opt/other-project/main.rs", "content": "fn main() {}\n" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                    inspect_first: false,
                    validation_required: false,
                    max_repairs: 2,
                },
            ),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.issues.iter().any(|issue| {
        matches!(issue.kind, VerifyIssueKind::RecipeTargetScopeRequired)
            && issue
                .detail
                .contains("current_repo scope must stay inside the current workspace")
    }));
}

#[test]
fn external_workspace_scope_requires_explicit_external_target_plan() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("去当前仓库外的另一个项目修问题。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "write_file".to_string(),
                args: json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope:
                        crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
                    inspect_first: false,
                    validation_required: false,
                    max_repairs: 2,
                },
            ),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.issues.iter().any(|issue| {
        matches!(issue.kind, VerifyIssueKind::RecipeTargetScopeRequired)
            && issue
                .detail
                .contains("external_workspace scope requires an explicit external path")
    }));
}

#[test]
fn greenfield_scope_requires_creation_plan() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("从零做一个新脚本并验证。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({ "command": "cargo check -p clawd" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
                    inspect_first: false,
                    validation_required: false,
                    max_repairs: 2,
                },
            ),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.issues.iter().any(|issue| {
        matches!(issue.kind, VerifyIssueKind::RecipeTargetScopeRequired)
            && issue
                .detail
                .contains("greenfield scope requires creating a new file")
    }));
}

#[test]
fn external_workspace_scope_accepts_explicit_external_path_plan() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("去另一个目录修问题，并验证通过。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "/opt/other-project/src/main.rs" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": "/opt/other-project/src/main.rs", "content": "fn main() {}\n" }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s3".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "cd /opt/other-project && cargo check",
                        "_clawd_validation": {
                            "profile": "code_change",
                            "validator_type": "build",
                            "validated_target": "/opt/other-project"
                        }
                    }),
                    depends_on: vec!["s2".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope:
                        crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                },
            ),
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
