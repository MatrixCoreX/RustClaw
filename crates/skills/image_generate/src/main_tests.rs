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
