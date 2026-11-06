//! Exercise the complete environment -> dedicated credential -> inheritance path.
//! Every case runs in a fresh process; parallel tests never mutate shared env.
use super::*;
use std::process::Command;

const ENDPOINT: &str = "https://main.example/v1";

fn isolated(name: &str, extra: &[(&str, &str)], exercise: impl FnOnce()) {
    const CHILD: &str = "APP_STT_CREDENTIAL_TEST_CHILD";
    if std::env::var(CHILD).ok().as_deref() == Some(name) {
        exercise();
        return;
    }
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            &format!("credential_selection_tests::{name}"),
            "--test-threads=1",
            "--nocapture",
        ])
        .env_clear()
        .env(CHILD, name)
        .env("OPENAI_BASE_URL", ENDPOINT)
        .env("OPENAI_API_KEY", "main-fixture-key")
        .env("MINIMAX_API_KEY", "main-fixture-key")
        .env("APP_SELECTED_LLM_VENDOR", "minimax")
        .env("APP_SELECTED_LLM_PROVIDER_TYPE", "openai_compat")
        .env("APP_SELECTED_LLM_HOSTED_RELAY", "false")
        .envs(extra.iter().copied())
        .output()
        .expect("isolated test child");
    assert!(
        output.status.success(),
        "isolated case {name} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 passed; 0 failed; 0 ignored"), "{stdout}");
}

fn vendor(endpoint: &str, key: &str) -> VendorConfig {
    VendorConfig {
        base_url: endpoint.into(),
        api_key: key.into(),
        model: "configured-model".into(),
        timeout_seconds: None,
    }
}

fn config(endpoint: &str, key: &str) -> RootConfig {
    RootConfig {
        llm: LlmConfig {
            selected_vendor: Some("minimax".into()),
            minimax: Some(vendor(ENDPOINT, "")),
            ..Default::default()
        },
        audio_transcribe: AudioTranscribeConfig {
            default_vendor: Some("minimax".into()),
            default_model: Some("configured-stt-model".into()),
            providers: AudioProviderOverrides {
                minimax: Some(vendor(endpoint, key)),
                ..Default::default()
            },
            ..Default::default()
        },
    }
}

fn minimax_key(cfg: &RootConfig) -> &str {
    &cfg.audio_transcribe
        .providers
        .minimax
        .as_ref()
        .unwrap()
        .api_key
}

#[test]
fn direct_same_endpoint_keeps_inline_stt_key() {
    isolated("direct_same_endpoint_keeps_inline_stt_key", &[], || {
        let mut cfg = config(ENDPOINT, "dedicated-fixture-key");
        apply_env_overrides(&mut cfg);
        assert_eq!(minimax_key(&cfg), "dedicated-fixture-key");
    });
}

#[test]
fn direct_same_endpoint_keeps_dedicated_environment_key() {
    isolated(
        "direct_same_endpoint_keeps_dedicated_environment_key",
        &[(
            "AUDIO_TRANSCRIBE_MINIMAX_API_KEY",
            "dedicated-env-fixture-key",
        )],
        || {
            let mut cfg = config(ENDPOINT, "inline-fixture-key");
            apply_env_overrides(&mut cfg);
            assert_eq!(minimax_key(&cfg), "dedicated-env-fixture-key");
        },
    );
}

#[test]
fn direct_missing_stt_key_inherits_same_vendor_main_key() {
    isolated(
        "direct_missing_stt_key_inherits_same_vendor_main_key",
        &[],
        || {
            let mut cfg = config(ENDPOINT, "");
            apply_env_overrides(&mut cfg);
            assert_eq!(minimax_key(&cfg), "main-fixture-key");
            assert_eq!(
                cfg.audio_transcribe.providers.minimax.unwrap().base_url,
                ENDPOINT
            );
        },
    );
}

#[test]
fn direct_different_endpoint_keeps_dedicated_key() {
    isolated("direct_different_endpoint_keeps_dedicated_key", &[], || {
        let mut cfg = config("https://audio.example/v1", "dedicated-fixture-key");
        apply_env_overrides(&mut cfg);
        assert_eq!(minimax_key(&cfg), "dedicated-fixture-key");
        assert_eq!(
            cfg.audio_transcribe.providers.minimax.unwrap().base_url,
            "https://audio.example/v1"
        );
    });
}

#[test]
fn enrolled_relay_matching_endpoint_uses_device_owned_key() {
    isolated(
        "enrolled_relay_matching_endpoint_uses_device_owned_key",
        &[
            ("APP_SELECTED_LLM_VENDOR", "custom"),
            ("APP_SELECTED_LLM_HOSTED_RELAY", "true"),
        ],
        || {
            let mut cfg = config(ENDPOINT, "dedicated-fixture-key");
            apply_env_overrides(&mut cfg);
            assert_eq!(minimax_key(&cfg), "main-fixture-key");
        },
    );
}

#[test]
fn enrolled_relay_different_endpoint_keeps_dedicated_key() {
    isolated(
        "enrolled_relay_different_endpoint_keeps_dedicated_key",
        &[
            ("APP_SELECTED_LLM_VENDOR", "custom"),
            ("APP_SELECTED_LLM_HOSTED_RELAY", "true"),
        ],
        || {
            let mut cfg = config("https://audio.example/v1", "dedicated-fixture-key");
            apply_env_overrides(&mut cfg);
            assert_eq!(minimax_key(&cfg), "dedicated-fixture-key");
        },
    );
}

