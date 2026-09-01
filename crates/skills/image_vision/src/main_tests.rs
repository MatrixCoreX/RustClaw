use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_code"], "execution_failed");
    assert_eq!(extra["message_key"], "skill.image_vision.execution_failed");
    assert_eq!(extra["retryable"], false);
}

#[test]
fn parse_vendor_ok() {
    assert_eq!(parse_vendor("openai"), Some(VendorKind::OpenAI));
    assert_eq!(parse_vendor("gemini"), Some(VendorKind::Google));
    assert_eq!(parse_vendor("claude"), Some(VendorKind::Anthropic));
    assert_eq!(parse_vendor("qwen"), Some(VendorKind::Qwen));
    assert_eq!(parse_vendor("xiaomi"), Some(VendorKind::Mimo));
}

#[test]
fn vendor_order_uses_only_independent_image_default() {
    assert_eq!(
        vendor_order(None, Some("minimax")),
        vec![VendorKind::MiniMax]
    );
}

#[test]
fn vendor_order_honors_explicit_request_only() {
    assert_eq!(
        vendor_order(Some("qwen"), Some("minimax")),
        vec![VendorKind::Qwen]
    );
}

#[test]
fn vendor_order_is_empty_without_image_configuration() {
    assert!(vendor_order(None, None).is_empty());
}

#[test]
fn split_data_url() {
    let (mime, data) = split_image_data("data:image/jpeg;base64,abc");
    assert_eq!(mime, "image/jpeg");
    assert_eq!(data, "abc");
}

#[test]
fn parse_action_normalizes_analyze_alias_to_describe() {
    let mut obj = Map::new();
    obj.insert("action".to_string(), Value::String("analyze".to_string()));

    assert_eq!(parse_action(&obj).as_deref(), Ok("describe"));
}

#[test]
fn parse_action_accepts_extract_text() {
    let mut obj = Map::new();
    obj.insert(
        "action".to_string(),
        Value::String("extract_text".to_string()),
    );

    assert_eq!(parse_action(&obj).as_deref(), Ok("extract_text"));
}

#[test]
fn parse_one_image_accepts_native_tool_text_wrapped_path() {
    let workspace = Path::new("/tmp/image-vision-workspace");
    let input = json!({
        "$text": "{\"path\":\"/tmp/image-vision-workspace/downloaded.webp\"}"
    });

    let parsed = parse_one_image(&input, workspace).expect("wrapped path should parse");

    match parsed {
        ImageSource::Path(path) => {
            assert_eq!(
                path,
                PathBuf::from("/tmp/image-vision-workspace/downloaded.webp")
            );
        }
        _ => panic!("expected local path"),
    }
}

#[test]
fn parse_one_image_rejects_text_wrapper_with_extra_fields() {
    let workspace = Path::new("/tmp/image-vision-workspace");
    let input = json!({
        "$text": "{\"path\":\"/tmp/image-vision-workspace/downloaded.webp\",\"url\":\"https://example.invalid/image.webp\"}"
    });

    let error = parse_one_image(&input, workspace).expect_err("ambiguous wrapper must fail");

    assert_eq!(error, "image text wrapper must contain only path");
}

#[test]
fn parse_images_rejects_ambiguous_singular_and_plural_inputs() {
    let workspace = Path::new("/tmp/image-vision-workspace");
    let obj = json!({
        "image": "/tmp/image-vision-workspace/downloaded.webp",
        "images": [{}],
    });

    let error = parse_images(obj.as_object().expect("object"), workspace)
        .expect_err("ambiguous image inputs must fail before provider dispatch");

    assert_eq!(error, "provide either images or image, not both");
}

#[test]
fn image_request_accepts_more_than_six_images_without_a_count_ceiling() {
    let images = (1..=9)
        .map(|index| {
            ImageSource::Path(PathBuf::from(format!(
                "/tmp/image-vision-workspace/page-{index}.webp"
            )))
        })
        .collect::<Vec<_>>();

    validate_image_request("extract_text", &images)
        .expect("the complete ordered image set must reach the provider");
}

#[test]
fn strip_think_blocks_removes_model_reasoning() {
    assert_eq!(
        strip_think_blocks("<think>hidden</think>\n可见内容").trim(),
        "可见内容"
    );
}

