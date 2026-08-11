use super::*;
use serde_json::json;

fn task(payload: &Value) -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 1,
        task_id: "task-delivery-service".to_string(),
        user_id: 1,
        chat_id: 9,
        user_key: None,
        channel: "telegram".to_string(),
        external_user_id: Some("user-1".to_string()),
        external_chat_id: Some("9".to_string()),
        kind: "ask".to_string(),
        payload_json: payload.to_string(),
    }
}

fn payload() -> Value {
    json!({
        "channel": "telegram",
        "external_user_id": "user-1",
        "external_chat_id": "9",
        "channel_ingress": {
            "schema_version": 1,
            "channel": "telegram",
            "adapter": "telegram_bot",
            "external_user_id": "user-1",
            "external_chat_id": "9",
            "reply_target": {"kind": "chat", "external_id": "9"},
            "locale": "zh-CN"
        }
    })
}

#[test]
fn provider_failure_contract_propagates_machine_fields_without_response_body() {
    let provider_error =
        claw_core::channel_provider_error::ChannelProviderError::from_http_response(
            "telegram_bot",
            "send_text",
            429,
            r#"{"error":{"code":"rate_limit","message":"private provider prose"}}"#,
        );
    let encoded = provider_error.to_string();
    let (error_code, message_key, diagnostic_id, provider_error_code, retryable) =
        delivery_failure_fields(&encoded);

    assert_eq!(error_code, "channel.provider.rate_limited");
    assert_eq!(message_key, "channel.error.provider_rate_limited");
    assert_eq!(diagnostic_id, provider_error.diagnostic_id);
    assert_eq!(provider_error_code.as_deref(), Some("rate_limit"));
    assert!(retryable);
    assert!(!encoded.contains("private provider prose"));
}

#[test]
fn scheduled_envelope_pins_ingress_context_and_stable_idempotency() {
    let state = AppState::test_default_with_fixture_provider();
    let payload = payload();
    let task = task(&payload);

    let first = build_scheduled_delivery_envelope(&state, &task, &payload, "result")
        .expect("build first envelope");
    let second = build_scheduled_delivery_envelope(&state, &task, &payload, "result")
        .expect("build second envelope");

    assert_eq!(first, second);
    assert_eq!(first.channel, ChannelKind::Telegram);
    assert_eq!(first.adapter, "telegram_bot");
    assert_eq!(first.locale, "zh-CN");
    assert_eq!(first.reply_target, ChannelReplyTarget::chat("9"));
    assert_eq!(first.source, ChannelDeliverySource::ScheduledTask);
}

#[test]
fn proactive_notice_is_localized_from_machine_params() {
    use std::collections::BTreeMap;

    let state = AppState::test_default_with_fixture_provider();
    let payload = payload();
    let task = task(&payload);
    let mut notice = claw_core::channel_notice::ChannelNotice::status(
        "media_discovery.background.status",
        "channel.notice.media_discovery_background_progress",
        claw_core::channel_notice::ChannelNoticeSeverity::Info,
    );
    notice.params = BTreeMap::from([
        ("elapsed_minutes".to_string(), "15".to_string()),
        ("items".to_string(), "4".to_string()),
        ("videos".to_string(), "1".to_string()),
        ("images".to_string(), "3".to_string()),
        ("duplicates".to_string(), "0".to_string()),
        ("failures".to_string(), "0".to_string()),
    ]);

    let envelope =
        build_proactive_notice_envelope(&state, &task, &payload, "progress-1", notice.clone())
            .expect("build proactive notice");
    assert_eq!(envelope.source, ChannelDeliverySource::ProactiveNotice);
    assert_eq!(envelope.notice, Some(notice));
    assert!(envelope.text_segments[0].text.contains("已运行 15 分钟"));
    assert!(envelope.text_segments[0].text.contains("视频 1 条"));
    assert_eq!(
        envelope.history_disposition(),
        claw_core::channel_delivery::ChannelDeliveryHistoryDisposition::TransportOnly
    );
}

#[test]
fn daemon_content_projections_have_stable_distinct_idempotency_keys() {
    let state = AppState::test_default_with_fixture_provider();
    let payload = payload();
    let task = task(&payload);
    let mut keys = Vec::new();
    for content in [
        ChannelTaskDeliveryContent::Full,
        ChannelTaskDeliveryContent::TextOnly,
        ChannelTaskDeliveryContent::MediaOnly,
    ] {
        let first = build_daemon_delivery_envelope(
            &state,
            &task,
            &payload,
            "result",
            ChannelDeliverySource::ImmediateDaemon,
            content,
            None,
        )
        .expect("build daemon envelope");
        let second = build_daemon_delivery_envelope(
            &state,
            &task,
            &payload,
            "result",
            ChannelDeliverySource::BackgroundCompletion,
            content,
            None,
        )
        .expect("build replay envelope");
        assert_eq!(first.idempotency_key, second.idempotency_key);
        keys.push(first.idempotency_key);
    }
    assert_ne!(keys[0], keys[1]);
    assert_ne!(keys[0], keys[2]);
    assert_ne!(keys[1], keys[2]);
}

