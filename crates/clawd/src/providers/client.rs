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
use serde::{Deserialize, Serialize};
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
const LLM_RATE_LIMIT_RETRY_TIMES_ENV: &str = "RUSTCLAW_LLM_RATE_LIMIT_RETRY_TIMES";
const DEFAULT_LLM_RATE_LIMIT_RETRY_TIMES: usize = 4;
const MAX_LLM_RATE_LIMIT_RETRY_TIMES: usize = 8;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    pub(crate) attempts: usize,
    pub(crate) retryable_error_count: usize,
    pub(crate) last_retry_error_kind: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderError {
    pub(crate) retryable: bool,
    pub(crate) kind: ProviderErrorKind,
    pub(crate) message: String,
    pub(crate) request_payload: Value,
    pub(crate) raw_response: Option<String>,
    pub(crate) usage: Option<LlmUsageSnapshot>,
    pub(crate) attempts: usize,
    pub(crate) retryable_error_count: usize,
    breaker_impact: BreakerImpact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BreakerImpact {
    /// 基础设施层失败：网络抖动 / 超时 / 5xx，应该累计到 provider breaker。
    Failure,
    /// provider 已经正常返回了一个可解析的 HTTP 响应，说明链路是通的；
    /// 即使业务上失败（4xx/429/安全拦截/格式异常），也不该继续把 breaker 往 Open 推。
    Healthy,
    /// 本地或配置层错误，不足以说明 provider 健康或故障。
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderErrorKind {
    Timeout,
    TransportRetryable,
    ProviderRetryableResponse,
    RateLimited,
    QuotaExhausted,
    ProviderNonRetryableBusiness,
    LocalNonRetryable,
}

impl ProviderErrorKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::TransportRetryable => "transport_retryable",
            Self::ProviderRetryableResponse => "provider_retryable_response",
            Self::RateLimited => "rate_limited",
            Self::QuotaExhausted => "quota_exhausted",
            Self::ProviderNonRetryableBusiness => "provider_non_retryable_business",
            Self::LocalNonRetryable => "local_non_retryable",
        }
    }

    pub(crate) fn background_wait_seconds(self) -> Option<u64> {
        match self {
            Self::QuotaExhausted => Some(3 * 60 * 60),
            Self::RateLimited => Some(60),
            Self::Timeout | Self::TransportRetryable | Self::ProviderRetryableResponse => Some(30),
            Self::ProviderNonRetryableBusiness | Self::LocalNonRetryable => None,
        }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl ProviderError {
    pub(crate) fn is_rate_limited(&self) -> bool {
        self.kind == ProviderErrorKind::RateLimited
    }

    pub(crate) fn background_wait_seconds(&self) -> Option<u64> {
        self.kind.background_wait_seconds()
    }

    pub(super) fn timeout(message: String, request_payload: Value) -> Self {
        Self {
            retryable: true,
            kind: ProviderErrorKind::Timeout,
            message,
            request_payload,
            raw_response: None,
            usage: None,
            attempts: 1,
            retryable_error_count: 0,
            breaker_impact: BreakerImpact::Failure,
        }
    }

    pub(super) fn retryable(message: String, request_payload: Value) -> Self {
        Self {
            retryable: true,
            kind: ProviderErrorKind::TransportRetryable,
            message,
            request_payload,
            raw_response: None,
            usage: None,
            attempts: 1,
            retryable_error_count: 0,
            breaker_impact: BreakerImpact::Failure,
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
            kind: ProviderErrorKind::ProviderRetryableResponse,
            message,
            request_payload,
            raw_response: Some(raw_response),
            usage,
            attempts: 1,
            retryable_error_count: 0,
            breaker_impact: BreakerImpact::Failure,
        }
    }

    pub(super) fn non_retryable(message: String, request_payload: Value) -> Self {
        Self {
            retryable: false,
            kind: ProviderErrorKind::LocalNonRetryable,
            message,
            request_payload,
            raw_response: None,
            usage: None,
            attempts: 1,
            retryable_error_count: 0,
            breaker_impact: BreakerImpact::Neutral,
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
            kind: ProviderErrorKind::ProviderNonRetryableBusiness,
            message,
            request_payload,
            raw_response: Some(raw_response),
            usage,
            attempts: 1,
            retryable_error_count: 0,
            breaker_impact: BreakerImpact::Healthy,
        }
    }

    pub(super) fn rate_limited_with_response(
        message: String,
        request_payload: Value,
        raw_response: String,
        usage: Option<LlmUsageSnapshot>,
    ) -> Self {
        Self {
            retryable: true,
            kind: ProviderErrorKind::RateLimited,
            message,
            request_payload,
            raw_response: Some(raw_response),
            usage,
            attempts: 1,
            retryable_error_count: 0,
            breaker_impact: BreakerImpact::Healthy,
        }
    }

    pub(super) fn quota_exhausted_with_response(
        message: String,
        request_payload: Value,
        raw_response: String,
        usage: Option<LlmUsageSnapshot>,
    ) -> Self {
        Self {
            retryable: false,
            kind: ProviderErrorKind::QuotaExhausted,
            message,
            request_payload,
            raw_response: Some(raw_response),
            usage,
            attempts: 1,
            retryable_error_count: 0,
            breaker_impact: BreakerImpact::Healthy,
        }
    }

    pub(crate) fn with_retry_metadata(
        mut self,
        attempts: usize,
        retryable_error_count: usize,
    ) -> Self {
        self.attempts = attempts.max(1);
        self.retryable_error_count = retryable_error_count;
        self
    }

    pub(crate) fn should_trip_breaker(&self) -> bool {
        self.breaker_impact == BreakerImpact::Failure
    }

    pub(crate) fn should_reset_breaker(&self) -> bool {
        self.breaker_impact == BreakerImpact::Healthy
    }

    pub(crate) fn observability_kind(&self) -> &'static str {
        self.kind.as_str()
    }
}