#[test]
fn provider_error_excerpt_redacts_secret_like_values() {
    let fake_openai_key = ["sk", "proj", "secret123456789"].join("-");
    let fake_plain_key = ["plain", "secret", "token"].join("-");
    let value = json!({
        "error": {
            "message": format!("Incorrect API key provided: {fake_openai_key}"),
            "api_key": fake_plain_key
        }
    });

    let excerpt = provider_error_excerpt(&value, 1000);

    assert!(!excerpt.contains(&fake_openai_key), "{excerpt}");
    assert!(
        !excerpt.contains(
            value
                .pointer("/error/api_key")
                .and_then(Value::as_str)
                .expect("fake api key")
        ),
        "{excerpt}"
    );
    assert!(excerpt.contains("[REDACTED_API_KEY]"), "{excerpt}");
    assert!(excerpt.contains("[REDACTED]"), "{excerpt}");
}

#[test]
fn select_model_override_prefers_independent_default_model() {
    let cfg = ImageSkillConfig {
        default_vendor: Some("minimax".to_string()),
        default_model: Some("MiniMax-M3".to_string()),
        minimax_models: Some(vec!["MiniMax-M2.7".to_string()]),
        ..ImageSkillConfig::default()
    };

    assert_eq!(
        select_model_override(&cfg, VendorKind::MiniMax, None),
        Some("MiniMax-M3")
    );
}

#[test]
fn select_model_override_does_not_leak_default_model_to_other_vendor() {
    let cfg = ImageSkillConfig {
        default_vendor: Some("minimax".to_string()),
        default_model: Some("MiniMax-M3".to_string()),
        models: Some(vec!["MiniMax-M3".to_string()]),
        ..ImageSkillConfig::default()
    };

    assert_eq!(
        select_model_override(&cfg, VendorKind::DeepSeek, None),
        None
    );
}

#[test]
fn select_model_override_honors_explicit_request() {
    let cfg = ImageSkillConfig {
        default_vendor: Some("minimax".to_string()),
        default_model: Some("MiniMax-M3".to_string()),
        minimax_models: Some(vec!["MiniMax-M2.7".to_string()]),
        ..ImageSkillConfig::default()
    };

    assert_eq!(
        select_model_override(&cfg, VendorKind::MiniMax, Some("custom-model")),
        Some("custom-model")
    );
}

fn vendor_cfg(base_url: &str, api_key: &str, model: &str) -> VendorConfig {
    VendorConfig {
        base_url: base_url.to_string(),
        api_key: api_key.to_string(),
        model: model.to_string(),
        timeout_seconds: Some(30),
    }
}

#[test]
fn resolve_vendor_config_inherits_shared_key_for_empty_provider_override() {
    let mut cfg = RootConfig::default();
    cfg.llm.minimax = Some(vendor_cfg(
        "https://shared.example/v1",
        "shared-minimax-key",
        "shared-model",
    ));
    cfg.image_vision.providers.minimax =
        Some(vendor_cfg("https://vision.example/v1", "", "vision-model"));

    let (vendor, resolved) =
        resolve_vendor_config(&cfg, VendorKind::MiniMax).expect("minimax config");

    assert_eq!(vendor, "minimax");
    assert_eq!(resolved.base_url, "https://vision.example/v1");
    assert_eq!(resolved.model, "vision-model");
    assert_eq!(resolved.api_key, "shared-minimax-key");
}

#[test]
fn resolve_vendor_config_inherits_shared_api_for_empty_provider_override() {
    let mut cfg = RootConfig::default();
    cfg.llm.minimax = Some(vendor_cfg(
        "https://shared.example/v1",
        "shared-minimax-key",
        "shared-model",
    ));
    cfg.image_vision.providers.minimax = Some(vendor_cfg("", "vision-minimax-key", "vision-model"));

    let (_, resolved) = resolve_vendor_config(&cfg, VendorKind::MiniMax).expect("minimax config");

    assert_eq!(resolved.base_url, "https://shared.example/v1");
    assert_eq!(resolved.api_key, "vision-minimax-key");
    assert_eq!(resolved.model, "vision-model");
}