#[test]
fn feishu_and_lark_receipts_use_distinct_provider_namespaces() {
    let state = AppState::test_default_with_fixture_provider();
    let mut feishu_payload = payload();
    feishu_payload["channel"] = json!("feishu");
    feishu_payload["channel_ingress"]["channel"] = json!("feishu");
    feishu_payload["channel_ingress"]["adapter"] = json!("feishu_open_platform");
    let mut feishu_task = task(&feishu_payload);
    feishu_task.channel = "feishu".to_string();

    let mut lark_payload = feishu_payload.clone();
    lark_payload["channel"] = json!("lark");
    lark_payload["channel_ingress"]["channel"] = json!("lark");
    lark_payload["channel_ingress"]["adapter"] = json!("lark_open_platform");
    let mut lark_task = task(&lark_payload);
    lark_task.channel = "lark".to_string();

    let feishu = build_scheduled_delivery_envelope(&state, &feishu_task, &feishu_payload, "result")
        .expect("build Feishu envelope");
    let lark = build_scheduled_delivery_envelope(&state, &lark_task, &lark_payload, "result")
        .expect("build Lark envelope");

    assert!(feishu
        .idempotency_key
        .starts_with("feishu_open_platform_delivery:"));
    assert!(lark
        .idempotency_key
        .starts_with("lark_open_platform_delivery:"));
    assert_ne!(feishu.idempotency_key, lark.idempotency_key);
}

#[test]
fn accepted_receipt_keeps_provider_ids_under_the_stable_idempotency_key() {
    let state = AppState::test_default_with_fixture_provider();
    let payload = payload();
    let task = task(&payload);
    let envelope = build_scheduled_delivery_envelope(&state, &task, &payload, "result")
        .expect("build envelope");
    let receipt = accepted_delivery_receipt(
        &envelope,
        crate::channel_send::ChannelSendOutcome {
            provider_message_ids: vec!["101".to_string(), "102".to_string()],
        },
        123,
    );

    assert_eq!(receipt.idempotency_key, envelope.idempotency_key);
    assert_eq!(receipt.delivery_id, envelope.delivery_id);
    assert_eq!(receipt.provider_message_ids, vec!["101", "102"]);
    assert_eq!(receipt.parts.len(), 2);
    assert_eq!(receipt.parts[0].part_index, 0);
    assert_eq!(receipt.parts[0].provider_message_id.as_deref(), Some("101"));
    assert_eq!(receipt.parts[1].part_index, 1);
    assert_eq!(receipt.parts[1].provider_message_id.as_deref(), Some("102"));
    assert_eq!(receipt.status, ChannelDeliveryStatus::Accepted);
    receipt.validate().expect("valid receipt");
}

#[tokio::test]
async fn failed_dispatch_records_one_terminal_receipt_and_deduplicates_replay() {
    let state = AppState::test_default_with_fixture_provider();
    let payload = payload();
    let task = task(&payload);
    let envelope = build_scheduled_delivery_envelope(&state, &task, &payload, "result")
        .expect("build envelope");

    let first = deliver_task_envelope(&state, &task, &payload, &envelope)
        .await
        .expect("first delivery result");
    assert_eq!(first.status, ChannelDeliveryServiceStatus::Failed);
    assert!(!first.accepted());
    assert!(!first.delivered());
    assert_eq!(first.error_code.as_deref(), Some("channel.delivery.failed"));
    assert_eq!(
        first.message_key.as_deref(),
        Some("channel.error.delivery_failed")
    );
    assert!(!first.retryable);
    let first_receipt = first.receipt.expect("failed receipt");
    assert_eq!(
        first_receipt.error_code.as_deref(),
        Some("channel.delivery.failed")
    );
    assert_eq!(
        first_receipt.message_key.as_deref(),
        Some("channel.error.delivery_failed")
    );

    let second = deliver_task_envelope(&state, &task, &payload, &envelope)
        .await
        .expect("deduplicated delivery result");
    assert_eq!(second.status, ChannelDeliveryServiceStatus::Failed);
    assert_eq!(
        second.error_code.as_deref(),
        Some("channel.delivery.failed")
    );
    assert_eq!(
        second.message_key.as_deref(),
        Some("channel.error.delivery_failed")
    );
    assert_eq!(second.receipt, Some(first_receipt));

    let db = state.core.db.get().expect("db connection");
    let event_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM channel_delivery_receipt_events WHERE idempotency_key = ?1",
            rusqlite::params![envelope.idempotency_key],
            |row| row.get(0),
        )
        .expect("event count");
    assert_eq!(event_count, 1);
}
