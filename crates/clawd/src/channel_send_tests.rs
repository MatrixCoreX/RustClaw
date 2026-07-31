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

#[test]
fn open_platform_provider_codes_override_legacy_http_status_for_each_region() {
    for source_adapter in ["feishu_open_platform", "lark_open_platform"] {
        let encoded = provider_http_error(
            source_adapter,
            "send_text",
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"code":230020,"msg":"private reply"}"#,
        );
        let decoded = ChannelProviderError::decode(&encoded).expect("machine provider error");
        assert_eq!(
            decoded.failure_class,
            claw_core::channel_provider_error::ChannelProviderFailureClass::RateLimited
        );
        assert_eq!(decoded.provider_error_code.as_deref(), Some("230020"));
        assert!(decoded.retryable);
        assert!(!encoded.contains("private reply"));
    }
}

#[test]
fn open_platform_monthly_quota_is_terminal_and_redacted() {
    let encoded = provider_http_error(
        "lark_open_platform",
        "send_text",
        reqwest::StatusCode::BAD_REQUEST,
        r#"{"code":99991403,"msg":"private reply"}"#,
    );
    let decoded = ChannelProviderError::decode(&encoded).expect("machine provider error");
    assert_eq!(
        decoded.failure_class,
        claw_core::channel_provider_error::ChannelProviderFailureClass::QuotaExhausted
    );
    assert!(!decoded.retryable);
    assert!(!encoded.contains("private reply"));
}

#[tokio::test]
async fn whatsapp_web_scheduled_send_is_blocked_by_default_with_machine_error() {
    let state = AppState::test_default_with_fixture_provider();
    assert!(!state.channels.whatsapp_web_allow_proactive_send);

    let encoded = send_whatsapp_web_bridge_text_message(
        &state,
        "recipient@s.whatsapp.net",
        "scheduled result",
        ChannelDeliverySource::ScheduledTask,
    )
    .await
    .expect_err("experimental proactive send must be opt-in");
    let error = ChannelProviderError::decode(&encoded).expect("machine provider error");
    assert_eq!(
        error.failure_class,
        claw_core::channel_provider_error::ChannelProviderFailureClass::PermissionDenied
    );
    assert_eq!(
        error.provider_error_code.as_deref(),
        Some("proactive_send_disabled")
    );
    assert!(!error.retryable);
}
