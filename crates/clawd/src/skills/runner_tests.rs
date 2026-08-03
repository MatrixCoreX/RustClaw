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
    let artifact_directory = std::path::Path::new("/runtime/artifacts/invocation");
    let paths = runner_additional_writable_paths(
        Some(secret_directory),
        Some(storage_directory),
        Some(artifact_directory),
    );

    assert_eq!(
        paths,
        vec![
            secret_directory.to_path_buf(),
            storage_directory.to_path_buf(),
            artifact_directory.to_path_buf(),
        ]
    );
    assert_eq!(
        runner_additional_writable_paths(None, Some(storage_directory), Some(artifact_directory)),
        vec![
            storage_directory.to_path_buf(),
            artifact_directory.to_path_buf()
        ]
    );
}

#[test]
fn artifact_paths_are_scoped_per_invocation_and_remapped_to_the_host() {
    let root = std::path::Path::new("/workspace");
    let first = invocation_artifact_output_directory(root, "task/one", "skill.demo");
    let second = invocation_artifact_output_directory(root, "task/one", "skill.demo");
    assert_ne!(first, second);
    assert!(first.starts_with(root.join(".agent-runtime/artifacts/skill-invocations/task_one")));

    let sandbox = std::path::Path::new("/run/agent-runtime-writable/2");
    let host = std::path::Path::new("/workspace/.agent-runtime/artifacts/invocation");
    let mut response = json!({
        "extra": {
            "outputs": [{"path": "/run/agent-runtime-writable/2/result.png"}]
        }
    });
    remap_sandbox_artifact_paths(&mut response, sandbox, host);
    assert_eq!(
        response.pointer("/extra/outputs/0/path"),
        Some(&json!(
            "/workspace/.agent-runtime/artifacts/invocation/result.png"
        ))
    );
}

#[test]
fn cancelled_capture_projection_keeps_only_machine_receipts_and_safe_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "agent_browser_cancel_receipts_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let run_root = root.join("capture/browser_web/2026-08-03/task-1");
    std::fs::create_dir_all(run_root.join("meta")).expect("metadata directory");
    std::fs::create_dir_all(run_root.join("processed/text")).expect("text directory");
    std::fs::write(run_root.join("processed/text/page.txt"), "evidence").expect("text artifact");
    std::fs::write(
        run_root.join("meta/manifest.jsonl"),
        concat!(
            "{\"receipt_id\":\"browser_page:task-1:1:abc\",\"ordinal\":1,",
            "\"status\":\"ok\",\"url\":\"https://example.com/?token=secret\",",
            "\"content_hash_sha256\":\"abc\",\"image_hash_sha256\":\"def\",",
            "\"text_path\":\"processed/text/page.txt\",",
            "\"html_path\":\"../../escape.html\"}\n",
            "not-json\n"
        ),
    )
    .expect("manifest");

    let projection = cancelled_capture_projection(&root, "task-1", "fixture_skill")
        .expect("completed page receipt projection");
    assert_eq!(projection["source"], "fixture_skill");
    assert_eq!(projection["status"], "cancelled_partial");
    assert_eq!(projection["hard_termination"], true);
    assert_eq!(projection["final_partial_generated"], false);
    assert_eq!(projection["completed_page_count"], 1);
    assert_eq!(
        projection["receipts"][0]["receipt_id"],
        "browser_page:task-1:1:abc"
    );
    assert_eq!(projection["receipts"][0]["image_hash_sha256"], "def");
    let encoded = projection.to_string();
    assert!(!encoded.contains("token=secret"));
    assert!(!encoded.contains("escape.html"));
    assert!(encoded.contains("page.txt"));

    std::fs::remove_dir_all(root).expect("cleanup projection fixture");
}

