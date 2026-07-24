use super::child_task_execution_policy_error;
use super::tests::{install_test_registry, test_state, test_task};

fn child_task(
    permission_profile: &str,
    allowed_capabilities: serde_json::Value,
) -> crate::ClaimedTask {
    let mut task = test_task();
    task.payload_json = serde_json::json!({
        "text": "inspect the declared scope",
        "task_role": "subagent_child",
        "parent_task_id": "parent-task",
        "child_task_id": "child-task",
        "child_task_contract": {
            "schema_version": 1,
            "parent_task_id": "parent-task",
            "child_task_id": "child-task",
            "permission_profile": permission_profile,
            "scope": {
                "objective": "inspect the declared scope",
                "allowed_capabilities": allowed_capabilities,
            },
        },
    })
    .to_string();
    task
}

fn install_filesystem_registry(state: &crate::AppState) {
    install_test_registry(
        state,
        r#"
[[skills]]
name = "fs_basic"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "low"
side_effect = false
input_schema = { type = "object", properties = { path = { type = "string" }, content = { type = "string" } } }
planner_capability_aliases = { "filesystem.read_file" = "filesystem.read_text_range" }
planner_capabilities = [
  { name = "filesystem.read_text_range", action = "read_text_range", effect = "observe", required = ["path"], risk_level = "low", isolation_profile = "read_only", network_access = false, filesystem_write = false, external_publish = false, credential_access = false, subprocess = false, package_install = false, privilege_escalation = false },
  { name = "filesystem.read_file", action = "read_text_range", effect = "observe", required = ["path"], risk_level = "low", isolation_profile = "read_only", network_access = false, filesystem_write = false, external_publish = false, credential_access = false, subprocess = false, package_install = false, privilege_escalation = false },
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", isolation_profile = "local_current_workspace", network_access = false, filesystem_write = true, external_publish = false, credential_access = false, subprocess = false, package_install = false, privilege_escalation = false },
  { name = "filesystem.publish_text", action = "publish_text", effect = "external", required = ["path"], risk_level = "high", isolation_profile = "local_worktree", network_access = true, filesystem_write = true, external_publish = true, credential_access = true, subprocess = false, package_install = false, privilege_escalation = false },
]
"#,
        &["fs_basic"],
    );
}

#[test]
fn non_child_task_bypasses_child_execution_policy() {
    let state = test_state();
    install_filesystem_registry(&state);
    let task = test_task();

    assert!(child_task_execution_policy_error(
        &state,
        &task,
        "fs_basic",
        &serde_json::json!({"action": "write_text"})
    )
    .is_none());
}

#[test]
fn read_only_child_accepts_declared_observe_capability() {
    let state = test_state();
    install_filesystem_registry(&state);
    let task = child_task(
        "read_only",
        serde_json::json!(["filesystem.read_text_range"]),
    );

    assert!(child_task_execution_policy_error(
        &state,
        &task,
        "fs_basic",
        &serde_json::json!({"action": "read_text_range", "path": "README.md"})
    )
    .is_none());
}

#[test]
fn read_only_child_accepts_legacy_alias_as_canonical_scope() {
    let state = test_state();
    install_filesystem_registry(&state);
    let task = child_task("read_only", serde_json::json!(["filesystem.read_file"]));

    assert!(child_task_execution_policy_error(
        &state,
        &task,
        "fs_basic",
        &serde_json::json!({"action": "read_text_range", "path": "README.md"})
    )
    .is_none());
}

#[test]
fn child_rejects_missing_or_out_of_scope_capability() {
    let state = test_state();
    install_filesystem_registry(&state);
    let missing = child_task("read_only", serde_json::json!([]));
    let out_of_scope = child_task(
        "read_only",
        serde_json::json!(["filesystem.read_text_range"]),
    );
    let args = serde_json::json!({
        "action": "write_text",
        "path": "result.txt",
        "content": "value"
    });

    let missing_error = child_task_execution_policy_error(&state, &missing, "fs_basic", &args)
        .expect("empty child allowlist must deny skill execution");
    let missing_extra = crate::skills::parse_structured_skill_error(&missing_error)
        .expect("structured error")
        .extra
        .expect("extra");
    assert!(missing_extra["violations"]
        .as_array()
        .expect("violations")
        .contains(&serde_json::json!("allowed_capabilities_missing")));

    let scope_error = child_task_execution_policy_error(&state, &out_of_scope, "fs_basic", &args)
        .expect("out-of-scope capability must be denied");
    let parsed = crate::skills::parse_structured_skill_error(&scope_error)
        .expect("child policy error should be structured");
    let extra = parsed.extra.expect("extra");
    assert_eq!(parsed.error_kind, "child_task_policy_violation");
    assert_eq!(
        extra["owner_layer"],
        serde_json::json!("child_task_execution_policy")
    );
    assert_eq!(
        extra["selected_capability"],
        serde_json::json!("filesystem.write_text")
    );
    assert_eq!(
        extra["policy_decision"],
        serde_json::json!(crate::policy_decision::PolicyDecision::Deny.as_token())
    );
    let retry = super::skill_execution_preflight::preflight_failure_metadata(&scope_error);
    assert_eq!(retry.reason, "child_task_policy_violation");
    assert_eq!(
        retry.retry_instruction,
        "child_task_policy=use_declared_capability_and_permission_profile;retry_same_policy=false"
    );
    assert!(extra["violations"]
        .as_array()
        .expect("violations")
        .contains(&serde_json::json!("capability_not_allowed")));
}

