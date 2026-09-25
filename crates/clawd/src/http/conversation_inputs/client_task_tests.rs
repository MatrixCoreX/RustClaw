use axum::http::{HeaderMap, HeaderValue};
use claw_core::{
    channel_ingress::{ChannelIngressAttachment, ChannelIngressEnvelope, ChannelReplyTarget},
    conversation_input::{
        ConversationInputClientTaskRequest, ConversationInputContent,
        ConversationInputDeliveryMode, ConversationInputScopeRef, ConversationInputSource,
        ConversationInputSubmission, ConversationInputTaskHandoffState,
    },
    types::{ChannelKind, SubmitTaskRequest, TaskKind},
};
use serde_json::json;

use super::*;

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

fn ui_recovery_request(key: &str, message_id: &str) -> ConversationInputClientTaskRequest {
    let conversation_id = "ui-recovery-thread";
    ConversationInputClientTaskRequest {
        input: ConversationInputSubmission {
            schema_version: 1,
            client_message_id: message_id.to_string(),
            scope: ConversationInputScopeRef {
                conversation_id: conversation_id.to_string(),
                agent_id: "main".to_string(),
                channel: "ui".to_string(),
                channel_account_id: "browser-session".to_string(),
            },
            content: vec![ConversationInputContent::Text {
                text: "Continue after the runtime restarts.".to_string(),
            }],
            delivery_mode: ConversationInputDeliveryMode::Auto,
            expected_task_id: None,
            expected_instruction_revision: None,
            source: ConversationInputSource::default(),
        },
        task: SubmitTaskRequest {
            user_id: None,
            chat_id: None,
            user_key: Some(key.to_string()),
            channel: Some(ChannelKind::Ui),
            external_user_id: Some("browser-user".to_string()),
            external_chat_id: Some(conversation_id.to_string()),
            ingress: Some(
                ChannelIngressEnvelope::new(ChannelKind::Ui, "interactive_client")
                    .with_account_id("browser-session")
                    .with_external_ids("browser-user", conversation_id)
                    .with_message_id(message_id)
                    .with_reply_target(ChannelReplyTarget::chat(conversation_id)),
            ),
            idempotency_key: Some(format!("provider:{message_id}")),
            kind: TaskKind::Ask,
            payload: json!({
                "text": "Continue after the runtime restarts.",
                "conversation_id": conversation_id,
                "agent_id": "main"
            }),
        },
    }
}

