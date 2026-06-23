use super::*;

#[test]
fn resolve_output_path_uses_workspace_relative_path() {
    let workspace = PathBuf::from("/tmp/rustclaw-video-test");
    let out = resolve_output_path(&workspace, "video/download", Some("tmp/out.mp4"))
        .expect("output path");
    assert_eq!(out, workspace.join("tmp/out.mp4"));
}

#[test]
fn dry_run_returns_request_payload_without_key() {
    let root = unique_temp_root("video-dry-run");
    let (text, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "prompt": "A small robot waves to the camera",
            "duration": 6,
            "dry_run": true
        }),
    )
    .expect("dry run");

    assert_eq!(text, "VIDEO_GENERATE_DRY_RUN");
    assert_eq!(extra["provider"], "minimax");
    assert_eq!(extra["dry_run"], true);
    assert_eq!(extra["request"]["model"], DEFAULT_MODEL);
    assert_eq!(extra["planned_outputs"][0]["type"], "video_file");
    assert!(
        extra["planned_outputs"][0]["path"]
            .as_str()
            .is_some_and(|path| path.ends_with(".mp4")),
        "{extra}"
    );
    assert!(extra["outputs"].as_array().unwrap().is_empty());
}

#[test]
fn dedicated_provider_empty_key_falls_back_to_shared_minimax_key() {
    let mut cfg = RootConfig::default();
    cfg.llm.minimax = Some(VendorConfig {
        base_url: "https://shared.example/v1".to_string(),
        api_key: "shared-key".to_string(),
        model: "MiniMax-Hailuo-02".to_string(),
        timeout_seconds: Some(77),
        adapter_kind: None,
    });
    cfg.video_generation.providers.minimax = Some(VendorConfig {
        base_url: "https://dedicated.example/v1".to_string(),
        api_key: String::new(),
        model: "MiniMax-Hailuo-2.3".to_string(),
        timeout_seconds: None,
        adapter_kind: None,
    });

    let resolved = resolved_vendor_config(&cfg, VendorKind::MiniMax).expect("provider");

    assert_eq!(resolved.base_url, "https://dedicated.example/v1");
    assert_eq!(resolved.api_key, "shared-key");
    assert_eq!(resolved.model, "MiniMax-Hailuo-2.3");
    assert_eq!(resolved.timeout_seconds, Some(77));
}

#[test]
fn custom_provider_can_dry_run_with_explicit_adapter_kind() {
    let root = unique_temp_root("video-custom-dry-run");
    let mut cfg = RootConfig::default();
    cfg.video_generation.default_vendor = Some("custom".to_string());
    cfg.video_generation.providers.custom = Some(VendorConfig {
        base_url: "https://custom.example/v1".to_string(),
        api_key: "custom-key".to_string(),
        model: "custom-video-model".to_string(),
        timeout_seconds: Some(120),
        adapter_kind: Some("minimax_compatible".to_string()),
    });

    let (_, extra) = execute(
        &cfg,
        &root,
        json!({
            "prompt": "A small robot waves to the camera",
            "dry_run": true
        }),
    )
    .expect("dry run");

    assert_eq!(extra["provider"], "custom");
    assert_eq!(extra["model"], "custom-video-model");
    assert_eq!(extra["model_kind"], "minimax_native");
}

#[test]
fn pending_video_task_response_exposes_skill_poll_adapter() {
    let (_, extra) = video_pending_task_response(
        "provider-task-1",
        "minimax",
        "MiniMax-Hailuo-2.3",
        VideoAdapterKind::MiniMaxNative,
        5,
        600,
        true,
        "video/download/out.mp4",
    );

    assert_eq!(
        extra["pending_async_job"]["job_id"],
        "provider:video_generate:minimax:provider-task-1"
    );
    assert_eq!(extra["pending_async_job"]["status"], "accepted");
    assert_eq!(
        extra["pending_async_job"]["poll_adapter"]["kind"],
        "skill_poll"
    );
    assert_eq!(
        extra["pending_async_job"]["poll_adapter"]["skill_name"],
        "video_generate"
    );
    assert_eq!(
        extra["pending_async_job"]["poll_adapter"]["args"]["action"],
        "poll"
    );
    assert!(extra["pending_async_job"]["poll_adapter"]
        .get("text")
        .is_none());
    assert!(extra["pending_async_job"]["poll_adapter"]
        .get("error_text")
        .is_none());
}

