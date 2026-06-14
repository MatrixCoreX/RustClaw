use std::path::Path;

use claw_core::channel_i18n::{text_from_path, text_with_vars_from_path};

use super::FeishuConfig;

pub(super) fn default_listen() -> String {
    "0.0.0.0:8789".to_string()
}

pub(super) fn default_clawd_base_url() -> String {
    "http://127.0.0.1:8787".to_string()
}

pub(super) fn default_request_timeout() -> u64 {
    30
}

pub(super) fn default_task_delivery_timeout() -> u64 {
    600
}

pub(super) fn default_text_chunk_chars() -> usize {
    4000
}

pub(super) fn default_feishu_language() -> String {
    "zh-CN".to_string()
}

pub(super) fn default_feishu_i18n_path() -> String {
    "configs/i18n/feishud.zh-CN.toml".to_string()
}

pub(super) fn default_feishu_api_base_url() -> String {
    "https://open.feishu.cn".to_string()
}

pub(super) fn default_feishu_image_inbox_dir() -> String {
    "data/feishud/image".to_string()
}

pub(super) fn default_feishu_video_inbox_dir() -> String {
    "data/feishud/video".to_string()
}

pub(super) fn default_feishu_audio_inbox_dir() -> String {
    "data/feishud/audio".to_string()
}

pub(super) fn default_feishu_file_inbox_dir() -> String {
    "data/feishud/file".to_string()
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn apply_string_env(target: &mut String, key: &str) {
    if let Some(value) = env_non_empty(key) {
        *target = value;
    }
}

pub(super) fn apply_env_overrides(config: &mut FeishuConfig) {
    apply_string_env(&mut config.feishu.app_id, "FEISHU_APP_ID");
    apply_string_env(&mut config.feishu.app_secret, "FEISHU_APP_SECRET");
    apply_string_env(
        &mut config.feishu.verification_token,
        "FEISHU_VERIFICATION_TOKEN",
    );
    apply_string_env(&mut config.feishu.encrypt_key, "FEISHU_ENCRYPT_KEY");
    apply_string_env(&mut config.feishu.language, "FEISHU_I18N_LANGUAGE");
    apply_string_env(&mut config.feishu.i18n_path, "FEISHU_I18N_PATH");
}

pub(super) fn resolve_i18n_path(language: &str, configured_path: &str) -> String {
    let lang = language.trim();
    if !lang.is_empty() {
        let candidate = format!("configs/i18n/feishud.{lang}.toml");
        if Path::new(&candidate).exists() {
            return candidate;
        }
    }
    configured_path.to_string()
}

pub(super) fn feishu_t(config: &FeishuConfig, key: &str, fallback: &str) -> String {
    text_from_path(&config.feishu.i18n_path, key, fallback)
}

pub(super) fn feishu_t_with(
    config: &FeishuConfig,
    key: &str,
    vars: &[(&str, &str)],
    fallback: &str,
) -> String {
    text_with_vars_from_path(&config.feishu.i18n_path, key, vars, fallback)
}
