//! OpenAI Compat 协议实现：覆盖 OpenAI 自家以及 Qwen / DeepSeek / Grok /
//! MiniMax (在 `api_format = openai_compat` 时) / 自托管模型等所有走
//! `/chat/completions` 兼容接口的 provider。
//!
//! 拆出来是为了让 `client.rs` 只保留协议中性的类型 + dispatcher，
//! 加新协议时只要新增一个文件 + 在 `client.rs::call_provider` 加一行分支。

use std::sync::Arc;

use serde_json::{json, Value};
use tracing::warn;

use super::client::{ChatRequestHints, LlmProviderResponse, ProviderError};
use super::openai_usage_snapshot;
use crate::LlmProviderRuntime;

pub(super) async fn call_openai_compat(
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

    let url = format!(
        "{}/chat/completions",
        provider.config.base_url.trim_end_matches('/')
    );

    let mut req_body = json!({
        "model": provider.config.model,
        "messages": [
            { "role": "user", "content": prompt }
        ],
        "stream": false
    });
    if let Some(map) = req_body.as_object_mut() {
        if let Some(t) = hints.temperature {
            map.insert("temperature".to_string(), json!(t));
        }
        if let Some(mt) = hints.max_tokens {
            map.insert("max_tokens".to_string(), json!(mt));
        }
    }

    let resp = provider
        .client
        .post(url)
        .bearer_auth(&provider.config.api_key)
        .json(&req_body)
        .send()
        .await
        .map_err(|err| {
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
    let usage = openai_usage_snapshot(&value);

    if let Some(reason) = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("finish_reason"))
        .and_then(|v| v.as_str())
    {
        if reason == "length" {
            warn!(
                "openai_compat response truncated: finish_reason=length model={}",
                provider.config.model
            );
        }
    }

    let text = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("message"))
        .and_then(|msg| msg.get("content"))
        .and_then(|content| content.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ProviderError::non_retryable_with_response(
                "missing choices[0].message.content".to_string(),
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
