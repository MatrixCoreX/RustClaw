use super::*;

#[test]
fn code_change_recipe_requires_profile_specific_verification() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result(false).output_contract),
            request_text: Some("修复当前仓库里的 clawd 入口逻辑，并验证编译或测试通过。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s3".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs" }),
                    depends_on: vec!["s2".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                },
            ),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    let issue = result
        .issues
        .iter()
        .find(|issue| {
            matches!(
                issue.kind,
                VerifyIssueKind::RecipeValidationAfterMutateRequired
            )
        })
        .expect("expected code_change validation issue");
    assert!(issue
        .detail
        .contains("code_change requires compile/test/build or runtime verification"));
}

#[test]
fn code_change_recipe_accepts_structured_cargo_check_verification() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result(false).output_contract),
            request_text: Some("修复当前仓库里的 clawd 入口逻辑，并验证编译通过。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s0".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
                    depends_on: vec!["s0".to_string()],
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "cargo check -p clawd",
                        "_clawd_validation": {
                            "profile": "code_change",
                            "validator_type": "build",
                            "validated_target": "clawd"
                        }
                    }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
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
            VerifyIssueKind::RecipeValidationAfterMutateRequired
                | VerifyIssueKind::RecipeInspectBeforeMutateRequired
        )
    }));
}

#[test]
fn code_change_recipe_accepts_run_cmd_cargo_check_verification() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result(false).output_contract),
            request_text: Some("修复当前仓库里的 clawd 入口逻辑，并验证编译通过。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s0".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
                    depends_on: vec!["s0".to_string()],
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({ "command": "cargo check -p clawd" }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                },
            ),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(result.issues.iter().all(|issue| !matches!(
        issue.kind,
        VerifyIssueKind::RecipeValidationAfterMutateRequired
    )));
}

#[test]
fn code_change_recipe_accepts_structured_custom_validation_step() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result(false).output_contract),
            request_text: Some("修复当前仓库里的脚本，并运行自定义检查脚本验证通过。"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s0".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "scripts/check.sh" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": "scripts/check.sh", "content": "#!/usr/bin/env bash\nexit 0\n" }),
                    depends_on: vec!["s0".to_string()],
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "bash scripts/check.sh",
                        "_clawd_validation": {
                            "profile": "code_change",
                            "validator_type": "custom",
                            "validated_target": "scripts/check.sh"
                        }
                    }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
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
            VerifyIssueKind::RecipeValidationAfterMutateRequired
                | VerifyIssueKind::RecipeInspectBeforeMutateRequired
        )
    }));
}