#[tokio::test]
async fn recovery_scan_creates_the_task_from_a_sanitized_durable_template() {
    let (state, headers, key) = authenticated_state();
    let request = ui_recovery_request(&key, "recovery-message-1");
    let identity = crate::require_auth_identity_for_api::<ConversationInputClientTaskReceipt>(
        &state, &headers,
    )
    .expect("identity");
    let template = recoverable_task_template(&request, &headers);
    let accepted = crate::repo::conversation_inputs::accept_conversation_input_with_task_template(
        &state.core.db,
        &crate::repo::conversation_inputs::AcceptConversationInput {
            owner_principal_id: identity.principal_id.clone(),
            submission: request.input.clone(),
            preparation_state:
                claw_core::conversation_input::ConversationInputPreparationState::Ready,
        },
        &template,
        &[],
    )
    .expect("persist recoverable input");
    let stored_template: String = state
        .core
        .db
        .get()
        .expect("db")
        .query_row(
            "SELECT template_json FROM conversation_input_task_templates WHERE input_id = ?1",
            [accepted.record.receipt.input_id.to_string()],
            |row| row.get(0),
        )
        .expect("stored template");
    assert!(!stored_template.contains(&key));
    assert!(!stored_template.contains("Continue after the runtime restarts."));

    assert_eq!(
        super::recovery::recover_task_creations_once(&state)
            .await
            .expect("recovery scan"),
        1
    );
    let recovered = crate::repo::conversation_inputs::get_conversation_input(
        &state.core.db,
        &identity.principal_id,
        accepted.record.receipt.input_id,
    )
    .expect("recovered input");
    let task_id = recovered.receipt.target_task_id.expect("bound task");
    let (task_count, payload): (i64, String) = state
        .core
        .db
        .get()
        .expect("db")
        .query_row(
            "SELECT COUNT(*), MAX(payload_json) FROM tasks WHERE task_id = ?1",
            [task_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("recovered task");
    assert_eq!(task_count, 1);
    assert!(payload.contains("Continue after the runtime restarts."));
    let template_count: i64 = state
        .core
        .db
        .get()
        .expect("db")
        .query_row(
            "SELECT COUNT(*) FROM conversation_input_task_templates WHERE input_id = ?1",
            [accepted.record.receipt.input_id.to_string()],
            |row| row.get(0),
        )
        .expect("template cleanup");
    assert_eq!(template_count, 0);
}

#[tokio::test]
async fn recovery_scan_rejects_an_input_after_its_principal_credential_is_revoked() {
    let (state, headers, key) = authenticated_state();
    let request = ui_recovery_request(&key, "recovery-message-revoked");
    let identity = crate::require_auth_identity_for_api::<ConversationInputClientTaskReceipt>(
        &state, &headers,
    )
    .expect("identity");
    let template = recoverable_task_template(&request, &headers);
    let accepted = crate::repo::conversation_inputs::accept_conversation_input_with_task_template(
        &state.core.db,
        &crate::repo::conversation_inputs::AcceptConversationInput {
            owner_principal_id: identity.principal_id.clone(),
            submission: request.input,
            preparation_state:
                claw_core::conversation_input::ConversationInputPreparationState::Ready,
        },
        &template,
        &[],
    )
    .expect("persist recoverable input");
    state
        .core
        .db
        .get()
        .expect("db")
        .execute(
            "UPDATE auth_keys SET enabled = 0 WHERE user_key = ?1",
            [&key],
        )
        .expect("revoke credential");

    assert_eq!(
        super::recovery::recover_task_creations_once(&state)
            .await
            .expect("recovery scan"),
        1
    );
    let rejected = crate::repo::conversation_inputs::get_conversation_input(
        &state.core.db,
        &identity.principal_id,
        accepted.record.receipt.input_id,
    )
    .expect("rejected input");
    assert_eq!(
        rejected.receipt.disposition,
        claw_core::conversation_input::ConversationInputDisposition::Rejected
    );
    assert_eq!(
        rejected.receipt.decision_ref.as_deref(),
        Some("authorization_revoked")
    );
}

pub(super) fn request(
    key: &str,
    message_id: &str,
    text: &str,
) -> ConversationInputClientTaskRequest {
    let conversation_id = "chat-42";
    let account_id = "bot-main";
    ConversationInputClientTaskRequest {
        input: ConversationInputSubmission {
            schema_version: 1,
            client_message_id: message_id.to_string(),
            scope: ConversationInputScopeRef {
                conversation_id: conversation_id.to_string(),
                agent_id: "main".to_string(),
                channel: "telegram".to_string(),
                channel_account_id: account_id.to_string(),
            },
            content: vec![ConversationInputContent::Text {
                text: text.to_string(),
            }],
            delivery_mode: ConversationInputDeliveryMode::Auto,
            expected_task_id: None,
            expected_instruction_revision: None,
            source: ConversationInputSource {
                provider_message_id: Some(message_id.to_string()),
                reply_to_message_id: None,
                received_at_ts: Some(100),
            },
        },
        task: SubmitTaskRequest {
            user_id: Some(42),
            chat_id: Some(42),
            user_key: Some(key.to_string()),
            channel: Some(ChannelKind::Telegram),
            external_user_id: Some("user-42".to_string()),
            external_chat_id: Some(conversation_id.to_string()),
            ingress: Some(
                ChannelIngressEnvelope::new(ChannelKind::Telegram, "telegram_bot")
                    .with_account_id(account_id)
                    .with_external_ids("user-42", conversation_id)
                    .with_message_id(message_id)
                    .with_reply_target(ChannelReplyTarget::chat(conversation_id)),
            ),
            idempotency_key: Some(format!("provider:{message_id}")),
            kind: TaskKind::Ask,
            payload: json!({ "text": text }),
        },
    }
}

#[tokio::test]
async fn concurrent_messages_share_one_task_and_one_delivery_owner() {
    let (state, headers, key) = authenticated_state();
    let (status, Json(first)) = accept_client_task(
        State(state.clone()),
        headers.clone(),
        Json(request(&key, "message-1", "Create the report.")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let first = first.data.expect("first handoff");
    assert_eq!(
        first.handoff_state,
        ConversationInputTaskHandoffState::TaskCreated
    );
    let task_id = first.input.target_task_id.expect("first task binding");

    let (status, Json(second)) = accept_client_task(
        State(state.clone()),
        headers.clone(),
        Json(request(
            &key,
            "message-2",
            "Keep the facts and change the format.",
        )),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let second = second.data.expect("second handoff");
    assert_eq!(
        second.handoff_state,
        ConversationInputTaskHandoffState::BoundExistingTask
    );
    assert_eq!(second.input.target_task_id, Some(task_id));
    assert!(
        !crate::repo::conversation_inputs::conversation_input_owns_initial_task_delivery(
            &state.core.db,
            &crate::require_auth_identity_for_api::<ConversationInputClientTaskReceipt>(
                &state, &headers,
            )
            .expect("identity")
            .principal_id,
            second.input.input_id,
        )
        .expect("non-owner binding")
    );

    let (status, Json(replayed)) = accept_client_task(
        State(state.clone()),
        headers,
        Json(request(&key, "message-1", "Create the report.")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let replayed = replayed.data.expect("replayed handoff");
    assert_eq!(
        replayed.handoff_state,
        ConversationInputTaskHandoffState::TaskCreated
    );
    assert!(replayed.input.replayed);
    assert_eq!(replayed.input.target_task_id, Some(task_id));

    let task_count: i64 = state
        .core
        .db
        .get()
        .expect("db")
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
        .expect("task count");
    assert_eq!(task_count, 1);
}

#[tokio::test]
async fn mismatched_task_scope_is_rejected_before_input_is_persisted() {
    let (state, headers, key) = authenticated_state();
    let mut request = request(&key, "message-mismatch", "Do the scoped task.");
    request.task.external_chat_id = Some("another-chat".to_string());
    let (status, Json(response)) =
        accept_client_task(State(state.clone()), headers, Json(request)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        response.error.as_deref(),
        Some("conversation_input_task_template_mismatch")
    );
    let input_count: i64 = state
        .core
        .db
        .get()
        .expect("db")
        .query_row("SELECT COUNT(*) FROM conversation_inputs", [], |row| {
            row.get(0)
        })
        .expect("input count");
    assert_eq!(input_count, 0);
}

#[tokio::test]
async fn waiting_input_can_finish_after_the_original_creation_claim_is_released() {
    let (state, headers, key) = authenticated_state();
    let first_request = request(&key, "message-stalled", "Create the initial report.");
    let identity = crate::require_auth_identity_for_api::<ConversationInputClientTaskReceipt>(
        &state, &headers,
    )
    .expect("identity");
    let scope = OwnedConversationInputScope {
        owner_principal_id: identity.principal_id.clone(),
        conversation: first_request.input.scope.clone(),
    };
    let first = crate::repo::conversation_inputs::accept_conversation_input(
        &state.core.db,
        &crate::repo::conversation_inputs::AcceptConversationInput {
            owner_principal_id: identity.principal_id.clone(),
            submission: first_request.input,
            preparation_state:
                claw_core::conversation_input::ConversationInputPreparationState::Ready,
        },
    )
    .expect("first input");
    let claim_token = match crate::repo::conversation_inputs::claim_or_bind_conversation_input_task(
        &state.core.db,
        &scope,
        first.record.receipt.input_id,
    )
    .expect("creation claim")
    {
        ConversationInputTaskClaimOutcome::Creator { claim_token, .. } => claim_token,
        outcome => panic!("unexpected claim outcome: {outcome:?}"),
    };

    let release_state = state.clone();
    let release_scope = scope.clone();
    let first_input_id = first.record.receipt.input_id;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
        crate::repo::conversation_inputs::release_conversation_input_task_claim(
            &release_state.core.db,
            &release_scope,
            first_input_id,
            claim_token,
        )
        .expect("release stalled claim");
    });

    let (status, Json(response)) = accept_client_task(
        State(state.clone()),
        headers,
        Json(request(
            &key,
            "message-after-stall",
            "Add a concise summary.",
        )),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let response = response.data.expect("handoff");
    assert_eq!(
        response.handoff_state,
        ConversationInputTaskHandoffState::BoundExistingTask
    );
    assert!(response.input.target_task_id.is_some());
    assert!(
        crate::repo::conversation_inputs::conversation_input_owns_initial_task_delivery(
            &state.core.db,
            &identity.principal_id,
            first_input_id,
        )
        .expect("delivery owner")
    );
}

#[tokio::test]
async fn attachment_only_message_is_prepared_before_task_handoff() {
    let (state, headers, key) = authenticated_state();
    let relative_path = format!(
        "conversation-input-tests/{}/fixture.txt",
        uuid::Uuid::new_v4()
    );
    let absolute_path = state.skill_rt.workspace_root.join(&relative_path);
    std::fs::create_dir_all(absolute_path.parent().expect("attachment parent"))
        .expect("create attachment parent");
    std::fs::write(&absolute_path, b"fixture").expect("write attachment");
    let mut request = request(&key, "message-attachment-only", "");
    let attachment = ChannelIngressAttachment {
        kind: "file".to_string(),
        path: relative_path.clone(),
        mime_type: Some("text/plain".to_string()),
        size: Some(7),
    };
    request
        .task
        .ingress
        .as_mut()
        .expect("ingress")
        .attachments
        .push(attachment.clone());
    request.task.payload["attachments"] = json!([attachment]);

    let (status, Json(response)) =
        accept_client_task(State(state.clone()), headers.clone(), Json(request)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let receipt = response.data.expect("handoff receipt");
    assert_eq!(
        receipt.handoff_state,
        ConversationInputTaskHandoffState::TaskCreated
    );
    let identity = crate::require_auth_identity_for_api::<ConversationInputClientTaskReceipt>(
        &state, &headers,
    )
    .expect("identity");
    let record = crate::repo::conversation_inputs::get_conversation_input(
        &state.core.db,
        &identity.principal_id,
        receipt.input.input_id,
    )
    .expect("prepared input");
    assert_eq!(
        record.receipt.preparation_state,
        claw_core::conversation_input::ConversationInputPreparationState::Ready
    );
    let attachments = crate::repo::conversation_inputs::conversation_input_attachments(
        &state.core.db,
        record.receipt.input_id,
    )
    .expect("resolved attachments");
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].workspace_rel_path, relative_path);
    assert!(attachments[0]
        .attachment_id
        .starts_with("channel_attachment:"));

    std::fs::remove_file(&absolute_path).expect("remove attachment");
    std::fs::remove_dir(absolute_path.parent().expect("attachment parent"))
        .expect("remove attachment parent");
}

#[tokio::test]
async fn ui_base64_attachment_is_materialized_without_persisting_inline_data() {
    let (state, headers, key) = authenticated_state();
    let client_message_id = format!("ui-attachment-{}", uuid::Uuid::new_v4());
    let conversation_id = "ui-thread-attachment";
    let task = SubmitTaskRequest {
        user_id: None,
        chat_id: None,
        user_key: Some(key.clone()),
        channel: Some(ChannelKind::Ui),
        external_user_id: Some("browser-user".to_string()),
        external_chat_id: Some(conversation_id.to_string()),
        ingress: None,
        idempotency_key: Some(client_message_id.clone()),
        kind: TaskKind::Ask,
        payload: json!({
            "text": "Inspect the attachment.",
            "conversation_id": conversation_id,
            "agent_id": "main",
            "attachments": [{
                "name": "fixture.txt",
                "mime_type": "text/plain",
                "kind": "file",
                "base64": "data:text/plain;base64,Zml4dHVyZQ=="
            }]
        }),
    };
    let request = ConversationInputClientTaskRequest::channel_text(
        task,
        client_message_id,
        conversation_id,
        "main",
        ChannelKind::Ui,
        "browser-session",
        "Inspect the attachment.",
        ConversationInputSource::default(),
    );

    let (status, Json(response)) =
        accept_client_task(State(state.clone()), headers.clone(), Json(request)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let handoff = response.data.expect("handoff receipt");
    let attachments = crate::repo::conversation_inputs::conversation_input_attachments(
        &state.core.db,
        handoff.input.input_id,
    )
    .expect("resolved attachments");
    assert_eq!(attachments.len(), 1);
    assert!(attachments[0].workspace_rel_path.starts_with("data/ui/"));
    assert_eq!(
        std::fs::read(
            state
                .skill_rt
                .workspace_root
                .join(&attachments[0].workspace_rel_path)
        )
        .expect("materialized attachment"),
        b"fixture"
    );
    let task_id = handoff.input.target_task_id.expect("task binding");
    let payload_json: String = state
        .core
        .db
        .get()
        .expect("db")
        .query_row(
            "SELECT payload_json FROM tasks WHERE task_id = ?1",
            [task_id.to_string()],
            |row| row.get(0),
        )
        .expect("task payload");
    assert!(!payload_json.contains("Zml4dHVyZQ"));
    assert!(payload_json.contains(&attachments[0].workspace_rel_path));

    let absolute_path = state
        .skill_rt
        .workspace_root
        .join(&attachments[0].workspace_rel_path);
    std::fs::remove_file(&absolute_path).expect("remove attachment");
    std::fs::remove_dir(absolute_path.parent().expect("attachment parent"))
        .expect("remove attachment parent");
}