#[test]
fn resolve_vendor_config_uses_main_connection_when_override_is_absent() {
    let mut cfg = RootConfig::default();
    cfg.llm.minimax = Some(vendor_cfg(
        "https://shared.example/v1",
        "shared-minimax-key",
        "shared-model",
    ));

    let (_, resolved) = resolve_vendor_config(&cfg, VendorKind::MiniMax).expect("minimax config");

    assert_eq!(resolved.base_url, "https://shared.example/v1");
    assert_eq!(resolved.api_key, "shared-minimax-key");
    assert_eq!(resolved.model, "shared-model");
}

#[test]
fn resolve_vendor_config_keeps_dedicated_provider_key() {
    let mut cfg = RootConfig::default();
    cfg.llm.minimax = Some(vendor_cfg(
        "https://shared.example/v1",
        "shared-minimax-key",
        "shared-model",
    ));
    cfg.image_vision.providers.minimax = Some(vendor_cfg(
        "https://vision.example/v1",
        "vision-minimax-key",
        "vision-model",
    ));

    let (_, resolved) = resolve_vendor_config(&cfg, VendorKind::MiniMax).expect("minimax config");

    assert_eq!(resolved.api_key, "vision-minimax-key");
    assert_eq!(resolved.base_url, "https://vision.example/v1");
    assert_eq!(resolved.model, "vision-model");
}

#[test]
fn minimax_compat_dispatches_multimodal_request() {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock minimax server");
    let address = listener.local_addr().expect("mock server address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept minimax request");
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut chunk).expect("read minimax request");
            assert!(read > 0, "request ended before headers");
            request.extend_from_slice(&chunk[..read]);
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .expect("content length");
        while request.len() < header_end + content_length {
            let read = stream.read(&mut chunk).expect("read minimax body");
            assert!(read > 0, "request ended before body");
            request.extend_from_slice(&chunk[..read]);
        }
        let request = String::from_utf8_lossy(&request);
        assert!(request.starts_with("POST /chat/completions "));
        assert!(request.contains("\"model\":\"MiniMax-M3\""));
        assert!(request.contains("data:image/png;base64,YWJj"));
        assert!(request.contains("\"temperature\":0.0"));
        assert!(!request.contains("\"detail\":\"high\""));

        let body = r#"{"choices":[{"message":{"content":"识别成功"}}]}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write minimax response");
    });

    let mut cfg = RootConfig::default();
    cfg.image_vision.adapter_mode = Some("compat".to_string());
    cfg.image_vision.providers.minimax = Some(vendor_cfg(
        &format!("http://{address}"),
        "test-minimax-key",
        "MiniMax-M3",
    ));
    let images = vec![ImageSource::Base64(
        "data:image/png;base64,YWJj".to_string(),
    )];

    let request = VisionRequest {
        prompt: "识别图片文字",
        images: &images,
        max_input_bytes: 1024,
        options: VisionRequestOptions { exact_text: true },
    };
    let result = call_vendor_vision(VendorKind::MiniMax, &cfg, Some("MiniMax-M3"), 5, request)
        .expect("minimax vision request");
    server.join().expect("mock minimax server");

    assert_eq!(result.0, "识别成功");
    assert_eq!(result.1, "MiniMax-M3");
    assert_eq!(result.2, "compat");
}

#[test]
fn extract_text_retries_once_when_provider_omits_an_input_page() {
    use std::io::{Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};

    fn receive_and_reply(mut stream: TcpStream, expected_prompt: &str, content: &str) {
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut chunk).expect("read vision request");
            assert!(read > 0, "request ended before headers");
            request.extend_from_slice(&chunk[..read]);
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .expect("content length");
        while request.len() < header_end + content_length {
            let read = stream.read(&mut chunk).expect("read vision body");
            assert!(read > 0, "request ended before body");
            request.extend_from_slice(&chunk[..read]);
        }
        let request = String::from_utf8_lossy(&request);
        assert!(request.contains(expected_prompt), "{request}");

        let body = json!({"choices":[{"message":{"content":content}}]}).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write vision response");
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock vision server");
    let address = listener.local_addr().expect("mock server address");
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept first vision request");
        receive_and_reply(
            stream,
            "base prompt",
            r#"{"pages":[{"text":"first image"}],"uncertainties":[]}"#,
        );
        let (stream, _) = listener.accept().expect("accept retry vision request");
        receive_and_reply(
            stream,
            "exactly 2 entries",
            r#"{"pages":[{"text":"first image"},{"text":"second image"}],"uncertainties":[]}"#,
        );
    });

    let mut cfg = RootConfig::default();
    cfg.image_vision.adapter_mode = Some("compat".to_string());
    cfg.image_vision.providers.minimax = Some(vendor_cfg(
        &format!("http://{address}"),
        "test-minimax-key",
        "MiniMax-M3",
    ));
    let images = vec![
        ImageSource::Base64("data:image/png;base64,YWJj".to_string()),
        ImageSource::Base64("data:image/png;base64,ZGVm".to_string()),
    ];

    let result = call_vendor_vision_for_action(
        VendorKind::MiniMax,
        &cfg,
        Some("MiniMax-M3"),
        5,
        "base prompt",
        "extract_text",
        &images,
        1024,
    )
    .expect("schema retry should recover");
    server.join().expect("mock vision server");

    assert_eq!(result.3, 2);
    let structured = parse_structured_narrative_action_output("extract_text", &result.0)
        .expect("structured result");
    match structured {
        StructuredNarrativeActionOutput::ExtractText(output) => {
            assert_eq!(output.pages.len(), 2);
            assert_eq!(output.pages[1].text, "second image");
        }
        _ => panic!("expected extract_text output"),
    }
}

