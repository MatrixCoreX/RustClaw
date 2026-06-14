use std::path::PathBuf;

use claw_core::config::AppConfig;

use super::{
    classify_prompt_source, matches_provider_override,
    recover_normalizer_text_from_openai_tool_calls, synthesize_llm_providers,
};

fn repo_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/config.toml")
        .canonicalize()
        .expect("repo config path should resolve")
}

#[test]
fn classify_prompt_source_uses_specific_classifier_labels() {
    assert_eq!(
        classify_prompt_source("prompts/delivery_text_classifier_prompt.md"),
        "delivery_classifier"
    );
    assert_eq!(
        classify_prompt_source("inline:direct_classifier"),
        "direct_classifier"
    );
    assert_eq!(
        classify_prompt_source("layered:prompts/direct_answer_gate_prompt.md#vendor=mimo"),
        "direct_answer_gate"
    );
    assert_eq!(
        classify_prompt_source("layered:prompts/contract_repair_judge_prompt.md#vendor=minimax"),
        "contract_repair"
    );
    assert_eq!(
        classify_prompt_source("layered:prompts/lightweight_execution_prompt.md#vendor=minimax"),
        "plan"
    );
    assert_eq!(
        classify_prompt_source("layered:prompts/user_response_composer_prompt.md#vendor=minimax"),
        "user_response_composer"
    );
    assert_eq!(
        classify_prompt_source(
            "layered:prompts/user_response_contract_validator_prompt.md#vendor=minimax"
        ),
        "user_response_validator"
    );
}

#[test]
fn normalizer_recovers_openai_tool_call_as_execution_contract() {
    let raw_response = r#"{
      "choices":[{
        "finish_reason":"tool_calls",
        "message":{
          "content":"<think>need file evidence</think>",
          "tool_calls":[{
            "type":"function",
            "function":{
              "name":"read_file",
              "arguments":"{\"file_path\":\"/home/guagua/rustclaw/README.md\"}"
            }
          }]
        }
      }]
    }"#;
    let recovered = recover_normalizer_text_from_openai_tool_calls(
        "layered:prompts/intent_normalizer_prompt.md#vendor=minimax",
        "REQUEST: 读取 README 开头内容，再用一句话总结\n",
        raw_response,
    )
    .expect("recover tool call");
    let value = serde_json::from_str::<serde_json::Value>(&recovered).expect("json");

    assert_eq!(value.get("mode").and_then(|v| v.as_str()), Some("act"));
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_hint")
            .and_then(|v| v.as_str()),
        Some("/home/guagua/rustclaw/README.md")
    );
    assert_eq!(
        value.get("resolved_user_intent").and_then(|v| v.as_str()),
        Some("读取 README 开头内容，再用一句话总结")
    );
}

#[test]
fn tool_call_recovery_ignores_non_normalizer_prompts() {
    let raw_response = r#"{
      "choices":[{
        "message":{
          "tool_calls":[{
            "function":{"arguments":"{\"path\":\"/tmp/a.txt\"}"}
          }]
        }
      }]
    }"#;

    assert!(recover_normalizer_text_from_openai_tool_calls(
        "layered:prompts/chat_response_prompt.md#vendor=minimax",
        "REQUEST: read a file",
        raw_response,
    )
    .is_none());
}

#[test]
fn provider_override_matches_vendor_alias() {
    assert!(matches_provider_override(
        "vendor-qwen",
        "openai_compat",
        "qwen"
    ));
    assert!(matches_provider_override(
        "vendor-custom",
        "openai_compat",
        "custom"
    ));
    assert!(matches_provider_override(
        "vendor-minimax",
        "openai_compat",
        "minimax"
    ));
    assert!(matches_provider_override(
        "vendor-mimo",
        "openai_compat",
        "mimo"
    ));
    assert!(matches_provider_override(
        "vendor-mimo",
        "openai_compat",
        "xiaomi"
    ));
    assert!(matches_provider_override(
        "vendor-openai",
        "openai_compat",
        "openai"
    ));
}

#[test]
fn provider_override_matches_runtime_name_and_type() {
    assert!(matches_provider_override(
        "vendor-qwen",
        "openai_compat",
        "vendor-qwen"
    ));
    assert!(matches_provider_override(
        "vendor-openai",
        "openai_compat",
        "openai_compat"
    ));
    assert!(!matches_provider_override(
        "vendor-qwen",
        "openai_compat",
        "google"
    ));
}