impl LlmProviderResponse {
    pub(crate) fn with_retry_metadata(
        mut self,
        attempts: usize,
        retryable_error_count: usize,
        last_retry_error_kind: Option<&'static str>,
    ) -> Self {
        self.attempts = attempts.max(1);
        self.retryable_error_count = retryable_error_count;
        self.last_retry_error_kind = last_retry_error_kind;
        self
    }
}

pub(crate) fn is_quota_exhausted_response(body_text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(body_text) else {
        return false;
    };
    const QUOTA_CODES: &[&str] = &[
        "account_quota_exhausted",
        "billing_hard_limit_reached",
        "credit_balance_exhausted",
        "insufficient_quota",
        "quota_exceeded",
        "quota_exhausted",
        "usage_limit_exceeded",
    ];
    [
        "/error/code",
        "/error/type",
        "/code",
        "/type",
        "/status_code",
        "/base_resp/status_code",
    ]
    .iter()
    .filter_map(|pointer| value.pointer(pointer))
    .filter_map(Value::as_str)
    .map(str::trim)
    .any(|code| QUOTA_CODES.contains(&code))
}

/// Optional per-call generation hints (`temperature` / `max_tokens`).
///
/// 旧调用点（plan/normalizer 等）走默认 `ChatRequestHints::default()`：
/// 不主动设置 temperature/max_tokens，让 provider 走自己的默认值，与原行为一致。
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
    let mut retryable_error_count = 0usize;
    let mut last_retry_error_kind = None;

    loop {
        attempts += 1;
        match call_provider(provider.clone(), prompt, hints).await {
            Ok(output) => {
                return Ok(output.with_retry_metadata(
                    attempts,
                    retryable_error_count,
                    last_retry_error_kind,
                ))
            }
            Err(err) if err.retryable => {
                retryable_error_count += 1;
                last_retry_error_kind = Some(err.observability_kind());
                let retry_limit = retry_limit_for_provider_error(&err);
                if attempts > retry_limit {
                    return Err(err.with_retry_metadata(attempts, retryable_error_count));
                }
                let delay = retry_delay_for_provider_error(&err, attempts);
                tokio::time::sleep(delay).await;
            }
            Err(err) => return Err(err.with_retry_metadata(attempts, retryable_error_count)),
        }
    }
}

