use super::*;

#[test]
fn destructive_run_cmd_requires_confirmation_without_resume() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("remove temp files"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({ "command": "rm -rf /tmp/rustclaw-verifier-test" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved);
    assert!(result.needs_confirmation);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    assert_eq!(
        result
            .permission_decision
            .pointer("/allowed")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/decision")
            .and_then(serde_json::Value::as_str),
        Some("require_confirmation")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("verify_confirmation_required")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/decision")
            .and_then(serde_json::Value::as_str),
        Some("require_confirmation")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/requires_confirmation")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    let conn = state.core.audit_db.get().expect("audit db");
    let (action, detail_json, user_id): (String, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT action, detail_json, user_id FROM audit_logs ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("latest audit row");
    assert_eq!(action, "plan_verifier.permission_decision");
    assert_eq!(user_id, Some(task.user_id));
    let detail: serde_json::Value =
        serde_json::from_str(detail_json.as_deref().expect("audit detail json")).unwrap();
    assert_eq!(detail["task_id"], task.task_id);
    assert_eq!(
        detail
            .pointer("/permission_decision/decision")
            .and_then(serde_json::Value::as_str),
        Some("require_confirmation")
    );
    assert_eq!(
        detail
            .pointer("/permission_decision/steps/0/decision")
            .and_then(serde_json::Value::as_str),
        Some("require_confirmation")
    );
}

#[test]
fn high_risk_external_generation_requires_confirmation_without_dry_run() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("generate media"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "image_generate".to_string(),
                args: json!({ "action": "generate", "prompt": "status card" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved);
    assert!(result.needs_confirmation);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/risk_level")
            .and_then(serde_json::Value::as_str),
        Some("high")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/decision")
            .and_then(serde_json::Value::as_str),
        Some("require_confirmation")
    );
}

#[test]
fn high_risk_external_generation_dry_run_skips_confirmation() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: Some("plan media dry run"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "image_generate".to_string(),
                args: json!({
                    "action": "generate",
                    "prompt": "status card",
                    "dry_run": true
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/risk_level")
            .and_then(serde_json::Value::as_str),
        Some("low")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/decision")
            .and_then(serde_json::Value::as_str),
        Some("allow")
    );
}

#[test]
fn non_exempt_invocation_still_requires_confirmation() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: Some("photo move"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "photo_organize".to_string(),
                args: json!({ "action": "organize", "mode": "move" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved);
    assert!(result.needs_confirmation);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[test]
fn ops_recipe_requires_inspect_before_mutate() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({ "command": "systemctl restart sing-box" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                    ..Default::default()
                },
            ),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        matches!(
            issue.kind,
            VerifyIssueKind::RecipeInspectBeforeMutateRequired
        )
    }));
}

#[test]
fn ops_recipe_requires_validation_after_mutate() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "configs/config.toml" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({ "command": "systemctl restart sing-box" }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                    ..Default::default()
                },
            ),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        matches!(
            issue.kind,
            VerifyIssueKind::RecipeValidationAfterMutateRequired
        )
    }));
}
