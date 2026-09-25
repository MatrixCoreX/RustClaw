use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    Json,
};
use claw_core::{
    conversation_input::{
        ConversationInputClientTaskReceipt, ConversationInputClientTaskRequest,
        ConversationInputContent, ConversationInputDeliveryMode, ConversationInputTaskHandoffState,
        OwnedConversationInputScope, CONVERSATION_INPUT_SCHEMA_VERSION,
    },
    types::{ApiResponse, AuthIdentity, ChannelKind, TaskKind},
};
use serde_json::json;

use super::{interrupt_bound_model_turn, parse_channel, store_error};
use crate::{repo::conversation_inputs::ConversationInputTaskClaimOutcome, AppState};

mod attachments;
mod recovery;

pub(crate) use recovery::spawn_task_creation_recovery_worker;

pub(crate) async fn accept_client_task(
    State(state): State<AppState>,
    mut headers: HeaderMap,
    Json(mut request): Json<ConversationInputClientTaskRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<ConversationInputClientTaskReceipt>>,
) {
    let Some(agent_id) = state.normalize_known_agent_id(Some(&request.input.scope.agent_id)) else {
        return crate::api_err(StatusCode::BAD_REQUEST, "conversation_input_agent_unknown");
    };
    request.input.scope.agent_id = agent_id;
    let Some(channel) = parse_channel(&request.input.scope.channel) else {
        return crate::api_err(
            StatusCode::BAD_REQUEST,
            "conversation_input_channel_unknown",
        );
    };
    let input_text = client_task_text_content(&request.input.content);
    if !headers.contains_key(claw_core::product_identity::AUTH_KEY_HEADER) {
        if let Some(user_key) = request
            .task
            .user_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let Ok(value) = HeaderValue::from_str(user_key) else {
                return crate::api_err(StatusCode::UNAUTHORIZED, "auth_key_invalid");
            };
            headers.insert(claw_core::product_identity::AUTH_KEY_HEADER, value);
        }
    }
    let identity = match crate::require_auth_identity_for_api::<ConversationInputClientTaskReceipt>(
        &state, &headers,
    ) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    request.task.user_key = Some(identity.user_key.clone());
    if channel == ChannelKind::Ui {
        if let Err(code) = prepare_ui_task_ingress(&state, &identity, &mut request) {
            return crate::api_err(StatusCode::BAD_REQUEST, code);
        }
    }
    if let Err(code) = validate_task_template(&request, channel, &input_text) {
        return crate::api_err(StatusCode::BAD_REQUEST, code);
    }
    let attachment_bindings = match attachments::prepare_attachment_bindings(
        &state.skill_rt.workspace_root,
        &identity.principal_id,
        &request,
    ) {
        Ok(bindings) => bindings,
        Err(code) => return crate::api_err(StatusCode::BAD_REQUEST, code),
    };
    request.input.content.retain(|content| {
        !matches!(content, ConversationInputContent::Text { text } if text.trim().is_empty())
    });
    request
        .input
        .content
        .extend(attachment_bindings.iter().map(|attachment| {
            ConversationInputContent::Attachment {
                attachment_id: attachment.attachment_id.clone(),
                media_type: attachment.mime_type.clone(),
                display_name: attachment.display_name.clone(),
            }
        }));
    if let Err(error) = request.input.validate() {
        return crate::api_err(StatusCode::BAD_REQUEST, super::error_code(error));
    }
    let task_template = recoverable_task_template(&request, &headers);
    let scope = OwnedConversationInputScope {
        owner_principal_id: identity.principal_id.clone(),
        conversation: request.input.scope.clone(),
    };
    let mut accepted =
        match crate::repo::conversation_inputs::accept_conversation_input_with_task_template(
            &state.core.db,
            &crate::repo::conversation_inputs::AcceptConversationInput {
                owner_principal_id: identity.principal_id.clone(),
                submission: request.input.clone(),
                preparation_state:
                    claw_core::conversation_input::ConversationInputPreparationState::Ready,
            },
            &task_template,
            &attachment_bindings,
        ) {
            Ok(accepted) => accepted.record,
            Err(error) => return store_error(error),
        };
    crate::conversation_input_event_transport::notify(&state);
    if request.input.delivery_mode == ConversationInputDeliveryMode::Defer {
        return handoff_response(
            accepted.receipt,
            ConversationInputTaskHandoffState::Deferred,
        );
    }
    if accepted.receipt.target_task_id.is_some() {
        interrupt_bound_model_turn(&state, &accepted.receipt);
        crate::conversation_input_event_transport::notify(&state);
        return existing_handoff_response(&state, &identity, accepted);
    }

    let (creator, claim_token) =
        match crate::repo::conversation_inputs::claim_or_bind_conversation_input_task(
            &state.core.db,
            &scope,
            accepted.receipt.input_id,
        ) {
            Ok(ConversationInputTaskClaimOutcome::Bound(record)) => {
                interrupt_bound_model_turn(&state, &record.receipt);
                crate::conversation_input_event_transport::notify(&state);
                return existing_handoff_response(&state, &identity, record);
            }
            Ok(ConversationInputTaskClaimOutcome::Waiting) => {
                match wait_for_binding_or_creation_claim(&state, &scope, accepted.receipt.input_id)
                    .await
                {
                    Ok(ConversationInputTaskClaimOutcome::Bound(record)) => {
                        interrupt_bound_model_turn(&state, &record.receipt);
                        crate::conversation_input_event_transport::notify(&state);
                        return existing_handoff_response(&state, &identity, record);
                    }
                    Ok(ConversationInputTaskClaimOutcome::Creator {
                        record,
                        claim_token,
                    }) => (record, claim_token),
                    Ok(ConversationInputTaskClaimOutcome::Waiting) => {
                        return crate::api_err(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "conversation_input_task_binding_pending",
                        );
                    }
                    Err(error) => return store_error(error),
                }
            }
            Ok(ConversationInputTaskClaimOutcome::Creator {
                record,
                claim_token,
            }) => (record, claim_token),
            Err(error) => return store_error(error),
        };

    let creator_text = client_task_text_content(&creator.content);
    prepare_creator_task(
        &mut request,
        &identity,
        creator.receipt.input_id,
        &creator_text,
        creator.source.provider_message_id.as_deref(),
    );
    let (task_status, Json(task_response)) =
        crate::submit_task(State(state.clone()), headers, Json(request.task)).await;
    let Some(task) = task_response.data else {
        let _ = crate::repo::conversation_inputs::release_conversation_input_task_claim(
            &state.core.db,
            &scope,
            creator.receipt.input_id,
            claim_token,
        );
        return crate::api_err(
            task_status,
            task_response
                .error
                .unwrap_or_else(|| "conversation_input_task_submit_failed".to_string()),
        );
    };
    if let Err(error) = crate::repo::conversation_inputs::complete_conversation_input_task_claim(
        &state.core.db,
        &scope,
        creator.receipt.input_id,
        claim_token,
        task.task_id,
    ) {
        let _ = crate::repo::conversation_inputs::release_conversation_input_task_claim(
            &state.core.db,
            &scope,
            creator.receipt.input_id,
            claim_token,
        );
        return store_error(error);
    }
    crate::conversation_input_event_transport::notify(&state);
    accepted = match crate::repo::conversation_inputs::get_conversation_input(
        &state.core.db,
        &identity.principal_id,
        accepted.receipt.input_id,
    ) {
        Ok(record) => record,
        Err(error) => return store_error(error),
    };
    let handoff_state = if accepted.receipt.input_id == creator.receipt.input_id {
        ConversationInputTaskHandoffState::TaskCreated
    } else {
        ConversationInputTaskHandoffState::BoundExistingTask
    };
    handoff_response(accepted.receipt, handoff_state)
}