fn retry_limit_for_provider_error(err: &ProviderError) -> usize {
    retry_limit_for_provider_error_with_rate_limit_retries(err, configured_rate_limit_retry_times())
}

fn retry_limit_for_provider_error_with_rate_limit_retries(
    err: &ProviderError,
    rate_limit_retries: usize,
) -> usize {
    if err.is_rate_limited() {
        rate_limit_retries.min(MAX_LLM_RATE_LIMIT_RETRY_TIMES)
    } else {
        LLM_RETRY_TIMES
    }
}

fn configured_rate_limit_retry_times() -> usize {
    let raw = std::env::var(LLM_RATE_LIMIT_RETRY_TIMES_ENV).ok();
    effective_rate_limit_retry_times(raw.as_deref())
}

fn effective_rate_limit_retry_times(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.min(MAX_LLM_RATE_LIMIT_RETRY_TIMES))
        .unwrap_or(DEFAULT_LLM_RATE_LIMIT_RETRY_TIMES)
}

fn retry_delay_for_provider_error(err: &ProviderError, attempts: usize) -> Duration {
    if err.is_rate_limited() {
        return rate_limit_retry_delay(attempts);
    }
    Duration::from_millis(250 * attempts as u64)
}

fn rate_limit_retry_delay(attempts: usize) -> Duration {
    const SECONDS_BY_ATTEMPT: &[u64] = &[5, 15, 30, 60];
    let index = attempts.saturating_sub(1);
    let seconds = SECONDS_BY_ATTEMPT
        .get(index)
        .copied()
        .unwrap_or(*SECONDS_BY_ATTEMPT.last().unwrap_or(&60));
    Duration::from_secs(seconds)
}

/// Phase 2.3 完整版：LLM provider 抽象。
///
/// 每种线上协议实现这个 trait 一次，注册到 [`PROVIDER_IMPLS`] 静态数组里，
/// dispatcher [`call_provider`] 通过 `name()` 匹配分发。新接入一种协议时：
///   1. 在 `providers/` 下新建模块 `xxx.rs`，写 `pub(super) async fn call_xxx(..)`。
///   2. 加一个零字段 unit struct + impl `LlmProvider`。
///   3. 在 [`PROVIDER_IMPLS`] 数组追加引用。
///
/// 与之前纯 `match` 写法相比，这里多了一层 trait object 间接调用，但换来
/// 三个好处：
///   * 测试里可以 mock provider 实现而不用动 dispatcher。
///   * 协议列表 [`PROVIDER_IMPLS`] 一处可见，避免散在 dispatcher 内部。
///   * 未来要做"按协议聚合指标 / 按协议级别熔断策略"等横向逻辑时，可以在
///     trait 里加新方法集中实现，而不必改各个具体 fn。
///
/// 不引入 `async-trait` crate（避免新 crate 依赖），用 returning-boxed-future
/// 模式实现 trait object 安全的 async 方法。
pub(crate) type ProviderCallFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<LlmProviderResponse, ProviderError>> + Send>,
>;

pub(crate) trait LlmProvider: Send + Sync + 'static {
    /// 与 toml 里 `[[llm_providers]].type` / 内部 `LlmProviderConfig::provider_type`
    /// 完全一致的协议短名。dispatcher 用它做选择。
    fn name(&self) -> &'static str;

    /// 单次请求实现。`prompt` 与 `hints` 拷贝进 future 以满足 `'static` 约束
    /// （保留 `Arc<LlmProviderRuntime>` 走零拷贝）。
    fn call(
        &self,
        provider: Arc<LlmProviderRuntime>,
        prompt: String,
        hints: ChatRequestHints,
    ) -> ProviderCallFuture;
}

