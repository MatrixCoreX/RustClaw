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
        "skill.image_generate.execution_failed"
    );
    assert_eq!(extra["retryable"], false);
}

#[test]
fn parse_vendor_aliases() {
    assert_eq!(parse_vendor("openai"), Some(VendorKind::OpenAI));
    assert_eq!(parse_vendor("gemini"), Some(VendorKind::Google));
    assert_eq!(parse_vendor("claude"), Some(VendorKind::Anthropic));
    assert_eq!(parse_vendor("xai"), Some(VendorKind::Grok));
    assert_eq!(parse_vendor("qwen"), Some(VendorKind::Qwen));
}

#[test]
fn extract_qwen_choice_image_url() {
    let v = json!({
        "output": {
            "choices": [{
                "message": {
                    "content": [{
                        "type": "image",
                        "image": "https://example.com/demo.png"
                    }]
                }
            }]
        }
    });
    assert_eq!(
        extract_qwen_output_image_url(&v),
        Some("https://example.com/demo.png")
    );
}

#[test]
fn resolve_output_path_uses_requested_workspace_path() {
    let workspace = PathBuf::from("/tmp/rustclaw");
    let out = resolve_output_path(
        &workspace,
        "image",
        Some("document/skill_generate_smoke.png"),
    )
    .expect("requested output path");

    assert_eq!(out, workspace.join("document/skill_generate_smoke.png"));
}

#[test]
fn provider_failures_do_not_use_local_fallback_by_default() {
    let root = unique_temp_root("image-generate-no-fallback");
    let err = execute(
        &RootConfig::default(),
        &root,
        json!({"prompt":"minimal smoke card","output_path":"document/out.png"}),
    )
    .expect_err("local fallback is disabled by default");

    assert!(err.contains("all providers failed"), "{err}");
    assert!(!root.join("document/out.png").exists());
}

#[test]
fn dry_run_returns_machine_payload_without_writing_file() {
    let root = unique_temp_root("image-generate-dry-run");
    let (text, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "prompt": "minimal smoke card",
            "output_path": "document/out.png",
            "dry_run": true,
            "vendor": "minimax",
            "model": "image-01"
        }),
    )
    .expect("dry-run should not require provider credentials");

    let out = root.join("document/out.png");
    assert_eq!(text, "IMAGE_GENERATE_DRY_RUN");
    assert!(!out.exists());
    assert_eq!(extra["action"], "generate");
    assert_eq!(extra["status"], "dry_run");
    assert_eq!(extra["dry_run"], true);
    assert_eq!(extra["would_mutate"], false);
    assert_eq!(extra["provider"], "minimax");
    assert_eq!(extra["model"], "image-01");
    assert_eq!(extra["model_kind"], "dry_run");
    assert_eq!(extra["message_key"], "image_generate.msg.dry_run");
    assert_eq!(extra["media_type"], "image");
    assert_eq!(
        extra["pending_async_job_contract"]["poll_adapter"]["kind"],
        "media_job_poll"
    );
    assert_eq!(
        extra["async_contract"]["poll_adapter"]["kind"],
        "media_job_poll"
    );
    assert_eq!(
        extra["planned_outputs"][0]["path"].as_str(),
        Some(out.to_string_lossy().as_ref())
    );
    assert!(extra["outputs"].as_array().is_some_and(Vec::is_empty));
}

#[test]
fn preview_generate_forces_no_provider_call_or_file_write() {
    let root = unique_temp_root("image-generate-preview");
    let (text, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "preview_generate",
            "prompt": "minimal status card",
            "size": "512x512",
            "output_path": "document/media_dry_run/tier1.png",
            "vendor": "minimax",
            "model": "image-01"
        }),
    )
    .expect("preview should not require provider credentials");

    let out = root.join("document/media_dry_run/tier1.png");
    assert_eq!(text, "IMAGE_GENERATE_DRY_RUN");
    assert!(!out.exists());
    assert!(!out.parent().expect("parent").exists());
    assert_eq!(extra["action"], "preview_generate");
    assert_eq!(extra["status"], "dry_run");
    assert_eq!(extra["would_mutate"], false);
    assert_eq!(extra["provider"], "minimax");
    assert_eq!(extra["model"], "image-01");
    assert_eq!(
        extra["field_value"]["planned_outputs"][0]["path"],
        json!(out)
    );
    assert_eq!(
        extra["field_value"]["async_contract"]["poll_adapter"]["kind"],
        "media_job_poll"
    );
}

#[test]
fn poll_dry_run_returns_structured_adapter_result() {
    let root = unique_temp_root("image-generate-poll-dry-run");
    let (text, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "poll",
            "task_id": "task-123",
            "job_id": "job-123",
            "output_path": "document/poll.png",
            "dry_run": true,
            "mock_status": "succeeded",
            "vendor": "minimax",
            "model": "image-01"
        }),
    )
    .expect("poll dry-run should not require provider credentials");

    assert_eq!(text, "IMAGE_TASK:task-123");
    assert_eq!(extra["status"], "succeeded");
    assert_eq!(
        extra["async_poll_adapter_result"]["final_result_json"]["outputs"][0]["type"],
        "image_file"
    );
}

#[test]
fn cancel_dry_run_returns_structured_adapter_result() {
    let root = unique_temp_root("image-generate-cancel-dry-run");
    let (text, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "cancel",
            "task_id": "task-456",
            "job_id": "job-456",
            "dry_run": true,
            "vendor": "minimax",
            "model": "image-01"
        }),
    )
    .expect("cancel dry-run should not require provider credentials");

    assert_eq!(text, "IMAGE_TASK_CANCELLED:task-456");
    assert_eq!(extra["status"], "cancelled");
    assert_eq!(
        extra["async_cancel_adapter_result"]["cancellation_result_json"]["status"],
        "cancelled"
    );
}

#[test]
fn explicit_local_fallback_writes_image_file() {
    let root = unique_temp_root("image-generate-local-fallback");
    let mut cfg = RootConfig::default();
    cfg.image_generation.local_fallback_enabled = true;

    let (text, extra) = execute(
        &cfg,
        &root,
        json!({"prompt":"minimal smoke card","output_path":"document/out.png"}),
    )
    .expect("local fallback should produce a file");

    let out = root.join("document/out.png");
    let bytes = std::fs::read(&out).expect("fallback image");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(text.contains(&format!("FILE:{}", out.display())), "{text}");
    assert_eq!(extra["message_key"], "image_generate.msg.saved");
    assert_eq!(extra["media_type"], "image");
    assert_eq!(
        extra["output_path"].as_str(),
        Some(out.to_string_lossy().as_ref())
    );
    assert_eq!(extra["provider"], "local_fallback");
    assert_eq!(extra["model_kind"], "local_fallback");
    assert_eq!(
        extra["outputs"][0]["path"].as_str(),
        Some(out.to_string_lossy().as_ref())
    );
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