#[test]
fn declared_skill_storage_descriptor_uses_its_mapped_sandbox_target() {
    let secret_directory = std::path::PathBuf::from("/runtime/secret-tokens");
    let storage_directory = std::path::PathBuf::from("/runtime/skill-data/fs_search");
    let sources = vec![secret_directory, storage_directory.clone()];
    let targets = vec![
        std::path::PathBuf::from("/run/agent-runtime-writable/0"),
        std::path::PathBuf::from("/run/agent-runtime-writable/1"),
    ];
    let sandbox_storage = sandbox_target_for_source(Some(&storage_directory), &sources, &targets)
        .expect("mapped storage target");
    let descriptor = crate::skill_storage::SkillStorageDescriptor {
        schema_version: 1,
        skill_name: "fs_search".to_string(),
        storage_kind: "sqlite",
        database_path: storage_directory.join("state.db").display().to_string(),
        database_busy_timeout_ms: 5_000,
        directory_path: None,
    };

    let mapped = map_storage_descriptor_to_sandbox(Some(descriptor), Some(&sandbox_storage))
        .expect("map descriptor")
        .expect("descriptor");

    assert_eq!(
        mapped.database_path,
        "/run/agent-runtime-writable/1/state.db"
    );
    assert_eq!(
        sandbox_target_for_source(Some(&sources[0]), &sources, &targets),
        Some(std::path::PathBuf::from("/run/agent-runtime-writable/0"))
    );
}

