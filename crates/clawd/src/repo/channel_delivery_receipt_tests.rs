use super::*;
use claw_core::channel_delivery::{
    ChannelConversationWindow, ChannelConversationWindowState, ChannelDeliveryEnvelope,
    ChannelDeliverySource, ChannelTextFormat, ChannelTextSegment,
    CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION, CHANNEL_DELIVERY_SCHEMA_VERSION,
};
use claw_core::channel_ingress::ChannelReplyTarget;
use claw_core::types::ChannelKind;

fn receipt(status: ChannelDeliveryStatus, updated_at_ts: u64) -> ChannelDeliveryReceipt {
    let terminal_success = matches!(
        status,
        ChannelDeliveryStatus::Delivered | ChannelDeliveryStatus::Read
    );
    ChannelDeliveryReceipt {
        schema_version: CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION,
        delivery_id: "delivery:task-1:final".to_string(),
        idempotency_key: "wechat:peer-1:task-1:final".to_string(),
        channel: ChannelKind::Wechat,
        adapter: "wechat_ilink".to_string(),
        status,
        provider_message_ids: terminal_success
            .then(|| vec!["provider-1".to_string()])
            .unwrap_or_default(),
        parts: Vec::new(),
        error_code: None,
        diagnostic_id: None,
        retryable: false,
        updated_at_ts,
    }
}

fn envelope() -> ChannelDeliveryEnvelope {
    ChannelDeliveryEnvelope {
        schema_version: CHANNEL_DELIVERY_SCHEMA_VERSION,
        delivery_id: "delivery:task-1:final".to_string(),
        task_id: Some("task-1".to_string()),
        source: ChannelDeliverySource::ScheduledTask,
        channel: ChannelKind::Wechat,
        adapter: "wechat_ilink".to_string(),
        reply_target: ChannelReplyTarget::user("peer-1"),
        locale: "zh-CN".to_string(),
        conversation_window: ChannelConversationWindow {
            state: ChannelConversationWindowState::Open,
            expires_at_ts: None,
            context_token: Some("context-1".to_string()),
        },
        idempotency_key: "wechat:peer-1:task-1:final".to_string(),
        text_segments: vec![ChannelTextSegment {
            text: "result".to_string(),
            format: ChannelTextFormat::Plain,
        }],
        artifacts: Vec::new(),
        previews: Vec::new(),
        notice: None,
    }
}

#[test]
fn receipt_store_is_idempotent_and_preserves_event_history() {
    let db = Connection::open_in_memory().expect("open sqlite");
    db.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys");
    ensure_channel_delivery_receipt_schema(&db).expect("ensure schema");

    let accepted = receipt(ChannelDeliveryStatus::Accepted, 10);
    assert_eq!(
        record_channel_delivery_receipt_in_db(&db, &accepted).expect("insert accepted"),
        RecordChannelDeliveryReceiptOutcome::Inserted
    );
    assert_eq!(
        record_channel_delivery_receipt_in_db(&db, &accepted).expect("repeat accepted"),
        RecordChannelDeliveryReceiptOutcome::Unchanged
    );

    let delivered = receipt(ChannelDeliveryStatus::Delivered, 20);
    assert_eq!(
        record_channel_delivery_receipt_in_db(&db, &delivered).expect("record delivered"),
        RecordChannelDeliveryReceiptOutcome::Updated
    );
    let read = receipt(ChannelDeliveryStatus::Read, 30);
    record_channel_delivery_receipt_in_db(&db, &read).expect("record read");

    let stored = load_channel_delivery_receipt_from_db(&db, &read.idempotency_key)
        .expect("load receipt")
        .expect("stored receipt");
    assert_eq!(stored.status, ChannelDeliveryStatus::Read);
    let event_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM channel_delivery_receipt_events WHERE idempotency_key = ?1",
            params![read.idempotency_key],
            |row| row.get(0),
        )
        .expect("event count");
    assert_eq!(event_count, 3);
}

