use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
    assert_eq!(extra["message_key"], "skill.image_edit.execution_failed");
    assert_eq!(extra["retryable"], false);
}

#[test]
fn success_response_exposes_media_machine_fields() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-image-edit-success-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("temp root");
    let cfg = ImageSkillConfig::default();
    let i18n = TextCatalog::for_lang(&root, &cfg, "en-US");
    let output_path = root.join("image/out.png");

    let (text, extra) =
        build_success_response(&i18n, &output_path, "minimax", "image-01", "native", "edit");

    assert!(text.contains(&format!("FILE:{}", output_path.display())));
    assert_eq!(extra["message_key"], "image_edit.msg.saved");
    assert_eq!(extra["provider"], "minimax");
    assert_eq!(extra["model"], "image-01");
    assert_eq!(extra["model_kind"], "native");
    assert_eq!(extra["action"], "edit");
    assert_eq!(extra["media_type"], "image");
    assert_eq!(
        extra["output_path"].as_str(),
        Some(output_path.to_string_lossy().as_ref())
    );
    assert_eq!(extra["outputs"][0]["type"], "image_file");

    let _ = std::fs::remove_dir_all(&root);
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
fn rewrite_for_restyle() {
    let v = rewrite_instruction("restyle", "make it watercolor");
    assert!(v.contains("restyle"));
}

#[test]
fn split_data_url() {
    let (mime, data) = split_image_data("data:image/png;base64,abc");
    assert_eq!(mime, "image/png");
    assert_eq!(data, "abc");
}

#[test]
fn image_args_accept_remote_url_object() {
    let obj = json!({
        "image": {"url": "https://example.com/logo.png"}
    })
    .as_object()
    .cloned()
    .unwrap();

    assert!(image_edit_args_has_image(&obj));
}

#[test]
fn first_image_preserves_url_from_images_array() {
    let obj = json!({
        "images": [{"url": "https://example.com/logo.png"}]
    })
    .as_object()
    .cloned()
    .unwrap();

    let image = first_image_from_images_array(&obj).expect("image source");
    assert_eq!(
        image
            .get("url")
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
        "https://example.com/logo.png"
    );
}

#[test]
fn native_edit_supports_local_upload_when_enabled() {
    let cfg = ImageSkillConfig {
        local_auto_upload_enabled: true,
        ..Default::default()
    };
    assert!(qwen_native_edit_inputs_supported(
        &cfg,
        "wanx2.1-imageedit",
        &ImageSource::Path(PathBuf::from("/tmp/demo.png")),
        Some(&ImageSource::Base64(
            "data:image/png;base64,abc".to_string()
        ))
    ));
}

#[test]
fn sanitize_oss_name_keeps_safe_chars() {
    assert_eq!(sanitize_oss_filename("a b/c?.png"), "a_b_c_.png");
}

#[test]
fn multimodal_native_edit_supports_local_without_oss() {
    let cfg = ImageSkillConfig::default();
    assert!(qwen_native_edit_inputs_supported(
        &cfg,
        "wan2.6-image",
        &ImageSource::Path(PathBuf::from("/tmp/demo.png")),
        None
    ));
    assert!(qwen_native_edit_inputs_supported(
        &cfg,
        "qwen-image-edit-max",
        &ImageSource::Path(PathBuf::from("/tmp/demo.png")),
        None
    ));
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
fn minimax_response_extracts_image_payloads() {
    let with_url = json!({
        "data": {
            "image_urls": ["https://example.com/out.png"]
        }
    });
    assert_eq!(
        minimax_response_image_url(&with_url),
        Some("https://example.com/out.png")
    );

    let with_b64 = json!({
        "data": {
            "images": [{
                "base64": "abc"
            }]
        }
    });
    assert_eq!(minimax_response_image_base64(&with_b64), Some("abc"));
}

#[test]
fn minimax_aspect_ratio_uses_size_ratio() {
    assert_eq!(size_to_minimax_aspect_ratio("1024x1024"), "1:1");
    assert_eq!(size_to_minimax_aspect_ratio("1024x768"), "4:3");
}

#[test]
fn parse_llm_selected_index_accepts_schema_valid_json() {
    assert_eq!(parse_llm_selected_index(r#"{"selected_index":2}"#), Some(2));
    assert_eq!(
        parse_llm_selected_index(r#"answer {"selected_index":0}"#),
        Some(0)
    );
}

#[test]
fn parse_llm_selected_index_rejects_extra_fields_and_out_of_range_values() {
    assert_eq!(
        parse_llm_selected_index(r#"{"selected_index":1,"reason":"extra"}"#),
        None
    );
    assert_eq!(parse_llm_selected_index(r#"{"selected_index":-2}"#), None);
}
