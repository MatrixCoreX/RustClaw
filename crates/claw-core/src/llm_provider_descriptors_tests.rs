use super::*;
use crate::config::LlmCredentialSource;

fn parse(text: &str) -> LlmConfig {
    toml::from_str(text).unwrap()
}

fn vendor(name: &str, extra: &str) -> String {
    format!("[{name}]\nbase_url='https://fixture.invalid/v1'\nmodel='{name}-default'\n{extra}\n")
}

fn explicit(name: &str, protocol: &str, priority: i32) -> String {
    format!("[[providers]]\nname='{name}'\ntype='{protocol}'\nbase_url='https://fixture.invalid/v1'\n\
        api_key='fixture-provider-key'\nmodel='{name}-default'\npriority={priority}\ntimeout_seconds=30\nmax_concurrency=2\n")
}

#[test]
fn runtime_protocols_are_closed_and_project_the_exact_catalogue_tokens() {
    for (wire, display) in [
        ("openai_compat", "openai_compatible"),
        ("google_gemini", "google_gemini"),
        ("anthropic_claude", "anthropic_messages"),
    ] {
        let protocol = ProviderProtocol::from_runtime(wire).unwrap();
        assert_eq!(protocol.runtime_name(), wire);
        assert_eq!(protocol.catalog_name(), display);
    }
    for unsupported in [
        "",
        "openai",
        "openai_compatible",
        "anthropic",
        " OPENAI_COMPAT ",
        "fixture",
    ] {
        assert!(ProviderProtocol::from_runtime(unsupported).is_none());
    }
}

#[test]
fn all_vendor_defaults_preserve_priorities_protocols_and_complete_connection_fields() {
    let names = [
        "openai",
        "google",
        "anthropic",
        "grok",
        "deepseek",
        "qwen",
        "minimax",
        "mimo",
        "custom",
    ];
    let extra = "api_key='fixture-provider-key'\ncontext_window_tokens=12345\ninput_modalities=['text','image']\n\
        supports_tools=true\nexpected_latency_ms=321\ntimeout_seconds=456\nmax_concurrency=7\n";
    let mut config = String::new();
    for name in names {
        config.push_str(&vendor(name, extra));
    }
    let values = configured_vendor_configs(&parse(&config));
    assert_eq!(values.len(), names.len());
    for (index, provider) in values.iter().enumerate() {
        let name = names[index];
        assert_eq!(provider.name, format!("vendor-{name}"));
        assert_eq!(provider.model, format!("{name}-default"));
        assert_eq!(provider.priority, index as i32 + 1);
        assert_eq!(provider.base_url, "https://fixture.invalid/v1");
        assert_eq!(provider.api_key, "fixture-provider-key");
        assert_eq!(provider.context_window_tokens, Some(12345));
        assert_eq!(provider.input_modalities, ["text", "image"]);
        assert!(provider.supports_tools);
        assert_eq!(provider.expected_latency_ms, Some(321));
        assert_eq!(provider.timeout_seconds, 456);
        assert_eq!(provider.max_concurrency, 7);
        let expected = match name {
            "google" => "google_gemini",
            "anthropic" => "anthropic_claude",
            _ => "openai_compat",
        };
        assert_eq!(provider.provider_type, expected);
    }
}

#[test]
fn dual_protocol_aliases_and_unrecognized_format_fallback_keep_runtime_behavior() {
    for name in ["minimax", "mimo"] {
        for value in ["anthropic", "anthropic_claude", " ANTHROPIC "] {
            let config = parse(&vendor(name, &format!("api_format='{value}'")));
            assert_eq!(
                configured_vendor_configs(&config)[0].provider_type,
                "anthropic_claude"
            );
        }
        for value in [
            None,
            Some(""),
            Some("   "),
            Some("openai"),
            Some("openai_compat"),
            Some("fixture-unknown"),
        ] {
            let extra = value
                .map(|value| format!("api_format='{value}'"))
                .unwrap_or_default();
            assert_eq!(
                configured_vendor_configs(&parse(&vendor(name, &extra)))[0].provider_type,
                "openai_compat"
            );
        }
    }
}

#[test]
fn fixed_protocol_vendors_do_not_adopt_the_dual_vendor_format_setting() {
    for (name, expected) in [
        ("google", "google_gemini"),
        ("anthropic", "anthropic_claude"),
        ("custom", "openai_compat"),
    ] {
        let config = parse(&vendor(name, "api_format='anthropic'"));
        assert_eq!(
            configured_vendor_configs(&config)[0].provider_type,
            expected
        );
    }
}

#[test]
fn selected_model_and_cross_vendor_override_have_the_original_distinct_defaults() {
    let config = parse(&format!(
        "selected_vendor='minimax'\nselected_model='selected-model'\n{}{}",
        vendor("minimax", ""),
        vendor("qwen", "")
    ));
    let selected = synthesized_provider_configs(&config, None, None);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].model, "selected-model");
    assert_eq!(
        synthesized_provider_configs(&config, Some("minimax"), None)[0].model,
        "selected-model"
    );
    assert_eq!(
        synthesized_provider_configs(&config, Some("qwen"), None)[0].model,
        "qwen-default"
    );
    assert_eq!(
        synthesized_provider_configs(&config, Some("qwen"), Some("task-model"))[0].model,
        "task-model"
    );
    let no_model = parse(&format!(
        "selected_vendor='minimax'\n{}",
        vendor("minimax", "")
    ));
    assert_eq!(
        synthesized_provider_configs(&no_model, None, None)[0].model,
        "minimax-default"
    );
}

