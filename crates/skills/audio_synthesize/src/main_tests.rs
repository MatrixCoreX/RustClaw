use super::*;

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
        extra["planned_outputs"][0]["path"].as_str(),
        Some(out.to_string_lossy().as_ref())
    );
    assert!(extra["outputs"].as_array().is_some_and(Vec::is_empty));
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
