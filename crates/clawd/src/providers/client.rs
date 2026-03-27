use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tracing::warn;

use super::{anthropic_usage_snapshot, gemini_usage_snapshot, openai_usage_snapshot};
use crate::{LLM_RETRY_TIMES, LlmProviderRuntime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnthropicAuthMode {
    XApiKey,
    AuthorizationBearer,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct LlmUsageSnapshot {
    pub(crate) prompt_tokens: Option<u64>,
    pub(crate) completion_tokens: Option<u64>,
    pub(crate) total_tokens: Option<u64>,
    pub(crate) input_tokens: Option<u64>,
    pub(crate) output_tokens: Option<u64>,
    pub(crate) reasoning_tokens: Option<u64>,
    pub(crate) cached_tokens: Option<u64>,
    pub(crate) cache_creation_input_tokens: Option<u64>,
    pub(crate) cache_read_input_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct LlmProviderResponse {
    pub(crate) text: String,
    pub(crate) request_payload: Value,
    pub(crate) raw_response: String,
    pub(crate) usage: Option<LlmUsageSnapshot>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderError {
    pub(crate) retryable: bool,
    pub(crate) message: String,
    pub(crate) request_payload: Value,
    pub(crate) raw_response: Option<String>,
    pub(crate) usage: Option<LlmUsageSnapshot>,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl ProviderError {
    fn retryable(message: String, request_payload: Value) -> Self {
        Self {
            retryable: true,
            message,
            request_payload,
            raw_response: None,
            usage: None,
        }
    }

    fn retryable_with_response(
        message: String,
        request_payload: Value,
        raw_response: String,
        usage: Option<LlmUsageSnapshot>,
    ) -> Self {
        Self {
            retryable: true,
            message,
            request_payload,
            raw_response: Some(raw_response),
            usage,
        }
    }

    fn non_retryable(message: String, request_payload: Value) -> Self {
        Self {
            retryable: false,
            message,
            request_payload,
            raw_response: None,
            usage: None,
        }
    }

    fn non_retryable_with_response(
        message: String,
        request_payload: Value,
        raw_response: String,
        usage: Option<LlmUsageSnapshot>,
    ) -> Self {
        Self {
            retryable: false,
            message,
            request_payload,
            raw_response: Some(raw_response),
            usage,
        }
    }
}

pub(crate) async fn call_provider_with_retry(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<LlmProviderResponse, ProviderError> {
    let mut attempts = 0usize;

    loop {
        attempts += 1;
        match call_provider(provider.clone(), prompt).await {
            Ok(output) => return Ok(output),
            Err(err) if err.retryable => {
                if attempts > LLM_RETRY_TIMES {
                    return Err(err);
                }
                tokio::time::sleep(Duration::from_millis(250 * attempts as u64)).await;
            }
            Err(err) => return Err(err),
        }
    }
}

async fn call_provider(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<LlmProviderResponse, ProviderError> {
    match provider.config.provider_type.as_str() {
        "openai_compat" => call_openai_compat(provider, prompt).await,
        "google_gemini" => call_google_gemini(provider, prompt).await,
        "anthropic_claude" => call_anthropic_claude(provider, prompt).await,
        other => Err(ProviderError::non_retryable(
            format!("unsupported provider type: {other}"),
            Value::Null,
        )),
    }
}

fn anthropic_messages_url(provider: &LlmProviderRuntime) -> String {
    let base = provider.config.base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/messages")
    } else {
        format!("{base}/v1/messages")
    }
}

fn anthropic_auth_mode(provider: &LlmProviderRuntime) -> AnthropicAuthMode {
    if provider.config.name.eq_ignore_ascii_case("vendor-minimax") {
        AnthropicAuthMode::AuthorizationBearer
    } else {
        AnthropicAuthMode::XApiKey
    }
}

async fn call_openai_compat(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
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

    let req_body = json!({
        "model": provider.config.model,
        "messages": [
            { "role": "user", "content": prompt }
        ],
        "stream": false
    });

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

async fn call_google_gemini(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
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
        "{}/models/{}:generateContent?key={}",
        provider.config.base_url.trim_end_matches('/'),
        provider.config.model,
        provider.config.api_key
    );

    let req_body = json!({
        "contents": [{
            "parts": [{ "text": prompt }]
        }]
    });

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
    })
}

async fn call_anthropic_claude(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
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
    let req_body = json!({
        "model": provider.config.model,
        "max_tokens": 4096,
        "messages": [
            { "role": "user", "content": prompt }
        ]
    });

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

    use super::{AnthropicAuthMode, anthropic_auth_mode, anthropic_messages_url};
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
