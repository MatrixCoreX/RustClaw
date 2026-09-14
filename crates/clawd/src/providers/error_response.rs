use reqwest::{header::HeaderMap, StatusCode};
use serde_json::Value;

use super::{
    client::{ProviderError, ProviderErrorKind},
    error_classification::classify,
    error_rules,
};
use crate::LlmProviderRuntime;

pub(super) fn response_error(
    provider: &LlmProviderRuntime,
    status: StatusCode,
    headers: &HeaderMap,
    body: &str,
    request: &Value,
) -> Option<ProviderError> {
    let rules = error_rules::active();
    let value = serde_json::from_str(body).unwrap_or(Value::Null);
    let found = classify(
        rules,
        &provider.config.name,
        &provider.config.base_url,
        status.as_u16(),
        &value,
    )?;
    let message = format!(
        "provider_error:http_{}:{}:{}:{}",
        found.status,
        found.profile,
        found.rule,
        found.kind.as_str()
    );
    tracing::warn!(provider = %provider.config.name, revision = %rules.revision,
        profile = found.profile, rule = found.rule, http_status = status.as_u16(),
        effective_status = found.status, failure_class = found.kind.as_str(),
        "provider_error_classified");
    // Raw body stays in model I/O evidence, never in the user-visible error string.
    let raw = super::output::provider_safe_raw_response(body);
    let usage = super::openai_usage_snapshot(&value)
        .or_else(|| super::anthropic_usage_snapshot(&value))
        .or_else(|| super::gemini_usage_snapshot(&value));
    let mut error = match found.kind {
        ProviderErrorKind::QuotaExhausted => {
            ProviderError::quota_exhausted_with_response(message, request.clone(), raw, usage)
        }
        ProviderErrorKind::RateLimited => {
            ProviderError::rate_limited_with_response(message, request.clone(), raw, usage)
        }
        ProviderErrorKind::ContextLengthExceeded => {
            ProviderError::context_length_exceeded_with_response(
                message,
                request.clone(),
                raw,
                usage,
            )
        }
        ProviderErrorKind::Timeout | ProviderErrorKind::ProviderRetryableResponse => {
            ProviderError::retryable_with_response(message, request.clone(), raw, usage)
        }
        _ => ProviderError::non_retryable_with_response(message, request.clone(), raw, usage),
    };
    error.kind = found.kind;
    error.retry_after_seconds = retry_after(headers, &value, chrono::Utc::now());
    Some(error)
}

fn retry_after(
    headers: &HeaderMap,
    value: &Value,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<u64> {
    let header = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            seconds(v).or_else(|| {
                chrono::DateTime::parse_from_rfc2822(v)
                    .ok()
                    .map(|date| (date.timestamp() - now.timestamp()).max(0) as u64)
            })
        });
    let rpc = value
        .pointer("/error/details")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|v| {
            v.get("@type").and_then(Value::as_str)
                == Some("type.googleapis.com/google.rpc.RetryInfo")
        })
        .filter_map(|v| v.get("retryDelay").and_then(Value::as_str))
        .filter_map(|v| v.strip_suffix('s').and_then(seconds))
        .max();
    header
        .into_iter()
        .chain(rpc)
        .filter(|s| {
            i64::try_from(*s)
                .ok()
                .and_then(chrono::TimeDelta::try_seconds)
                .and_then(|delay| now.checked_add_signed(delay))
                .is_some()
        })
        .max()
}

fn seconds(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let value = raw.parse::<f64>().ok()?;
    (value.is_finite() && value >= 0.0 && value < i64::MAX as f64).then(|| value.ceil() as u64)
}

#[cfg(test)]
#[path = "error_response_tests.rs"]
mod tests;