fn recoverable_task_template(
    request: &ConversationInputClientTaskRequest,
    headers: &HeaderMap,
) -> crate::repo::conversation_inputs::ConversationInputTaskTemplate {
    let mut task = request.task.clone();
    task.user_key = None;
    task.idempotency_key = None;
    if let Some(payload) = task.payload.as_object_mut() {
        payload.remove("text");
        payload.remove("attachments");
        payload.remove("conversation_input_id");
        payload.remove(crate::task_execution_policy::POLICY_PAYLOAD_FIELD);
    }
    if let Some(ingress) = task.ingress.as_mut() {
        ingress.attachments.clear();
        ingress.context_token = None;
        ingress.bound_user_id = None;
        ingress.conversation_chat_id = None;
        ingress.message_id = None;
    }
    crate::repo::conversation_inputs::ConversationInputTaskTemplate {
        schema_version: 1,
        task,
        client_origin: crate::task_execution_policy::client_origin_from_headers(headers)
            .map(ToString::to_string),
        execution_mode: crate::task_execution_policy::execution_mode_from_headers(headers)
            .map(ToString::to_string),
    }
}

fn prepare_ui_task_ingress(
    state: &AppState,
    identity: &AuthIdentity,
    request: &mut ConversationInputClientTaskRequest,
) -> Result<(), String> {
    if request.task.ingress.is_some() {
        return Ok(());
    }
    crate::ui_attachments::materialize_ui_task_attachments(
        state,
        &mut request.task.payload,
        identity.user_id,
        identity.chat_id,
        &request.input.client_message_id,
    )?;
    let attachments = request
        .task
        .payload
        .get("attachments")
        .and_then(serde_json::Value::as_array)
        .map(|attachments| {
            attachments
                .iter()
                .cloned()
                .map(serde_json::from_value)
                .collect::<Result<Vec<claw_core::channel_ingress::ChannelIngressAttachment>, _>>()
        })
        .transpose()
        .map_err(|_| "conversation_input_task_attachment_mismatch".to_string())?
        .unwrap_or_default();
    let conversation_id = request.input.scope.conversation_id.clone();
    let external_user_id = request
        .task
        .external_user_id
        .clone()
        .unwrap_or_else(|| identity.user_id.to_string());
    let mut ingress = claw_core::channel_ingress::ChannelIngressEnvelope::new(
        ChannelKind::Ui,
        "interactive_client",
    )
    .with_account_id(request.input.scope.channel_account_id.clone())
    .with_external_ids(&external_user_id, &conversation_id)
    .with_message_id(request.input.client_message_id.clone())
    .with_reply_target(claw_core::channel_ingress::ChannelReplyTarget::chat(
        &conversation_id,
    ));
    if let Some(received_at_ts) = request.input.source.received_at_ts {
        ingress = ingress.with_received_at_ts(received_at_ts);
    }
    ingress.attachments = attachments;
    request.task.channel = Some(ChannelKind::Ui);
    request.task.external_user_id = Some(external_user_id);
    request.task.external_chat_id = Some(conversation_id);
    request.task.ingress = Some(ingress);
    Ok(())
}

