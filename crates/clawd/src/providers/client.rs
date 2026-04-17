//! Provider 调用入口（dispatcher） + 协议中性的公共类型与 HTTP 客户端构造。
//!
//! Phase 2.3：每种协议的实际请求 / 解析逻辑放在独立子模块：
//! - [`super::openai_compat`]
//! - [`super::google_gemini`]
//! - [`super::anthropic_claude`]
//!
//! 加新协议时：写一个新模块 + 在 [`call_provider`] 加一行分支即可。

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

use crate::{LlmProviderRuntime, LLM_RETRY_TIMES};

/// 连接池里每个 host 最大闲置连接数。LLM 调用高峰期常见 2-4 家 provider 同时打，
/// 90 足以吃掉短时突发，同时避免闲置太多句柄。
const LLM_POOL_MAX_IDLE_PER_HOST: usize = 90;

/// 闲置连接最长保留时间；超过则关闭，防止 provider 端静默断开后下次调用仍复用
/// 死连接导致一次额外 retry。
const LLM_POOL_IDLE_TIMEOUT_SECS: u64 = 90;

/// TCP keep-alive 心跳间隔；对长 idle 的 LLM 流式/长文本请求能降低 NAT
/// 或中间网关静默断链导致的"首次失败 + 再 retry"开销。
const LLM_TCP_KEEPALIVE_SECS: u64 = 60;

/// 所有 LLM provider 共享的 `reqwest::Client` 构造器：
/// 统一设置超时 + 连接池 + TCP keep-alive，避免 `Client::new()` 裸建导致
/// 每次调用重新握手。`timeout_seconds` 为单次请求墙钟上限。
///
/// 注意：当前 reqwest 未启用 `http2` feature，这里只做 TCP 层 keep-alive；
/// 若后续启用 HTTP/2 feature 可在此补 `http2_keep_alive_interval`。
pub(crate) fn build_llm_http_client(timeout_seconds: u64) -> reqwest::Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .pool_max_idle_per_host(LLM_POOL_MAX_IDLE_PER_HOST)
        .pool_idle_timeout(Duration::from_secs(LLM_POOL_IDLE_TIMEOUT_SECS))
        .tcp_keepalive(Duration::from_secs(LLM_TCP_KEEPALIVE_SECS))
        .build()
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
    pub(super) fn retryable(message: String, request_payload: Value) -> Self {
        Self {
            retryable: true,
            message,
            request_payload,
            raw_response: None,
            usage: None,
        }
    }

    pub(super) fn retryable_with_response(
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

    pub(super) fn non_retryable(message: String, request_payload: Value) -> Self {
        Self {
            retryable: false,
            message,
            request_payload,
            raw_response: None,
            usage: None,
        }
    }

    pub(super) fn non_retryable_with_response(
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

/// Phase 2.2: chat 风格调用的可选 hint（temperature / max_tokens）。
///
/// 旧调用点（plan/normalizer 等）走默认 `ChatRequestHints::default()`：
/// 不主动设置 temperature/max_tokens，让 provider 走自己的默认值，与原行为一致。
///
/// chat 这种"闲聊/创作"调用走显式 hints：温度调低、按文本长度选 max_tokens
/// 上限，等价 chat-skill 子进程里原来的逻辑。
#[derive(Debug, Clone, Default)]
pub(crate) struct ChatRequestHints {
    pub(crate) temperature: Option<f64>,
    pub(crate) max_tokens: Option<u64>,
}

pub(crate) async fn call_provider_with_retry(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<LlmProviderResponse, ProviderError> {
    call_provider_with_retry_with_hints(provider, prompt, &ChatRequestHints::default()).await
}

pub(crate) async fn call_provider_with_retry_with_hints(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
    hints: &ChatRequestHints,
) -> Result<LlmProviderResponse, ProviderError> {
    let mut attempts = 0usize;

    loop {
        attempts += 1;
        match call_provider(provider.clone(), prompt, hints).await {
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

/// Provider 协议 dispatcher：仅做协议匹配，每个分支调到独立模块。
///
/// 加新协议时：在 `providers/` 下新建模块 + 在这里加一行分支。
async fn call_provider(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
    hints: &ChatRequestHints,
) -> Result<LlmProviderResponse, ProviderError> {
    match provider.config.provider_type.as_str() {
        "openai_compat" => super::openai_compat::call_openai_compat(provider, prompt, hints).await,
        "google_gemini" => super::google_gemini::call_google_gemini(provider, prompt, hints).await,
        "anthropic_claude" => {
            super::anthropic_claude::call_anthropic_claude(provider, prompt, hints).await
        }
        other => Err(ProviderError::non_retryable(
            format!("unsupported provider type: {other}"),
            Value::Null,
        )),
    }
}
