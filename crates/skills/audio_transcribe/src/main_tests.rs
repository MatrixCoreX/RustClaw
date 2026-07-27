use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("provider_request_failed", true);

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_code"], "provider_request_failed");
    assert_eq!(extra["error_kind"], "provider_request_failed");
    assert_eq!(
        extra["message_key"],
        "skill.audio_transcribe.provider_request_failed"
    );
    assert_eq!(extra["retryable"], true);
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
    assert!(matches!(parse_vendor("local"), Some(VendorKind::Custom)));
    assert!(matches!(
        parse_vendor("whisper.cpp"),
        Some(VendorKind::Custom)
    ));
}

fn vendor_cfg(base_url: &str, api_key: &str) -> VendorConfig {
    VendorConfig {
        base_url: base_url.to_string(),
        api_key: api_key.to_string(),
        model: "local-whisper".to_string(),
        timeout_seconds: None,
    }
}

#[test]
fn local_custom_provider_allows_missing_api_key() {
    let cfg = vendor_cfg("http://127.0.0.1:8178/v1", "");
    assert_eq!(provider_auth_token("custom", &cfg).unwrap(), None);

    let placeholder = vendor_cfg("http://localhost:8178/v1", "REPLACE_ME_CUSTOM_API_KEY");
    assert_eq!(provider_auth_token("custom", &placeholder).unwrap(), None);
}

#[test]
fn remote_or_non_custom_provider_requires_api_key() {
    let remote = vendor_cfg("https://example.com/v1", "");
    assert!(provider_auth_token("custom", &remote).is_err());

    let qwen = vendor_cfg("http://127.0.0.1:8178/v1", "");
    assert!(provider_auth_token("qwen", &qwen).is_err());
}

#[test]
fn mime_guess_from_ext() {
    assert_eq!(guess_audio_mime(Path::new("a.wav")), "audio/wav");
    assert_eq!(guess_audio_mime(Path::new("a.mp3")), "audio/mpeg");
    assert_eq!(guess_audio_mime(Path::new("a.ogg")), "audio/ogg");
    assert_eq!(guess_audio_mime(Path::new("voice.webm")), "audio/webm");
}

#[test]
fn qwen_chat_model_uses_input_audio_adapter() {
    let cfg = AudioTranscribeConfig {
        qwen_chat_models: Some(vec!["qwen3-asr-flash".to_string()]),
        ..AudioTranscribeConfig::default()
    };

    assert!(qwen_uses_chat_asr_model(&cfg, "qwen3-asr-flash"));
    assert_eq!(
        planned_model_kind(&cfg, VendorKind::Qwen, "qwen3-asr-flash"),
        "chat_audio"
    );
    assert!(!qwen_uses_chat_asr_model(&cfg, "qwen3-asr-flash-filetrans"));
}

#[test]
fn qwen_chat_request_uses_data_url_and_structured_asr_options() {
    let body = qwen_chat_request_body(
        "qwen3-asr-flash",
        "data:audio/webm;base64,ZmFrZQ==",
        "Keep product names exact.",
        Some("zh"),
    );

    assert_eq!(body["model"], "qwen3-asr-flash");
    assert_eq!(body["stream"], false);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(
        body["messages"][1]["content"][0]["input_audio"]["data"],
        "data:audio/webm;base64,ZmFrZQ=="
    );
    assert_eq!(body["asr_options"]["language"], "zh");
    assert_eq!(body["asr_options"]["enable_itn"], true);
}

#[test]
fn qwen_chat_response_extracts_compatible_and_native_shapes() {
    let compatible = json!({
        "choices": [{"message": {"content": "  hello world  "}}]
    });
    let native = json!({
        "output": {"choices": [{"message": {"content": [
            {"text": "first"}, {"text": "second"}
        ]}}]}
    });

    assert_eq!(
        extract_qwen_chat_transcript(&compatible).as_deref(),
        Some("hello world")
    );
    assert_eq!(
        extract_qwen_chat_transcript(&native).as_deref(),
        Some("first\nsecond")
    );
}

#[test]
fn compatible_response_rejects_empty_json_transcript() {
    assert_eq!(
        extract_compatible_transcript(r#"{"text":""}"#),
        Err("transcription result is empty".to_string())
    );
    assert_eq!(
        extract_compatible_transcript(r#"{"text":" recorded words "}"#).as_deref(),
        Ok("recorded words")
    );
}

#[test]
fn provider_http_failures_distinguish_rejected_and_retryable_statuses() {
    let rejected = provider_http_failure(400, "bad request".to_string());
    assert_eq!(rejected.code, "provider_rejected");
    assert!(!rejected.retryable);

    let throttled = provider_http_failure(429, "rate limited".to_string());
    assert_eq!(throttled.code, "provider_request_failed");
    assert!(throttled.retryable);
}

#[test]
fn render_prompt_with_hint() {
    let got = render_transcribe_prompt("A __TRANSCRIBE_HINT__ B", "hint");
    assert_eq!(got, "A hint B");
}

#[test]
fn select_vendor_keeps_default_minimax() {
    let got = select_vendor(None, Some("minimax"), Some("qwen"));
    assert_eq!(got, VendorKind::MiniMax);
}

#[test]
fn select_vendor_keeps_explicit_minimax_request() {
    let got = select_vendor(Some("minimax"), Some("qwen"), Some("openai"));
    assert_eq!(got, VendorKind::MiniMax);
}

#[test]
fn sanitize_oss_name_keeps_safe_chars() {
    assert_eq!(sanitize_oss_filename("a b/c?.wav"), "a_b_c_.wav");
}

#[test]
fn preview_transcribe_returns_plan_without_file_or_provider_credentials() {
    let root =
        std::env::temp_dir().join(format!("rustclaw-audio-transcribe-preview-{}", unix_ts()));
    let cfg = RootConfig {
        audio_transcribe: AudioTranscribeConfig {
            default_vendor: Some("qwen".to_string()),
            default_model: Some("qwen3-asr-flash".to_string()),
            qwen_chat_models: Some(vec!["qwen3-asr-flash".to_string()]),
            providers: AudioProviderOverrides {
                qwen: Some(vendor_cfg(
                    "https://dashscope.aliyuncs.com/compatible-mode/v1",
                    "",
                )),
                ..AudioProviderOverrides::default()
            },
            ..AudioTranscribeConfig::default()
        },
        ..RootConfig::default()
    };

    let (text, extra) = execute(
        &cfg,
        &root,
        json!({
            "action": "preview_transcribe",
            "file": "document/media_dry_run/audio_check.mp3"
        }),
    )
    .expect("preview must not require the input file or provider credentials");

    assert_eq!(text, "AUDIO_TRANSCRIBE_PREVIEW");
    assert_eq!(extra["action"], "preview_transcribe");
    assert_eq!(extra["status"], "dry_run");
    assert_eq!(extra["dry_run"], true);
    assert_eq!(extra["provider_call"], false);
    assert_eq!(extra["filesystem_write"], false);
    assert_eq!(extra["provider"], "qwen");
    assert_eq!(extra["model"], "qwen3-asr-flash");
    assert_eq!(extra["model_kind"], "chat_audio");
    assert_eq!(
        extra["input_path"],
        "document/media_dry_run/audio_check.mp3"
    );
    assert_eq!(extra["input_exists"], false);
}
