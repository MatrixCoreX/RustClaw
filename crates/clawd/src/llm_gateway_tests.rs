use std::path::PathBuf;

use claw_core::config::AppConfig;

use super::{
    build_providers_for_selection, classify_prompt_source, matches_provider_override,
    synthesize_llm_providers,
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
        classify_prompt_source("layered:prompts/user_response_composer_prompt.md#vendor=minimax"),
        "user_response_composer"
    );
    assert_eq!(
        classify_prompt_source(
            "layered:prompts/user_response_contract_validator_prompt.md#vendor=minimax"
        ),
        "user_response_validator"
    );
    assert_eq!(
        classify_prompt_source(
            "layered:prompts/native_action_protocol.md+prompts/native_turn_context.md"
        ),
        "plan"
    );
    assert_eq!(
        classify_prompt_source("inline:native_plan_contract_repair"),
        "plan"
    );
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

#[test]
fn configured_vendor_capabilities_reach_provider_runtime() {
    let path = repo_config_path();
    let config =
        AppConfig::load(path.to_str().expect("utf-8 path")).expect("config fixture should load");

    let minimax = build_providers_for_selection(&config, Some("minimax"), None)
        .into_iter()
        .next()
        .expect("minimax runtime provider");
    assert_eq!(
        minimax.config.input_modalities,
        vec!["text".to_string(), "image".to_string(), "video".to_string()]
    );
    assert!(minimax.config.supports_tools);
    assert_eq!(minimax.config.expected_latency_ms, Some(5_000));

    let mimo = build_providers_for_selection(&config, Some("mimo"), None)
        .into_iter()
        .next()
        .expect("mimo runtime provider");
    assert_eq!(mimo.config.input_modalities, vec!["text".to_string()]);
    assert!(mimo.config.supports_tools);
    assert_eq!(mimo.config.expected_latency_ms, Some(5_000));

    let anthropic = build_providers_for_selection(&config, Some("anthropic"), None)
        .into_iter()
        .next()
        .expect("anthropic runtime provider");
    assert_eq!(anthropic.config.provider_type, "anthropic_claude");
    assert!(anthropic.config.supports_tools);
    assert!(anthropic.model_capabilities().native_tools);
    assert!(!anthropic.model_capabilities().streaming);

    let google = build_providers_for_selection(&config, Some("google"), None)
        .into_iter()
        .next()
        .expect("google runtime provider");
    assert_eq!(google.config.provider_type, "google_gemini");
    assert!(google.config.supports_tools);
    assert!(google.model_capabilities().native_tools);
    assert!(google.model_capabilities().structured_output);
    assert!(!google.model_capabilities().streaming);
}
