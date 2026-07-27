use super::*;

#[test]
fn local_clawd_base_url_accepts_loopback_test_override() {
    assert_eq!(
        local_clawd_base_url_from_internal_listen(Some("127.0.0.1:59871")),
        "http://127.0.0.1:59871"
    );
    assert_eq!(
        local_clawd_base_url_from_internal_listen(Some("[::1]:59872")),
        "http://[::1]:59872"
    );
}

#[test]
fn local_clawd_base_url_rejects_non_loopback_override() {
    assert_eq!(
        local_clawd_base_url_from_internal_listen(Some("0.0.0.0:8787")),
        "http://127.0.0.1:8787"
    );
    assert_eq!(
        local_clawd_base_url_from_internal_listen(Some("192.168.1.10:8787")),
        "http://127.0.0.1:8787"
    );
    assert_eq!(
        local_clawd_base_url_from_internal_listen(Some("invalid")),
        "http://127.0.0.1:8787"
    );
    assert_eq!(
        local_clawd_base_url_from_internal_listen(None),
        "http://127.0.0.1:8787"
    );
}

#[test]
fn selected_provider_credentials_include_vendor_and_protocol_aliases() {
    assert_eq!(
        selected_provider_api_key_env_names("minimax", "openai_compat"),
        vec!["MINIMAX_API_KEY", "OPENAI_API_KEY"]
    );
    assert_eq!(
        selected_provider_api_key_env_names("mimo", "openai_compat"),
        vec!["MIMO_API_KEY", "OPENAI_API_KEY"]
    );
    assert_eq!(
        selected_provider_api_key_env_names("openai", "openai_compat"),
        vec!["OPENAI_API_KEY"]
    );
    assert_eq!(
        selected_provider_api_key_env_names("anthropic", "anthropic_claude"),
        vec!["ANTHROPIC_API_KEY"]
    );
    assert_eq!(
        selected_provider_api_key_env_names("google", "google_gemini"),
        vec!["GOOGLE_API_KEY"]
    );
    assert_eq!(
        selected_provider_api_key_env_names("custom", "openai_compat"),
        vec!["OPENAI_API_KEY"]
    );
    assert!(selected_provider_api_key_env_names("fixture", "fixture_replay").is_empty());
}

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
    assert_eq!(
        action_scoped_runner_sandbox_mode(ToolSandboxMode::DangerFull, Some(&preview_mapping())),
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

    let context = build_runner_skill_context(
        &state,
        &task,
        "ui",
        serde_json::json!({}),
        None,
        Some(&execution),
    );

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

#[test]
fn runner_context_exposes_only_the_calling_skills_storage_descriptor() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        claim_attempt: 1,
        task_id: "task-runner-storage".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("rk-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let descriptor = state
        .core
        .skill_storage
        .descriptor("kb")
        .expect("KB descriptor");
    let context = build_runner_skill_context(
        &state,
        &task,
        "ui",
        serde_json::json!({}),
        Some(descriptor),
        None,
    );
    assert_eq!(
        context.pointer("/skill_storage/skill_name"),
        Some(&json!("kb"))
    );
    assert_eq!(
        context.pointer("/skill_storage/storage_kind"),
        Some(&json!("sqlite"))
    );
    assert!(context.get("database_sqlite_path").is_none());
    assert!(context.get("database_busy_timeout_ms").is_none());
}
