use super::*;

#[test]
fn telegram_success_response_projects_stable_provider_message_id() {
    let body = r#"{"ok":true,"result":{"message_id":12345,"text":"private reply"}}"#;
    assert_eq!(
        telegram_message_id("send_text", body).as_deref(),
        Ok("12345")
    );
}

#[test]
fn telegram_success_without_message_id_is_a_redacted_invalid_response() {
    let body = r#"{"ok":true,"result":{"text":"private reply"}}"#;
    let error = telegram_message_id("send_text", body).expect_err("missing id must fail");
    let decoded = ChannelProviderError::decode(&error).expect("machine provider error");
    assert_eq!(
        decoded.failure_class,
        claw_core::channel_provider_error::ChannelProviderFailureClass::InvalidResponse
    );
    assert!(!error.contains("private reply"));
}

#[test]
fn telegram_http_rate_limit_keeps_retry_after_without_response_prose() {
    let body = r#"{"ok":false,"error_code":429,"description":"private reply","parameters":{"retry_after":11}}"#;
    let encoded = provider_http_error(
        "telegram_bot",
        "send_text",
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        body,
    );
    let decoded = ChannelProviderError::decode(&encoded).expect("machine provider error");
    assert_eq!(decoded.retry_after_seconds, Some(11));
    assert!(decoded.retryable);
    assert!(!encoded.contains("private reply"));
}
