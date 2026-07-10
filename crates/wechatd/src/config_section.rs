//! WeChat channel TOML (`configs/channels/wechat.toml`).

use std::path::Path;

use claw_core::channel_i18n::{text_from_path, text_with_vars_from_path};
use serde::Deserialize;

fn default_typing_refresh_secs() -> u64 {
    5
}

fn default_task_delivery_timeout_seconds() -> u64 {
    600
}

fn default_cdn_base_url() -> String {
    "https://novac2c.cdn.weixin.qq.com/c2c".to_string()
}

fn default_image_inbox_dir() -> String {
    "data/wechatd/image".to_string()
}

fn default_video_inbox_dir() -> String {
    "data/wechatd/video".to_string()
}

fn default_audio_inbox_dir() -> String {
    "data/wechatd/audio".to_string()
}

fn default_file_inbox_dir() -> String {
    "data/wechatd/file".to_string()
}

fn default_language() -> String {
    "zh-CN".to_string()
}

fn default_i18n_path() -> String {
    "configs/i18n/wechatd.zh-CN.toml".to_string()
}

#[derive(Clone, Deserialize)]
pub struct AppConfig {
    pub wechat: WechatSection,
}

#[derive(Clone, Deserialize)]
pub struct WechatSection {
    pub enabled: bool,
    pub listen: String,
    pub clawd_base_url: String,
    pub api_base_url: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_i18n_path")]
    pub i18n_path: String,
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub wechat_uin_base64: String,
    pub request_timeout_seconds: u64,
    /// Soft timeout threshold for task delivery polling (seconds).
    /// Once exceeded, we notify the user that execution is still in progress and keep polling.
    #[serde(default = "default_task_delivery_timeout_seconds")]
    pub task_delivery_timeout_seconds: u64,
    pub longpoll_timeout_ms: u64,
    pub text_chunk_chars: usize,
    /// Optional `SKRouteTag` header (same as OpenClaw weixin plugin / `openclaw.json` routeTag).
    #[serde(default)]
    pub sk_route_tag: String,
    /// Interval between `sendtyping` refreshes while waiting for clawd (ms-equivalent: use seconds).
    #[serde(default = "default_typing_refresh_secs")]
    pub typing_refresh_interval_secs: u64,
    /// CDN base for media download/upload (OpenClaw default).
    #[serde(default = "default_cdn_base_url")]
    pub cdn_base_url: String,
    #[serde(default = "default_image_inbox_dir")]
    pub image_inbox_dir: String,
    #[serde(default = "default_video_inbox_dir")]
    pub video_inbox_dir: String,
    #[serde(default = "default_audio_inbox_dir")]
    pub audio_inbox_dir: String,
    #[serde(default = "default_file_inbox_dir")]
    pub file_inbox_dir: String,
}

pub fn resolve_wechat_i18n_path(language: &str, configured_path: &str) -> String {
    let lang = language.trim();
    if !lang.is_empty() {
        let candidate = format!("configs/i18n/wechatd.{lang}.toml");
        if Path::new(&candidate).exists() {
            return candidate;
        }
    }
    configured_path.to_string()
}

pub fn wechat_t(config: &WechatSection, key: &str) -> String {
    let path = resolve_wechat_i18n_path(&config.language, &config.i18n_path);
    text_from_path(&path, key, key)
}

pub fn wechat_t_with(config: &WechatSection, key: &str, vars: &[(&str, &str)]) -> String {
    let path = resolve_wechat_i18n_path(&config.language, &config.i18n_path);
    text_with_vars_from_path(&path, key, vars, key)
}