#[test]
fn parse_language_choice_accepts_schema_valid_json() {
    assert_eq!(
        parse_language_choice_from_llm(r#"{"language":"Chinese (Simplified)"}"#).as_deref(),
        Some("Chinese (Simplified)")
    );
    assert_eq!(
        parse_language_choice_from_llm(r#"answer {"language":"English"}"#).as_deref(),
        Some("English")
    );
}

#[test]
fn parse_language_choice_rejects_extra_fields_and_unknown() {
    assert_eq!(
        parse_language_choice_from_llm(r#"{"language":"English","confidence":0.9}"#),
        None
    );
    assert_eq!(
        parse_language_choice_from_llm(r#"{"language":"unknown"}"#),
        None
    );
}

#[test]
fn parse_structured_narrative_action_output_accepts_describe_json() {
    let raw = r#"{
        "summary":"A Rust logo on a white background.",
        "objects":["logo","text"],
        "visible_text":["Rust"],
        "uncertainties":[]
    }"#;
    let parsed = parse_structured_narrative_action_output("describe", raw).expect("describe parse");
    match parsed {
        StructuredNarrativeActionOutput::Describe(out) => {
            assert_eq!(out.summary, "A Rust logo on a white background.");
            assert_eq!(out.visible_text, vec!["Rust"]);
        }
        _ => panic!("expected describe output"),
    }
}

#[test]
fn parse_structured_narrative_action_output_accepts_compare_json() {
    let raw = r#"{
        "summary":"The two screenshots are largely the same.",
        "similarities":["same layout"],
        "differences":["different button color"],
        "notable_changes":["one button is highlighted"],
        "uncertainties":[]
    }"#;
    let parsed = parse_structured_narrative_action_output("compare", raw).expect("compare parse");
    match parsed {
        StructuredNarrativeActionOutput::Compare(out) => {
            assert_eq!(out.differences, vec!["different button color"]);
        }
        _ => panic!("expected compare output"),
    }
}

#[test]
fn parse_structured_narrative_action_output_accepts_screenshot_summary_json() {
    let raw = r#"{
        "purpose":"A settings page.",
        "critical_text":["Privacy settings"],
        "warnings":["Unsaved changes"],
        "next_actions":["Review settings"],
        "uncertainties":[]
    }"#;
    let parsed = parse_structured_narrative_action_output("screenshot_summary", raw)
        .expect("screenshot summary parse");
    match parsed {
        StructuredNarrativeActionOutput::ScreenshotSummary(out) => {
            assert_eq!(out.warnings, vec!["Unsaved changes"]);
        }
        _ => panic!("expected screenshot summary output"),
    }
}

#[test]
fn parse_structured_narrative_action_output_rejects_extra_fields() {
    let raw = r#"{
        "summary":"A Rust logo on a white background.",
        "objects":["logo","text"],
        "visible_text":["Rust"],
        "uncertainties":[],
        "unexpected":"drift"
    }"#;
    assert!(parse_structured_narrative_action_output("describe", raw).is_none());
}

