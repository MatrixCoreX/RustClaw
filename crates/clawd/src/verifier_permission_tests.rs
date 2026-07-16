use serde_json::json;

use super::tests::{plan_result, route_result, test_state, test_task};
use super::{verify_plan, VerifyInput, VerifyIssueKind, VerifyMode};
use crate::PlanStep;

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
