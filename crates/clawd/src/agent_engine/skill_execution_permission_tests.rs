use super::preflight_permission_decision;
use super::tests::{install_test_registry, test_state};

#[test]
fn preflight_permission_decision_uses_registry_policy() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
timeout_seconds = 60
planner_capabilities = [
  { name = "system.run_command", effect = "external", required = ["command"], risk_level = "high", once_per_task = true, idempotent = false, dedup_scope = "action", execution_mode = "async_preferred", async_adapter_kind = "local_process_poll", subprocess = true },
]
"#,
        &["run_cmd"],
    );
    let args = serde_json::json!({"command": "ls"});

    let permission = preflight_permission_decision(
        &state,
        "run_cmd",
        &args,
        "registry_policy_probe",
        "registry_policy_probe",
    );

    assert_eq!(permission["risk_level"], serde_json::json!("high"));
    assert_eq!(permission["decision"], serde_json::json!("deny"));
    assert_eq!(permission["needs_confirmation"], true);
    assert_eq!(permission["action_effect"], serde_json::json!("observe"));
    assert_eq!(permission["canonical_skill"], serde_json::json!("run_cmd"));
    assert_eq!(
        permission.pointer("/command_policy/policy_authority"),
        Some(&serde_json::json!("planner_structured_args"))
    );
    assert_eq!(
        permission.pointer("/command_policy/effect"),
        Some(&serde_json::json!("observe"))
    );
    assert_eq!(
        permission
            .pointer("/registry_policy/available")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        permission
            .pointer("/registry_policy/once_per_task")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        permission
            .pointer("/registry_policy/dedup_scope")
            .and_then(serde_json::Value::as_str),
        Some("action")
    );
    assert_eq!(
        permission
            .pointer("/registry_policy/idempotent")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/isolation_profile")
            .and_then(serde_json::Value::as_str),
        Some("remote_executor")
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/network_access")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/filesystem_write")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/external_publish")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/credential_access")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/execution_mode")
            .and_then(serde_json::Value::as_str),
        Some("async_preferred")
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/timeout_class")
            .and_then(serde_json::Value::as_str),
        Some("long_tail")
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/timeout_seconds")
            .and_then(serde_json::Value::as_u64),
        Some(60)
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/cancellation_mode")
            .and_then(serde_json::Value::as_str),
        Some("task_token_and_async_adapter")
    );
    assert_eq!(
        permission.pointer("/capability_policy/permission_scopes"),
        Some(&serde_json::json!([
            "network",
            "external_publish",
            "subprocess"
        ]))
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/evidence_contract")
            .and_then(serde_json::Value::as_str),
        Some("capability_result_envelope_v1")
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/hook_contract")
            .and_then(serde_json::Value::as_str),
        Some("pre_and_post_tool_use_v1")
    );
    assert_eq!(
        permission
            .pointer("/sandbox_profile")
            .and_then(serde_json::Value::as_str),
        Some("remote_executor")
    );
    assert_eq!(
        permission
            .pointer("/sandbox/source")
            .and_then(serde_json::Value::as_str),
        Some("registry_capability_policy")
    );
    assert_eq!(
        permission
            .pointer("/sandbox/network_access")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        permission
            .pointer("/workspace_scope/scope")
            .and_then(serde_json::Value::as_str),
        Some("unspecified")
    );
    assert_eq!(
        permission
            .pointer("/workspace_scope/untrusted_path_present")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
}
