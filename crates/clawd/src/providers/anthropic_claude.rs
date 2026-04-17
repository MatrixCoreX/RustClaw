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
use super::client::{ChatRequestHints, LlmProviderResponse, ProviderError};
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
    let max_tokens = hints.max_tokens.unwrap_or(4096);
    let mut req_body = json!({
        "model": provider.config.model,
        "max_tokens": max_tokens,
        "messages": [
            { "role": "user", "content": prompt }
        ]
    });
    if let Some(t) = hints.temperature {
        if let Some(map) = req_body.as_object_mut() {
            map.insert("temperature".to_string(), json!(t));
        }
    }

    let request = provider
        .client
        .post(url)
        .header("anthropic-version", "2023-06-01");
    let request = match anthropic_auth_mode(&provider) {
        AnthropicAuthMode::XApiKey => request.header("x-api-key", &provider.config.api_key),
        AnthropicAuthMode::AuthorizationBearer => request.bearer_auth(&provider.config.api_key),
    };

    let resp = request.json(&req_body).send().await.map_err(|err| {
        if err.is_timeout() {
            ProviderError::retryable(format!("timeout: {err}"), req_body.clone())
        } else {
            ProviderError::retryable(format!("request failed: {err}"), req_body.clone())
        }
    })?;

    let status = resp.status();
    let body_text = resp.text().await.map_err(|err| {
        ProviderError::retryable(format!("read response failed: {err}"), req_body.clone())
    })?;

    if status.as_u16() == 429 || status.is_server_error() {
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
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use claw_core::config::LlmProviderConfig;
    use reqwest::Client;
    use tokio::sync::Semaphore;

    use super::{anthropic_auth_mode, anthropic_messages_url, AnthropicAuthMode};
    use crate::LlmProviderRuntime;

    fn provider(name: &str, base_url: &str) -> LlmProviderRuntime {
        LlmProviderRuntime {
            config: LlmProviderConfig {
                name: name.to_string(),
                provider_type: "anthropic_claude".to_string(),
                base_url: base_url.to_string(),
                api_key: "test-key".to_string(),
                model: "test-model".to_string(),
                priority: 1,
                timeout_seconds: 30,
                max_concurrency: 1,
            },
            client: Client::new(),
            semaphore: Arc::new(Semaphore::new(1)),
            breaker: Arc::new(crate::providers::CircuitBreaker::new()),
        }
    }

    #[test]
    fn anthropic_messages_url_appends_v1_when_base_url_has_no_version() {
        let provider = provider("vendor-minimax", "https://api.minimaxi.com/anthropic");
        assert_eq!(
            anthropic_messages_url(&provider),
            "https://api.minimaxi.com/anthropic/v1/messages"
        );
    }

    #[test]
    fn anthropic_messages_url_reuses_existing_v1_suffix() {
        let provider = provider("vendor-anthropic", "https://api.anthropic.com/v1");
        assert_eq!(
            anthropic_messages_url(&provider),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn minimax_anthropic_uses_bearer_auth() {
        let provider = provider("vendor-minimax", "https://api.minimaxi.com/anthropic");
        assert_eq!(
            anthropic_auth_mode(&provider),
            AnthropicAuthMode::AuthorizationBearer
        );
    }

    #[test]
    fn anthropic_vendor_uses_x_api_key_auth() {
        let provider = provider("vendor-anthropic", "https://api.anthropic.com/v1");
        assert_eq!(anthropic_auth_mode(&provider), AnthropicAuthMode::XApiKey);
    }
}