#[test]
fn declared_directory_storage_uses_only_its_mapped_sandbox_directory() {
    let host_storage = std::path::PathBuf::from("/runtime/skill-data/media_download");
    let sandbox_storage = std::path::PathBuf::from("/run/agent-runtime-writable/3");
    let descriptor = crate::skill_storage::SkillStorageDescriptor {
        schema_version: 1,
        skill_name: "media_download".to_string(),
        storage_kind: "directory",
        database_path: String::new(),
        database_busy_timeout_ms: 0,
        directory_path: Some(host_storage.display().to_string()),
    };

    let mapped = map_storage_descriptor_to_sandbox(Some(descriptor), Some(&sandbox_storage))
        .expect("map directory descriptor")
        .expect("directory descriptor");

    assert_eq!(
        mapped.directory_path.as_deref(),
        Some("/run/agent-runtime-writable/3")
    );
    assert_eq!(mapped.database_path, "");
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

fn async_mapping(adapter_kind: &str, execution_mode: &str) -> PlannerCapabilityMapping {
    toml::from_str(&format!(
        r#"
name = "test.background"
action = "run"
effect = "mutate"
execution_mode = "{execution_mode}"
async_adapter_kind = "{adapter_kind}"
"#
    ))
    .expect("async mapping")
}

#[test]
fn only_local_process_async_capabilities_use_the_durable_runner_supervisor() {
    assert!(local_process_durable_background_requested(Some(
        &async_mapping("local_process_poll", "async_preferred")
    )));
    assert!(local_process_durable_background_requested(Some(
        &async_mapping("local_process_poll", "async_required")
    )));
    assert!(!local_process_durable_background_requested(Some(
        &async_mapping("media_job_poll", "async_preferred")
    )));
    assert!(!local_process_durable_background_requested(Some(
        &async_mapping("local_process_poll", "sync_short")
    )));
    assert!(!local_process_durable_background_requested(None));
}

#[test]
fn durable_skill_endpoint_tokens_cover_the_retention_window() {
    assert_eq!(
        skill_secret_token_ttl(false, 86_400),
        Duration::from_secs(300)
    );
    assert_eq!(skill_secret_token_ttl(true, 60), Duration::from_secs(300));
    assert_eq!(
        skill_secret_token_ttl(true, 86_400),
        Duration::from_secs(86_400)
    );
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
fn warm_reuse_requires_exact_readonly_package_and_action_guards() {
    let mapping = preview_mapping();
    assert!(stateless_readonly_reuse_allowed(
        skill_sdk::ExecutionProfile::StatelessReadonly,
        skill_sdk::SandboxProfile::ReadOnly,
        &[Capability::FsRead],
        Some(&mapping),
        false,
        false,
        false,
    ));
    assert!(!stateless_readonly_reuse_allowed(
        skill_sdk::ExecutionProfile::PerRequest,
        skill_sdk::SandboxProfile::ReadOnly,
        &[Capability::FsRead],
        Some(&mapping),
        false,
        false,
        false,
    ));
    assert!(!stateless_readonly_reuse_allowed(
        skill_sdk::ExecutionProfile::StatelessReadonly,
        skill_sdk::SandboxProfile::ReadOnly,
        &[Capability::FsRead, Capability::Net],
        Some(&mapping),
        false,
        false,
        false,
    ));
    assert!(!stateless_readonly_reuse_allowed(
        skill_sdk::ExecutionProfile::StatelessReadonly,
        skill_sdk::SandboxProfile::ReadOnly,
        &[Capability::FsRead],
        Some(&mapping),
        true,
        false,
        false,
    ));
    assert!(!stateless_readonly_reuse_allowed(
        skill_sdk::ExecutionProfile::StatelessReadonly,
        skill_sdk::SandboxProfile::WorkspaceWrite,
        &[Capability::FsRead],
        Some(&mapping),
        false,
        false,
        false,
    ));
}

#[test]
fn runner_dispatch_metadata_is_machine_readable() {
    let mut response = json!({"status": "ok", "extra": {}});
    add_runner_dispatch_metadata(&mut response, "per_request", Some("warm_pool_low_memory"));
    assert_eq!(
        response.pointer("/extra/runner_dispatch/mode"),
        Some(&json!("per_request"))
    );
    assert_eq!(
        response.pointer("/extra/runner_dispatch/fallback_reason"),
        Some(&json!("warm_pool_low_memory"))
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
        None,
        std::path::Path::new("/runtime/artifacts/invocation"),
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
    assert_eq!(
        context
            .get("artifact_output_directory")
            .and_then(serde_json::Value::as_str),
        Some("/runtime/artifacts/invocation")
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
        Some(descriptor),
        std::path::Path::new("/runtime/artifacts/invocation"),
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
        binding: Default::default(),
        registry: Some(std::sync::Arc::new(registry)),
        skills_list: std::sync::Arc::new(enabled),
    });
    let descriptor = storage_descriptor_for_skill(&state, "kb")
        .expect("valid KB storage declaration")
        .expect("KB storage descriptor");
    assert_eq!(descriptor.schema_version, 3);
    assert_eq!(descriptor.skill_name, "kb");
}

#[test]
fn successful_runner_results_require_the_exact_execution_binding() {
    let response = json!({
        "status": "ok",
        "extra": {
            "execution_binding": {
                "skill_name": "fixture_skill",
                "version": "2.0.0",
                "manifest_digest": "a".repeat(64),
                "receipt_digest": "b".repeat(64),
                "registry_generation": 7,
                "registry_generation_digest": "c".repeat(64),
                "base_registry_digest": "f".repeat(64),
                "overlay_generation_digest": "0".repeat(64),
                "policy_digest": "d".repeat(64),
                "admission_receipt_digest": "e".repeat(64)
            }
        }
    });
    validate_runner_execution_binding(
        &response,
        "fixture_skill",
        "2.0.0",
        &"a".repeat(64),
        &"b".repeat(64),
        7,
        Some(&"c".repeat(64)),
        Some(&"f".repeat(64)),
        Some(&"0".repeat(64)),
        Some(&"d".repeat(64)),
        Some(&"e".repeat(64)),
    )
    .expect("exact execution binding");

    let mut mismatched = response;
    mismatched["extra"]["execution_binding"]["version"] = json!("3.0.0");
    assert!(validate_runner_execution_binding(
        &mismatched,
        "fixture_skill",
        "2.0.0",
        &"a".repeat(64),
        &"b".repeat(64),
        7,
        Some(&"c".repeat(64)),
        Some(&"f".repeat(64)),
        Some(&"0".repeat(64)),
        Some(&"d".repeat(64)),
        Some(&"e".repeat(64)),
    )
    .expect_err("version mismatch must fail closed")
    .contains("field=version"));
}

#[test]
fn runner_resolution_errors_do_not_claim_an_execution_binding() {
    let response = json!({
        "status": "error",
        "extra": {"error_code": "runner_resolution_failed"}
    });
    validate_runner_execution_binding(
        &response,
        "fixture_skill",
        "2.0.0",
        &"a".repeat(64),
        &"b".repeat(64),
        7,
        Some(&"c".repeat(64)),
        Some(&"f".repeat(64)),
        Some(&"0".repeat(64)),
        Some(&"d".repeat(64)),
        Some(&"e".repeat(64)),
    )
    .expect("pre-execution structured failure is allowed without an actual binding");
}

#[test]
fn pinned_poll_binding_is_normalized_for_exact_runner_dispatch() {
    let binding = json!({
        "skill_name": "fixture_skill",
        "version": "2.0.0",
        "manifest_digest": "a".repeat(64),
        "receipt_digest": "b".repeat(64),
        "registry_generation": 7,
        "registry_generation_digest": "c".repeat(64),
        "base_registry_digest": "f".repeat(64),
        "overlay_generation_digest": "0".repeat(64),
        "policy_digest": "d".repeat(64),
        "admission_receipt_digest": "e".repeat(64)
    });

    let pinned = parse_pinned_runner_execution_binding(&binding, "fixture_skill")
        .expect("valid pinned binding");

    assert_eq!(pinned.version, "2.0.0");
    assert_eq!(pinned.manifest_digest, "a".repeat(64));
    assert_eq!(pinned.package_receipt, "b".repeat(64));
    assert_eq!(pinned.value, binding);
}

#[test]
fn pinned_poll_binding_rejects_skill_drift_and_unknown_fields() {
    let mut binding = json!({
        "skill_name": "fixture_skill",
        "version": "2.0.0",
        "manifest_digest": "a".repeat(64),
        "receipt_digest": "b".repeat(64),
        "registry_generation": 7,
        "registry_generation_digest": null,
        "base_registry_digest": null,
        "overlay_generation_digest": null,
        "policy_digest": null,
        "admission_receipt_digest": null
    });
    assert!(
        parse_pinned_runner_execution_binding(&binding, "replacement_skill")
            .expect_err("skill drift must fail closed")
            .contains("skill mismatch")
    );

    binding["current_pointer"] = json!(true);
    assert!(
        parse_pinned_runner_execution_binding(&binding, "fixture_skill")
            .expect_err("unknown binding fields must fail closed")
            .contains("unknown fields")
    );
}

#[cfg(unix)]
#[test]
fn durable_job_metadata_write_failure_removes_partial_job_directory() {
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::temp_dir().join(format!(
        "agent_durable_job_fault_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&root).expect("create tempdir");
    let plan = DurableRunnerJobPlan::new(&root, "fixture_skill", 600, None);
    plan.create_directories().expect("create durable job paths");
    let guard = DurableRunnerJobSetupGuard::new(&plan.job_dir);
    std::fs::write(plan.job_dir.join("partial"), "before-fault").expect("write partial metadata");
    std::fs::set_permissions(&plan.job_dir, std::fs::Permissions::from_mode(0o500))
        .expect("make metadata directory read-only");

    let error = write_durable_job_metadata(&plan.job_dir, "pid", "123")
        .expect_err("injected metadata write failure");

    std::fs::set_permissions(&plan.job_dir, std::fs::Permissions::from_mode(0o700))
        .expect("restore directory permissions for cleanup");
    drop(guard);
    assert!(error.contains("durable_skill_job_metadata_write_failed"));
    assert!(!plan.job_dir.exists());
    std::fs::remove_dir_all(root).expect("remove tempdir");
}

#[test]
fn durable_queue_scope_uses_an_isolated_single_lane_root() {
    let root = std::env::temp_dir().join(format!(
        "agent_durable_queue_scope_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let ordinary = DurableRunnerJobPlan::new(&root, "fixture_skill", 600, None);
    let queued = DurableRunnerJobPlan::new(
        &root,
        "fixture_skill",
        600,
        Some("__dispatch_queue__fixture_skill__user__7"),
    );

    assert!(!ordinary.queue_scoped);
    assert!(queued.queue_scoped);
    assert!(ordinary.skill_slot_root.ends_with("skills/fixture_skill"));
    assert!(queued.skill_slot_root.starts_with(
        claw_core::workspace_state::workspace_state_root(&root)
            .join("durable_skill_slots/queues/fixture_skill")
    ));
    assert_eq!(
        queued
            .skill_slot_root
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::len),
        Some(64)
    );
    assert_ne!(ordinary.skill_slot_root, queued.skill_slot_root);
}
