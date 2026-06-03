use super::*;

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