#[test]
fn absent_selected_vendor_keeps_all_vendor_defaults_instead_of_overriding_every_model() {
    let config = parse(&format!(
        "selected_model='inactive-selection'\n{}{}",
        vendor("minimax", ""),
        vendor("qwen", "")
    ));
    let sources = synthesized_provider_configs(&config, None, None);
    assert_eq!(sources.len(), 2);
    assert!(sources
        .iter()
        .all(|provider| provider.model.ends_with("-default")));
    let overridden = effective_provider_configs(&config, None, Some("process-model"));
    assert!(overridden
        .iter()
        .all(|provider| provider.model == "process-model"));
}

#[test]
fn explicit_provider_list_replaces_vendor_templates_and_retains_own_names_keys_and_priority() {
    let config = parse(
        &(vendor("minimax", "")
            + &explicit("second", "google_gemini", 5)
            + &explicit("first", "openai_compat", 1)),
    );
    let sources = effective_provider_configs(&config, None, None);
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0].name, "first");
    assert_eq!(sources[1].name, "second");
    assert!(sources
        .iter()
        .all(|provider| provider.api_key == "fixture-provider-key"));
    let overridden = effective_provider_configs(&config, Some("second"), Some("task-model"));
    assert_eq!(overridden.len(), 1);
    assert_eq!(overridden[0].name, "second");
    assert_eq!(overridden[0].model, "task-model");
}

#[test]
fn unsupported_explicit_protocol_and_untrusted_enrollment_are_not_executable() {
    let mut config = parse(
        &(explicit("unsupported", "fixture-protocol", 1)
            + &explicit("supported", "openai_compat", 2)),
    );
    config.providers[1].params.device_key_enrollment = true;
    let sources = effective_provider_configs(&config, None, None);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].name, "supported");
    assert!(!sources[0].params.device_key_enrollment);
}

#[test]
fn relay_ownership_requires_canonical_vendor_protocol_model_and_normalized_endpoint() {
    let config = parse(
        &(vendor("custom", "")
            + "[hosted_relay]\nenabled=true\nvendor='custom'\n\
        model='custom-default'\nbase_url='https://fixture.invalid/v1/'\n"),
    );
    let enrolled = effective_provider_configs(&config, None, None);
    assert_eq!(enrolled.len(), 1);
    assert!(enrolled[0].params.device_key_enrollment);
    let mut changed = enrolled[0].clone();
    for (name, value) in [
        ("name", "custom"),
        ("provider_type", "anthropic_claude"),
        ("model", "different"),
        ("base_url", "https://other.invalid/v1"),
    ] {
        let original = changed.clone();
        match name {
            "name" => changed.name = value.into(),
            "provider_type" => changed.provider_type = value.into(),
            "model" => changed.model = value.into(),
            "base_url" => changed.base_url = value.into(),
            _ => unreachable!(),
        }
        assert!(!matches_hosted_relay(&config, &changed));
        changed = original;
    }
}

#[test]
fn runtime_owned_resolved_credentials_survive_configuration_projection() {
    let mut config = parse(&explicit("fixture-explicit", "openai_compat", 1));
    assert_eq!(
        config.providers[0].params.credential_source,
        LlmCredentialSource::Managed
    );
    config.providers[0].params.credential_source = LlmCredentialSource::Resolved;
    config.providers[0].params.default_temperature = Some(0.3);
    config.providers[0].params.default_max_tokens = Some(123);
    config.providers[0].params.top_p = Some(0.9);
    config.providers[0].params.stream = Some(true);
    let source = &effective_provider_configs(&config, None, None)[0];
    assert_eq!(
        source.params.credential_source,
        LlmCredentialSource::Resolved
    );
    assert_eq!(source.params.default_temperature, Some(0.3));
    assert_eq!(source.params.default_max_tokens, Some(123));
    assert_eq!(source.params.top_p, Some(0.9));
    assert_eq!(source.params.stream, Some(true));
}

#[test]
fn override_name_matching_retains_vendor_alias_runtime_name_and_protocol_forms() {
    for name in [
        "mimo",
        "xiaomi",
        "vendor-mimo",
        "openai_compat",
        " VENDOR-MIMO ",
    ] {
        assert!(matches_provider_override(
            "vendor-mimo",
            "openai_compat",
            name
        ));
    }
    assert!(!matches_provider_override(
        "vendor-qwen",
        "openai_compat",
        "minimax"
    ));
}

#[test]
fn managed_secret_names_match_the_runtime_and_invalid_names_keep_inline_ownership() {
    for (name, expected) in [
        ("vendor-minimax", "minimax"),
        (" vendor- custom ", "custom"),
        ("vendor-other_1", "other_1"),
    ] {
        assert_eq!(credential_vendor_name(name), Some(expected));
    }
    for name in [
        "fixture-explicit",
        "custom",
        "vendor-",
        "vendor- ",
        "vendor-UPPER",
        "vendor-with-dash",
        "vendor-非ascii",
    ] {
        assert_eq!(credential_vendor_name(name), None);
    }
}
