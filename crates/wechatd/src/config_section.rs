//! WeChat channel TOML (`configs/channels/wechat.toml`).

use serde::Deserialize;

fn default_typing_refresh_secs() -> u64 {
    5
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
}
