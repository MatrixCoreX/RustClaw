use super::*;

#[test]
fn destructive_run_cmd_requires_confirmation_without_resume() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
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
fn readonly_cli_help_run_cmd_action_is_low_risk_without_confirmation() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("inspect local cli surface"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({
                    "action": "inspect_cli_help",
                    "command": "target/release/clawcli resume --help 2>&1 | head -80",
                    "timeout_seconds": 10,
                    "max_output_bytes": 24000
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved);
    assert!(!result.needs_confirmation);
    assert!(result.issues.iter().all(|issue| {
        !matches!(
            issue.kind,
            VerifyIssueKind::ConfirmationRequired | VerifyIssueKind::RiskBudgetExceeded
        )
    }));
    assert_eq!(
        result
            .permission_decision
            .pointer("/allowed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
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
fn workspace_validation_run_cmd_is_low_risk_without_confirmation() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("validate generated code"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({
                    "command": "python3 test_calc_core.py",
                    "cwd": "run/nl_eval_tmp/codex_cli_continuous_20260711_new",
                    "timeout_seconds": 30
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation);
    assert!(result.issues.iter().all(|issue| {
        !matches!(
            issue.kind,
            VerifyIssueKind::ConfirmationRequired | VerifyIssueKind::RiskBudgetExceeded
        )
    }));
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
fn sandboxed_workspace_greenfield_run_cmd_does_not_require_confirmation() {
    let state = test_state();
    let task = test_task();
    let command = "set -e\nmkdir -p run/new_calc && cd run/new_calc && cat > calc.py <<'EOF'\ndef add(a, b):\n    return a + b\nEOF\nls -la";
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("create a new project"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({"command": command}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation, "issues: {:?}", result.issues);
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/decision")
            .and_then(serde_json::Value::as_str),
        Some("allow")
    );
}

#[test]
fn greenfield_run_cmd_keeps_confirmation_for_destructive_or_unsandboxed_commands() {
    let task = test_task();
    for (sandbox_mode, command) in [
        (
            claw_core::config::ToolSandboxMode::WorkspaceWrite,
            "mkdir -p run/new_calc && rm -rf run/existing",
        ),
        (
            claw_core::config::ToolSandboxMode::DangerFull,
            "mkdir -p run/new_calc && touch run/new_calc/calc.py",
        ),
        (
            claw_core::config::ToolSandboxMode::WorkspaceWrite,
            "set +e; mkdir -p run/new_calc && touch run/new_calc/calc.py",
        ),
    ] {
        let mut tools_config = ToolsConfig::default();
        tools_config.sandbox_mode = sandbox_mode;
        let mut state = test_state();
        state.skill_rt.tools_policy =
            Arc::new(ToolsPolicy::from_config(&tools_config).expect("sandbox tools policy"));
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                output_contract: Some(&route_result()),
                request_text: Some("create a new project"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({"command": command}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );
        assert!(result.approved, "issues: {:?}", result.issues);
        assert!(result.needs_confirmation, "issues: {:?}", result.issues);
    }
}

#[test]
fn workspace_inline_python_probe_run_cmd_is_low_risk_without_confirmation() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("validate generated code"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({
                    "command": "python3 - <<'PY'\nfrom calc_core import safe_div\nprint(safe_div(1, 0))\nPY",
                    "cwd": "run/nl_eval_tmp/codex_cli_continuous_20260711_new",
                    "timeout_seconds": 30
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation);
    assert!(result.issues.iter().all(|issue| {
        !matches!(
            issue.kind,
            VerifyIssueKind::ConfirmationRequired | VerifyIssueKind::RiskBudgetExceeded
        )
    }));
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
fn workspace_inline_python_probe_with_arrow_output_is_low_risk_without_confirmation() {
    let state = test_state();
    let task = test_task();
    let project_dir = state
        .skill_rt
        .workspace_root
        .join("run/nl_eval_tmp/codex_cli_continuous_20260711_new");
    let command = format!(
        "cd {} && python3 - <<'PY'\nfrom calc_core import safe_div\nresult = safe_div(1, 0)\nprint(\"safe_div(1,0) =>\", result)\nassert result == {{\"ok\": False, \"error_code\": \"division_by_zero\"}}, result\nPY\necho \"EXIT=$?\"",
        project_dir.display()
    );
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("validate generated code"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({
                    "command": command,
                    "cwd": "run/nl_eval_tmp/codex_cli_continuous_20260711_new",
                    "timeout_seconds": 30
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation);
    assert!(result.issues.iter().all(|issue| {
        !matches!(
            issue.kind,
            VerifyIssueKind::ConfirmationRequired | VerifyIssueKind::RiskBudgetExceeded
        )
    }));
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/risk_level")
            .and_then(serde_json::Value::as_str),
        Some("low")
    );
}

#[test]
fn external_workspace_validation_run_cmd_keeps_confirmation_boundary() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("validate external code"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({
                    "command": "python3 test_calc_core.py",
                    "cwd": "/var/tmp/external_project",
                    "timeout_seconds": 30
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(
        result.needs_confirmation,
        "external validation should stay bounded"
    );
    assert!(result.issues.iter().any(|issue| {
        matches!(
            issue.kind,
            VerifyIssueKind::ConfirmationRequired | VerifyIssueKind::RiskBudgetExceeded
        )
    }));
}

#[test]
fn high_risk_external_generation_requires_confirmation_without_dry_run() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
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
            output_contract: Some(&route_result()),
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
            output_contract: Some(&route_result()),
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
            output_contract: Some(&route_result()),
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
            output_contract: Some(&route_result()),
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
