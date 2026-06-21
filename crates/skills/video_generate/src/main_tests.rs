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
