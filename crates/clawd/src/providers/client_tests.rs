use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::{is_quota_exhausted_response, BreakerImpact, ProviderError, PROVIDER_IMPLS};
use serde_json::Value;
use std::future::pending;

#[test]
fn provider_impls_names_are_unique_and_cover_known_protocols() {
    let names: Vec<&'static str> = PROVIDER_IMPLS.iter().map(|p| p.name()).collect();
    let unique: HashSet<&'static str> = names.iter().copied().collect();
    assert_eq!(
        names.len(),
        unique.len(),
        "duplicate provider names in PROVIDER_IMPLS: {names:?}"
    );
    for required in [
        "openai_compat",
        "google_gemini",
        "anthropic_claude",
        // §7.5: fixture_replay 是 cargo test 的回放后端，必须始终在表里，
        // 否则 intent_to_finalize_replay 静默失活。
        "fixture_replay",
    ] {
        assert!(
            unique.contains(required),
            "PROVIDER_IMPLS missing required protocol {required}; got {names:?}"
        );
    }
}

#[test]
fn provider_error_marks_breaker_impact_by_failure_class() {
    let retryable = ProviderError::timeout("opaque timeout detail".to_string(), Value::Null);
    assert!(retryable.should_trip_breaker());
    assert!(!retryable.should_reset_breaker());
    assert_eq!(retryable.observability_kind(), "timeout");

    let rate_limited = ProviderError::rate_limited_with_response(
        "http 429".to_string(),
        Value::Null,
        "{}".to_string(),
        None,
    );
    assert!(!rate_limited.should_trip_breaker());
    assert!(rate_limited.should_reset_breaker());
    assert!(rate_limited.is_rate_limited());
    assert_eq!(rate_limited.observability_kind(), "rate_limited");

    let quota = ProviderError::quota_exhausted_with_response(
        "http 429 rate_limit_error usage limit exceeded".to_string(),
        Value::Null,
        "{}".to_string(),
        None,
    );
    assert!(!quota.is_rate_limited());
    assert_eq!(quota.observability_kind(), "quota_exhausted");

    let business = ProviderError::non_retryable_with_response(
        "http 400".to_string(),
        Value::Null,
        "{}".to_string(),
        None,
    );
    assert!(!business.should_trip_breaker());
    assert!(business.should_reset_breaker());
    assert_eq!(
        business.observability_kind(),
        "provider_non_retryable_business"
    );

    let local = ProviderError::non_retryable("unsupported".to_string(), Value::Null);
    assert!(!local.should_trip_breaker());
    assert!(!local.should_reset_breaker());
    assert_eq!(local.observability_kind(), "local_non_retryable");

    assert_eq!(retryable.breaker_impact, BreakerImpact::Failure);
    assert_eq!(rate_limited.breaker_impact, BreakerImpact::Healthy);
    assert_eq!(local.breaker_impact, BreakerImpact::Neutral);
}

#[test]
fn quota_exhausted_detector_uses_machine_fields_only() {
    assert!(is_quota_exhausted_response(
        "{\"error\":{\"code\":\"insufficient_quota\"}}"
    ));
    assert!(is_quota_exhausted_response(
        "{\"base_resp\":{\"status_code\":\"quota_exhausted\"}}"
    ));
    assert!(!is_quota_exhausted_response(
        "{\"error\":{\"message\":\"usage limit exceeded (2056)\"}}"
    ));
}

#[test]
fn provider_error_kind_does_not_parse_misleading_message_text() {
    let local = ProviderError::non_retryable(
        "timeout 429 quota exceeded recharge".to_string(),
        Value::Null,
    );

    assert_eq!(local.observability_kind(), "local_non_retryable");
    assert_eq!(local.background_wait_seconds(), None);
}

#[test]
fn rate_limit_retry_policy_uses_longer_structured_backoff() {
    let rate_limited = ProviderError::rate_limited_with_response(
        "http 429".to_string(),
        Value::Null,
        "{}".to_string(),
        None,
    );
    let timeout = ProviderError::timeout("opaque timeout detail".to_string(), Value::Null);

    assert_eq!(
        super::retry_limit_for_provider_error_with_rate_limit_retries(&rate_limited, 4),
        4
    );
    assert_eq!(
        super::retry_limit_for_provider_error_with_rate_limit_retries(&rate_limited, 99),
        super::MAX_LLM_RATE_LIMIT_RETRY_TIMES
    );
    assert_eq!(
        super::retry_limit_for_provider_error_with_rate_limit_retries(&timeout, 4),
        crate::LLM_RETRY_TIMES
    );

    assert_eq!(
        super::retry_delay_for_provider_error(&rate_limited, 1),
        std::time::Duration::from_secs(5)
    );
    assert_eq!(
        super::retry_delay_for_provider_error(&rate_limited, 4),
        std::time::Duration::from_secs(60)
    );
    assert_eq!(
        super::retry_delay_for_provider_error(&timeout, 2),
        std::time::Duration::from_millis(500)
    );
}

#[test]
fn rate_limit_retry_times_env_parser_is_bounded() {
    assert_eq!(super::effective_rate_limit_retry_times(None), 4);
    assert_eq!(super::effective_rate_limit_retry_times(Some("6")), 6);
    assert_eq!(
        super::effective_rate_limit_retry_times(Some("999")),
        super::MAX_LLM_RATE_LIMIT_RETRY_TIMES
    );
    assert_eq!(super::effective_rate_limit_retry_times(Some("bad")), 4);
}

#[test]
fn provider_retry_metadata_is_attached_to_results() {
    let response = super::LlmProviderResponse {
        text: "{}".to_string(),
        request_payload: Value::Null,
        raw_response: "{}".to_string(),
        usage: None,
        attempts: 1,
        retryable_error_count: 0,
        last_retry_error_kind: None,
    }
    .with_retry_metadata(3, 2, Some("timeout"));
    assert_eq!(response.attempts, 3);
    assert_eq!(response.retryable_error_count, 2);
    assert_eq!(response.last_retry_error_kind, Some("timeout"));

    let error = ProviderError::timeout("opaque timeout detail".to_string(), Value::Null)
        .with_retry_metadata(4, 4);
    assert_eq!(error.attempts, 4);
    assert_eq!(error.retryable_error_count, 4);
}

#[tokio::test]
async fn provider_call_future_is_bounded_by_dispatch_timeout() {
    let err = super::await_provider_call_with_timeout(
        "fixture_replay",
        1,
        Box::pin(async {
            pending::<Result<super::LlmProviderResponse, super::ProviderError>>().await
        }),
    )
    .await
    .expect_err("pending provider call should time out");

    assert!(err.retryable);
    assert_eq!(err.observability_kind(), "timeout");
    assert!(err.message.contains("provider_call_timeout"));
    assert!(err.message.contains("timeout_seconds=1"));
}

#[tokio::test]
async fn model_turn_timeout_cancels_the_inflight_future() {
    struct DropSignal(Arc<AtomicBool>);
    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_from_future = cancelled.clone();
    let err = super::await_model_turn_call_with_timeout(
        "openai_compat",
        1,
        Box::pin(async move {
            let _drop_signal = DropSignal(cancelled_from_future);
            pending::<Result<super::ModelTurnProviderResponse, super::ProviderError>>().await
        }),
    )
    .await
    .expect_err("pending model turn should time out");

    assert!(cancelled.load(Ordering::SeqCst));
    assert!(err.retryable);
    assert_eq!(err.observability_kind(), "timeout");
    assert!(err.message.contains("provider_model_turn_timeout"));
}
