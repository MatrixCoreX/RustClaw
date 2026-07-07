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
        "skill.audio_transcribe.execution_failed"
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