#[test]
fn poll_dry_run_running_returns_adapter_reschedule_result() {
    let root = unique_temp_root("video-poll-running");
    let (_, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "poll",
            "task_id": "provider-task-running",
            "job_id": "provider:video_generate:minimax:provider-task-running",
            "vendor": "minimax",
            "dry_run": true,
            "mock_status": "Processing",
            "poll_after_seconds": 3,
            "expires_at": unix_ts() as i64 + 600
        }),
    )
    .expect("poll dry run");

    assert_eq!(extra["async_poll_adapter_result"]["status"], "running");
    assert_eq!(
        extra["async_poll_adapter_result"]["job_id"],
        "provider:video_generate:minimax:provider-task-running"
    );
    assert!(extra["async_poll_adapter_result"].get("text").is_none());
    assert!(extra["async_poll_adapter_result"]
        .get("error_text")
        .is_none());
}

#[test]
fn poll_dry_run_queueing_returns_adapter_accepted_result() {
    let root = unique_temp_root("video-poll-accepted");
    let (_, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "poll",
            "task_id": "provider-task-accepted",
            "job_id": "provider:video_generate:minimax:provider-task-accepted",
            "vendor": "minimax",
            "dry_run": true,
            "mock_status": "Queueing",
            "poll_after_seconds": 3,
            "expires_at": unix_ts() as i64 + 600
        }),
    )
    .expect("poll dry run");

    assert_eq!(extra["async_poll_adapter_result"]["status"], "accepted");
    assert_eq!(
        extra["async_poll_adapter_result"]["message_key"],
        "clawd.task.async_job_pending"
    );
}

#[test]
fn poll_dry_run_success_returns_adapter_final_result() {
    let root = unique_temp_root("video-poll-success");
    let (_, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "poll",
            "task_id": "provider-task-success",
            "job_id": "provider:video_generate:minimax:provider-task-success",
            "vendor": "minimax",
            "dry_run": true,
            "mock_status": "Success",
            "mock_file_id": "file-1",
            "download": false,
            "poll_after_seconds": 3,
            "expires_at": unix_ts() as i64 + 600
        }),
    )
    .expect("poll dry run");

    assert_eq!(extra["async_poll_adapter_result"]["status"], "succeeded");
    assert_eq!(
        extra["async_poll_adapter_result"]["final_result_json"]["source"],
        "video_generate_poll_adapter"
    );
    assert_eq!(
        extra["async_poll_adapter_result"]["final_result_json"]["file_id"],
        "file-1"
    );
    assert!(extra["async_poll_adapter_result"].get("text").is_none());
    assert!(extra["async_poll_adapter_result"]
        .get("error_text")
        .is_none());
}

#[test]
fn poll_dry_run_failed_returns_adapter_failure_result() {
    let root = unique_temp_root("video-poll-failed");
    let (_, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "poll",
            "task_id": "provider-task-failed",
            "job_id": "provider:video_generate:minimax:provider-task-failed",
            "vendor": "minimax",
            "dry_run": true,
            "mock_status": "Fail",
            "poll_after_seconds": 3,
            "expires_at": unix_ts() as i64 + 600
        }),
    )
    .expect("poll dry run");

    assert_eq!(extra["async_poll_adapter_result"]["status"], "failed");
    assert_eq!(
        extra["async_poll_adapter_result"]["error_code"],
        "provider_video_job_failed"
    );
    assert_eq!(
        extra["async_poll_adapter_result"]["failure_result_json"]["source"],
        "video_generate_poll_adapter"
    );
}

#[test]
fn poll_dry_run_expired_returns_adapter_expired_result() {
    let root = unique_temp_root("video-poll-expired");
    let (_, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "poll",
            "task_id": "provider-task-expired",
            "job_id": "provider:video_generate:minimax:provider-task-expired",
            "vendor": "minimax",
            "dry_run": true,
            "poll_after_seconds": 3,
            "expires_at": unix_ts() as i64 - 1
        }),
    )
    .expect("poll dry run");

    assert_eq!(extra["async_poll_adapter_result"]["status"], "expired");
    assert_eq!(
        extra["async_poll_adapter_result"]["error_code"],
        "async_poll_expired"
    );
}

#[test]
fn local_image_path_converts_to_data_url() {
    let root = unique_temp_root("video-image-source");
    let image = root.join("image.png");
    std::fs::write(&image, b"fake-png").expect("write image");

    let value = image_arg_to_api_value(&root, Some(&json!({"path": "image.png"})), 1024)
        .expect("image source")
        .expect("data url");

    assert!(value.starts_with("data:image/png;base64,"));
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