#[test]
fn render_structured_narrative_action_output_keeps_model_primary_text() {
    let output = StructuredNarrativeActionOutput::ScreenshotSummary(ImageScreenshotSummaryOut {
        purpose: "设置页面".to_string(),
        critical_text: vec!["隐私设置".to_string()],
        warnings: vec!["有未保存更改".to_string()],
        next_actions: vec!["检查后保存".to_string()],
        uncertainties: vec![],
    });
    let rendered = render_structured_narrative_action_output(&output, Some("zh-CN"), false);
    assert_eq!(rendered, "设置页面");
}

#[test]
fn describe_default_reply_includes_visible_text_from_same_model_result() {
    let output = StructuredNarrativeActionOutput::Describe(ImageDescribeOut {
        summary: "一张店铺门口的照片。".to_string(),
        objects: vec!["店铺".to_string()],
        visible_text: vec!["营业时间".to_string(), "09:00-18:00".to_string()],
        uncertainties: vec![],
    });

    let rendered = render_structured_narrative_action_output(&output, Some("zh-CN"), true);

    assert_eq!(rendered, "一张店铺门口的照片。\n\n营业时间\n09:00-18:00");
}

#[test]
fn describe_renderer_preserves_source_marker_without_numbering_other_lines() {
    let output = StructuredNarrativeActionOutput::Describe(ImageDescribeOut {
        summary: "A photographed note.".to_string(),
        objects: vec!["note".to_string()],
        visible_text: vec!["1. source marker".to_string(), "unmarked line".to_string()],
        uncertainties: vec![],
    });

    let rendered = render_structured_narrative_action_output(&output, Some("en"), true);

    assert_eq!(
        rendered,
        "A photographed note.\n\n1. source marker\nunmarked line"
    );
}

#[test]
fn describe_default_reply_omits_text_section_when_model_sees_no_text() {
    let output = StructuredNarrativeActionOutput::Describe(ImageDescribeOut {
        summary: "一只猫坐在窗边。".to_string(),
        objects: vec!["猫".to_string(), "窗户".to_string()],
        visible_text: vec![],
        uncertainties: vec![],
    });

    let rendered = render_structured_narrative_action_output(&output, Some("zh-CN"), true);

    assert_eq!(rendered, "一只猫坐在窗边。");
    assert!(!rendered.contains("图片文字"));
}

#[test]
fn describe_with_user_instruction_does_not_append_default_text_section() {
    let output = StructuredNarrativeActionOutput::Describe(ImageDescribeOut {
        summary: "主色调是蓝色。".to_string(),
        objects: vec!["海报".to_string()],
        visible_text: vec!["SALE".to_string()],
        uncertainties: vec![],
    });

    let rendered = render_structured_narrative_action_output(&output, Some("zh-CN"), false);

    assert_eq!(rendered, "主色调是蓝色。");
    assert!(!rendered.contains("SALE"));
}

#[test]
fn extract_text_merges_pages_in_input_order_without_source_labels() {
    let raw = r#"{
        "pages":[{"text":"第一页文字"},{"text":"   "},{"text":"Second page"}],
        "uncertainties":[]
    }"#;
    let parsed =
        parse_structured_narrative_action_output("extract_text", raw).expect("extract text parse");
    let rendered = render_structured_narrative_action_output(&parsed, Some("zh-CN"), false);

    assert_eq!(rendered, "第一页文字\n\nSecond page");
    assert!(!rendered.contains("Image"));
}

#[test]
fn extract_text_structure_requires_one_page_per_input_image() {
    let two_pages = r#"{
        "pages":[{"text":"first"},{"text":"second"}],
        "uncertainties":[]
    }"#;

    assert!(structured_output_is_complete("extract_text", two_pages, 2));
    assert!(!structured_output_is_complete("extract_text", two_pages, 3));
    assert!(!structured_output_is_complete(
        "extract_text",
        "unstructured text",
        1
    ));
    assert!(structured_output_is_complete("extract", "free form", 9));
}

#[test]
fn extract_text_converts_double_escaped_newline_markers_to_real_line_breaks() {
    let raw = r#"{
        "pages":[{"text":"第一行\\n第二行\\r\\n第三行\\r第四行"}],
        "uncertainties":[]
    }"#;
    let parsed =
        parse_structured_narrative_action_output("extract_text", raw).expect("extract text parse");
    let rendered = render_structured_narrative_action_output(&parsed, Some("zh-CN"), false);

    assert_eq!(rendered, "第一行\n第二行\n第三行\n第四行");
    assert!(!rendered.contains("\\n"));
    assert!(!rendered.contains("\\r"));
}

