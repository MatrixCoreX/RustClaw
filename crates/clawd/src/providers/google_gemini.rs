//! Google Gemini (`generateContent`) 协议实现。
//!
//! 与 OpenAI Compat 形态有显著差异：
//! - 走 `?key=` query string 鉴权而非 Bearer token；
//! - 请求体用 `contents[].parts[].text`；
//! - hints 通过 `generationConfig.{temperature, maxOutputTokens}` 传入；
//! - safety filter / recitation 等需要把响应转 non-retryable error。

use std::sync::Arc;

use serde_json::{json, Value};
use tracing::warn;

use super::client::{is_quota_exhausted_429, ChatRequestHints, LlmProviderResponse, ProviderError};
use super::gemini_usage_snapshot;
use crate::LlmProviderRuntime;

pub(super) async fn call_google_gemini(
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

    // §P4.4 E3.a: 通过 LlmProviderRuntime::api_key() 走 SecretsBroker；broker
    // 没装/没声明就回落到 config.api_key（行为零变化）。Gemini 把 key 拼在
    // URL query 里，这里 expose 一次给 url 模板用，作用域结束即丢弃。
    let api_key = provider.api_key();
    let url = format!(
        "{}/models/{}:generateContent?key={}",
        provider.config.base_url.trim_end_matches('/'),
        provider.config.model,
        &*api_key
    );

    // Phase 2.5: hints 优先 → 否则回退到 provider.config.params。
    let params = &provider.config.params;
    let effective_temperature = hints.temperature.or(params.default_temperature);
    let effective_max_tokens = hints.max_tokens.or(params.default_max_tokens);
    let effective_top_p = params.top_p;

    let mut req_body = json!({
        "contents": [{
            "parts": [{ "text": prompt }]
        }]
    });
    if effective_temperature.is_some()
        || effective_max_tokens.is_some()
        || effective_top_p.is_some()
    {
        let mut gen_cfg = serde_json::Map::new();
        if let Some(t) = effective_temperature {
            gen_cfg.insert("temperature".to_string(), json!(t));
        }
        if let Some(mt) = effective_max_tokens {
            gen_cfg.insert("maxOutputTokens".to_string(), json!(mt));
        }
        if let Some(tp) = effective_top_p {
            gen_cfg.insert("topP".to_string(), json!(tp));
        }
        if let Some(map) = req_body.as_object_mut() {
            map.insert("generationConfig".to_string(), Value::Object(gen_cfg));
        }
    }

    let resp = provider
        .client
        .post(url)
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
    let usage = gemini_usage_snapshot(&value);

    if let Some(block_reason) = value
        .get("promptFeedback")
        .and_then(|v| v.get("blockReason"))
        .and_then(|v| v.as_str())
    {
        return Err(ProviderError::non_retryable_with_response(
            format!("gemini prompt blocked: blockReason={block_reason}"),
            req_body.clone(),
            body_text.clone(),
            usage.clone(),
        ));
    }

    if let Some(finish_reason) = value
        .get("candidates")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("finishReason"))
        .and_then(|v| v.as_str())
    {
        match finish_reason {
            "MAX_TOKENS" => {
                warn!(
                    "gemini response truncated: finishReason=MAX_TOKENS model={}",
                    provider.config.model
                );
            }
            "SAFETY" => {
                return Err(ProviderError::non_retryable_with_response(
                    format!(
                        "gemini response blocked by safety filter: finishReason=SAFETY model={}",
                        provider.config.model
                    ),
                    req_body.clone(),
                    body_text.clone(),
                    usage.clone(),
                ));
            }
            "RECITATION" => {
                return Err(ProviderError::non_retryable_with_response(
                    format!(
                        "gemini response blocked: finishReason=RECITATION model={}",
                        provider.config.model
                    ),
                    req_body.clone(),
                    body_text.clone(),
                    usage.clone(),
                ));
            }
            _ => {}
        }
    }

    let text = value
        .get("candidates")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|v| v.as_array())
        .and_then(|parts| {
            let mut merged = String::new();
            for p in parts {
                if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                    merged.push_str(t);
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
                "missing candidates[0].content.parts[*].text".to_string(),
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