fn validate_task_template(
    request: &ConversationInputClientTaskRequest,
    channel: ChannelKind,
    input_text: &str,
) -> Result<(), &'static str> {
    if !matches!(request.task.kind, TaskKind::Ask)
        || request.task.channel != Some(channel)
        || request.task.external_chat_id.as_deref()
            != Some(request.input.scope.conversation_id.as_str())
        || request
            .task
            .payload
            .get("text")
            .and_then(|value| value.as_str())
            != Some(input_text)
    {
        return Err("conversation_input_task_template_mismatch");
    }
    let Some(ingress) = request.task.ingress.as_ref() else {
        return Err("conversation_input_task_ingress_required");
    };
    if ingress.channel != channel
        || ingress.account_id.as_deref().unwrap_or_default()
            != request.input.scope.channel_account_id
        || ingress.external_chat_id.as_deref() != Some(request.input.scope.conversation_id.as_str())
    {
        return Err("conversation_input_task_ingress_mismatch");
    }
    let payload_attachments = request
        .task
        .payload
        .get("attachments")
        .and_then(serde_json::Value::as_array)
        .map(|attachments| {
            attachments
                .iter()
                .cloned()
                .map(serde_json::from_value)
                .collect::<Result<Vec<claw_core::channel_ingress::ChannelIngressAttachment>, _>>()
        })
        .transpose()
        .map_err(|_| "conversation_input_task_attachment_mismatch")?
        .unwrap_or_default();
    if payload_attachments != ingress.attachments {
        return Err("conversation_input_task_attachment_mismatch");
    }
    if input_text.trim().is_empty() && ingress.attachments.is_empty() {
        return Err("conversation_input_text_or_attachment_required");
    }
    Ok(())
}