#[test]
fn receipt_store_rejects_terminal_regression_and_identity_conflict() {
    let db = Connection::open_in_memory().expect("open sqlite");
    ensure_channel_delivery_receipt_schema(&db).expect("ensure schema");
    let delivered = receipt(ChannelDeliveryStatus::Delivered, 20);
    record_channel_delivery_receipt_in_db(&db, &delivered).expect("record delivered");

    let accepted = receipt(ChannelDeliveryStatus::Accepted, 30);
    let regression = record_channel_delivery_receipt_in_db(&db, &accepted)
        .expect_err("delivered may not regress to accepted");
    assert!(regression
        .to_string()
        .contains("channel_delivery_receipt_transition_invalid"));

    let mut conflict = receipt(ChannelDeliveryStatus::Read, 40);
    conflict.delivery_id = "delivery:other".to_string();
    let conflict_error = record_channel_delivery_receipt_in_db(&db, &conflict)
        .expect_err("identity conflict must fail");
    assert!(conflict_error
        .to_string()
        .contains("channel_delivery_receipt_identity_conflict"));
}

#[test]
fn retryable_failure_can_start_a_new_accepted_attempt_without_losing_history() {
    let db = Connection::open_in_memory().expect("open sqlite");
    ensure_channel_delivery_receipt_schema(&db).expect("ensure schema");
    let mut failed = receipt(ChannelDeliveryStatus::Failed, 10);
    failed.error_code = Some("provider.rate_limited".to_string());
    failed.diagnostic_id = Some("diag:1".to_string());
    failed.retryable = true;
    record_channel_delivery_receipt_in_db(&db, &failed).expect("record retryable failure");

    let accepted = receipt(ChannelDeliveryStatus::Accepted, 20);
    record_channel_delivery_receipt_in_db(&db, &accepted).expect("start retry attempt");
    let event_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM channel_delivery_receipt_events",
            [],
            |row| row.get(0),
        )
        .expect("event count");
    assert_eq!(event_count, 2);
}

#[test]
fn dispatch_claim_prevents_concurrent_and_post_crash_resends() {
    let db = Connection::open_in_memory().expect("open sqlite");
    ensure_channel_delivery_receipt_schema(&db).expect("ensure schema");
    let envelope = envelope();

    let first = claim_channel_delivery_dispatch_in_db(&db, &envelope, 100, 30)
        .expect("claim first dispatch");
    let lease_token = match first {
        ClaimChannelDeliveryDispatchOutcome::Acquired { lease_token } => lease_token,
        other => panic!("unexpected first claim: {other:?}"),
    };
    assert_eq!(
        claim_channel_delivery_dispatch_in_db(&db, &envelope, 110, 30)
            .expect("observe active claim"),
        ClaimChannelDeliveryDispatchOutcome::InProgress
    );
    assert_eq!(
        claim_channel_delivery_dispatch_in_db(&db, &envelope, 131, 30)
            .expect("observe expired ambiguous claim"),
        ClaimChannelDeliveryDispatchOutcome::QueryRequired
    );
    assert_eq!(
        claim_channel_delivery_dispatch_in_db(&db, &envelope, 200, 30)
            .expect("query remains required"),
        ClaimChannelDeliveryDispatchOutcome::QueryRequired
    );
    let missing_receipt =
        complete_channel_delivery_dispatch_in_db(&db, &envelope.idempotency_key, &lease_token, 140)
            .expect_err("completion requires a receipt");
    assert!(missing_receipt
        .to_string()
        .contains("channel_delivery_dispatch_receipt_required"));
}

#[test]
fn completed_dispatch_returns_existing_receipt_without_resend() {
    let db = Connection::open_in_memory().expect("open sqlite");
    ensure_channel_delivery_receipt_schema(&db).expect("ensure schema");
    let envelope = envelope();
    let lease_token = match claim_channel_delivery_dispatch_in_db(&db, &envelope, 100, 30)
        .expect("claim dispatch")
    {
        ClaimChannelDeliveryDispatchOutcome::Acquired { lease_token } => lease_token,
        other => panic!("unexpected claim: {other:?}"),
    };
    let accepted = receipt(ChannelDeliveryStatus::Accepted, 110);
    record_channel_delivery_receipt_in_db(&db, &accepted).expect("record accepted receipt");
    complete_channel_delivery_dispatch_in_db(&db, &envelope.idempotency_key, &lease_token, 111)
        .expect("complete dispatch");

    assert_eq!(
        claim_channel_delivery_dispatch_in_db(&db, &envelope, 120, 30)
            .expect("load existing receipt"),
        ClaimChannelDeliveryDispatchOutcome::ExistingReceipt(accepted)
    );
}
