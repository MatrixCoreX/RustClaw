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
        "skill.audio_synthesize.execution_failed"
    );
    assert_eq!(extra["retryable"], false);
}

#[test]
fn parse_vendor_aliases() {
    assert!(matches!(parse_vendor("openai"), Some(VendorKind::OpenAI)));
    assert!(matches!(parse_vendor("gemini"), Some(VendorKind::Google)));
    assert!(matches!(
        parse_vendor("claude"),
        Some(VendorKind::Anthropic)
    ));
    assert!(matches!(parse_vendor("xai"), Some(VendorKind::Grok)));
}

#[test]
fn normalize_and_ext() {
    assert_eq!(normalize_format("mp3"), "mp3");
    assert_eq!(normalize_format("unknown"), "opus");
    assert_eq!(mimo_audio_format("mp3"), "mp3");
    assert_eq!(google_audio_encoding("mp3"), "MP3");
    assert_eq!(output_ext("opus"), "ogg");
}

#[test]
fn mimo_voice_metadata_comes_from_config() {
    assert!(canonical_voice_name(MIMO_TTS_VOICES, "mimo_default").is_some());
    assert!(canonical_voice_name(MIMO_TTS_VOICES, "冰糖").is_none());

    let cfg = AudioSynthesizeConfig {
        mimo_voices: Some(vec!["mimo_default".to_string(), "冰糖".to_string()]),
        ..AudioSynthesizeConfig::default()
    };

    assert_eq!(
        resolve_voice_for_vendor(VendorKind::Mimo, Some("冰糖"), None, &cfg),
        "冰糖"
    );
}

#[test]
fn dry_run_returns_machine_payload_without_writing_file() {
    let root = unique_temp_root("audio-synthesize-dry-run");
    let (text, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "text": "hello from dry-run",
            "output_path": "audio/out.mp3",
            "format": "mp3",
            "dry_run": true,
            "vendor": "mimo",
            "model": "mimo-v2.5-tts"
        }),
    )
    .expect("dry-run should not require provider credentials");

    let out = root.join("audio/out.mp3");
    assert_eq!(text, "AUDIO_SYNTHESIZE_DRY_RUN");
    assert!(!out.exists());
    assert_eq!(extra["dry_run"], true);
    assert_eq!(extra["provider"], "mimo");
    assert_eq!(extra["model"], "mimo-v2.5-tts");
    assert_eq!(extra["model_kind"], "dry_run");
    assert_eq!(extra["response_format"], "mp3");
    assert_eq!(
        extra["pending_async_job_contract"]["poll_adapter"]["kind"],
        "media_job_poll"
    );
    assert_eq!(
        extra["planned_outputs"][0]["path"].as_str(),
        Some(out.to_string_lossy().as_ref())
    );
    assert!(extra["outputs"].as_array().is_some_and(Vec::is_empty));
}

#[test]
fn poll_dry_run_returns_structured_adapter_result() {
    let root = unique_temp_root("audio-synthesize-poll-dry-run");
    let (text, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "poll",
            "task_id": "task-123",
            "job_id": "job-123",
            "output_path": "audio/poll.mp3",
            "format": "mp3",
            "dry_run": true,
            "mock_status": "succeeded",
            "vendor": "minimax",
            "model": "speech-2.8-turbo"
        }),
    )
    .expect("poll dry-run should not require provider credentials");

    assert_eq!(text, "AUDIO_TASK:task-123");
    assert_eq!(extra["status"], "succeeded");
    assert_eq!(
        extra["async_poll_adapter_result"]["final_result_json"]["outputs"][0]["type"],
        "audio_file"
    );
}

#[test]
fn cancel_dry_run_returns_structured_adapter_result() {
    let root = unique_temp_root("audio-synthesize-cancel-dry-run");
    let (text, extra) = execute(
        &RootConfig::default(),
        &root,
        json!({
            "action": "cancel",
            "task_id": "task-456",
            "job_id": "job-456",
            "dry_run": true,
            "vendor": "minimax",
            "model": "speech-2.8-turbo"
        }),
    )
    .expect("cancel dry-run should not require provider credentials");

    assert_eq!(text, "AUDIO_TASK_CANCELLED:task-456");
    assert_eq!(extra["status"], "cancelled");
    assert_eq!(
        extra["async_cancel_adapter_result"]["cancellation_result_json"]["status"],
        "cancelled"
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
