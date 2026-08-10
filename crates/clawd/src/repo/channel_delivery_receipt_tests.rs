use super::*;
use claw_core::channel_delivery::{
    ChannelConversationWindow, ChannelConversationWindowState, ChannelDeliveryEnvelope,
    ChannelDeliverySource, ChannelTextFormat, ChannelTextSegment,
    CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION, CHANNEL_DELIVERY_SCHEMA_VERSION,
};
use claw_core::channel_ingress::ChannelReplyTarget;
use claw_core::types::ChannelKind;
use r2d2_sqlite::SqliteConnectionManager;

fn pool() -> crate::db_init::DbPool {
    r2d2::Pool::builder()
        .max_size(1)
        .build(SqliteConnectionManager::memory())
        .expect("sqlite pool")
}

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
        message_key: None,
        diagnostic_id: None,
        provider_error_code: None,
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
fn dispatch_claim_prevents_concurrent_and_ambiguous_resends() {
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
            .expect("keep ambiguous dispatch quarantined"),
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
fn active_dispatch_renewal_prevents_large_media_resend_after_the_original_lease() {
    let db = Connection::open_in_memory().expect("open sqlite");
    ensure_channel_delivery_receipt_schema(&db).expect("ensure schema");
    let envelope = envelope();
    let lease_token = match claim_channel_delivery_dispatch_in_db(&db, &envelope, 100, 30)
        .expect("claim dispatch")
    {
        ClaimChannelDeliveryDispatchOutcome::Acquired { lease_token } => lease_token,
        other => panic!("unexpected claim: {other:?}"),
    };

    assert!(renew_channel_delivery_dispatch_in_db(
        &db,
        &envelope.idempotency_key,
        &lease_token,
        120,
        30,
    )
    .expect("renew active dispatch"));
    assert_eq!(
        claim_channel_delivery_dispatch_in_db(&db, &envelope, 135, 30)
            .expect("observe renewed dispatch"),
        ClaimChannelDeliveryDispatchOutcome::InProgress
    );

    let accepted = receipt(ChannelDeliveryStatus::Accepted, 140);
    record_channel_delivery_receipt_in_db(&db, &accepted).expect("record accepted receipt");
    complete_channel_delivery_dispatch_in_db(&db, &envelope.idempotency_key, &lease_token, 141)
        .expect("complete renewed dispatch");
}

#[test]
fn dispatch_renewal_rejects_a_non_owner_lease_token() {
    let db = Connection::open_in_memory().expect("open sqlite");
    ensure_channel_delivery_receipt_schema(&db).expect("ensure schema");
    let envelope = envelope();
    let _lease_token = match claim_channel_delivery_dispatch_in_db(&db, &envelope, 100, 30)
        .expect("claim dispatch")
    {
        ClaimChannelDeliveryDispatchOutcome::Acquired { lease_token } => lease_token,
        other => panic!("unexpected claim: {other:?}"),
    };

    assert!(!renew_channel_delivery_dispatch_in_db(
        &db,
        &envelope.idempotency_key,
        "different-owner",
        120,
        30,
    )
    .expect("reject non-owner renewal"));
    assert_eq!(
        claim_channel_delivery_dispatch_in_db(&db, &envelope, 131, 30)
            .expect("expired original dispatch"),
        ClaimChannelDeliveryDispatchOutcome::QueryRequired
    );
}

#[test]
fn retryable_failure_receipt_allows_a_new_dispatch_claim() {
    let db = Connection::open_in_memory().expect("open sqlite");
    ensure_channel_delivery_receipt_schema(&db).expect("ensure schema");
    let envelope = envelope();
    let lease_token = match claim_channel_delivery_dispatch_in_db(&db, &envelope, 100, 30)
        .expect("claim dispatch")
    {
        ClaimChannelDeliveryDispatchOutcome::Acquired { lease_token } => lease_token,
        other => panic!("unexpected claim: {other:?}"),
    };
    let mut failed = receipt(ChannelDeliveryStatus::Failed, 110);
    failed.error_code = Some("provider.rate_limited".to_string());
    failed.diagnostic_id = Some("diag:retryable".to_string());
    failed.retryable = true;
    record_channel_delivery_receipt_in_db(&db, &failed).expect("record retryable failure");
    complete_channel_delivery_dispatch_in_db(&db, &envelope.idempotency_key, &lease_token, 111)
        .expect("complete failed attempt");

    assert!(matches!(
        claim_channel_delivery_dispatch_in_db(&db, &envelope, 120, 30)
            .expect("claim retry attempt"),
        ClaimChannelDeliveryDispatchOutcome::Acquired { .. }
    ));
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

#[test]
fn whatsapp_window_is_persistent_monotonic_and_expires_after_twenty_four_hours() {
    let pool = pool();
    record_whatsapp_cloud_inbound(&pool, "phone-1", "user-1", 1_000).expect("record inbound");
    record_whatsapp_cloud_inbound(&pool, "phone-1", "user-1", 900).expect("ignore older inbound");

    let open =
        whatsapp_cloud_conversation_window(&pool, "phone-1", "user-1", 1_001).expect("open window");
    assert_eq!(open.state, ChannelConversationWindowState::Open);
    assert_eq!(
        open.expires_at_ts,
        Some(1_000 + claw_core::channel_whatsapp_cloud::WHATSAPP_CUSTOMER_SERVICE_WINDOW_SECONDS)
    );
    let closed = whatsapp_cloud_conversation_window(
        &pool,
        "phone-1",
        "user-1",
        open.expires_at_ts.expect("expiry") + 1,
    )
    .expect("closed window");
    assert_eq!(closed.state, ChannelConversationWindowState::Closed);
}

#[test]
fn whatsapp_wamid_statuses_advance_receipt_without_out_of_order_regression() {
    let pool = pool();
    let mut accepted = receipt(ChannelDeliveryStatus::Accepted, 10);
    accepted.channel = ChannelKind::Whatsapp;
    accepted.adapter = "whatsapp_cloud".to_string();
    accepted.provider_message_ids = vec!["wamid.fixture".to_string()];
    record_channel_delivery_receipt(&pool, &accepted).expect("record accepted");

    assert_eq!(
        record_whatsapp_cloud_provider_status(
            &pool,
            "wamid.fixture",
            WhatsappDeliveryEventStatus::Delivered,
            20,
            None,
        )
        .expect("record delivered"),
        RecordWhatsappProviderStatusOutcome::Updated
    );
    assert_eq!(
        record_whatsapp_cloud_provider_status(
            &pool,
            "wamid.fixture",
            WhatsappDeliveryEventStatus::Accepted,
            30,
            None,
        )
        .expect("ignore sent regression"),
        RecordWhatsappProviderStatusOutcome::Unchanged
    );
    record_whatsapp_cloud_provider_status(
        &pool,
        "wamid.fixture",
        WhatsappDeliveryEventStatus::Read,
        40,
        None,
    )
    .expect("record read");

    let db = pool.get().expect("db");
    let stored = load_channel_delivery_receipt_from_db(&db, &accepted.idempotency_key)
        .expect("load receipt")
        .expect("receipt");
    assert_eq!(stored.status, ChannelDeliveryStatus::Read);
    assert_eq!(stored.provider_message_ids, vec!["wamid.fixture"]);
}

#[test]
fn whatsapp_failed_status_retains_provider_code_and_diagnostic_id() {
    let pool = pool();
    let mut accepted = receipt(ChannelDeliveryStatus::Accepted, 10);
    accepted.channel = ChannelKind::Whatsapp;
    accepted.adapter = "whatsapp_cloud".to_string();
    accepted.provider_message_ids = vec!["wamid.failed".to_string()];
    record_channel_delivery_receipt(&pool, &accepted).expect("record accepted");
    record_whatsapp_cloud_provider_status(
        &pool,
        "wamid.failed",
        WhatsappDeliveryEventStatus::Failed,
        20,
        Some("131047"),
    )
    .expect("record failure");

    let db = pool.get().expect("db");
    let stored = load_channel_delivery_receipt_from_db(&db, &accepted.idempotency_key)
        .expect("load receipt")
        .expect("receipt");
    assert_eq!(stored.status, ChannelDeliveryStatus::Failed);
    assert_eq!(stored.provider_error_code.as_deref(), Some("131047"));
    assert!(stored.diagnostic_id.is_some());
}

#[test]
fn whatsapp_status_arriving_before_accepted_receipt_is_replayed() {
    let pool = pool();
    assert_eq!(
        record_whatsapp_cloud_provider_status(
            &pool,
            "wamid.race",
            WhatsappDeliveryEventStatus::Delivered,
            20,
            None,
        )
        .expect("store pending status"),
        RecordWhatsappProviderStatusOutcome::UnknownMessage
    );
    let mut accepted = receipt(ChannelDeliveryStatus::Accepted, 10);
    accepted.channel = ChannelKind::Whatsapp;
    accepted.adapter = "whatsapp_cloud".to_string();
    accepted.provider_message_ids = vec!["wamid.race".to_string()];
    record_channel_delivery_receipt(&pool, &accepted).expect("record and replay");

    let db = pool.get().expect("db");
    let stored = load_channel_delivery_receipt_from_db(&db, &accepted.idempotency_key)
        .expect("load receipt")
        .expect("receipt");
    assert_eq!(stored.status, ChannelDeliveryStatus::Delivered);
    let pending_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM whatsapp_cloud_pending_provider_statuses",
            [],
            |row| row.get(0),
        )
        .expect("pending count");
    assert_eq!(pending_count, 0);
}
