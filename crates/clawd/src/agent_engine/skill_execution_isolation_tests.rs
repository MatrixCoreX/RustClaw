use super::skill_execution_preflight::{
    capability_isolation_artifact_refs, capability_isolation_policy_error,
};

use super::tests::{install_test_registry, test_state, unique_suffix};
use std::fs;

#[test]
fn capability_isolation_preflight_rejects_read_only_write_policy() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
input_schema = { type = "object", properties = { path = { type = "string" }, content = { type = "string" } } }
planner_capabilities = [
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", isolation_profile = "read_only", network_access = false, filesystem_write = true, external_publish = false, credential_access = false },
]
"#,
        &["write_file"],
    );
    let args = serde_json::json!({
        "action": "write_text",
        "path": "out.txt",
        "content": "value"
    });

    let err = capability_isolation_policy_error(&state, "write_file", &args)
        .expect("read_only profile must reject filesystem_write");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("isolation preflight error should be structured");
    let extra = parsed.extra.as_ref().expect("extra");

    assert_eq!(parsed.error_kind, "isolation_policy_violation");
    assert_eq!(
        extra.pointer("/violations/0"),
        Some(&serde_json::json!("filesystem_write"))
    );
    assert_eq!(
        extra.pointer("/permission_decision/reason_code"),
        Some(&serde_json::json!("isolation_policy_violation"))
    );
    assert_eq!(
        extra.pointer("/permission_decision/owner_layer"),
        Some(&serde_json::json!("capability_isolation_preflight"))
    );
    assert_eq!(
        extra.pointer("/permission_decision/capability_policy/isolation_profile"),
        Some(&serde_json::json!("read_only"))
    );
    assert_eq!(
        super::preflight_failure_metadata(&err).reason,
        "isolation_policy_violation"
    );
}

#[test]
fn capability_isolation_preflight_rejects_worktree_publish_credentials() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
input_schema = { type = "object", properties = { path = { type = "string" }, content = { type = "string" } } }
planner_capabilities = [
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", isolation_profile = "local_worktree", network_access = false, filesystem_write = true, external_publish = true, credential_access = true },
]
"#,
        &["write_file"],
    );
    let args = serde_json::json!({
        "action": "write_text",
        "path": "out.txt",
        "content": "value"
    });
    let manifest = state
        .skill_manifest("write_file")
        .expect("write_file manifest");
    let capability = manifest
        .planner_capabilities
        .first()
        .expect("planner capability");
    assert_eq!(
        capability
            .isolation_profile
            .map(|profile| profile.as_token()),
        Some("local_worktree")
    );
    assert_eq!(capability.external_publish, Some(true));
    assert_eq!(capability.credential_access, Some(true));

    let err = capability_isolation_policy_error(&state, "write_file", &args)
        .expect("local_worktree profile must reject publish and credential access");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("isolation preflight error should be structured");
    let extra = parsed.extra.as_ref().expect("extra");

    assert_eq!(parsed.error_kind, "isolation_policy_violation");
    assert_eq!(
        extra.pointer("/violations/0"),
        Some(&serde_json::json!("external_publish"))
    );
    assert_eq!(
        extra.pointer("/violations/1"),
        Some(&serde_json::json!("credential_access"))
    );
    assert_eq!(
        extra.pointer("/permission_decision/capability_policy/isolation_profile"),
        Some(&serde_json::json!("local_worktree"))
    );
}

#[test]
fn capability_isolation_preflight_allows_local_api_with_read_only_filesystem() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "task_control"
enabled = true
kind = "runner"
planner_kind = "tool"
capabilities = ["net"]
planner_capabilities = [
  { name = "task_control.list", action = "list", effect = "observe", risk_level = "low", isolation_profile = "local_current_workspace", network_access = true, filesystem_write = false, external_publish = false, credential_access = false, subprocess = false },
]
"#,
        &["task_control"],
    );
    let args = serde_json::json!({"action": "list"});

    assert!(
        capability_isolation_policy_error(&state, "task_control", &args).is_none(),
        "local API access must not imply workspace write access"
    );
}

#[test]
fn capability_isolation_artifact_refs_report_cleanup_workspace() {
    let mut state = test_state();
    state.skill_rt.workspace_root = std::env::temp_dir().join(format!(
        "rustclaw_isolation_artifact_refs_{}_{}",
        std::process::id(),
        unique_suffix()
    ));
    fs::create_dir_all(&state.skill_rt.workspace_root).expect("create workspace");
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
input_schema = { type = "object", properties = { path = { type = "string" }, content = { type = "string" } } }
planner_capabilities = [
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", isolation_profile = "local_temp_workspace", network_access = false, filesystem_write = true, external_publish = false, credential_access = false },
]
"#,
        &["write_file"],
    );
    let args = serde_json::json!({
        "action": "write_text",
        "path": "out.txt",
        "content": "value"
    });

    let refs = capability_isolation_artifact_refs(&state, "task-skill-exec", "write_file", &args);

    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0]["kind"], "execution_isolation_workspace");
    assert_eq!(refs[0]["profile"], "local_temp_workspace");
    assert_eq!(refs[0]["cleanup_ref"], "isolation:temp:task-skill-exec");
    assert!(refs[0]["artifact_path"]
        .as_str()
        .expect("artifact path")
        .contains("task-skill-exec"));

    let _ = fs::remove_dir_all(&state.skill_rt.workspace_root);
}
