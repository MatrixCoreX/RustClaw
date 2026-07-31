use super::*;

#[test]
fn http_error_extracts_only_machine_provider_code_and_never_copies_body() {
    let body = r#"{"error":{"code":190,"message":"secret-token user prose"}}"#;
    let error = ChannelProviderError::from_http_response("whatsapp_cloud", "send_text", 401, body);

    assert_eq!(
        error.failure_class,
        ChannelProviderFailureClass::Authentication
    );
    assert_eq!(error.provider_error_code.as_deref(), Some("190"));
    assert_eq!(error.error_code, "channel.provider.authentication");
    assert_eq!(error.message_key, "channel.error.provider_authentication");
    assert!(!error.retryable);

    let encoded = error.to_string();
    assert!(!encoded.contains("secret-token"));
    assert!(!encoded.contains("user prose"));
    assert_eq!(ChannelProviderError::decode(&encoded), Some(error));
}

#[test]
fn status_classification_has_stable_retry_semantics() {
    let cases = [
        (403, ChannelProviderFailureClass::PermissionDenied, false),
        (429, ChannelProviderFailureClass::RateLimited, true),
        (413, ChannelProviderFailureClass::PayloadRejected, false),
        (503, ChannelProviderFailureClass::ProviderUnavailable, true),
        (418, ChannelProviderFailureClass::Unknown, false),
    ];

    for (status, class, retryable) in cases {
        let error =
            ChannelProviderError::from_http_response("telegram_bot", "send_text", status, "{}");
        assert_eq!(error.failure_class, class);
        assert_eq!(error.retryable, retryable);
        assert!(error.is_valid());
    }
}

#[test]
fn prose_provider_codes_and_unstructured_bodies_are_discarded() {
    for body in [
        r#"{"code":"please retry with secret abc"}"#,
        "upstream returned private customer content",
    ] {
        let error =
            ChannelProviderError::from_http_response("wechat_ilink", "poll_events", 500, body);
        assert_eq!(error.provider_error_code, None);
        assert!(!error.to_string().contains(body));
    }
}

#[test]
fn transport_and_invalid_response_errors_remain_machine_only() {
    let transport = ChannelProviderError::from_transport(
        "lark_open_platform",
        "upload_media",
        ChannelProviderTransportKind::Timeout,
        "https://provider.invalid?token=private",
    );
    let invalid = ChannelProviderError::invalid_response(
        "feishu_open_platform",
        "auth_token",
        "private response fragment",
    );

    assert_eq!(
        transport.failure_class,
        ChannelProviderFailureClass::Transport
    );
    assert!(transport.retryable);
    assert!(!transport.to_string().contains("private"));
    assert_eq!(
        invalid.failure_class,
        ChannelProviderFailureClass::InvalidResponse
    );
    assert!(!invalid.retryable);
    assert!(!invalid.to_string().contains("private"));
}

#[test]
fn decoder_rejects_tampered_machine_contracts() {
    let error = ChannelProviderError::from_http_response(
        "telegram_bot",
        "send_text",
        429,
        r#"{"error_code":"rate_limit"}"#,
    );
    let mut value = serde_json::to_value(error).expect("encode error");
    value["retryable"] = serde_json::Value::Bool(false);
    let encoded = format!(
        "{}{}",
        CHANNEL_PROVIDER_ERROR_PREFIX,
        serde_json::to_string(&value).expect("serialize tampered error")
    );

    assert_eq!(ChannelProviderError::decode(&encoded), None);
}
