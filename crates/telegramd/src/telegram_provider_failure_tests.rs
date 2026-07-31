use super::*;
use std::time::Duration;

#[test]
fn typed_telegram_api_errors_map_without_description_matching() {
    let cases = [
        (
            RequestError::Api(ApiError::BotBlocked),
            ChannelProviderFailureClass::RecipientBlocked,
            "bot_blocked",
        ),
        (
            RequestError::Api(ApiError::ChatNotFound),
            ChannelProviderFailureClass::TargetNotFound,
            "chat_not_found",
        ),
        (
            RequestError::Api(ApiError::NotEnoughRightsToPostMessages),
            ChannelProviderFailureClass::PermissionDenied,
            "not_enough_rights_to_post",
        ),
    ];
    for (input, expected_class, expected_code) in cases {
        let error = telegram_request_error("send_text", &input);
        assert_eq!(error.failure_class, expected_class);
        assert_eq!(error.provider_error_code.as_deref(), Some(expected_code));
        assert!(!error.retryable);
        assert!(error.is_valid());
    }
}

#[test]
fn typed_retry_after_preserves_bounded_backoff() {
    let error = telegram_request_error(
        "send_text",
        &RequestError::RetryAfter(Duration::from_secs(23)),
    );
    assert_eq!(
        error.failure_class,
        ChannelProviderFailureClass::RateLimited
    );
    assert_eq!(error.retry_after_seconds, Some(23));
    assert!(error.retryable);
}

#[test]
fn invalid_json_remains_a_redacted_machine_failure() {
    let invalid_json = RequestError::InvalidJson {
        source: serde_json::from_str::<serde_json::Value>("{").unwrap_err(),
        raw: "private provider body".into(),
    };
    let error = telegram_request_error("send_text", &invalid_json);
    assert_eq!(
        error.failure_class,
        ChannelProviderFailureClass::InvalidResponse
    );
    assert!(!error.to_string().contains("private provider body"));
}
