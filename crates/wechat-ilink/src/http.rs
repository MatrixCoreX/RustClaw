//! Minimal ilink JSON POST (Authorization + optional SKRouteTag + X-WECHAT-UIN).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

/// Per-request routing / UIN headers (from channel config).
#[derive(Clone, Copy)]
pub struct IlinkAuth<'a> {
    pub sk_route_tag: &'a str,
    pub wechat_uin_base64: &'a str,
}

fn current_ts_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_millis() as u64)
        .unwrap_or(0)
}

pub fn build_wechat_uin_header(explicit_trimmed: &str) -> String {
    if !explicit_trimmed.trim().is_empty() {
        return explicit_trimmed.trim().to_string();
    }
    let value = (current_ts_ms() % (u32::MAX as u64)) as u32;
    BASE64_STANDARD.encode(value.to_string())
}

#[derive(Serialize)]
pub struct BaseInfo {
    pub channel_version: String,
}

pub fn base_info(channel_version: &str) -> BaseInfo {
    BaseInfo {
        channel_version: channel_version.to_string(),
    }
}

pub async fn post_ilink_json<T: Serialize>(
    client: &Client,
    ilink_base_url: &str,
    token: &str,
    auth: IlinkAuth<'_>,
    endpoint: &str,
    body: &T,
    timeout_ms: u64,
) -> Result<Value, String> {
    let url = format!(
        "{}/{}",
        ilink_base_url.trim_end_matches('/'),
        endpoint.trim_start_matches('/')
    );
    let uin = build_wechat_uin_header(auth.wechat_uin_base64);
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("AuthorizationType", "ilink_bot_token")
        .header("Authorization", format!("Bearer {token}"))
        .header("X-WECHAT-UIN", uin)
        .json(body)
        .timeout(Duration::from_millis(timeout_ms.max(1_000)));
    let t = auth.sk_route_tag.trim();
    if !t.is_empty() {
        req = req.header("SKRouteTag", t);
    }
    let response = req
        .send()
        .await
        .map_err(|e| format!("wechat ilink request failed: {e}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("wechat ilink status={status} body={body}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("wechat ilink response parse failed: {e}"))
}
