use super::*;
use claw_core::channel_ingress::{ChannelIngressAttachment, ChannelIngressEnvelope};
use claw_core::types::{ChannelKind, TaskKind};
use serde_json::json;

fn test_state() -> AppState {
    AppState::test_default_with_fixture_provider().with_seeded_db_schema()
}

fn pending_input(idempotency_key: &str, message_id: &str) -> PendingChannelRequestStoreRequest {
    let mut ingress = ChannelIngressEnvelope::new(ChannelKind::Telegram, "telegram_bot")
        .with_external_ids("user-7", "chat-9")
        .with_message_id(message_id)
        .with_context_token("context-11");
    ingress.attachments.push(ChannelIngressAttachment {
        kind: "image".to_string(),
        path: "data/inbox/image.jpg".to_string(),
        mime_type: Some("image/jpeg".to_string()),
        size: Some(12),
    });
    PendingChannelRequestStoreRequest {
        idempotency_key: idempotency_key.to_string(),
        expires_in_seconds: Some(120),
        request: SubmitTaskRequest {
            user_id: None,
            chat_id: None,
            user_key: None,
            channel: Some(ChannelKind::Telegram),
            external_user_id: Some("user-7".to_string()),
            external_chat_id: Some("chat-9".to_string()),
            ingress: Some(ingress),
            idempotency_key: None,
            kind: TaskKind::Ask,
            payload: json!({"text": "describe this"}),
        },
    }
}

#[test]
fn duplicate_store_reuses_pending_request_and_preserves_machine_context() {
    let state = test_state();
    let input = pending_input("telegram:message-1", "message-1");
    let first = store_pending_channel_request(&state, &input).expect("store pending request");
    let repeated = store_pending_channel_request(&state, &input).expect("repeat pending request");
    assert_eq!(first.pending_request_id, repeated.pending_request_id);
    assert_eq!(first.external_chat_id.as_deref(), Some("chat-9"));
    assert_eq!(first.context_token.as_deref(), Some("context-11"));

    let candidate =
        pending_channel_resume_candidate(&state, "telegram", Some("user-7"), Some("chat-9"))
            .expect("load pending")
            .expect("candidate");
    let request = candidate.request.expect("stored request");
    assert_eq!(
        request.idempotency_key.as_deref(),
        Some("telegram:message-1")
    );
    assert_eq!(
        request_attachment_paths(&request).collect::<Vec<_>>(),
        vec!["data/inbox/image.jpg"]
    );
}

#[test]
fn latest_request_supersedes_older_request_for_same_conversation() {
    let state = test_state();
    let first =
        store_pending_channel_request(&state, &pending_input("telegram:message-1", "message-1"))
            .expect("store first");
    let second =
        store_pending_channel_request(&state, &pending_input("telegram:message-2", "message-2"))
            .expect("store second");
    assert_ne!(first.pending_request_id, second.pending_request_id);

    let db = state.core.db.get().expect("main db");
    let first_status: (String, String) = db
        .query_row(
            "SELECT status, error_code FROM pending_channel_requests WHERE pending_request_id = ?1",
            [first.pending_request_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read superseded row");
    assert_eq!(first_status.0, "invalid");
    assert_eq!(first_status.1, "pending_request_superseded");
}

#[test]
fn expiry_and_finish_transitions_are_restart_safe_and_idempotent() {
    let state = test_state();
    let stored =
        store_pending_channel_request(&state, &pending_input("telegram:message-3", "message-3"))
            .expect("store pending");
    {
        let db = state.core.db.get().expect("main db");
        db.execute(
            "UPDATE pending_channel_requests SET expires_at = 0 WHERE pending_request_id = ?1",
            [stored.pending_request_id.to_string()],
        )
        .expect("expire pending");
    }
    let expired =
        pending_channel_resume_candidate(&state, "telegram", Some("user-7"), Some("chat-9"))
            .expect("load expired")
            .expect("expired candidate");
    assert_eq!(expired.status.status, "expired");
    assert_eq!(
        expired.status.error_code.as_deref(),
        Some("pending_request_expired")
    );
    assert!(expired.request.is_none());

    let live =
        store_pending_channel_request(&state, &pending_input("telegram:message-4", "message-4"))
            .expect("store live");
    let task_id = Uuid::new_v4();
    let submitted =
        finish_pending_channel_resume(&state, live.pending_request_id, Some(task_id), None)
            .expect("finish resume");
    let repeated =
        finish_pending_channel_resume(&state, live.pending_request_id, Some(Uuid::new_v4()), None)
            .expect("repeat finish");
    assert_eq!(submitted.task_id, Some(task_id));
    assert_eq!(repeated.task_id, Some(task_id));
}
