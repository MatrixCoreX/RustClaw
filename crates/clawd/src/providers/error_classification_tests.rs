use super::*;
use serde_json::json;

fn kind(name: &str, status: u16, value: Value) -> Option<ProviderErrorKind> {
    classify(
        super::super::error_rules::active(),
        name,
        "https://example.invalid",
        status,
        &value,
    )
    .map(|c| c.kind)
}

#[test]
fn every_catalog_code_has_a_string_and_numeric_fixture() {
    let rules = super::super::error_rules::active();
    for profile in &rules.profiles {
        let url = format!(
            "https://{}",
            profile
                .hosts
                .first()
                .map(String::as_str)
                .unwrap_or("example.invalid")
        );
        for rule in &profile.rules {
            for code in &rule.codes {
                let expected = ProviderErrorKind::from_str(&rule.class).unwrap();
                for wire in
                    std::iter::once(json!(code)).chain(code.parse::<u64>().ok().map(|n| json!(n)))
                {
                    let result =
                        classify(rules, "custom", &url, 400, &json!({"error":{"code":wire}}))
                            .unwrap();
                    assert_eq!(result.kind, expected, "profile={} code={code}", profile.id);
                }
            }
        }
    }
}

#[test]
fn minimax_2056_wrapped_in_anthropic_error_is_quota_not_rate() {
    assert_eq!(
        kind(
            "vendor-minimax",
            400,
            json!({"error":{"type":"invalid_request_error","message":"echoed input (2056)"}})
        ),
        Some(ProviderErrorKind::ProviderNonRetryableBusiness)
    );
    for message in [
        "已达到 Token Plan 用量上限 (2056)",
        "opaque (2056)",
        "unknown language (2056)  ",
    ] {
        assert_eq!(
            kind(
                "vendor-minimax",
                429,
                json!({"type":"error","error":{
                    "type":"rate_limit_error","message":message,"http_code":"429"
                }})
            ),
            Some(ProviderErrorKind::QuotaExhausted)
        );
    }
    for message in [
        "usage limit exceeded",
        "2056",
        "(2056) try again",
        "(20560)",
        "(２０５６)",
    ] {
        assert_eq!(
            kind(
                "vendor-minimax",
                429,
                json!({"error":{"type":"rate_limit_error","message":message}})
            ),
            Some(ProviderErrorKind::RateLimited)
        );
    }
    assert_eq!(
        kind(
            "vendor-minimax",
            429,
            json!({"error":{"code":1004,"message":"opaque (2056)"}})
        ),
        Some(ProviderErrorKind::ProviderNonRetryableBusiness)
    );
}

#[test]
fn identical_codes_do_not_cross_provider_boundaries() {
    assert_eq!(
        kind("vendor-minimax", 429, json!({"error":{"code":1001}})),
        Some(ProviderErrorKind::Timeout)
    );
    assert_eq!(
        kind("zhipu", 429, json!({"error":{"code":1001}})),
        Some(ProviderErrorKind::ProviderNonRetryableBusiness)
    );
    assert_eq!(
        kind("custom", 429, json!({"error":{"code":1001}})),
        Some(ProviderErrorKind::RateLimited)
    );
    assert_eq!(
        kind(
            "vendor-openai",
            429,
            json!({"error":{"code":"insufficient_quota"}})
        ),
        Some(ProviderErrorKind::QuotaExhausted)
    );
    assert_eq!(
        kind(
            "vendor-qwen",
            429,
            json!({"error":{"code":"insufficient_quota"}})
        ),
        Some(ProviderErrorKind::RateLimited)
    );
    assert_eq!(
        kind("custom", 429, json!({"error":{"message":"opaque (2056)"}})),
        Some(ProviderErrorKind::RateLimited)
    );
}