#[test]
fn extract_text_normalizes_platform_line_endings_without_changing_other_escapes() {
    let normalized = normalize_extracted_text_newlines("第一行\r\n第二行\r第三行\\t保留");

    assert_eq!(normalized, "第一行\n第二行\n第三行\\t保留");
}

#[test]
fn extract_text_rejects_structured_output_without_visible_text() {
    let output = StructuredNarrativeActionOutput::ExtractText(ImageTextExtractionOut {
        pages: vec![ImageTextPageOut {
            text: "   ".to_string(),
        }],
        uncertainties: vec!["blurred".to_string()],
    });

    assert!(!image_text_output_has_visible_text(Some(&output), ""));
    assert!(image_text_output_has_visible_text(None, "fallback text"));
}

#[test]
fn image_text_revision_chunks_are_unicode_safe_and_complete() {
    let source = format!("{}\n{}", "مرحبا".repeat(1_200), "नमस्ते".repeat(100));
    let chunks = split_image_text_revision_chunks(&source, 6_000);

    assert!(chunks.len() >= 2);
    assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 6_000));
    assert_eq!(chunks.concat(), source);
}

#[test]
fn image_text_revision_falls_back_to_raw_text_without_provider() {
    let raw = "今天天汽很好".to_string();
    let workspace = tempfile::tempdir().expect("tempdir");

    let (reviewed, metadata) =
        review_recognized_image_text(&RootConfig::default(), workspace.path(), raw.clone(), 30);

    assert_eq!(reviewed, raw);
    assert_eq!(metadata["status"], "fallback_raw");
    assert_eq!(metadata["reviewed_by_model"], false);
}

#[test]
fn image_text_revision_prompt_requires_semantic_reflow() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let prompt = load_image_text_revision_prompt_template(workspace.path(), "minimax");

    assert!(prompt.contains("Reflow text by semantic structure"));
    assert!(prompt.contains("Merge visual soft wraps"));
    assert!(prompt.contains("paragraph boundary, heading, list item, table row"));
    assert!(prompt.contains("Do not introduce any new line-start numbering"));
    assert!(prompt.contains("preserve it exactly"));
}

#[test]
fn image_text_extraction_prompt_preserves_visible_markers_without_inventing_them() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let prompt = action_instruction(workspace.path(), "minimax", "extract_text", "normal", None);

    assert!(prompt.contains("Preserve every line-start marker that is visibly present"));
    assert!(prompt.contains("Never add a line-start number"));
}

#[test]
fn image_description_prompt_preserves_only_source_visible_line_markers() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let prompt = build_prompt(
        workspace.path(),
        "minimax",
        "describe",
        "normal",
        None,
        Some("zh-CN"),
        None,
    );

    assert!(prompt.contains("array items already represents reading order"));
    assert!(prompt.contains("never add a number, bullet, Markdown prefix"));
    assert!(prompt.contains("visibly part of the image, preserve that marker exactly"));
}

#[test]
fn image_description_schema_retry_repeats_source_marker_contract() {
    let prompt = structured_output_retry_prompt("base prompt", "describe", 1);

    assert!(prompt.contains("previous response did not satisfy the required JSON schema"));
    assert!(prompt.contains("Array order already records reading order"));
    assert!(prompt.contains("Preserve a line-start marker only when it is visibly present"));
}

#[test]
fn image_text_schema_retry_requires_exact_input_cardinality() {
    let prompt = structured_output_retry_prompt("base prompt", "extract_text", 7);

    assert!(prompt.contains("exactly 7 entries"));
    assert!(prompt.contains("Never merge, omit, or duplicate an input image"));
    assert!(prompt.contains("empty `text` string"));
}

#[test]
fn image_text_revision_integrity_rejects_changed_numbers_and_large_omissions() {
    assert!(!image_text_revision_preserves_source(
        "订单 20260811 金额 128.50",
        "订单 20260812 金额 128.50"
    ));
    assert!(!image_text_revision_preserves_source(
        &"A long source passage. ".repeat(40),
        "short"
    ));
    assert!(image_text_revision_preserves_source(
        "今天天汽很好 20260811",
        "今天天气很好。20260811"
    ));
}

