use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
    assert_eq!(
        extra["message_key"],
        "skill.music_generate.execution_failed"
    );
    assert_eq!(extra["retryable"], false);
}

#[test]
fn normalize_format_defaults_to_mp3() {
    assert_eq!(normalize_format("wav"), "wav");
    assert_eq!(normalize_format("unknown"), "mp3");
}

#[test]
fn dry_run_can_generate_with_prompt_only_by_enabling_lyrics_optimizer() {
    let root = unique_temp_root("music-dry-run");
    let (text, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "prompt": "Upbeat jazz song about a summer beach",
            "dry_run": true
        }),
    )
    .expect("dry run");

    assert_eq!(text, "MUSIC_GENERATE_DRY_RUN");
    assert_eq!(extra["provider"], "minimax");
    assert_eq!(extra["dry_run"], true);
    assert_eq!(extra["adapter_kind"], "media_job_poll");
    assert_eq!(extra["request"]["lyrics_optimizer"], true);
    assert_eq!(extra["planned_outputs"][0]["type"], "audio_file");
    assert!(
        extra["planned_outputs"][0]["path"]
            .as_str()
            .is_some_and(|path| path.ends_with(".mp3")),
        "{extra}"
    );
    assert_eq!(
        extra["pending_async_job_contract"]["poll_adapter"]["kind"],
        "media_job_poll"
    );
    assert_eq!(
        extra["pending_async_job_contract"]["poll_adapter"]["skill_name"],
        "music_generate"
    );
    assert_eq!(
        extra["pending_async_job_contract"]["result_ref"],
        extra["pending_async_job_contract"]["job_id"]
    );
    assert_eq!(
        extra["pending_async_job_contract"]["cancel_token"],
        extra["pending_async_job_contract"]["cancel_ref"]
    );
    assert_eq!(extra["pending_async_job_contract"]["retryable"], true);
    assert!(extra.get("pending_async_job").is_none());
    assert!(extra["outputs"].as_array().unwrap().is_empty());
}

#[test]
fn preview_action_forces_dry_run_without_writing_file() {
    let root = unique_temp_root("music-preview");
    let (text, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "preview_generate",
            "prompt": "A short ambient cue",
            "output_path": "music/preview.mp3",
            "dry_run": false
        }),
    )
    .expect("preview should force dry-run");

    assert_eq!(text, "MUSIC_GENERATE_DRY_RUN");
    assert_eq!(extra["dry_run"], true);
    assert!(!root.join("music/preview.mp3").exists());
}

#[test]
fn dedicated_provider_empty_key_falls_back_to_shared_minimax_key() {
    let mut cfg = RootConfig::default();
    cfg.llm.minimax = Some(VendorConfig {
        base_url: "https://shared.example/v1".to_string(),
        api_key: "shared-key".to_string(),
        model: "music-2.6-free".to_string(),
        timeout_seconds: Some(88),
        adapter_kind: None,
    });
    cfg.music_generation.providers.minimax = Some(VendorConfig {
        base_url: "https://dedicated.example/v1".to_string(),
        api_key: String::new(),
        model: "music-2.6".to_string(),
        timeout_seconds: None,
        adapter_kind: None,
    });

    let resolved = resolved_vendor_config(&cfg, VendorKind::MiniMax).expect("provider");

    assert_eq!(resolved.base_url, "https://dedicated.example/v1");
    assert_eq!(resolved.api_key, "shared-key");
    assert_eq!(resolved.model, "music-2.6");
    assert_eq!(resolved.timeout_seconds, Some(88));
}

#[test]
fn custom_provider_can_dry_run_with_explicit_adapter_kind() {
    let root = unique_temp_root("music-custom-dry-run");
    let mut cfg = RootConfig::default();
    cfg.music_generation.default_vendor = Some("custom".to_string());
    cfg.music_generation.providers.custom = Some(VendorConfig {
        base_url: "https://custom.example/v1".to_string(),
        api_key: "custom-key".to_string(),
        model: "custom-music-model".to_string(),
        timeout_seconds: Some(300),
        adapter_kind: Some("minimax_compatible".to_string()),
    });

    let (_, extra) = execute(
        &cfg,
        &root,
        json!({
            "prompt": "Warm ambient piano loop",
            "is_instrumental": true,
            "dry_run": true
        }),
    )
    .expect("dry run");

    assert_eq!(extra["provider"], "custom");
    assert_eq!(extra["model"], "custom-music-model");
    assert_eq!(extra["model_kind"], "minimax_native");
}

#[test]
fn resolve_output_path_uses_requested_workspace_path() {
    let workspace = PathBuf::from("/tmp/rustclaw-music-test");
    let out = resolve_output_path(&workspace, "music/download", Some("tmp/song.mp3"), "mp3")
        .expect("output path");
    assert_eq!(out, workspace.join("tmp/song.mp3"));
}

