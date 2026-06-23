use super::*;

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
    assert_eq!(extra["request"]["lyrics_optimizer"], true);
    assert_eq!(extra["planned_outputs"][0]["type"], "audio_file");
    assert!(
        extra["planned_outputs"][0]["path"]
            .as_str()
            .is_some_and(|path| path.ends_with(".mp3")),
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
