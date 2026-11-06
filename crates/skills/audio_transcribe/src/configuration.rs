use super::workspace_root;
use super::{AudioTranscribeConfig, LlmConfig, RootConfig, VendorConfig};
use claw_core::llm_connection_environment::ConnectionEnvironment;
use toml::Value as TomlValue;

pub(super) fn load_root_config() -> RootConfig {
    let root = workspace_root();
    let core_cfg = match std::fs::read_to_string(root.join("configs/config.toml"))
        .ok()
        .and_then(|s| toml::from_str::<TomlValue>(&s).ok())
    {
        Some(v) => v,
        None => TomlValue::Table(toml::map::Map::new()),
    };
    let audio_cfg = match std::fs::read_to_string(root.join("configs/audio.toml"))
        .ok()
        .and_then(|s| toml::from_str::<TomlValue>(&s).ok())
    {
        Some(v) => v,
        None => TomlValue::Table(toml::map::Map::new()),
    };
    let mut cfg = RootConfig::default();
    if let Some(v) = core_cfg.get("llm").cloned() {
        if let Ok(parsed) = v.try_into::<LlmConfig>() {
            cfg.llm = parsed;
        }
    }
    if let Some(v) = audio_cfg.get("audio_transcribe").cloned() {
        if let Ok(parsed) = v.try_into::<AudioTranscribeConfig>() {
            cfg.audio_transcribe = parsed;
        }
    }
    apply_env_overrides(&mut cfg);
    cfg
}

fn env_non_empty(key: &str) -> Option<String> {
    claw_core::secrets::env_non_empty_resolved_or_none(key)
}

pub(super) fn configured_api_key(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty() && !value.starts_with("REPLACE_ME_")).then_some(value)
}

fn apply_vendor_api_key_env(target: &mut Option<VendorConfig>, key: &str) {
    if let (Some(value), Some(cfg)) = (
        env_non_empty(key).filter(|value| configured_api_key(value).is_some()),
        target.as_mut(),
    ) {
        cfg.api_key = value;
    }
}

pub(super) fn inherit_provider_connection_from_llm(
    target: &mut Option<VendorConfig>,
    source: &Option<VendorConfig>,
) {
    let (Some(target), Some(source)) = (target.as_mut(), source.as_ref()) else {
        return;
    };
    if target.base_url.trim().is_empty() && !source.base_url.trim().is_empty() {
        target.base_url = source.base_url.clone();
    }
    if configured_api_key(&target.api_key).is_none()
        && configured_api_key(&source.api_key).is_some()
    {
        target.api_key = source.api_key.clone();
    }
}

pub(super) fn apply_selected_key_for_matching_endpoint(
    target: &mut Option<VendorConfig>,
    selected_base_url: Option<&str>,
    selected_api_key: Option<&str>,
) {
    let (Some(target), Some(selected_base_url), Some(selected_api_key)) =
        (target.as_mut(), selected_base_url, selected_api_key)
    else {
        return;
    };
    let target_base_url = target.base_url.trim().trim_end_matches('/');
    let selected_base_url = selected_base_url.trim().trim_end_matches('/');
    if !target_base_url.is_empty()
        && target_base_url == selected_base_url
        && !selected_api_key.trim().is_empty()
    {
        target.api_key = selected_api_key.trim().to_string();
    }
}

fn apply_option_string_env(target: &mut Option<String>, key: &str) {
    if let Some(value) = env_non_empty(key) {
        *target = Some(value);
    }
}

pub(super) fn apply_env_overrides(cfg: &mut RootConfig) {
    let selected_base_url = env_non_empty("OPENAI_BASE_URL");
    let selected_minimax_api_key = env_non_empty("MINIMAX_API_KEY");
    let connection = ConnectionEnvironment::from_env_with(|name| std::env::var(name).ok());
    for (vendor, target) in [
        ("openai", &mut cfg.llm.openai),
        ("google", &mut cfg.llm.google),
        ("anthropic", &mut cfg.llm.anthropic),
        ("grok", &mut cfg.llm.grok),
        ("deepseek", &mut cfg.llm.deepseek),
        ("qwen", &mut cfg.llm.qwen),
        ("minimax", &mut cfg.llm.minimax),
        ("custom", &mut cfg.llm.custom),
    ] {
        if connection.permits_vendor_credential(vendor) {
            for name in claw_core::config::llm_vendor_api_key_env_names(vendor) {
                apply_vendor_api_key_env(target, name);
            }
        }
    }

    apply_vendor_api_key_env(
        &mut cfg.audio_transcribe.providers.openai,
        "AUDIO_TRANSCRIBE_OPENAI_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.audio_transcribe.providers.google,
        "AUDIO_TRANSCRIBE_GOOGLE_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.audio_transcribe.providers.anthropic,
        "AUDIO_TRANSCRIBE_ANTHROPIC_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.audio_transcribe.providers.grok,
        "AUDIO_TRANSCRIBE_GROK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.audio_transcribe.providers.deepseek,
        "AUDIO_TRANSCRIBE_DEEPSEEK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.audio_transcribe.providers.qwen,
        "AUDIO_TRANSCRIBE_QWEN_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.audio_transcribe.providers.minimax,
        "AUDIO_TRANSCRIBE_MINIMAX_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.audio_transcribe.providers.custom,
        "AUDIO_TRANSCRIBE_CUSTOM_API_KEY",
    );
    inherit_provider_connection_from_llm(
        &mut cfg.audio_transcribe.providers.openai,
        &cfg.llm.openai,
    );
    inherit_provider_connection_from_llm(
        &mut cfg.audio_transcribe.providers.google,
        &cfg.llm.google,
    );
    inherit_provider_connection_from_llm(
        &mut cfg.audio_transcribe.providers.anthropic,
        &cfg.llm.anthropic,
    );
    inherit_provider_connection_from_llm(&mut cfg.audio_transcribe.providers.grok, &cfg.llm.grok);
    inherit_provider_connection_from_llm(
        &mut cfg.audio_transcribe.providers.deepseek,
        &cfg.llm.deepseek,
    );
    inherit_provider_connection_from_llm(&mut cfg.audio_transcribe.providers.qwen, &cfg.llm.qwen);
    inherit_provider_connection_from_llm(
        &mut cfg.audio_transcribe.providers.minimax,
        &cfg.llm.minimax,
    );
    inherit_provider_connection_from_llm(
        &mut cfg.audio_transcribe.providers.custom,
        &cfg.llm.custom,
    );
    // A shared endpoint does not make a direct main credential authoritative.
    // Only the host's explicitly enrolled relay owns this credential override.
    if connection.is_enrolled_relay() {
        apply_selected_key_for_matching_endpoint(
            &mut cfg.audio_transcribe.providers.minimax,
            selected_base_url.as_deref(),
            selected_minimax_api_key.as_deref(),
        );
    }
    apply_option_string_env(
        &mut cfg.audio_transcribe.oss_access_key_id,
        "AUDIO_TRANSCRIBE_OSS_ACCESS_KEY_ID",
    );
    apply_option_string_env(
        &mut cfg.audio_transcribe.oss_access_key_secret,
        "AUDIO_TRANSCRIBE_OSS_ACCESS_KEY_SECRET",
    );
}
