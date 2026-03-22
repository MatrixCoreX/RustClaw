//! Weixin ilink bot HTTP helpers (aligned with `@tencent-weixin/openclaw-weixin`).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config_section::WechatSection;

const DEFAULT_CONFIG_API_TIMEOUT_MS: u64 = 10_000;

#[derive(Serialize)]
pub struct BaseInfo {
    pub channel_version: String,
}

pub fn base_info() -> BaseInfo {
    BaseInfo {
        channel_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

pub fn build_wechat_uin_header(config: &WechatSection) -> String {
    if !config.wechat_uin_base64.trim().is_empty() {
        return config.wechat_uin_base64.trim().to_string();
    }
    let value = (current_ts_ms() % (u32::MAX as u64)) as u32;
    BASE64_STANDARD.encode(value.to_string())
}

fn current_ts_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_millis() as u64)
        .unwrap_or(0)
}

/// Attach optional `SKRouteTag` (GET or POST).
pub fn apply_route_tag(
    req: reqwest::RequestBuilder,
    section: &WechatSection,
) -> reqwest::RequestBuilder {
    let t = section.sk_route_tag.trim();
    if t.is_empty() {
        req
    } else {
        req.header("SKRouteTag", t)
    }
}

pub async fn post_json<T: Serialize>(
    client: &Client,
    config: &WechatSection,
    base_url: &str,
    token: &str,
    endpoint: &str,
    body: &T,
    timeout_ms: u64,
) -> Result<Value, String> {
    let url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        endpoint.trim_start_matches('/')
    );
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("AuthorizationType", "ilink_bot_token")
        .header("Authorization", format!("Bearer {token}"))
        .header("X-WECHAT-UIN", build_wechat_uin_header(config))
        .json(body)
        .timeout(Duration::from_millis(timeout_ms.max(1_000)));
    req = apply_route_tag(req, config);
    let response = req
        .send()
        .await
        .map_err(|e| format!("wechat request failed: {e}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("wechat request status={status} body={body}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("wechat response parse failed: {e}"))
}

#[derive(Serialize)]
struct GetConfigReq<'a> {
    ilink_user_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_token: Option<&'a str>,
    base_info: BaseInfo,
}

#[derive(Debug, Deserialize)]
struct GetConfigResp {
    ret: Option<i64>,
    #[serde(default)]
    typing_ticket: Option<String>,
}

/// `ilink/bot/getconfig` — on upstream `ret == 0`, returns `Some(ticket)` (possibly empty).
/// `Ok(None)` means `ret != 0` (caller should retry/back off).
pub async fn get_config(
    client: &Client,
    config: &WechatSection,
    base_url: &str,
    token: &str,
    ilink_user_id: &str,
    context_token: Option<&str>,
) -> Result<Option<String>, String> {
    let body = GetConfigReq {
        ilink_user_id,
        context_token,
        base_info: base_info(),
    };
    let v = post_json(
        client,
        config,
        base_url,
        token,
        "ilink/bot/getconfig",
        &body,
        DEFAULT_CONFIG_API_TIMEOUT_MS,
    )
    .await?;
    let parsed: GetConfigResp = serde_json::from_value(v)
        .map_err(|e| format!("getconfig decode failed: {e}"))?;
    if parsed.ret == Some(0) {
        Ok(Some(parsed.typing_ticket.unwrap_or_default()))
    } else {
        Ok(None)
    }
}

#[derive(Serialize)]
struct SendTypingBody<'a> {
    ilink_user_id: &'a str,
    typing_ticket: &'a str,
    status: i64,
    base_info: BaseInfo,
}

/// `status`: `1` = typing, `2` = cancel (OpenClaw weixin plugin convention).
pub async fn send_typing_once(
    client: &Client,
    config: &WechatSection,
    base_url: &str,
    token: &str,
    ilink_user_id: &str,
    typing_ticket: &str,
    status: i64,
) -> Result<(), String> {
    let body = SendTypingBody {
        ilink_user_id,
        typing_ticket,
        status,
        base_info: base_info(),
    };
    post_json(
        client,
        config,
        base_url,
        token,
        "ilink/bot/sendtyping",
        &body,
        DEFAULT_CONFIG_API_TIMEOUT_MS,
    )
    .await?;
    Ok(())
}