#[test]
fn read_only_child_rejects_declared_write_capability() {
    let state = test_state();
    install_filesystem_registry(&state);
    let task = child_task("read_only", serde_json::json!(["filesystem.write_text"]));

    let error = child_task_execution_policy_error(
        &state,
        &task,
        "fs_basic",
        &serde_json::json!({
            "action": "write_text",
            "path": "result.txt",
            "content": "value"
        }),
    )
    .expect("read-only child must reject mutation");
    let extra = crate::skills::parse_structured_skill_error(&error)
        .expect("structured error")
        .extra
        .expect("extra");

    assert!(extra["violations"]
        .as_array()
        .expect("violations")
        .contains(&serde_json::json!("read_only_effect_required")));
    assert!(extra["violations"]
        .as_array()
        .expect("violations")
        .contains(&serde_json::json!("filesystem_write")));
}

#[test]
fn local_worktree_child_accepts_scoped_write_but_rejects_publish() {
    let mut state = test_state();
    install_filesystem_registry(&state);
    let worktree_root = std::env::temp_dir().join(format!(
        "rustclaw_child_policy_worktree_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&worktree_root).expect("create worktree fixture");
    std::fs::write(
        worktree_root.join(".rustclaw-isolation.json"),
        serde_json::json!({
            "marker_kind": "rustclaw_execution_isolation",
            "profile": "local_worktree"
        })
        .to_string(),
    )
    .expect("write worktree marker");
    state.skill_rt.workspace_root = worktree_root.clone();
    let write_task = child_task(
        "local_worktree",
        serde_json::json!(["filesystem.write_text"]),
    );
    let publish_task = child_task(
        "local_worktree",
        serde_json::json!(["filesystem.publish_text"]),
    );

    assert!(child_task_execution_policy_error(
        &state,
        &write_task,
        "fs_basic",
        &serde_json::json!({
            "action": "write_text",
            "path": "result.txt",
            "content": "value"
        })
    )
    .is_none());

    let error = child_task_execution_policy_error(
        &state,
        &publish_task,
        "fs_basic",
        &serde_json::json!({"action": "publish_text", "path": "result.txt"}),
    )
    .expect("worktree child must not publish or read credentials");
    let extra = crate::skills::parse_structured_skill_error(&error)
        .expect("structured error")
        .extra
        .expect("extra");
    for violation in ["network_access", "external_publish", "credential_access"] {
        assert!(extra["violations"]
            .as_array()
            .expect("violations")
            .contains(&serde_json::json!(violation)));
    }
    std::fs::remove_dir_all(worktree_root).expect("remove worktree fixture");
}

#[test]
fn local_worktree_child_rejects_unbound_primary_workspace() {
    let state = test_state();
    install_filesystem_registry(&state);
    let task = child_task(
        "local_worktree",
        serde_json::json!(["filesystem.write_text"]),
    );

    let error = child_task_execution_policy_error(
        &state,
        &task,
        "fs_basic",
        &serde_json::json!({
            "action": "write_text",
            "path": "notes.txt",
            "content": "isolated"
        }),
    )
    .expect("unbound worktree child must fail closed");
    let parsed = crate::skills::parse_structured_skill_error(&error).expect("structured error");
    let extra = parsed.extra.expect("extra");

    assert_eq!(parsed.error_kind, "child_task_policy_violation");
    assert!(extra["violations"]
        .as_array()
        .expect("violations")
        .contains(&serde_json::json!("child_worktree_binding_required")));
}

#[test]
fn child_rejects_non_machine_allowlist_tokens_and_unknown_profile() {
    let state = test_state();
    install_filesystem_registry(&state);
    let task = child_task(
        "unrestricted",
        serde_json::json!(["filesystem.read_text_range", "invalid capability token"]),
    );

    let error = child_task_execution_policy_error(
        &state,
        &task,
        "fs_basic",
        &serde_json::json!({"action": "read_text_range", "path": "README.md"}),
    )
    .expect("invalid child contract must be denied");
    let extra = crate::skills::parse_structured_skill_error(&error)
        .expect("structured error")
        .extra
        .expect("extra");

    assert_eq!(extra["invalid_allowed_capability_count"], 1);
    assert!(extra["violations"]
        .as_array()
        .expect("violations")
        .contains(&serde_json::json!("invalid_allowed_capability")));
    assert!(extra["violations"]
        .as_array()
        .expect("violations")
        .contains(&serde_json::json!("permission_profile_unsupported")));
}
