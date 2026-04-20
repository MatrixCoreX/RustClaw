//! OpenAI Compat 协议实现：覆盖 OpenAI 自家以及 Qwen / DeepSeek / Grok /
//! MiniMax (在 `api_format = openai_compat` 时) / 自托管模型等所有走
//! `/chat/completions` 兼容接口的 provider。
//!
//! 拆出来是为了让 `client.rs` 只保留协议中性的类型 + dispatcher，
//! 加新协议时只要新增一个文件 + 在 `client.rs::call_provider` 加一行分支。

use std::sync::Arc;

use serde_json::{json, Value};
use tracing::warn;

use super::client::{is_quota_exhausted_429, ChatRequestHints, LlmProviderResponse, ProviderError};
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

    // Phase 2.5: hints 优先 → 没传时回退到 provider.config.params 里的 default_*。
    // 都没设则不写字段，保留 vendor 自己的默认行为（与 Phase 2.5 之前完全一致）。
    let params = &provider.config.params;
    let effective_temperature = hints.temperature.or(params.default_temperature);
    let effective_max_tokens = hints.max_tokens.or(params.default_max_tokens);
    let effective_top_p = params.top_p;
    // stream 默认 false（clawd 当前不消费流式响应），允许 toml 显式覆盖。
    let effective_stream = params.stream.unwrap_or(false);

    let mut req_body = json!({
        "model": provider.config.model,
        "messages": [
            { "role": "user", "content": prompt }
        ],
        "stream": effective_stream
    });
    if let Some(map) = req_body.as_object_mut() {
        if let Some(t) = effective_temperature {
            map.insert("temperature".to_string(), json!(t));
        }
        if let Some(mt) = effective_max_tokens {
            map.insert("max_tokens".to_string(), json!(mt));
        }
        if let Some(tp) = effective_top_p {
            map.insert("top_p".to_string(), json!(tp));
        }
    }

    // §P4.4 E3.a: 通过 LlmProviderRuntime::api_key() 走 SecretsBroker；broker
    // 没装/没声明就回落到 config.api_key（行为零变化）。
    let api_key = provider.api_key();
    let resp = provider
        .client
        .post(url)
        .bearer_auth(&*api_key)
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

    if status.as_u16() == 429 {
        let err = if is_quota_exhausted_429(&body_text) {
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
