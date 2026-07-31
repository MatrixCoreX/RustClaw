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
    assert!(first.error_text.is_some());
    let first_receipt = first.receipt.expect("failed receipt");
    assert_eq!(
        first_receipt.error_code.as_deref(),
        Some("channel.send_failed")
    );

    let second = deliver_task_envelope(&state, &task, &payload, &envelope)
        .await
        .expect("deduplicated delivery result");
    assert_eq!(second.status, ChannelDeliveryServiceStatus::Failed);
    assert!(second.error_text.is_none());
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
