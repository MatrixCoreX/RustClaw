use std::path::Path;

use claw_core::channel_i18n::{text_from_path, text_with_vars_from_path};

use super::LarkConfig;

pub(super) fn default_listen() -> String {
    "0.0.0.0:8790".to_string()
}

pub(super) fn default_clawd_base_url() -> String {
    "http://127.0.0.1:8787".to_string()
}

/// 国际版 Lark 默认端点，与 feishud 的 open.feishu.cn 分开
pub(super) fn default_api_base_url() -> String {
    "https://open.larksuite.com".to_string()
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

pub(super) fn default_lark_language() -> String {
    "en-US".to_string()
}

pub(super) fn default_lark_i18n_path() -> String {
    "configs/i18n/larkd.en-US.toml".to_string()
}

pub(super) fn default_lark_image_inbox_dir() -> String {
    "data/larkd/image".to_string()
}

pub(super) fn default_lark_video_inbox_dir() -> String {
    "data/larkd/video".to_string()
}

pub(super) fn default_lark_audio_inbox_dir() -> String {
    "data/larkd/audio".to_string()
}

pub(super) fn default_lark_file_inbox_dir() -> String {
    "data/larkd/file".to_string()
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

pub(super) fn apply_env_overrides(config: &mut LarkConfig) {
    apply_string_env(&mut config.lark.app_id, "LARK_APP_ID");
    apply_string_env(&mut config.lark.app_secret, "LARK_APP_SECRET");
    apply_string_env(
        &mut config.lark.verification_token,
        "LARK_VERIFICATION_TOKEN",
    );
    apply_string_env(&mut config.lark.encrypt_key, "LARK_ENCRYPT_KEY");
    apply_string_env(&mut config.lark.language, "LARK_I18N_LANGUAGE");
    apply_string_env(&mut config.lark.i18n_path, "LARK_I18N_PATH");
}

pub(super) fn resolve_i18n_path(language: &str, configured_path: &str) -> String {
    let lang = language.trim();
    if !lang.is_empty() {
        let candidate = format!("configs/i18n/larkd.{lang}.toml");
        if Path::new(&candidate).exists() {
            return candidate;
        }
    }
    configured_path.to_string()
}

pub(super) fn lark_t(config: &LarkConfig, key: &str, fallback: &str) -> String {
    text_from_path(&config.lark.i18n_path, key, fallback)
}

pub(super) fn lark_t_with(
    config: &LarkConfig,
    key: &str,
    vars: &[(&str, &str)],
    fallback: &str,
) -> String {
    text_with_vars_from_path(&config.lark.i18n_path, key, vars, fallback)
}
