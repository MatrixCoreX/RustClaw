use claw_core::channel_ingress::{
    ChannelIngressEnvelope, ChannelReplyTarget, ChannelReplyTargetKind,
};
use claw_core::types::{ChannelKind, SubmitTaskRequest, TaskKind};
use serde_json::json;
use uuid::Uuid;

use super::{
    build_channel_ingress_snapshot, build_submit_task_payload, hydrate_submit_task_from_ingress,
    insert_submitted_task,
};

fn request(ingress: Option<ChannelIngressEnvelope>) -> SubmitTaskRequest {
    SubmitTaskRequest {
        user_id: Some(7),
        chat_id: Some(8),
        user_key: None,
        channel: Some(ChannelKind::Wechat),
        external_user_id: Some("wx-user".to_string()),
        external_chat_id: Some("wx-chat".to_string()),
        ingress,
        idempotency_key: None,
        kind: TaskKind::Ask,
        payload: json!({"text": "hello"}),
    }
}

#[test]
fn ingress_hydration_rejects_conflicting_machine_identity() {
    let ingress = ChannelIngressEnvelope::new(ChannelKind::Wechat, "wechat_ilink")
        .with_external_ids("different-user", "wx-chat");
    let mut req = request(Some(ingress));

    assert_eq!(
        hydrate_submit_task_from_ingress(&mut req),
        Err("channel_ingress_external_user_conflict")
    );
}

#[test]
fn ingress_hydration_supports_envelope_only_channel_identifiers() {
    let ingress = ChannelIngressEnvelope::new(ChannelKind::Feishu, "feishu_open_platform")
        .with_external_ids("open-id", "chat-id");
    let mut req = request(Some(ingress));
    req.channel = None;
    req.external_user_id = None;
    req.external_chat_id = None;

    hydrate_submit_task_from_ingress(&mut req).expect("hydrate ingress request");

    assert_eq!(req.channel, Some(ChannelKind::Feishu));
    assert_eq!(req.external_user_id.as_deref(), Some("open-id"));
    assert_eq!(req.external_chat_id.as_deref(), Some("chat-id"));
}

#[test]
fn snapshot_freezes_bound_context_and_materialized_attachments() {
    let requested = ChannelIngressEnvelope::new(ChannelKind::Wechat, "wechat_ilink")
        .with_external_ids("wx-user", "wx-chat")
        .with_message_id("message-1")
        .with_reply_target(ChannelReplyTarget::user("wx-user"))
        .with_locale("zh-CN")
        .with_context_token("context-1");
    let payload = json!({
        "text": "inspect image",
        "attachments": [{
            "kind": "image",
            "path": "data/inbox/image.jpg",
            "mime_type": "image/jpeg",
            "size": 42
        }]
    });

    let snapshot = build_channel_ingress_snapshot(
        Some(&requested),
        ChannelKind::Wechat,
        70,
        80,
        Some("wx-user"),
        Some("wx-chat"),
        &payload,
    );

    assert_eq!(snapshot.bound_user_id, Some(70));
    assert_eq!(snapshot.conversation_chat_id, Some(80));
    assert_eq!(snapshot.message_id.as_deref(), Some("message-1"));
    assert_eq!(snapshot.context_token.as_deref(), Some("context-1"));
    assert_eq!(
        snapshot.reply_target.expect("reply target").kind,
        ChannelReplyTargetKind::User
    );
    assert_eq!(snapshot.attachments.len(), 1);
    assert_eq!(snapshot.attachments[0].path, "data/inbox/image.jpg");
}

#[test]
fn legacy_request_gets_canonical_channel_defaults() {
    let snapshot = build_channel_ingress_snapshot(
        None,
        ChannelKind::Telegram,
        70,
        80,
        Some("telegram-user"),
        Some("telegram-chat"),
        &json!({"message_id": 123}),
    );

    assert_eq!(snapshot.adapter, "telegram_bot");
    assert_eq!(snapshot.message_id.as_deref(), Some("123"));
    assert_eq!(
        snapshot.reply_target.expect("reply target").kind,
        ChannelReplyTargetKind::Chat
    );
}

#[test]
fn payload_and_task_row_keep_the_same_platform_message_id() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("main db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    crate::ensure_schedule_schema(&db).expect("schedule schema");
    crate::ensure_memory_schema(&db).expect("memory schema");
    crate::ensure_channel_schema(&db).expect("channel schema");
    crate::ensure_task_lease_schema(&db).expect("task lease schema");
    crate::ensure_key_auth_schema(&db).expect("auth schema");
    crate::repo::ensure_principal_ownership_schema(&db).expect("principal ownership schema");
    drop(db);

    let snapshot = build_channel_ingress_snapshot(
        None,
        ChannelKind::Feishu,
        70,
        80,
        Some("open-id"),
        Some("chat-id"),
        &json!({"message_id": "platform-message-1"}),
    );
    let payload = build_submit_task_payload(
        json!({"text": "hello"}),
        snapshot,
        ChannelKind::Feishu,
        Some("open-id"),
        Some("chat-id"),
        None,
        "default",
        None,
        "call-1",
    );
    let task_id = Uuid::new_v4();
    insert_submitted_task(
        &state,
        &task_id,
        70,
        80,
        None,
        ChannelKind::Feishu,
        Some("open-id"),
        Some("chat-id"),
        Some("platform-message-1"),
        None,
        "ask",
        &payload.to_string(),
    )
    .expect("insert task with message id");

    let db = state.core.db.get().expect("main db");
    let (message_id, payload_json): (String, String) = db
        .query_row(
            "SELECT message_id, payload_json FROM tasks WHERE task_id = ?1",
            [task_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read submitted task");
    let stored_payload: serde_json::Value =
        serde_json::from_str(&payload_json).expect("decode stored payload");

    assert_eq!(message_id, "platform-message-1");
    assert_eq!(
        stored_payload["channel_ingress"]["message_id"],
        "platform-message-1"
    );
    assert_eq!(
        stored_payload["channel_ingress"]["adapter"],
        "feishu_open_platform"
    );
}

#[test]
fn submitted_task_idempotency_key_allows_exactly_one_task() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    let first = insert_submitted_task(
        &state,
        &first_id,
        70,
        80,
        Some("rk-test"),
        ChannelKind::Feishu,
        Some("open-id"),
        Some("chat-id"),
        Some("platform-message-2"),
        Some("pending:feishu:platform-message-2"),
        "ask",
        &json!({"text": "hello"}).to_string(),
    )
    .expect("insert first task");
    let second = insert_submitted_task(
        &state,
        &second_id,
        70,
        80,
        Some("rk-test"),
        ChannelKind::Feishu,
        Some("open-id"),
        Some("chat-id"),
        Some("platform-message-2"),
        Some("pending:feishu:platform-message-2"),
        "ask",
        &json!({"text": "hello again"}).to_string(),
    )
    .expect("reuse first task");
    assert_eq!(first, (first_id, true));
    assert_eq!(second, (first_id, false));
    let db = state.core.db.get().expect("main db");
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE idempotency_key = ?1",
            ["pending:feishu:platform-message-2"],
            |row| row.get(0),
        )
        .expect("count idempotent tasks");
    assert_eq!(count, 1);
}