pub(crate) struct OpenAiCompatProvider;
pub(crate) struct GoogleGeminiProvider;
pub(crate) struct AnthropicClaudeProvider;

/// §7.5: fixture 回放 provider，零 HTTP，仅供 `cargo test` / nl-replay 使用。
/// 实际行为定义在 [`super::fixture_replay::FixtureReplayProvider`]，这里只
/// 把它纳入 [`PROVIDER_IMPLS`] 静态注册表，让生产 dispatcher 走正常路径。
pub(crate) use super::fixture_replay::FixtureReplayProvider;

impl LlmProvider for OpenAiCompatProvider {
    fn name(&self) -> &'static str {
        "openai_compat"
    }
    fn call(
        &self,
        provider: Arc<LlmProviderRuntime>,
        prompt: String,
        hints: ChatRequestHints,
    ) -> ProviderCallFuture {
        Box::pin(async move {
            super::openai_compat::call_openai_compat(provider, &prompt, &hints).await
        })
    }
}

impl LlmProvider for GoogleGeminiProvider {
    fn name(&self) -> &'static str {
        "google_gemini"
    }
    fn call(
        &self,
        provider: Arc<LlmProviderRuntime>,
        prompt: String,
        hints: ChatRequestHints,
    ) -> ProviderCallFuture {
        Box::pin(async move {
            super::google_gemini::call_google_gemini(provider, &prompt, &hints).await
        })
    }
}

impl LlmProvider for AnthropicClaudeProvider {
    fn name(&self) -> &'static str {
        "anthropic_claude"
    }
    fn call(
        &self,
        provider: Arc<LlmProviderRuntime>,
        prompt: String,
        hints: ChatRequestHints,
    ) -> ProviderCallFuture {
        Box::pin(async move {
            super::anthropic_claude::call_anthropic_claude(provider, &prompt, &hints).await
        })
    }
}

/// 注册的 provider 列表。dispatcher 按顺序遍历找 `name()` 匹配项。
/// 加新协议时往这里追加一项。
pub(crate) static PROVIDER_IMPLS: &[&dyn LlmProvider] = &[
    &OpenAiCompatProvider,
    &GoogleGeminiProvider,
    &AnthropicClaudeProvider,
    &FixtureReplayProvider,
];

/// Provider 协议 dispatcher：通过 [`LlmProvider::name`] 匹配 trait object 实现。
async fn call_provider(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
    hints: &ChatRequestHints,
) -> Result<LlmProviderResponse, ProviderError> {
    let provider_type = provider.config.provider_type.as_str();
    for impl_ref in PROVIDER_IMPLS {
        if impl_ref.name() == provider_type {
            let timeout_seconds = provider.config.timeout_seconds.max(1);
            return await_provider_call_with_timeout(
                provider_type,
                timeout_seconds,
                impl_ref.call(provider.clone(), prompt.to_string(), hints.clone()),
            )
            .await;
        }
    }
    Err(ProviderError::non_retryable(
        format!("unsupported provider type: {provider_type}"),
        Value::Null,
    ))
}

async fn await_provider_call_with_timeout(
    provider_type: &str,
    timeout_seconds: u64,
    call: ProviderCallFuture,
) -> Result<LlmProviderResponse, ProviderError> {
    match tokio::time::timeout(Duration::from_secs(timeout_seconds.max(1)), call).await {
        Ok(result) => result,
        Err(_) => Err(ProviderError::timeout(
            format!(
                "provider_call_timeout provider_type={provider_type} timeout_seconds={}",
                timeout_seconds.max(1)
            ),
            Value::Null,
        )),
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
