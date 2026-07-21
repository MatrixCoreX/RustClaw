use super::*;

fn preview_mapping() -> PlannerCapabilityMapping {
    toml::from_str(
        r#"
name = "image.preview_generate"
action = "preview_generate"
effect = "observe"
risk_level = "low"
isolation_profile = "read_only"
network_access = false
filesystem_write = false
external_publish = false
credential_access = false
subprocess = false
"#,
    )
    .expect("preview mapping")
}

fn local_api_mapping() -> PlannerCapabilityMapping {
    toml::from_str(
        r#"
name = "task_control.list"
action = "list"
effect = "observe"
risk_level = "low"
isolation_profile = "local_current_workspace"
network_access = true
filesystem_write = false
external_publish = false
credential_access = false
subprocess = false
"#,
    )
    .expect("local API mapping")
}

#[test]
fn read_only_preview_removes_network_write_execution_and_credentials() {
    let capabilities = vec![
        Capability::Llm,
        Capability::Net,
        Capability::FsRead,
        Capability::FsWrite,
        Capability::Exec,
        Capability::ExecSudo,
        Capability::Secrets("image_generation_minimax_api_key".to_string()),
    ];

    let effective = action_scoped_runner_capabilities(capabilities, Some(&preview_mapping()));

    assert_eq!(effective, vec![Capability::FsRead]);
}

#[test]
fn read_only_preview_forces_read_only_process_sandbox() {
    assert_eq!(
        action_scoped_runner_sandbox_mode(
            ToolSandboxMode::WorkspaceWrite,
            Some(&preview_mapping())
        ),
        ToolSandboxMode::ReadOnly
    );
    assert_eq!(
        action_scoped_runner_sandbox_mode(ToolSandboxMode::DangerFull, None),
        ToolSandboxMode::DangerFull
    );
}

#[test]
fn read_only_local_api_action_retains_network_only() {
    let capabilities = vec![
        Capability::Net,
        Capability::FsRead,
        Capability::FsWrite,
        Capability::Exec,
    ];

    let effective = action_scoped_runner_capabilities(capabilities, Some(&local_api_mapping()));

    assert_eq!(effective, vec![Capability::Net, Capability::FsRead]);
    assert_eq!(
        action_scoped_runner_sandbox_mode(
            ToolSandboxMode::WorkspaceWrite,
            Some(&local_api_mapping())
        ),
        ToolSandboxMode::ReadOnly
    );
}

#[test]
fn runner_context_carries_internal_idempotency_contract_outside_skill_args() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        claim_attempt: 3,
        task_id: "task-runner-idempotency".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let execution = crate::skills::SkillExecutionContext {
        action_ref: "skill:demo:action:publish".to_string(),
        idempotency_key: "stable-key".to_string(),
        attempt_no: 2,
    };

    let context =
        build_runner_skill_context(&state, &task, "ui", serde_json::json!({}), Some(&execution));

    assert_eq!(
        context.pointer("/execution/schema_version"),
        Some(&serde_json::json!(1))
    );
    assert_eq!(
        context
            .pointer("/execution/idempotency_key")
            .and_then(serde_json::Value::as_str),
        Some("stable-key")
    );
    assert_eq!(
        context
            .pointer("/execution/attempt_no")
            .and_then(serde_json::Value::as_i64),
        Some(2)
    );
}