#[test]
fn minimax_uses_openai_compat_runtime_when_selected() {
    let path = repo_config_path();
    let mut config =
        AppConfig::load(path.to_str().expect("utf-8 path")).expect("config fixture should load");
    config.llm.selected_vendor = Some("minimax".to_string());
    config.llm.selected_model = Some("MiniMax-M2.7".to_string());

    let providers = synthesize_llm_providers(&config, None, None);
    let minimax = providers
        .iter()
        .find(|provider| provider.name == "vendor-minimax")
        .expect("minimax provider should be synthesized");

    assert_eq!(minimax.provider_type, "openai_compat");
}

#[test]
fn mimo_uses_openai_compat_runtime_when_selected() {
    let path = repo_config_path();
    let mut config =
        AppConfig::load(path.to_str().expect("utf-8 path")).expect("config fixture should load");
    config.llm.selected_vendor = Some("mimo".to_string());
    config.llm.selected_model = Some("mimo-v2.5-pro".to_string());

    let providers = synthesize_llm_providers(&config, None, None);
    let mimo = providers
        .iter()
        .find(|provider| provider.name == "vendor-mimo")
        .expect("mimo provider should be synthesized");

    assert_eq!(mimo.provider_type, "openai_compat");
}

#[test]
fn provider_override_without_model_uses_target_vendor_default_model() {
    let path = repo_config_path();
    let mut config =
        AppConfig::load(path.to_str().expect("utf-8 path")).expect("config fixture should load");
    config.llm.selected_vendor = Some("mimo".to_string());
    config.llm.selected_model = Some("mimo-v2.5-pro".to_string());
    let qwen_default = config.llm.qwen.as_ref().expect("qwen config").model.clone();

    let providers = synthesize_llm_providers(&config, Some("qwen"), None);
    let qwen = providers
        .iter()
        .find(|provider| provider.name == "vendor-qwen")
        .expect("qwen provider should be synthesized");

    assert_eq!(qwen.model, qwen_default);
    assert_ne!(qwen.model, "mimo-v2.5-pro");
}

#[test]
fn mimo_respects_api_format_anthropic() {
    let path = repo_config_path();
    let mut config =
        AppConfig::load(path.to_str().expect("utf-8 path")).expect("config fixture should load");
    config.llm.selected_vendor = Some("mimo".to_string());
    config.llm.selected_model = Some("mimo-v2.5-pro".to_string());
    if let Some(ref mut mimo) = config.llm.mimo {
        mimo.api_format = Some("anthropic_claude".to_string());
    }

    let providers = synthesize_llm_providers(&config, None, None);
    let mimo = providers
        .iter()
        .find(|provider| provider.name == "vendor-mimo")
        .expect("mimo provider should be synthesized");

    assert_eq!(mimo.provider_type, "anthropic_claude");
}

#[test]
fn minimax_respects_api_format_anthropic() {
    let path = repo_config_path();
    let mut config =
        AppConfig::load(path.to_str().expect("utf-8 path")).expect("config fixture should load");
    config.llm.selected_vendor = Some("minimax".to_string());
    config.llm.selected_model = Some("MiniMax-M2.7".to_string());
    if let Some(ref mut mm) = config.llm.minimax {
        mm.api_format = Some("anthropic_claude".to_string());
    }

    let providers = synthesize_llm_providers(&config, None, None);
    let minimax = providers
        .iter()
        .find(|provider| provider.name == "vendor-minimax")
        .expect("minimax provider should be synthesized");

    assert_eq!(minimax.provider_type, "anthropic_claude");
}

#[test]
fn minimax_defaults_openai_when_api_format_missing_or_empty() {
    let path = repo_config_path();
    let mut config =
        AppConfig::load(path.to_str().expect("utf-8 path")).expect("config fixture should load");
    config.llm.selected_vendor = Some("minimax".to_string());
    if let Some(ref mut mm) = config.llm.minimax {
        mm.api_format = None;
    }

    let providers = synthesize_llm_providers(&config, None, None);
    let minimax = providers
        .iter()
        .find(|p| p.name == "vendor-minimax")
        .expect("vendor-minimax when api_format unset");
    assert_eq!(minimax.provider_type, "openai_compat");

    if let Some(ref mut mm) = config.llm.minimax {
        mm.api_format = Some("   ".to_string());
    }
    let providers = synthesize_llm_providers(&config, None, None);
    let minimax = providers
        .iter()
        .find(|p| p.name == "vendor-minimax")
        .expect("vendor-minimax when api_format blank");
    assert_eq!(minimax.provider_type, "openai_compat");
}
