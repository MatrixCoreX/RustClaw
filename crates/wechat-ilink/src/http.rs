//! Minimal ilink JSON POST (Authorization + optional SKRouteTag + X-WECHAT-UIN).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

use claw_core::channel_provider_error::{
    ChannelProviderError, ChannelProviderFailureClass, ChannelProviderTransportKind,
};

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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
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
    let response = req.send().await.map_err(|error| {
        let kind = if error.is_timeout() {
            ChannelProviderTransportKind::Timeout
        } else if error.is_connect() {
            ChannelProviderTransportKind::Connect
        } else {
            ChannelProviderTransportKind::Request
        };
        ChannelProviderError::from_transport(
            "wechat_ilink",
            endpoint.rsplit('/').next().unwrap_or("request"),
            kind,
            &error.to_string(),
        )
        .to_string()
    })?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let operation = endpoint.rsplit('/').next().unwrap_or("request");
        return Err(
            claw_core::channel_provider_error::ChannelProviderError::from_http_response(
                "wechat_ilink",
                operation,
                status.as_u16(),
                &body,
            )
            .to_string(),
        );
    }
    let value: Value = serde_json::from_str(&body).map_err(|error| {
        ChannelProviderError::invalid_response(
            "wechat_ilink",
            endpoint.rsplit('/').next().unwrap_or("request"),
            &error.to_string(),
        )
        .to_string()
    })?;
    if let Some(error) =
        decode_ilink_provider_failure(endpoint.rsplit('/').next().unwrap_or("request"), &value)
    {
        return Err(error.to_string());
    }
    Ok(value)
}

pub fn decode_ilink_provider_failure(
    operation: &str,
    value: &Value,
) -> Option<ChannelProviderError> {
    let code = value
        .get("errcode")
        .and_then(Value::as_i64)
        .filter(|code| *code != 0)
        .or_else(|| {
            value
                .get("ret")
                .and_then(Value::as_i64)
                .filter(|code| *code != 0)
        })?;
    let failure_class = if code == -14 {
        ChannelProviderFailureClass::Authentication
    } else {
        ChannelProviderFailureClass::Unknown
    };
    let code_text = code.to_string();
    Some(ChannelProviderError::from_machine_failure(
        "wechat_ilink",
        operation,
        failure_class,
        Some(200),
        Some(&code_text),
        None,
        &format!("ilink_provider_code:{code_text}"),
    ))
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
