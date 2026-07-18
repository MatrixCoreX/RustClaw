use std::sync::Arc;

use claw_core::config::{ToolApprovalPolicy, ToolSandboxMode, ToolsConfig};
use serde_json::json;

use super::tests::{plan_result, route_result, test_state, test_task};
use super::{verify_plan, VerifyInput, VerifyIssueKind, VerifyMode};
use crate::PlanStep;

#[test]
fn command_permission_preview_uses_verifier_policy_tokens() {
    let state = test_state();
    let preview = super::preview_command_permission_decision(
        &state,
        "sudo rm -rf /tmp/rustclaw-never-run",
        None,
        false,
    );

    assert_eq!(preview["status_code"], "permission_preflight_complete");
    assert_eq!(preview["action"], "preview_command_permission");
    assert_eq!(preview["decision"], "deny");
    assert_eq!(preview["risk_level"], "high");
    assert_eq!(preview["confirmation_required"], false);
    assert_eq!(preview["would_execute"], false);
    assert!(preview["reason_codes"]
        .as_array()
        .is_some_and(|reasons| reasons.iter().any(|reason| reason == "sudo_not_allowed")));
    assert!(preview.get("text").is_none());
    assert!(preview.get("error_text").is_none());
}

#[test]
fn command_permission_preview_preserves_confirmation_decision() {
    let state = test_state();
    let preview = super::preview_command_permission_decision(
        &state,
        "rm -rf target/clawd-preview-never-run",
        Some("."),
        true,
    );

    assert_eq!(preview["decision"], "require_confirmation");
    assert_eq!(preview["risk_level"], "high");
    assert_eq!(preview["confirmation_required"], true);
    assert_eq!(preview["reason_codes"], json!(["confirmation_required"]));
    assert_eq!(
        preview.pointer("/workspace_scope/scope"),
        Some(&json!("unspecified"))
    );
    assert_eq!(
        preview.pointer("/workspace_scope/cwd_present"),
        Some(&json!(true))
    );
}

fn state_with_tool_policy(
    sandbox_mode: ToolSandboxMode,
    approval_policy: ToolApprovalPolicy,
) -> crate::AppState {
    let mut state = test_state();
    let config = ToolsConfig {
        sandbox_mode,
        approval_policy,
        ..ToolsConfig::default()
    };
    state.skill_rt.tools_policy =
        Arc::new(crate::ToolsPolicy::from_config(&config).expect("test tools policy"));
    state
}

#[test]
fn workspace_fs_basic_mutation_does_not_emit_route_ceiling_or_confirmation_noise() {
    let state = test_state();
    let task = test_task();
    let route = route_result();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: json!({
                        "action": "make_dir",
                        "path": "run/nl_eval_tmp/verifier_workspace_mutation"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: json!({
                        "action": "write_text",
                        "path": "run/nl_eval_tmp/verifier_workspace_mutation/calc_core.py",
                        "content": "def add(a, b):\n    return a + b\n"
                    }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation, "issues: {:?}", result.issues);
    assert!(result.issues.iter().all(|issue| {
        !matches!(
            issue.kind,
            VerifyIssueKind::RiskBudgetExceeded | VerifyIssueKind::ConfirmationRequired
        )
    }));
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/decision")
            .and_then(serde_json::Value::as_str),
        Some("allow")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/sandbox_profile")
            .and_then(serde_json::Value::as_str),
        Some("local_current_workspace")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/sandbox/source")
            .and_then(serde_json::Value::as_str),
        Some("registry_capability_policy")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/workspace_scope/scope")
            .and_then(serde_json::Value::as_str),
        Some("workspace_scoped")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/workspace_scope/untrusted_path_present")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/1/decision")
            .and_then(serde_json::Value::as_str),
        Some("allow")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/1/workspace_scope/path_arg_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/1/sandbox/filesystem_write")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn read_only_sandbox_blocks_workspace_write() {
    let state = state_with_tool_policy(ToolSandboxMode::ReadOnly, ToolApprovalPolicy::Never);
    let result = verify_plan(
        &state,
        &test_task(),
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({
                    "action": "write_text",
                    "path": "run/sandbox/read_only.txt",
                    "content": "blocked"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved, "issues: {:?}", result.issues);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::SandboxPolicyDenied)));
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/decision")
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/sandbox_denial_reason")
            .and_then(serde_json::Value::as_str),
        Some("sandbox_read_only_write_denied")
    );
}

#[test]
fn workspace_sandbox_blocks_package_install_contract() {
    let state = state_with_tool_policy(ToolSandboxMode::WorkspaceWrite, ToolApprovalPolicy::OnRisk);
    let result = verify_plan(
        &state,
        &test_task(),
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "package_manager".to_string(),
                args: json!({ "action": "install", "package": "example-package" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation);
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/sandbox_denial_reason")
            .and_then(serde_json::Value::as_str),
        Some("sandbox_workspace_privilege_denied")
    );
}

#[test]
fn always_approval_requires_confirmation_for_workspace_write() {
    let state = state_with_tool_policy(ToolSandboxMode::WorkspaceWrite, ToolApprovalPolicy::Always);
    let result = verify_plan(
        &state,
        &test_task(),
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({
                    "action": "write_text",
                    "path": "run/sandbox/approval.txt",
                    "content": "pending"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(result.needs_confirmation);
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/approval_policy")
            .and_then(serde_json::Value::as_str),
        Some("always")
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
fn never_approval_does_not_confirm_sandbox_allowed_workspace_write() {
    let state = state_with_tool_policy(ToolSandboxMode::WorkspaceWrite, ToolApprovalPolicy::Never);
    let result = verify_plan(
        &state,
        &test_task(),
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({
                    "action": "write_text",
                    "path": "run/sandbox/no_approval.txt",
                    "content": "allowed"
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
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/approval_policy")
            .and_then(serde_json::Value::as_str),
        Some("never")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/decision")
            .and_then(serde_json::Value::as_str),
        Some("allow")
    );
}
