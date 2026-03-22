//! WeChat channel TOML (`configs/channels/wechat.toml`).

use serde::Deserialize;

fn default_typing_refresh_secs() -> u64 {
    5
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
    pub bot_token: String,
    pub wechat_uin_base64: String,
    pub request_timeout_seconds: u64,
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
