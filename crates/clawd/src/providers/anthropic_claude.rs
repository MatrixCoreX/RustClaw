//! Anthropic Messages API 协议实现。
//!
//! 关键差异：
//! - URL 必须形如 `<base>/v1/messages`，base_url 既可能已经含 `/v1`（如
//!   `https://api.anthropic.com/v1`），也可能不含（如 MiniMax 的
//!   `https://api.minimaxi.com/anthropic`），见 [`anthropic_messages_url`]。
//! - 鉴权：Anthropic 自家用 `x-api-key`，MiniMax 走 OAuth `Authorization: Bearer`，
//!   见 [`anthropic_auth_mode`]。
//! - 必须显式 `max_tokens`，没传走 4096 默认。
//! - 响应在 `content[*]` 数组中合并 `type=text` 段；其余 type（tool_use 等）
//!   当前不解析（chat 链路用不到）。

use std::sync::Arc;

use serde_json::{json, Value};
use tracing::warn;

use super::anthropic_usage_snapshot;
use super::client::{
    is_quota_exhausted_response, ChatRequestHints, LlmProviderResponse, ProviderError,
};
use crate::LlmProviderRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AnthropicAuthMode {
    XApiKey,
    AuthorizationBearer,
}

pub(super) fn anthropic_messages_url(provider: &LlmProviderRuntime) -> String {
    let base = provider.config.base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/messages")
    } else {
        format!("{base}/v1/messages")
    }
}

pub(super) fn anthropic_auth_mode(provider: &LlmProviderRuntime) -> AnthropicAuthMode {
    if provider.config.name.eq_ignore_ascii_case("vendor-minimax") {
        AnthropicAuthMode::AuthorizationBearer
    } else {
        AnthropicAuthMode::XApiKey
    }
}

pub(super) async fn call_anthropic_claude(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
    hints: &ChatRequestHints,
) -> Result<LlmProviderResponse, ProviderError> {
    let _permit = provider
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| {
            ProviderError::non_retryable(format!("semaphore closed: {err}"), Value::Null)
        })?;

    let url = anthropic_messages_url(&provider);
    // Phase 2.5: hints → params → 4096 fallback。Anthropic 协议要求必须传 max_tokens，
    // 所以这里**一定**会得到一个值，与 Phase 2.5 之前完全一致。
    let params = &provider.config.params;
    let max_tokens = hints
        .max_tokens
        .or(params.default_max_tokens)
        .unwrap_or(4096);
    let effective_temperature = hints.temperature.or(params.default_temperature);
    let effective_top_p = params.top_p;
    let mut req_body = json!({
        "model": provider.config.model,
        "max_tokens": max_tokens,
        "messages": [
            { "role": "user", "content": prompt }
        ]
    });
    if let Some(map) = req_body.as_object_mut() {
        if let Some(t) = effective_temperature {
            map.insert("temperature".to_string(), json!(t));
        }
        if let Some(tp) = effective_top_p {
            map.insert("top_p".to_string(), json!(tp));
        }
    }

    // §P4.4 E3.a: 通过 LlmProviderRuntime::api_key() 走 SecretsBroker；broker
    // 没装/没声明就回落到 config.api_key（行为零变化）。
    let api_key = provider.api_key();
    let request = provider
        .client
        .post(url)
        .header("anthropic-version", "2023-06-01");
    let request = match anthropic_auth_mode(&provider) {
        AnthropicAuthMode::XApiKey => request.header("x-api-key", &*api_key),
        AnthropicAuthMode::AuthorizationBearer => request.bearer_auth(&*api_key),
    };

    let resp = request.json(&req_body).send().await.map_err(|err| {
        if err.is_timeout() {
            ProviderError::timeout(format!("timeout: {err}"), req_body.clone())
        } else {
            ProviderError::retryable(format!("request failed: {err}"), req_body.clone())
        }
    })?;

    let status = resp.status();
    let body_text = resp.text().await.map_err(|err| {
        ProviderError::retryable(format!("read response failed: {err}"), req_body.clone())
    })?;

    if status.as_u16() == 429 {
        let err = if is_quota_exhausted_response(&body_text) {
            ProviderError::quota_exhausted_with_response(
                format!("http {}: {}", status.as_u16(), body_text),
                req_body.clone(),
                body_text,
                None,
            )
        } else {
            ProviderError::rate_limited_with_response(
                format!("http {}: {}", status.as_u16(), body_text),
                req_body.clone(),
                body_text,
                None,
            )
        };
        return Err(err);
    }

    if status.is_server_error() {
        return Err(ProviderError::retryable_with_response(
            format!("http {}: {}", status.as_u16(), body_text),
            req_body.clone(),
            body_text,
            None,
        ));
    }

    if !status.is_success() {
        return Err(ProviderError::non_retryable_with_response(
            format!("http {}: {}", status.as_u16(), body_text),
            req_body.clone(),
            body_text,
            None,
        ));
    }

    let value: Value = serde_json::from_str(&body_text).map_err(|err| {
        ProviderError::non_retryable_with_response(
            format!("parse response failed: {err}"),
            req_body.clone(),
            body_text.clone(),
            None,
        )
    })?;
    let usage = anthropic_usage_snapshot(&value);

    if let Some(stop_reason) = value.get("stop_reason").and_then(|v| v.as_str()) {
        if stop_reason == "max_tokens" {
            warn!(
                "anthropic response truncated: stop_reason=max_tokens model={}",
                provider.config.model
            );
        }
    }

    let text = value
        .get("content")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            let mut merged = String::new();
            for item in arr {
                if item.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                        merged.push_str(t);
                    }
                }
            }
            if merged.is_empty() {
                None
            } else {
                Some(merged)
            }
        })
        .ok_or_else(|| {
            ProviderError::non_retryable_with_response(
                "missing content[*].text".to_string(),
                req_body.clone(),
                body_text.clone(),
                usage.clone(),
            )
        })?;

    Ok(LlmProviderResponse {
        text,
        request_payload: req_body,
        raw_response: body_text,
        usage,
        attempts: 1,
        retryable_error_count: 0,
        last_retry_error_kind: None,
    })
}

#[cfg(test)]
#[path = "anthropic_claude_tests.rs"]
mod tests;
