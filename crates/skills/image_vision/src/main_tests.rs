use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
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
fn vendor_order_keeps_defaults_then_appends_fallbacks() {
    let order = vendor_order(None, Some("mimo"), Some("minimax"));

    assert_eq!(order.first(), Some(&VendorKind::Mimo));
    assert_eq!(order.get(1), Some(&VendorKind::MiniMax));
    assert!(order.contains(&VendorKind::Qwen));
    assert_eq!(order.len(), 8);
}

#[test]
fn vendor_order_honors_explicit_request_only() {
    assert_eq!(
        vendor_order(Some("qwen"), Some("mimo"), Some("minimax")),
        vec![VendorKind::Qwen]
    );
}

#[test]
fn split_data_url() {
    let (mime, data) = split_image_data("data:image/jpeg;base64,abc");
    assert_eq!(mime, "image/jpeg");
    assert_eq!(data, "abc");
}

#[test]
fn strip_base64_data_url_returns_payload_only() {
    assert_eq!(
        strip_base64_data_url("data:image/png;base64,abc123"),
        "abc123"
    );
    assert_eq!(strip_base64_data_url(" raw123 "), "raw123");
}

#[test]
fn text_base64_image_marker_is_language_neutral() {
    let mut content = String::from("inspect");
    append_text_base64_image(&mut content, 1, "abc123");

    assert!(content.contains("[image_base64:abc123]"));
    assert!(!content.contains("图片"));
}

#[test]
fn parse_action_normalizes_analyze_alias_to_describe() {
    let mut obj = Map::new();
    obj.insert("action".to_string(), Value::String("analyze".to_string()));

    assert_eq!(parse_action(&obj).as_deref(), Ok("describe"));
}

#[test]
fn minimax_mcp_api_host_strips_openai_compat_suffix() {
    assert_eq!(
        minimax_mcp_api_host("https://api.minimaxi.com/v1"),
        "https://api.minimaxi.com"
    );
    assert_eq!(
        minimax_mcp_api_host("https://api.minimaxi.com"),
        "https://api.minimaxi.com"
    );
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
fn select_model_override_prefers_vendor_pool_over_global_default() {
    let mut cfg = ImageSkillConfig {
        default_vendor: Some("mimo".to_string()),
        default_model: Some("mimo-v2.5".to_string()),
        models: Some(vec!["mimo-v2.5".to_string()]),
        ..ImageSkillConfig::default()
    };
    cfg.minimax_models = Some(vec!["MiniMax-Text-01".to_string()]);

    assert_eq!(
        select_model_override(&cfg, VendorKind::MiniMax, None),
        Some("MiniMax-Text-01")
    );
}

#[test]
fn select_model_override_does_not_leak_default_model_to_other_vendor() {
    let cfg = ImageSkillConfig {
        default_vendor: Some("mimo".to_string()),
        default_model: Some("mimo-v2.5".to_string()),
        models: Some(vec!["mimo-v2.5".to_string(), "mimo-v2-omni".to_string()]),
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
        default_vendor: Some("mimo".to_string()),
        default_model: Some("mimo-v2.5".to_string()),
        minimax_models: Some(vec!["MiniMax-Text-01".to_string()]),
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
    let rendered = render_structured_narrative_action_output(&output, Some("zh-CN"));
    assert_eq!(rendered, "设置页面");
}