#[test]
fn success_envelopes_and_model_content_are_not_errors() {
    for value in [
        json!({"choices":[{"message":{"content":"{\"error\":{\"code\":\"2056\"}}"}}]}),
        json!({"base_resp":{"status_code":0},"choices":[{"message":{"content":"ok"}}]}),
        json!({"error":null,"choices":[]}),
        json!({"code":"insufficient_quota","choices":[]}),
        json!({"type":"message","content":[{"type":"text","text":"quota_exhausted"}]}),
    ] {
        assert_eq!(kind("vendor-minimax", 200, value), None);
    }
    assert_eq!(
        kind(
            "vendor-minimax",
            200,
            json!({"base_resp":{"status_code":2056}})
        ),
        Some(ProviderErrorKind::QuotaExhausted)
    );
    assert_eq!(
        kind(
            "vendor-minimax",
            200,
            json!({"base_resp":{"status_code":1234567}})
        ),
        Some(ProviderErrorKind::ProviderNonRetryableBusiness)
    );
    assert_eq!(
        kind("custom", 200, json!({"error":{"code":"new_unknown_code"}})),
        Some(ProviderErrorKind::ProviderNonRetryableBusiness)
    );
}

#[test]
fn http_fallback_preserves_unknown_provider_behavior_without_guessing() {
    for (status, expected) in [
        (400, ProviderErrorKind::ProviderNonRetryableBusiness),
        (401, ProviderErrorKind::ProviderNonRetryableBusiness),
        (402, ProviderErrorKind::QuotaExhausted),
        (403, ProviderErrorKind::ProviderNonRetryableBusiness),
        (408, ProviderErrorKind::Timeout),
        (413, ProviderErrorKind::ProviderNonRetryableBusiness),
        (421, ProviderErrorKind::ProviderNonRetryableBusiness),
        (429, ProviderErrorKind::RateLimited),
        (500, ProviderErrorKind::ProviderRetryableResponse),
        (503, ProviderErrorKind::ProviderRetryableResponse),
        (504, ProviderErrorKind::Timeout),
        (529, ProviderErrorKind::ProviderRetryableResponse),
    ] {
        assert_eq!(kind("custom", status, Value::Null), Some(expected));
    }
    assert_eq!(
        kind(
            "vendor-anthropic",
            429,
            json!({"error":{"type":"rate_limit_error"}})
        ),
        Some(ProviderErrorKind::RateLimited)
    );
}

#[test]
fn machine_code_takes_precedence_over_broad_type_and_http_status() {
    assert_eq!(
        kind("custom", 429, json!({"status_code":"quota_exhausted"})),
        Some(ProviderErrorKind::QuotaExhausted)
    );
    assert_eq!(
        kind("custom", 400, json!({"status":"context_length_exceeded"})),
        Some(ProviderErrorKind::ContextLengthExceeded)
    );
    assert_eq!(
        kind(
            "groq",
            498,
            json!({"error":{"type":"invalid_request_error"}})
        ),
        Some(ProviderErrorKind::ProviderRetryableResponse)
    );
    assert_eq!(
        kind(
            "custom",
            498,
            json!({"error":{"type":"invalid_request_error"}})
        ),
        Some(ProviderErrorKind::ProviderNonRetryableBusiness)
    );
    assert_eq!(
        kind(
            "vendor-openai",
            429,
            json!({"error":{"code":"context_length_exceeded","type":"rate_limit_error"}})
        ),
        Some(ProviderErrorKind::ContextLengthExceeded)
    );
    assert_eq!(
        kind(
            "vendor-google",
            429,
            json!({"error":{"code":429,"status":"RESOURCE_EXHAUSTED"}})
        ),
        Some(ProviderErrorKind::RateLimited)
    );
    assert_eq!(
        kind(
            "openrouter",
            200,
            json!({"error":{"code":402,"message":"opaque"}})
        ),
        Some(ProviderErrorKind::QuotaExhausted)
    );
    assert_eq!(
        kind("custom", 200, json!({"error":{"code":402}})),
        Some(ProviderErrorKind::ProviderNonRetryableBusiness)
    );
    assert_eq!(
        kind(
            "vendor-minimax",
            200,
            json!({"error":{"http_code":"429","message":"opaque (2056)"}})
        ),
        Some(ProviderErrorKind::QuotaExhausted)
    );
}