#[test]
fn absent_connection_metadata_does_not_replace_dedicated_key() {
    isolated(
        "absent_connection_metadata_does_not_replace_dedicated_key",
        &[
            ("APP_SELECTED_LLM_VENDOR", ""),
            ("APP_SELECTED_LLM_PROVIDER_TYPE", ""),
            ("APP_SELECTED_LLM_HOSTED_RELAY", ""),
        ],
        || {
            let mut cfg = config(ENDPOINT, "dedicated-fixture-key");
            apply_env_overrides(&mut cfg);
            assert_eq!(minimax_key(&cfg), "dedicated-fixture-key");
        },
    );
}

#[test]
fn invalid_relay_flag_does_not_replace_dedicated_key() {
    isolated(
        "invalid_relay_flag_does_not_replace_dedicated_key",
        &[("APP_SELECTED_LLM_HOSTED_RELAY", "unknown")],
        || {
            let mut cfg = config(ENDPOINT, "dedicated-fixture-key");
            apply_env_overrides(&mut cfg);
            assert_eq!(minimax_key(&cfg), "dedicated-fixture-key");
        },
    );
}

fn assert_other_vendor_alias_is_not_an_openai_credential() {
    let mut cfg = config(ENDPOINT, "");
    cfg.llm.openai = Some(vendor("https://openai.example/v1", ""));
    cfg.audio_transcribe.providers.openai = Some(vendor("https://openai.example/v1", ""));
    apply_env_overrides(&mut cfg);
    assert_eq!(cfg.audio_transcribe.providers.openai.unwrap().api_key, "");
}

#[test]
fn minimax_protocol_alias_is_not_an_openai_stt_credential() {
    isolated(
        "minimax_protocol_alias_is_not_an_openai_stt_credential",
        &[],
        assert_other_vendor_alias_is_not_an_openai_credential,
    );
}

#[test]
fn qwen_protocol_alias_is_not_an_openai_stt_credential() {
    isolated(
        "qwen_protocol_alias_is_not_an_openai_stt_credential",
        &[
            ("APP_SELECTED_LLM_VENDOR", "qwen"),
            ("MINIMAX_API_KEY", ""),
            ("QWEN_API_KEY", "main-fixture-key"),
        ],
        assert_other_vendor_alias_is_not_an_openai_credential,
    );
}

#[test]
fn selected_openai_key_can_fill_openai_stt_credential() {
    isolated(
        "selected_openai_key_can_fill_openai_stt_credential",
        &[
            ("APP_SELECTED_LLM_VENDOR", "openai"),
            ("MINIMAX_API_KEY", ""),
        ],
        || {
            let mut cfg = config(ENDPOINT, "");
            cfg.llm.openai = Some(vendor("https://openai.example/v1", ""));
            cfg.audio_transcribe.providers.openai = Some(vendor("https://openai.example/v1", ""));
            apply_env_overrides(&mut cfg);
            assert_eq!(
                cfg.audio_transcribe.providers.openai.unwrap().api_key,
                "main-fixture-key"
            );
        },
    );
}

#[test]
fn standalone_canonical_environment_remains_supported() {
    isolated(
        "standalone_canonical_environment_remains_supported",
        &[
            ("APP_SELECTED_LLM_VENDOR", ""),
            ("APP_SELECTED_LLM_PROVIDER_TYPE", ""),
            ("APP_SELECTED_LLM_HOSTED_RELAY", ""),
            ("OPENAI_BASE_URL", ""),
            ("OPENAI_API_KEY", ""),
        ],
        || {
            let mut cfg = config(ENDPOINT, "");
            apply_env_overrides(&mut cfg);
            assert_eq!(minimax_key(&cfg), "main-fixture-key");
        },
    );
}

#[test]
fn credential_resolution_never_changes_configured_stt_backend() {
    isolated(
        "credential_resolution_never_changes_configured_stt_backend",
        &[],
        || {
            let mut cfg = config("https://audio.example/v1", "dedicated-fixture-key");
            cfg.audio_transcribe.default_vendor = Some("custom".into());
            cfg.audio_transcribe.default_model = Some("local-whisper".into());
            apply_env_overrides(&mut cfg);
            assert_eq!(
                cfg.audio_transcribe.default_vendor.as_deref(),
                Some("custom")
            );
            assert_eq!(
                cfg.audio_transcribe.default_model.as_deref(),
                Some("local-whisper")
            );
            assert_eq!(minimax_key(&cfg), "dedicated-fixture-key");
        },
    );
}

#[test]
fn enrolled_relay_endpoint_matching_preserves_normalization() {
    isolated(
        "enrolled_relay_endpoint_matching_preserves_normalization",
        &[
            ("APP_SELECTED_LLM_HOSTED_RELAY", "true"),
            ("OPENAI_BASE_URL", " https://main.example/v1/ "),
        ],
        || {
            let mut cfg = config(" https://main.example/v1/// ", "dedicated-fixture-key");
            apply_env_overrides(&mut cfg);
            assert_eq!(minimax_key(&cfg), "main-fixture-key");
        },
    );
}
