//! Provider adapter boundary: wire errors -> existing runtime failure classes.
use serde_json::Value;

use super::{
    client::ProviderErrorKind,
    error_rules::{ErrorRules, Profile},
};

#[derive(Debug)]
pub(super) struct Classification<'a> {
    pub kind: ProviderErrorKind,
    pub profile: &'a str,
    pub rule: &'a str,
    pub status: u16,
}

pub(super) fn classify<'a>(
    rules: &'a ErrorRules,
    name: &str,
    base_url: &str,
    http_status: u16,
    value: &Value,
) -> Option<Classification<'a>> {
    let profile = rules.profile(name, base_url);
    let envelope = error_envelope(value, profile, &rules.success_codes);
    if (200..300).contains(&http_status) && !envelope {
        return None;
    }
    // Some compatible gateways report their real HTTP status inside a 200/SSE error.
    // Numeric vendor business codes are NEVER implicitly treated as HTTP statuses.
    let status = if (200..300).contains(&http_status) && envelope {
        ["/error/http_code", "/error/status_code"]
            .into_iter()
            .chain(profile.http_status_paths.iter().map(String::as_str))
            .find_map(|path| {
                scalar(value.pointer(path)?)
                    .and_then(|s| s.parse::<u16>().ok())
                    .filter(|s| (400..600).contains(s))
            })
            .unwrap_or(http_status)
    } else {
        http_status
    };
    let codes = profile
        .code_paths
        .iter()
        .chain(rules.code_paths.iter())
        .filter_map(|path| value.pointer(path).and_then(scalar))
        .collect::<Vec<_>>();
    let suffix_codes = profile
        .message_code_paths
        .iter()
        .filter(|_| profile.message_code_http_statuses.contains(&status))
        .filter_map(|path| value.pointer(path).and_then(Value::as_str))
        .filter_map(numeric_suffix)
        .collect::<Vec<_>>();
    // A concrete business code outranks broad error types. The provider-scoped
    // suffix bridge (MiniMax's Anthropic wrapper) is checked before those types.
    for candidate in
        std::iter::once(profile).chain((profile.id != "default").then(|| rules.by_id("default")))
    {
        for code in codes.iter().chain(suffix_codes.iter()) {
            if let Some(rule) = candidate.rules.iter().find(|r| {
                (r.http_statuses.is_empty() || r.http_statuses.contains(&status))
                    && r.codes.contains(code)
            }) {
                return Some(Classification {
                    kind: ProviderErrorKind::from_str(&rule.class).unwrap(),
                    profile: &candidate.id,
                    rule: &rule.id,
                    status,
                });
            }
        }
        if let Some(rule) = candidate
            .rules
            .iter()
            .find(|r| r.codes.is_empty() && r.http_statuses.contains(&status))
        {
            return Some(Classification {
                kind: ProviderErrorKind::from_str(&rule.class).unwrap(),
                profile: &candidate.id,
                rule: &rule.id,
                status,
            });
        }
    }
    let (kind, rule) = match status {
        402 => (ProviderErrorKind::QuotaExhausted, "http_payment_required"),
        408 | 504 => (ProviderErrorKind::Timeout, "http_timeout"),
        429 => (ProviderErrorKind::RateLimited, "http_rate_limit"),
        500..=599 => (ProviderErrorKind::ProviderRetryableResponse, "http_service"),
        _ => (
            ProviderErrorKind::ProviderNonRetryableBusiness,
            "unknown_business",
        ),
    };
    Some(Classification {
        kind,
        profile: &profile.id,
        rule,
        status,
    })
}

fn error_envelope(value: &Value, profile: &Profile, success_codes: &[String]) -> bool {
    let success_codes = profile.success_codes.as_deref().unwrap_or(success_codes);
    value.get("error").is_some_and(|v| match v {
        Value::Null | Value::Bool(false) => false,
        Value::String(s) => !s.is_empty(),
        _ => true,
    }) || value.get("type").and_then(Value::as_str) == Some("error")
        || profile.failure_code_paths.iter().any(|path| {
            value
                .pointer(path)
                .and_then(scalar)
                .is_some_and(|code| !code.is_empty() && !success_codes.contains(&code))
        })
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().into()),
        Value::Number(value) if value.is_i64() || value.is_u64() => Some(value.to_string()),
        _ => None,
    }
}

fn numeric_suffix(message: &str) -> Option<String> {
    let prefix = message.trim_end().strip_suffix(')')?;
    let (_, code) = prefix.rsplit_once('(')?;
    (!code.is_empty() && code.bytes().all(|b| b.is_ascii_digit())).then(|| code.into())
}

#[cfg(test)]
#[path = "error_classification_tests.rs"]
mod tests;