fn client_task_text_content(content: &[ConversationInputContent]) -> String {
    content
        .iter()
        .filter_map(|content| match content {
            ConversationInputContent::Text { text } => Some(text.as_str()),
            ConversationInputContent::Attachment { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn wait_for_binding_or_creation_claim(
    state: &AppState,
    scope: &OwnedConversationInputScope,
    input_id: uuid::Uuid,
) -> Result<
    ConversationInputTaskClaimOutcome,
    crate::repo::conversation_inputs::ConversationInputStoreError,
> {
    const ATTEMPTS: usize = 200;
    for _ in 0..ATTEMPTS {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        match crate::repo::conversation_inputs::claim_or_bind_conversation_input_task(
            &state.core.db,
            scope,
            input_id,
        )? {
            ConversationInputTaskClaimOutcome::Waiting => {}
            outcome => return Ok(outcome),
        }
    }
    Ok(ConversationInputTaskClaimOutcome::Waiting)
}

fn prepare_creator_task(
    request: &mut ConversationInputClientTaskRequest,
    identity: &AuthIdentity,
    input_id: uuid::Uuid,
    creator_text: &str,
    provider_message_id: Option<&str>,
) {
    request.task.user_key = Some(identity.user_key.clone());
    request.task.idempotency_key = Some(format!("conversation-input:{input_id}"));
    if let Some(payload) = request.task.payload.as_object_mut() {
        payload.insert("text".to_string(), json!(creator_text));
        payload.insert("conversation_input_id".to_string(), json!(input_id));
        payload.insert(
            "conversation_id".to_string(),
            json!(request.input.scope.conversation_id),
        );
        payload.insert("agent_id".to_string(), json!(request.input.scope.agent_id));
    }
    if let Some(ingress) = request.task.ingress.as_mut() {
        ingress.message_id = provider_message_id.map(ToString::to_string);
    }
}

fn existing_handoff_response(
    state: &AppState,
    identity: &AuthIdentity,
    record: claw_core::conversation_input::ConversationInputRecord,
) -> (
    StatusCode,
    Json<ApiResponse<ConversationInputClientTaskReceipt>>,
) {
    let owns_delivery =
        crate::repo::conversation_inputs::conversation_input_owns_initial_task_delivery(
            &state.core.db,
            &identity.principal_id,
            record.receipt.input_id,
        )
        .unwrap_or(false);
    handoff_response(
        record.receipt,
        if owns_delivery {
            ConversationInputTaskHandoffState::TaskCreated
        } else {
            ConversationInputTaskHandoffState::BoundExistingTask
        },
    )
}

fn handoff_response(
    input: claw_core::conversation_input::ConversationInputReceipt,
    handoff_state: ConversationInputTaskHandoffState,
) -> (
    StatusCode,
    Json<ApiResponse<ConversationInputClientTaskReceipt>>,
) {
    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            ok: true,
            data: Some(ConversationInputClientTaskReceipt {
                schema_version: CONVERSATION_INPUT_SCHEMA_VERSION,
                input,
                handoff_state,
            }),
            error: None,
        }),
    )
}

#[cfg(test)]
#[path = "client_task_tests.rs"]
mod tests;
