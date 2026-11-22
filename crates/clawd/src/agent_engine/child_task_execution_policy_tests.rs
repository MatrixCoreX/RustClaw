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
fn frozen_argument_scope_denies_widening_missing_bounds_and_alias_bypasses() {
    let state = test_state();
    install_filesystem_registry(&state);
    let mut task = child_task(
        "read_only",
        serde_json::json!(["filesystem.read_text_range"]),
    );
    let rule = serde_json::json!([{"capability":"filesystem.read_file","allowed_args":[{
        "type":"object","required":["path","mode","start_line","end_line"],
        "properties":{"path":{"enum":["README.md"]},"mode":{"enum":["range"]},
            "start_line":{"type":"integer","minimum":10,"maximum":20},
            "end_line":{"type":"integer","minimum":10,"maximum":20}}
    }]}]);
    let mut payload: serde_json::Value = serde_json::from_str(&task.payload_json).unwrap();
    payload["child_task_contract"]["scope"]["capability_argument_constraints"] = rule;
    task.payload_json = payload.to_string();
    let valid = serde_json::json!({"action":"read_text_range","path":"README.md","mode":"range","start_line":10,"end_line":20});
    assert!(child_task_execution_policy_error(&state, &task, "fs_basic", &valid).is_none());
    for (field, value) in [
        ("start_line", serde_json::json!(1)),
        ("end_line", serde_json::json!(21)),
        ("path", serde_json::json!("OTHER.md")),
        ("mode", serde_json::json!("head")),
    ] {
        let mut args = valid.clone();
        args[field] = value;
        let err = child_task_execution_policy_error(&state, &task, "fs_basic", &args).unwrap();
        let extra = crate::skills::parse_structured_skill_error(&err)
            .unwrap()
            .extra
            .unwrap();
        assert_eq!(extra["execution_dispatched"], false);
        assert_eq!(extra["side_effect_applied"], false);
        assert!(extra["violations"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("capability_arguments_out_of_scope")));
    }
    let mut args = valid;
    args.as_object_mut().unwrap().remove("end_line");
    assert!(child_task_execution_policy_error(&state, &task, "fs_basic", &args).is_some());
}

#[test]
fn argument_scope_admission_is_bounded_and_can_allow_disjoint_slices() {
    use super::child_task_execution_policy::valid_capability_argument_constraints as valid;
    let allowed = vec!["filesystem.read_text_range".to_string()];
    let schema = serde_json::json!({"type":"object","required":["start_line"],"properties":{"start_line":{"type":"integer","minimum":10,"maximum":20}}});
    let good = serde_json::json!([{"capability":allowed[0],"allowed_args":[schema,{
        "type":"object","required":["start_line"],"properties":{"start_line":{"enum":[50]}}
    }]}]);
    assert!(valid(&good, &allowed));
    for schema in [
        serde_json::json!({"type":"object"}),
        serde_json::json!({"type":"object","required":["path","end_line"],"properties":{"end_line":{"enum":[20]}}}),
        serde_json::json!({"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}),
    ] {
        assert!(!crate::repo::child_tasks::bounded_argument_schema_supported(&schema));
        assert!(
            !crate::repo::child_tasks::bounded_argument_schema_validation_issues(&schema)
                .is_empty()
        );
    }
    let mut invalid = good.clone();
    invalid[0]["allowed_args"][0]["$ref"] = serde_json::json!("https://invalid.example/schema");
    assert!(!valid(&invalid, &allowed));
    let mut invalid = good.clone();
    invalid[0]["capability"] = serde_json::json!("filesystem.write_text");
    assert!(!valid(&invalid, &allowed));
    let mut invalid = good.clone();
    invalid[0]["allowed_args"] = serde_json::json!([]);
    assert!(!valid(&invalid, &allowed));
    let schemas = good[0]["allowed_args"].as_array().unwrap();
    for line in [10, 15, 20, 50] {
        assert!(schemas.iter().any(
            |s| crate::repo::child_tasks::bounded_argument_schema_matches(
                s,
                &serde_json::json!({"start_line":line})
            )
        ));
    }
    assert!(!schemas.iter().any(
        |s| crate::repo::child_tasks::bounded_argument_schema_matches(
            s,
            &serde_json::json!({"start_line":30})
        )
    ));
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
    assert_eq!(parsed.error_code, "child_task_policy_violation");
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
        "agent_child_policy_worktree_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&worktree_root).expect("create worktree fixture");
    std::fs::write(
        worktree_root.join(".agent-isolation.json"),
        serde_json::json!({
            "marker_kind": "agent_execution_isolation",
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

    assert_eq!(parsed.error_code, "child_task_policy_violation");
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
