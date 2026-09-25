use axum::http::HeaderValue;
use claw_core::conversation_input::{ConversationInputContent, ConversationInputErrorCode};
use claw_core::types::ChannelKind;

use super::*;

#[test]
fn text_content_preserves_order_and_language() {
    let text = text_content(&[
        ConversationInputContent::Text {
            text: "先保留原结果。".to_string(),
        },
        ConversationInputContent::Text {
            text: "Then change only the format.".to_string(),
        },
    ])
    .expect("text content");
    assert_eq!(text, "先保留原结果。\nThen change only the format.");
}

#[test]
fn attachment_is_not_silently_accepted_before_preparation_is_connected() {
    let error = text_content(&[ConversationInputContent::Attachment {
        attachment_id: "artifact:fixture".to_string(),
        media_type: Some("application/pdf".to_string()),
        display_name: Some("fixture.pdf".to_string()),
    }])
    .expect_err("attachment path not connected yet");
    assert_eq!(error, "conversation_input_attachment_submission_not_ready");
}

#[test]
fn channel_parser_accepts_only_registered_channel_tokens() {
    assert_eq!(parse_channel("ui"), Some(ChannelKind::Ui));
    assert_eq!(parse_channel("wechat"), Some(ChannelKind::Wechat));
    assert_eq!(parse_channel("UI"), None);
    assert_eq!(parse_channel("unknown"), None);
}

#[test]
fn structured_errors_have_stable_machine_codes() {
    assert_eq!(
        error_code(ConversationInputErrorCode::IdempotencyConflict),
        "conversation_input_idempotency_conflict"
    );
    assert_eq!(
        error_code(ConversationInputErrorCode::TargetConflict),
        "conversation_input_target_conflict"
    );
    assert_eq!(
        error_code(ConversationInputErrorCode::DatabaseFailed),
        "conversation_input_database_failed"
    );
}

fn submission(client_message_id: &str, text: &str) -> ConversationInputSubmission {
    ConversationInputSubmission {
        schema_version: 1,
        client_message_id: client_message_id.to_string(),
        scope: ConversationInputScopeRef {
            conversation_id: "conversation-http-1".to_string(),
            agent_id: "main".to_string(),
            channel: "ui".to_string(),
            channel_account_id: "browser-session".to_string(),
        },
        content: vec![ConversationInputContent::Text {
            text: text.to_string(),
        }],
        delivery_mode: ConversationInputDeliveryMode::Auto,
        expected_task_id: None,
        expected_instruction_revision: None,
        source: claw_core::conversation_input::ConversationInputSource::default(),
    }
}

fn authenticated_state() -> (crate::AppState, HeaderMap, String) {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let key = crate::repo::auth::create_auth_key(&state, "admin").expect("create auth key");
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        HeaderValue::from_str(&key).expect("auth header"),
    );
    (state, headers, key)
}