#[test]
fn image_text_revision_integrity_protects_identifiers_and_limits_rewriting() {
    assert!(image_text_revision_preserves_source(
        "访问 https://example.com/A-1 型号 MiniMax-M3，状态 OK",
        "访问 https://example.com/A-1，型号 MiniMax-M3；状态 OK。"
    ));
    assert!(!image_text_revision_preserves_source(
        "访问 https://example.com/A-1 型号 MiniMax-M3",
        "访问 https://example.com/B-1 型号 MiniMax-M3"
    ));
    assert!(!image_text_revision_preserves_source(
        &"the original recognized passage contains stable words ".repeat(12),
        &"completely rewritten content replaces every source token ".repeat(12)
    ));
}

#[test]
fn image_text_revision_reassembles_using_source_boundaries() {
    let source_chunks = vec!["first section \n".to_string(), "第二部分".to_string()];
    let reviewed_chunks = vec!["first section".to_string(), "第二部分。".to_string()];

    assert_eq!(
        join_image_text_revision_chunks(&source_chunks, &reviewed_chunks),
        "first section \n第二部分。"
    );
}

#[test]
fn extract_text_preserves_raw_artifact_without_delivering_it() {
    let directory = tempfile::tempdir().expect("tempdir");
    let context = json!({"artifact_output_directory": directory.path()});
    let args = Map::new();
    let mut review = json!({"status": "reviewed", "reviewed_by_model": true});

    attach_raw_text_artifact(Some(&context), &args, "原始识别", &mut review)
        .expect("write raw text artifact");

    assert_eq!(review["raw_artifact"]["deliver_to_user"], false);
    assert_eq!(review["raw_artifact"]["filename"], "image_text_ai_raw.txt");
    let path = PathBuf::from(
        review["raw_artifact"]["path"]
            .as_str()
            .expect("raw artifact path"),
    );
    assert_eq!(fs::read_to_string(path).expect("raw text"), "原始识别\n");
}

#[test]
fn extract_text_writes_delivery_artifact_by_default() {
    let directory = tempfile::tempdir().expect("tempdir");
    let context = json!({"artifact_output_directory": directory.path()});
    let args = Map::new();
    let mut extra = json!({});

    attach_text_artifact(Some(&context), &args, "识别结果", &mut extra)
        .expect("write text artifact");

    assert_eq!(extra["delivery"]["deliver_to_user"], true);
    assert_eq!(extra["artifacts"][0]["filename"], "image_text_ai.txt");
    assert_eq!(
        extra["artifacts"][0]["recognition_source"],
        "multimodal_model"
    );
    let path = PathBuf::from(
        extra["artifacts"][0]["path"]
            .as_str()
            .expect("artifact path"),
    );
    assert_eq!(
        fs::read_to_string(path).expect("artifact text"),
        "识别结果\n"
    );
}

#[test]
fn extract_text_artifact_contains_real_newlines_instead_of_visible_markers() {
    let directory = tempfile::tempdir().expect("tempdir");
    let context = json!({"artifact_output_directory": directory.path()});
    let args = Map::new();
    let mut extra = json!({});
    let text = normalize_extracted_text_newlines("第一行\\n第二行");

    attach_text_artifact(Some(&context), &args, &text, &mut extra).expect("write text artifact");

    let path = PathBuf::from(
        extra["artifacts"][0]["path"]
            .as_str()
            .expect("artifact path"),
    );
    let artifact_text = fs::read_to_string(path).expect("artifact text");
    assert_eq!(artifact_text, "第一行\n第二行\n");
    assert!(!artifact_text.contains("\\n"));
}

#[test]
fn extract_text_can_save_without_delivery() {
    let directory = tempfile::tempdir().expect("tempdir");
    let context = json!({"artifact_output_directory": directory.path()});
    let mut args = Map::new();
    args.insert("deliver_to_user".to_string(), Value::Bool(false));
    args.insert(
        "output_name".to_string(),
        Value::String("note-text.txt".to_string()),
    );
    let mut extra = json!({});

    attach_text_artifact(Some(&context), &args, "saved", &mut extra).expect("save text artifact");

    assert_eq!(extra["delivery"]["deliver_to_user"], false);
    assert_eq!(extra["artifacts"], json!([]));
    assert!(extra.get("output_path").is_none());
    assert_eq!(extra["saved_files"][0]["filename"], "note-text.txt");
}