#[test]
fn poll_dry_run_running_returns_adapter_reschedule_result() {
    let root = unique_temp_root("music-poll-running");
    let (_, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "poll",
            "task_id": "provider-task-running",
            "job_id": "provider:music_generate:minimax:provider-task-running",
            "vendor": "minimax",
            "dry_run": true,
            "mock_status": "processing",
            "poll_after_ms": 2500,
            "expires_at": unix_ts() as i64 + 600
        }),
    )
    .expect("poll dry run");

    assert_eq!(extra["async_poll_adapter_result"]["status"], "running");
    assert_eq!(extra["async_poll_adapter_result"]["poll_after_seconds"], 3);
    assert_eq!(extra["async_poll_adapter_result"]["poll_after_ms"], 3_000);
    assert_eq!(extra["async_poll_adapter_result"]["retryable"], true);
    assert_eq!(
        extra["async_poll_adapter_result"]["job_id"],
        "provider:music_generate:minimax:provider-task-running"
    );
    assert!(extra["async_poll_adapter_result"].get("text").is_none());
    assert!(extra["async_poll_adapter_result"]
        .get("error_text")
        .is_none());
}

#[test]
fn poll_dry_run_success_returns_adapter_final_result() {
    let root = unique_temp_root("music-poll-success");
    let (_, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "poll",
            "task_id": "provider-task-success",
            "job_id": "provider:music_generate:minimax:provider-task-success",
            "vendor": "minimax",
            "dry_run": true,
            "mock_status": "succeeded",
            "output_path": "music/success.mp3",
            "poll_after_seconds": 3,
            "expires_at": unix_ts() as i64 + 600
        }),
    )
    .expect("poll dry run");

    assert_eq!(extra["async_poll_adapter_result"]["status"], "succeeded");
    assert_eq!(
        extra["async_poll_adapter_result"]["final_result_json"]["source"],
        "music_generate_poll_adapter"
    );
    assert!(
        extra["async_poll_adapter_result"]["final_result_json"]["output_path"]
            .as_str()
            .is_some_and(|path| path.ends_with("music/success.mp3")),
        "{extra}"
    );
    assert!(extra["async_poll_adapter_result"].get("text").is_none());
    assert!(extra["async_poll_adapter_result"]
        .get("error_text")
        .is_none());
}

#[test]
fn cancel_dry_run_returns_adapter_cancelled_result() {
    let root = unique_temp_root("music-cancel-dry-run");
    let (_, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "cancel",
            "task_id": "provider-task-cancel",
            "job_id": "provider:music_generate:minimax:provider-task-cancel",
            "vendor": "minimax",
            "dry_run": true
        }),
    )
    .expect("cancel dry run");

    assert_eq!(extra["status"], "cancelled");
    assert_eq!(extra["async_cancel_adapter_result"]["status"], "cancelled");
    assert_eq!(extra["async_poll_adapter_result"]["status"], "cancelled");
    assert_eq!(
        extra["async_cancel_adapter_result"]["cancellation_result_json"]["source"],
        "music_generate_cancel_adapter"
    );
    assert_eq!(
        extra["async_cancel_adapter_result"]["message_key"],
        "clawd.task.cancelled"
    );
    assert_eq!(extra["async_cancel_adapter_result"]["retryable"], false);
    assert!(extra["async_cancel_adapter_result"].get("text").is_none());
    assert!(extra["async_cancel_adapter_result"]
        .get("error_text")
        .is_none());
}

#[test]
fn cancel_live_without_provider_adapter_returns_structured_contract() {
    let root = unique_temp_root("music-cancel-adapter-contract");
    let (_, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "cancel",
            "task_id": "provider-task-cancel",
            "job_id": "provider:music_generate:minimax:provider-task-cancel",
            "vendor": "minimax"
        }),
    )
    .expect("cancel contract");

    assert_eq!(extra["status"], "requires_provider_adapter");
    assert_eq!(
        extra["async_cancel_adapter_result"]["status"],
        "requires_provider_adapter"
    );
    assert_eq!(
        extra["async_cancel_adapter_result"]["error_code"],
        "provider_cancel_adapter_missing"
    );
    assert_eq!(
        extra["async_cancel_adapter_result"]["provider_cancel_contract"]["provider"],
        "minimax"
    );
    assert!(extra["async_cancel_adapter_result"].get("text").is_none());
    assert!(extra["async_cancel_adapter_result"]
        .get("error_text")
        .is_none());
}

fn unique_temp_root(name: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "rustclaw-{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("temp root");
    root
}
