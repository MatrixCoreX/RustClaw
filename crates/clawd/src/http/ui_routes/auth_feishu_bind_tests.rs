use super::*;
use claw_core::channel_ingress::{ChannelIngressAttachment, ChannelIngressEnvelope};
use claw_core::types::{ChannelKind, TaskKind};

fn test_state() -> AppState {
    AppState::test_default_with_fixture_provider().with_seeded_db_schema()
}

fn pending_request(
    external_chat_id: &str,
    idempotency_key: &str,
    attachment: Option<&str>,
) -> PendingChannelRequestStoreRequest {
    let mut ingress = ChannelIngressEnvelope::new(ChannelKind::Whatsapp, "whatsapp_web")
        .with_external_ids("participant@s.whatsapp.net", external_chat_id)
        .with_message_id("provider-message-1")
        .with_context_token("provider-context-1");
    if let Some(path) = attachment {
        ingress.attachments.push(ChannelIngressAttachment {
            kind: "image".to_string(),
            path: path.to_string(),
            mime_type: Some("image/jpeg".to_string()),
            size: Some(1),
        });
    }
    PendingChannelRequestStoreRequest {
        idempotency_key: idempotency_key.to_string(),
        expires_in_seconds: Some(120),
        request: claw_core::types::SubmitTaskRequest {
            user_id: None,
            chat_id: None,
            user_key: None,
            channel: Some(ChannelKind::Whatsapp),
            external_user_id: Some("participant@s.whatsapp.net".to_string()),
            external_chat_id: Some(external_chat_id.to_string()),
            ingress: Some(ingress),
            idempotency_key: None,
            kind: TaskKind::Ask,
            payload: json!({"text": "continue original request"}),
        },
    }
}

#[tokio::test]
async fn private_binding_resumes_original_group_request_exactly_once() {
    let state = test_state();
    let user_key = create_auth_key(&state, "user").expect("create auth key");
    store_pending_channel_request(
        &state,
        &pending_request(
            "group@g.us",
            "pending:whatsapp_web:provider-message-1",
            None,
        ),
    )
    .expect("store group request");
    let bind = BindChannelKeyRequest {
        channel: ChannelKind::Whatsapp,
        telegram_bot_name: None,
        external_user_id: Some("participant@s.whatsapp.net".to_string()),
        external_chat_id: Some("participant@s.whatsapp.net".to_string()),
        user_key,
    };

    let (status, Json(first)) = bind_channel_key(State(state.clone()), Json(bind.clone())).await;
    assert_eq!(status, StatusCode::OK);
    let first = first.data.expect("bind response");
    let first_resume = first.pending_resume.expect("pending resume");
    assert_eq!(first_resume.status, "submitted");
    assert_eq!(first_resume.external_chat_id.as_deref(), Some("group@g.us"));
    let first_task_id = first_resume.task_id.expect("resumed task id");

    let (status, Json(repeated)) = bind_channel_key(State(state.clone()), Json(bind)).await;
    assert_eq!(status, StatusCode::OK);
    let repeated_task_id = repeated
        .data
        .and_then(|data| data.pending_resume)
        .and_then(|resume| resume.task_id)
        .expect("repeated resumed task id");
    assert_eq!(first_task_id, repeated_task_id);

    let db = state.core.db.get().expect("main db");
    let (count, external_chat_id): (i64, String) = db
        .query_row(
            "SELECT COUNT(*), MAX(external_chat_id) FROM tasks WHERE idempotency_key = ?1",
            ["pending:whatsapp_web:provider-message-1"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read resumed task");
    assert_eq!(count, 1);
    assert_eq!(external_chat_id, "group@g.us");
    drop(db);

    let resolved = resolve_channel_binding_identity(
        &state,
        "whatsapp",
        Some("participant@s.whatsapp.net"),
        Some("group@g.us"),
    )
    .expect("resolve group binding");
    assert!(resolved.is_some(), "private binding should apply to original group participant");
}

#[tokio::test]
async fn missing_attachment_stops_resume_without_creating_task() {
    let state = test_state();
    let user_key = create_auth_key(&state, "user").expect("create auth key");
    store_pending_channel_request(
        &state,
        &pending_request(
            "participant@s.whatsapp.net",
            "pending:whatsapp_web:missing-attachment",
            Some("data/inbox/missing.jpg"),
        ),
    )
    .expect("store request");
    let (status, Json(response)) = bind_channel_key(
        State(state.clone()),
        Json(BindChannelKeyRequest {
            channel: ChannelKind::Whatsapp,
            telegram_bot_name: None,
            external_user_id: Some("participant@s.whatsapp.net".to_string()),
            external_chat_id: Some("participant@s.whatsapp.net".to_string()),
            user_key,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let resume = response
        .data
        .and_then(|data| data.pending_resume)
        .expect("stopped resume");
    assert_eq!(resume.status, "invalid");
    assert_eq!(
        resume.error_code.as_deref(),
        Some("pending_request_attachment_missing")
    );
    assert!(resume.task_id.is_none());
    let db = state.core.db.get().expect("main db");
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE idempotency_key = ?1",
            ["pending:whatsapp_web:missing-attachment"],
            |row| row.get(0),
        )
        .expect("count tasks");
    assert_eq!(count, 0);
}
