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
fn only_an_active_runtime_sandbox_is_inherited_by_the_skill_child() {
    assert_eq!(inherited_sandbox_backend("bubblewrap"), Some("bubblewrap"));
    assert_eq!(
        inherited_sandbox_backend("macos_seatbelt"),
        Some("macos_seatbelt")
    );
    assert_eq!(inherited_sandbox_backend("direct"), None);
}

#[test]
fn declared_skill_storage_directory_is_an_explicit_sandbox_write_target() {
    let secret_directory = std::path::Path::new("/runtime/secret-tokens");
    let storage_directory = std::path::Path::new("/runtime/skill-data/fs_search");
    let paths = runner_additional_writable_paths(Some(secret_directory), Some(storage_directory));

    assert_eq!(
        paths,
        vec![
            secret_directory.to_path_buf(),
            storage_directory.to_path_buf()
        ]
    );
    assert_eq!(
        runner_additional_writable_paths(None, Some(storage_directory)),
        vec![storage_directory.to_path_buf()]
    );
}

#[test]
fn declared_skill_storage_descriptor_uses_its_mapped_sandbox_target() {
    let secret_directory = std::path::PathBuf::from("/runtime/secret-tokens");
    let storage_directory = std::path::PathBuf::from("/runtime/skill-data/fs_search");
    let sources = vec![secret_directory, storage_directory.clone()];
    let targets = vec![
        std::path::PathBuf::from("/run/rustclaw-writable/0"),
        std::path::PathBuf::from("/run/rustclaw-writable/1"),
    ];
    let sandbox_storage = sandbox_target_for_source(Some(&storage_directory), &sources, &targets)
        .expect("mapped storage target");
    let descriptor = crate::skill_storage::SkillStorageDescriptor {
        schema_version: 1,
        skill_name: "fs_search".to_string(),
        storage_kind: "sqlite",
        database_path: storage_directory.join("state.db").display().to_string(),
        database_busy_timeout_ms: 5_000,
    };

    let mapped = map_storage_descriptor_to_sandbox(Some(descriptor), Some(&sandbox_storage))
        .expect("map descriptor")
        .expect("descriptor");

    assert_eq!(mapped.database_path, "/run/rustclaw-writable/1/state.db");
    assert_eq!(
        sandbox_target_for_source(Some(&sources[0]), &sources, &targets),
        Some(std::path::PathBuf::from("/run/rustclaw-writable/0"))
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
        .descriptor("kb", 3)
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
    assert_eq!(
        context.pointer("/skill_storage/schema_version"),
        Some(&json!(3))
    );
    assert!(context.get("database_sqlite_path").is_none());
    assert!(context.get("database_busy_timeout_ms").is_none());
}

#[test]
fn registry_storage_schema_version_is_forwarded_to_the_skill() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load workspace skills registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = std::sync::Arc::new(crate::SkillViewsSnapshot {
        registry: Some(std::sync::Arc::new(registry)),
        skills_list: std::sync::Arc::new(enabled),
    });
    let descriptor = storage_descriptor_for_skill(&state, "kb")
        .expect("valid KB storage declaration")
        .expect("KB storage descriptor");
    assert_eq!(descriptor.schema_version, 3);
    assert_eq!(descriptor.skill_name, "kb");
}