#[tokio::test]
async fn first_input_creates_one_task_and_followup_targets_the_same_task() {
    let (state, headers, _) = authenticated_state();
    let (status, Json(first)) = accept_input(
        State(state.clone()),
        headers.clone(),
        Json(submission("message-1", "Create a short report.")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let first = first.data.expect("first receipt");
    let task_id = first.target_task_id.expect("created task binding");
    let active_model_turn = state
        .worker
        .model_turn_interrupt_token(&task_id.to_string());
    assert_eq!(
        first.disposition,
        claw_core::conversation_input::ConversationInputDisposition::Applied
    );

    let (status, Json(second)) = accept_input(
        State(state.clone()),
        headers.clone(),
        Json(submission(
            "message-2",
            "Keep the content and change only the format.",
        )),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let second = second.data.expect("follow-up receipt");
    assert_eq!(second.target_task_id, Some(task_id));
    assert_eq!(
        second.disposition,
        claw_core::conversation_input::ConversationInputDisposition::Pending
    );
    assert!(active_model_turn.is_cancelled());
    assert!(state
        .worker
        .model_turn_interrupt_token(&task_id.to_string())
        .is_cancelled());
    state
        .worker
        .acknowledge_model_turn_conversation_input(&task_id.to_string());
    assert!(!state
        .worker
        .model_turn_interrupt_token(&task_id.to_string())
        .is_cancelled());

    let database = state.core.db.get().expect("database");
    let task_count: i64 = database
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
        .expect("task count");
    assert_eq!(task_count, 1);
}

#[tokio::test]
async fn an_existing_owned_attachment_task_can_be_adopted_as_the_conversation_focus() {
    let (state, headers, key) = authenticated_state();
    let request = SubmitTaskRequest {
        user_id: None,
        chat_id: None,
        user_key: Some(key),
        channel: Some(ChannelKind::Ui),
        external_user_id: None,
        external_chat_id: None,
        ingress: None,
        idempotency_key: Some("attachment-task-1".to_string()),
        kind: TaskKind::Ask,
        payload: serde_json::json!({
            "text": "Inspect the attached report.",
            "conversation_id": "conversation-http-1",
            "agent_id": "main",
            "attachments": [{
                "name": "report.txt",
                "mime_type": "text/plain",
                "kind": "file",
                "size": 6,
                "base64": "data:text/plain;base64,cmVwb3J0"
            }]
        }),
    };
    let (task_status, Json(task_response)) =
        crate::submit_task(State(state.clone()), headers.clone(), Json(request)).await;
    assert_eq!(task_status, StatusCode::OK);
    let task_id = task_response.data.expect("submitted task").task_id;
    let active_model_turn = state
        .worker
        .model_turn_interrupt_token(&task_id.to_string());

    let mut initial = submission("attachment-message-1", "Inspect the attached report.");
    initial.expected_task_id = Some(task_id);
    let (status, Json(adopted)) =
        accept_input(State(state.clone()), headers.clone(), Json(initial)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let adopted = adopted.data.expect("adopted receipt");
    assert_eq!(adopted.target_task_id, Some(task_id));
    assert_eq!(
        adopted.disposition,
        claw_core::conversation_input::ConversationInputDisposition::Applied
    );
    assert_eq!(adopted.instruction_revision, 1);
    assert_eq!(adopted.execution_epoch, 1);
    assert!(active_model_turn.is_cancelled());

    let mut followup = submission(
        "attachment-message-2",
        "Keep the findings and prioritize unresolved items.",
    );
    followup.expected_task_id = Some(task_id);
    followup.expected_instruction_revision = Some(1);
    let (status, Json(followup)) =
        accept_input(State(state.clone()), headers, Json(followup)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let followup = followup.data.expect("follow-up receipt");
    assert_eq!(followup.target_task_id, Some(task_id));
    assert_eq!(
        followup.disposition,
        claw_core::conversation_input::ConversationInputDisposition::Pending
    );

    let database = state.core.db.get().expect("database");
    let task_count: i64 = database
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
        .expect("task count");
    assert_eq!(task_count, 1);
}

#[tokio::test]
async fn retry_replays_receipt_and_changed_payload_conflicts() {
    let (state, headers, _) = authenticated_state();
    let original = submission("message-1", "Create a short report.");
    let (first_status, Json(first)) = accept_input(
        State(state.clone()),
        headers.clone(),
        Json(original.clone()),
    )
    .await;
    assert_eq!(first_status, StatusCode::ACCEPTED);
    let first = first.data.expect("first receipt");
    let task_id = first.target_task_id.expect("first task binding");
    let active_model_turn = state
        .worker
        .model_turn_interrupt_token(&task_id.to_string());

    let (retry_status, Json(retry)) =
        accept_input(State(state.clone()), headers.clone(), Json(original)).await;
    assert_eq!(retry_status, StatusCode::ACCEPTED);
    let retry = retry.data.expect("retry receipt");
    assert_eq!(retry.input_id, first.input_id);
    assert!(retry.replayed);
    assert!(!active_model_turn.is_cancelled());

    let (conflict_status, Json(conflict)) = accept_input(
        State(state),
        headers,
        Json(submission("message-1", "Replace the entire report.")),
    )
    .await;
    assert_eq!(conflict_status, StatusCode::CONFLICT);
    assert_eq!(
        conflict.error.as_deref(),
        Some("conversation_input_idempotency_conflict")
    );
}

#[tokio::test]
async fn lost_response_can_be_recovered_by_client_message_id() {
    let (state, headers, _) = authenticated_state();
    let (status, Json(created)) = accept_input(
        State(state.clone()),
        headers.clone(),
        Json(submission("recover-message", "Preserve this exact input.")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let created = created.data.expect("created receipt");

    let (status, Json(page)) = list_inputs(
        State(state),
        headers,
        Query(ConversationInputListQuery {
            conversation_id: "conversation-http-1".to_string(),
            agent_id: "main".to_string(),
            channel: "ui".to_string(),
            channel_account_id: "browser-session".to_string(),
            client_message_id: Some("recover-message".to_string()),
            after_input_seq: 0,
            limit: 50,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page = page.data.expect("recovery page");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].receipt.input_id, created.input_id);
    assert!(page.next_after_input_seq.is_none());
}

#[tokio::test]
async fn input_events_expose_bounded_owner_scoped_recovery_cursor() {
    let (state, headers, _) = authenticated_state();
    let (status, _) = accept_input(
        State(state.clone()),
        headers.clone(),
        Json(submission("event-message", "Record this input.")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, Json(page)) = list_input_events(
        State(state),
        headers,
        Query(ConversationInputEventListQuery {
            conversation_id: "conversation-http-1".to_string(),
            agent_id: "main".to_string(),
            channel: "ui".to_string(),
            channel_account_id: "browser-session".to_string(),
            after_event_seq: 0,
            limit: 1,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page = page.data.expect("event page");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].event_kind, "accepted");
    assert_eq!(page.next_after_event_seq, Some(1));
}

#[tokio::test]
async fn client_message_recovery_is_scoped_to_authenticated_owner() {
    let (state, headers, _) = authenticated_state();
    let (status, _) = accept_input(
        State(state.clone()),
        headers,
        Json(submission("private-message", "Owner-only input.")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let other_key = crate::repo::auth::create_auth_key(&state, "other").expect("other auth key");
    let mut other_headers = HeaderMap::new();
    other_headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        HeaderValue::from_str(&other_key).expect("other auth header"),
    );
    let (status, Json(response)) = list_inputs(
        State(state),
        other_headers,
        Query(ConversationInputListQuery {
            conversation_id: "conversation-http-1".to_string(),
            agent_id: "main".to_string(),
            channel: "ui".to_string(),
            channel_account_id: "browser-session".to_string(),
            client_message_id: Some("private-message".to_string()),
            after_input_seq: 0,
            limit: 50,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        response.error.as_deref(),
        Some("conversation_input_not_found")
    );
}

#[tokio::test]
async fn withdraw_is_owner_scoped_idempotent_and_interrupts_the_stale_model_turn() {
    let (state, headers, _) = authenticated_state();
    let (_, Json(first)) = accept_input(
        State(state.clone()),
        headers.clone(),
        Json(submission("message-1", "Create a report.")),
    )
    .await;
    let first = first.data.expect("first receipt");
    let task_id = first.target_task_id.expect("bound task");
    state
        .worker
        .acknowledge_model_turn_conversation_input(&task_id.to_string());
    let (_, Json(second)) = accept_input(
        State(state.clone()),
        headers.clone(),
        Json(submission("message-2", "Change only the final format.")),
    )
    .await;
    let second = second.data.expect("second receipt");
    state
        .worker
        .acknowledge_model_turn_conversation_input(&task_id.to_string());

    let (status, Json(withdrawn)) = withdraw_input(
        State(state.clone()),
        headers.clone(),
        AxumPath(second.input_id),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let withdrawn = withdrawn.data.expect("withdrawn receipt");
    assert_eq!(
        withdrawn.disposition,
        claw_core::conversation_input::ConversationInputDisposition::Withdrawn
    );
    assert!(state
        .worker
        .model_turn_interrupt_token(&task_id.to_string())
        .is_cancelled());

    let (status, Json(replayed)) =
        withdraw_input(State(state.clone()), headers, AxumPath(second.input_id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        replayed.data.expect("replayed withdrawal").decision_ref,
        withdrawn.decision_ref
    );

    let other_key = crate::repo::auth::create_auth_key(&state, "other").expect("other auth key");
    let mut other_headers = HeaderMap::new();
    other_headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        HeaderValue::from_str(&other_key).expect("other auth header"),
    );
    let (status, Json(response)) = withdraw_input(
        State(state.clone()),
        other_headers,
        AxumPath(second.input_id),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        response.error.as_deref(),
        Some("conversation_input_not_found")
    );

    let (status, Json(response)) =
        withdraw_input(State(state), HeaderMap::new(), AxumPath(first.input_id)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.error.as_deref(), Some("auth_key_required"));
}

#[tokio::test]
async fn activating_an_idle_deferred_input_creates_one_followup_task() {
    let (state, headers, _) = authenticated_state();
    let mut deferred = submission("deferred-message", "Run this only when activated.");
    deferred.delivery_mode = ConversationInputDeliveryMode::Defer;
    let (status, Json(accepted)) =
        accept_input(State(state.clone()), headers.clone(), Json(deferred)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let accepted = accepted.data.expect("deferred receipt");
    assert_eq!(
        accepted.disposition,
        claw_core::conversation_input::ConversationInputDisposition::Deferred
    );
    assert_eq!(accepted.target_task_id, None);

    let (status, Json(activated)) = activate_input(
        State(state.clone()),
        headers.clone(),
        AxumPath(accepted.input_id),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let activated = activated.data.expect("activated receipt");
    let task_id = activated.target_task_id.expect("created task");
    assert_eq!(
        activated.disposition,
        claw_core::conversation_input::ConversationInputDisposition::Applied
    );

    let (status, Json(replayed)) =
        activate_input(State(state.clone()), headers, AxumPath(accepted.input_id)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let replayed = replayed.data.expect("activation replay");
    assert_eq!(replayed.target_task_id, Some(task_id));
    assert!(replayed.replayed);
    let task_count: i64 = state
        .core
        .db
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
        .expect("task count");
    assert_eq!(task_count, 1);
}
