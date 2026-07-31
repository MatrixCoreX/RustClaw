use super::*;

#[test]
fn decodes_wamids_from_send_response() {
    let ids = decode_message_ids(
        "send_text",
        r#"{"messages":[{"id":"wamid.first"},{"id":"wamid.second"}]}"#,
    )
    .expect("valid send response");
    assert_eq!(ids, vec!["wamid.first", "wamid.second"]);
}

#[test]
fn rejects_send_response_without_wamid() {
    let error =
        decode_message_ids("send_text", r#"{"messages":[]}"#).expect_err("message id is mandatory");
    assert_eq!(
        error.failure_class,
        ChannelProviderFailureClass::InvalidResponse
    );
}

#[test]
fn customer_service_window_is_exactly_twenty_four_hours() {
    let inbound = 1_000;
    assert!(customer_service_window_is_open(
        inbound,
        inbound + WHATSAPP_CUSTOMER_SERVICE_WINDOW_SECONDS
    ));
    assert!(!customer_service_window_is_open(
        inbound,
        inbound + WHATSAPP_CUSTOMER_SERVICE_WINDOW_SECONDS + 1
    ));
}

#[test]
fn template_policy_requires_explicit_machine_tokens() {
    assert_eq!(
        WhatsappTemplatePolicy::from_config("task_ready", "zh_CN"),
        Some(WhatsappTemplatePolicy {
            name: "task_ready".to_string(),
            language: "zh_CN".to_string(),
        })
    );
    assert!(WhatsappTemplatePolicy::from_config("", "zh_CN").is_none());
    assert!(WhatsappTemplatePolicy::from_config("chosen by model", "zh_CN").is_none());
}

#[test]
fn parses_terminal_webhook_status_without_human_diagnostics() {
    let payload: WhatsappWebhookPayload = serde_json::from_str(
        r#"{"entry":[{"changes":[{"value":{"statuses":[{"id":"wamid.x","status":"failed","timestamp":"123","recipient_id":"1","errors":[{"code":131047,"title":"ignored"}]}]}}]}]}"#,
    )
    .expect("webhook payload");
    let status = payload.statuses().next().expect("one status");
    assert_eq!(
        status.delivery_status(),
        Some(WhatsappDeliveryEventStatus::Failed)
    );
    assert_eq!(status.timestamp, Some(123));
    assert_eq!(status.provider_error_code().as_deref(), Some("131047"));
}

#[test]
fn classifies_known_whatsapp_provider_codes() {
    let error = provider_error_from_response(
        "send_text",
        400,
        r#"{"error":{"code":131047,"message":"not retained"}}"#,
    );
    assert_eq!(
        error.failure_class,
        ChannelProviderFailureClass::PayloadRejected
    );
    assert_eq!(error.provider_error_code.as_deref(), Some("131047"));
    assert!(!error.retryable);
}

#[test]
fn accepted_daemon_event_derives_stable_receipt_identity() {
    let event = WhatsappAcceptedDeliveryEvent {
        schema_version: WHATSAPP_ACCEPTED_DELIVERY_EVENT_SCHEMA_VERSION,
        task_id: "123e4567-e89b-12d3-a456-426614174000".to_string(),
        response_digest: "a".repeat(64),
        provider_message_ids: vec!["wamid.fixture".to_string()],
        accepted_at_ts: 123,
    };
    assert!(event.validate());
    assert_eq!(
        event.delivery_id(),
        "delivery:123e4567-e89b-12d3-a456-426614174000:whatsapp-daemon:aaaaaaaaaaaaaaaa"
    );
    assert_eq!(event.idempotency_key(), event.idempotency_key());
}
