use axum::{
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use claw_core::conversation_input::{
    ConversationInputContent, ConversationInputDeliveryMode, ConversationInputPreparationState,
    ConversationInputReceipt, ConversationInputRecord, ConversationInputScopeRef,
    ConversationInputSubmission, OwnedConversationInputScope,
};
use claw_core::types::{ApiResponse, AuthIdentity, ChannelKind, SubmitTaskRequest, TaskKind};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::warn;
use uuid::Uuid;

use crate::repo::conversation_inputs::{
    AcceptConversationInput, ConversationInputEventRecord, ConversationInputStoreError,
    ConversationInputTaskClaimOutcome,
};
use crate::AppState;

mod client_task;

pub(crate) use client_task::accept_client_task;
pub(crate) use client_task::spawn_task_creation_recovery_worker;

#[derive(Debug, Serialize)]
pub(crate) struct ConversationInputPage {
    schema_version: u32,
    items: Vec<ConversationInputRecord>,
    next_after_input_seq: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConversationInputListQuery {
    conversation_id: String,
    #[serde(default = "default_agent_id")]
    agent_id: String,
    channel: String,
    #[serde(default)]
    channel_account_id: String,
    #[serde(default)]
    client_message_id: Option<String>,
    #[serde(default)]
    after_input_seq: u64,
    #[serde(default = "default_page_limit")]
    limit: u32,
}

#[derive(Debug, Serialize)]
pub(crate) struct ConversationInputEventPage {
    schema_version: u32,
    items: Vec<ConversationInputEventRecord>,
    next_after_event_seq: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConversationInputEventListQuery {
    conversation_id: String,
    #[serde(default = "default_agent_id")]
    agent_id: String,
    channel: String,
    #[serde(default)]
    channel_account_id: String,
    #[serde(default)]
    after_event_seq: u64,
    #[serde(default = "default_page_limit")]
    limit: u32,
}

pub(crate) async fn accept_input(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut submission): Json<ConversationInputSubmission>,
) -> (StatusCode, Json<ApiResponse<ConversationInputReceipt>>) {
    let identity =
        match crate::require_auth_identity_for_api::<ConversationInputReceipt>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    if let Err(error) = submission.validate() {
        return crate::api_err(StatusCode::BAD_REQUEST, error_code(error));
    }
    let Some(agent_id) = state.normalize_known_agent_id(Some(&submission.scope.agent_id)) else {
        return crate::api_err(StatusCode::BAD_REQUEST, "conversation_input_agent_unknown");
    };
    submission.scope.agent_id = agent_id;
    let Some(channel) = parse_channel(&submission.scope.channel) else {
        return crate::api_err(
            StatusCode::BAD_REQUEST,
            "conversation_input_channel_unknown",
        );
    };
    if let Err(code) = text_content(&submission.content) {
        return crate::api_err(StatusCode::UNPROCESSABLE_ENTITY, code);
    }
    let scope = OwnedConversationInputScope {
        owner_principal_id: identity.principal_id.clone(),
        conversation: submission.scope.clone(),
    };
    let mut accepted = match crate::repo::conversation_inputs::accept_conversation_input(
        &state.core.db,
        &AcceptConversationInput {
            owner_principal_id: identity.principal_id.clone(),
            submission: submission.clone(),
            preparation_state: ConversationInputPreparationState::Ready,
        },
    ) {
        Ok(accepted) => accepted,
        Err(error) => return store_error(error),
    };
    crate::conversation_input_event_transport::notify(&state);
    let adopt_expected_task = accepted.record.receipt.target_task_id.is_none()
        && submission.delivery_mode == ConversationInputDeliveryMode::Auto
        && submission.expected_task_id.is_some();
    if adopt_expected_task {
        let expected_task_id = submission.expected_task_id.expect("checked above");
        accepted.record = match crate::repo::conversation_inputs::bind_conversation_input_to_task(
            &state.core.db,
            &scope,
            accepted.record.receipt.input_id,
            expected_task_id,
            crate::repo::conversation_inputs::ConversationInputTaskBinding::ExistingActiveTask,
        ) {
            Ok(record) => {
                crate::conversation_input_event_transport::notify(&state);
                record
            }
            Err(error) => return store_error(error),
        };
    }
    if submission.delivery_mode == ConversationInputDeliveryMode::Defer {
        return accepted_response(accepted.record.receipt);
    }

    drive_auto_input(state, headers, identity, scope, accepted.record, channel).await
}

async fn drive_auto_input(
    state: AppState,
    headers: HeaderMap,
    identity: AuthIdentity,
    scope: OwnedConversationInputScope,
    accepted: ConversationInputRecord,
    channel: ChannelKind,
) -> (StatusCode, Json<ApiResponse<ConversationInputReceipt>>) {
    if accepted.receipt.target_task_id.is_some() {
        interrupt_bound_model_turn(&state, &accepted.receipt);
        crate::conversation_input_event_transport::notify(&state);
        return accepted_response(accepted.receipt);
    }

    let (creator_record, claim_token) =
        match crate::repo::conversation_inputs::claim_or_bind_conversation_input_task(
            &state.core.db,
            &scope,
            accepted.receipt.input_id,
        ) {
            Ok(ConversationInputTaskClaimOutcome::Bound(record)) => {
                interrupt_bound_model_turn(&state, &record.receipt);
                crate::conversation_input_event_transport::notify(&state);
                return accepted_response(record.receipt);
            }
            Ok(ConversationInputTaskClaimOutcome::Waiting) => {
                let receipt = wait_for_task_binding(
                    &state,
                    &identity.principal_id,
                    accepted.receipt.input_id,
                )
                .await
                .unwrap_or(accepted.receipt);
                interrupt_bound_model_turn(&state, &receipt);
                crate::conversation_input_event_transport::notify(&state);
                return accepted_response(receipt);
            }
            Ok(ConversationInputTaskClaimOutcome::Creator {
                record,
                claim_token,
            }) => (record, claim_token),
            Err(error) => return store_error(error),
        };

    let creator_input_id = creator_record.receipt.input_id;
    let creator_text = match text_content(&creator_record.content) {
        Ok(text) => text,
        Err(code) => {
            let _ = crate::repo::conversation_inputs::release_conversation_input_task_claim(
                &state.core.db,
                &scope,
                creator_input_id,
                claim_token,
            );
            return crate::api_err(StatusCode::UNPROCESSABLE_ENTITY, code);
        }
    };

    let request = SubmitTaskRequest {
        user_id: None,
        chat_id: None,
        user_key: Some(identity.user_key.clone()),
        channel: Some(channel),
        external_user_id: None,
        external_chat_id: None,
        ingress: None,
        idempotency_key: Some(format!("conversation-input:{}", creator_input_id)),
        kind: TaskKind::Ask,
        payload: json!({
            "text": creator_text,
            "conversation_id": scope.conversation.conversation_id,
            "agent_id": scope.conversation.agent_id,
            "conversation_input_id": creator_input_id,
            "source": "conversation_input",
        }),
    };
    let (task_status, Json(task_response)) =
        crate::submit_task(State(state.clone()), headers, Json(request)).await;
    let Some(task) = task_response.data else {
        let _ = crate::repo::conversation_inputs::release_conversation_input_task_claim(
            &state.core.db,
            &scope,
            creator_input_id,
            claim_token,
        );
        return crate::api_err(
            task_status,
            task_response
                .error
                .unwrap_or_else(|| "conversation_input_task_submit_failed".to_string()),
        );
    };
    match crate::repo::conversation_inputs::complete_conversation_input_task_claim(
        &state.core.db,
        &scope,
        creator_input_id,
        claim_token,
        task.task_id,
    ) {
        Ok(_) => {
            crate::conversation_input_event_transport::notify(&state);
            match crate::repo::conversation_inputs::get_conversation_input(
                &state.core.db,
                &identity.principal_id,
                accepted.receipt.input_id,
            ) {
                Ok(record) => accepted_response(record.receipt),
                Err(error) => store_error(error),
            }
        }
        Err(error) => {
            let _ = crate::repo::conversation_inputs::release_conversation_input_task_claim(
                &state.core.db,
                &scope,
                creator_input_id,
                claim_token,
            );
            store_error(error)
        }
    }
}

fn interrupt_bound_model_turn(state: &AppState, receipt: &ConversationInputReceipt) {
    if receipt.replayed {
        return;
    }
    if let Some(task_id) = receipt.target_task_id {
        if receipt.disposition
            == claw_core::conversation_input::ConversationInputDisposition::Pending
        {
            if let Err(error) =
                crate::repo::conversation_inputs::wake_task_for_pending_conversation_input(
                    &state.core.db,
                    task_id,
                    receipt.input_id,
                )
            {
                warn!(
                    task_id = %task_id,
                    input_id = %receipt.input_id,
                    %error,
                    "conversation_input_checkpoint_wake_failed"
                );
            }
        }
        state
            .worker
            .interrupt_model_turn_for_conversation_input(&task_id.to_string());
    }
}

async fn wait_for_task_binding(
    state: &AppState,
    owner_principal_id: &str,
    input_id: Uuid,
) -> Option<ConversationInputReceipt> {
    const ATTEMPTS: usize = 20;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        match crate::repo::conversation_inputs::get_conversation_input(
            &state.core.db,
            owner_principal_id,
            input_id,
        ) {
            Ok(record) if record.receipt.target_task_id.is_some() => return Some(record.receipt),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

pub(crate) async fn get_input(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(input_id): AxumPath<Uuid>,
) -> (StatusCode, Json<ApiResponse<ConversationInputRecord>>) {
    let identity =
        match crate::require_auth_identity_for_api::<ConversationInputRecord>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    match crate::repo::conversation_inputs::get_conversation_input(
        &state.core.db,
        &identity.principal_id,
        input_id,
    ) {
        Ok(record) => crate::api_ok(record),
        Err(error) => store_error(error),
    }
}

pub(crate) async fn withdraw_input(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(input_id): AxumPath<Uuid>,
) -> (StatusCode, Json<ApiResponse<ConversationInputReceipt>>) {
    let identity =
        match crate::require_auth_identity_for_api::<ConversationInputReceipt>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    match crate::repo::conversation_inputs::withdraw_conversation_input(
        &state.core.db,
        &identity.principal_id,
        input_id,
    ) {
        Ok(record) => {
            if let Some(task_id) = record.receipt.target_task_id {
                state
                    .worker
                    .interrupt_model_turn_for_conversation_input(&task_id.to_string());
            }
            crate::conversation_input_event_transport::notify(&state);
            crate::api_ok(record.receipt)
        }
        Err(error) => store_error(error),
    }
}

pub(crate) async fn activate_input(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(input_id): AxumPath<Uuid>,
) -> (StatusCode, Json<ApiResponse<ConversationInputReceipt>>) {
    let identity =
        match crate::require_auth_identity_for_api::<ConversationInputReceipt>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let activated = match crate::repo::conversation_inputs::activate_deferred_conversation_input(
        &state.core.db,
        &identity.principal_id,
        input_id,
    ) {
        Ok(record) => record,
        Err(error) => return store_error(error),
    };
    crate::conversation_input_event_transport::notify(&state);
    let scope = OwnedConversationInputScope {
        owner_principal_id: identity.principal_id.clone(),
        conversation: activated.receipt.scope.clone(),
    };
    let Some(channel) = parse_channel(&scope.conversation.channel) else {
        return crate::api_err(
            StatusCode::BAD_REQUEST,
            "conversation_input_channel_unknown",
        );
    };
    drive_auto_input(state, headers, identity, scope, activated, channel).await
}

pub(crate) async fn list_inputs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ConversationInputListQuery>,
) -> (StatusCode, Json<ApiResponse<ConversationInputPage>>) {
    let identity =
        match crate::require_auth_identity_for_api::<ConversationInputPage>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let scope = OwnedConversationInputScope {
        owner_principal_id: identity.principal_id,
        conversation: ConversationInputScopeRef {
            conversation_id: query.conversation_id,
            agent_id: query.agent_id,
            channel: query.channel,
            channel_account_id: query.channel_account_id,
        },
    };
    if let Some(client_message_id) = query
        .client_message_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return match crate::repo::conversation_inputs::get_conversation_input_by_client_message_id(
            &state.core.db,
            &scope,
            client_message_id,
        ) {
            Ok(record) => crate::api_ok(ConversationInputPage {
                schema_version: 1,
                items: vec![record],
                next_after_input_seq: None,
            }),
            Err(error) => store_error(error),
        };
    }
    match crate::repo::conversation_inputs::list_conversation_inputs(
        &state.core.db,
        &scope,
        query.after_input_seq,
        query.limit,
    ) {
        Ok(items) => {
            let next_after_input_seq = (items.len() == query.limit.clamp(1, 100) as usize)
                .then(|| items.last().map(|item| item.receipt.input_seq))
                .flatten();
            crate::api_ok(ConversationInputPage {
                schema_version: 1,
                items,
                next_after_input_seq,
            })
        }
        Err(error) => store_error(error),
    }
}

pub(crate) async fn list_input_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ConversationInputEventListQuery>,
) -> (StatusCode, Json<ApiResponse<ConversationInputEventPage>>) {
    let identity = match crate::require_auth_identity_for_api::<ConversationInputEventPage>(
        &state, &headers,
    ) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let scope = OwnedConversationInputScope {
        owner_principal_id: identity.principal_id,
        conversation: ConversationInputScopeRef {
            conversation_id: query.conversation_id,
            agent_id: query.agent_id,
            channel: query.channel,
            channel_account_id: query.channel_account_id,
        },
    };
    match crate::repo::conversation_inputs::list_conversation_input_events(
        &state.core.db,
        &scope,
        query.after_event_seq,
        query.limit,
    ) {
        Ok(items) => {
            let next_after_event_seq = (items.len() == query.limit.clamp(1, 100) as usize)
                .then(|| items.last().map(|item| item.event_seq))
                .flatten();
            crate::api_ok(ConversationInputEventPage {
                schema_version: 1,
                items,
                next_after_event_seq,
            })
        }
        Err(error) => store_error(error),
    }
}

fn text_content(content: &[ConversationInputContent]) -> Result<String, &'static str> {
    let mut parts = Vec::new();
    for item in content {
        match item {
            ConversationInputContent::Text { text } => parts.push(text.as_str()),
            ConversationInputContent::Attachment { .. } => {
                return Err("conversation_input_attachment_submission_not_ready")
            }
        }
    }
    if parts.is_empty() {
        return Err("conversation_input_text_required");
    }
    Ok(parts.join("\n"))
}

pub(crate) fn parse_channel(value: &str) -> Option<ChannelKind> {
    match value {
        "telegram" => Some(ChannelKind::Telegram),
        "whatsapp" => Some(ChannelKind::Whatsapp),
        "ui" => Some(ChannelKind::Ui),
        "wechat" => Some(ChannelKind::Wechat),
        "feishu" => Some(ChannelKind::Feishu),
        "lark" => Some(ChannelKind::Lark),
        _ => None,
    }
}

fn error_code(code: claw_core::conversation_input::ConversationInputErrorCode) -> &'static str {
    match code {
        claw_core::conversation_input::ConversationInputErrorCode::InvalidRequest => {
            "conversation_input_invalid_request"
        }
        claw_core::conversation_input::ConversationInputErrorCode::Unauthorized => {
            "conversation_input_unauthorized"
        }
        claw_core::conversation_input::ConversationInputErrorCode::TargetConflict => {
            "conversation_input_target_conflict"
        }
        claw_core::conversation_input::ConversationInputErrorCode::IdempotencyConflict => {
            "conversation_input_idempotency_conflict"
        }
        claw_core::conversation_input::ConversationInputErrorCode::PreparationFailed => {
            "conversation_input_preparation_failed"
        }
        claw_core::conversation_input::ConversationInputErrorCode::CapacityExceeded => {
            "conversation_input_capacity_exceeded"
        }
        claw_core::conversation_input::ConversationInputErrorCode::ModelUnavailable => {
            "conversation_input_model_unavailable"
        }
        claw_core::conversation_input::ConversationInputErrorCode::NotFound => {
            "conversation_input_not_found"
        }
        claw_core::conversation_input::ConversationInputErrorCode::DatabaseFailed => {
            "conversation_input_database_failed"
        }
    }
}

fn accepted_response(
    receipt: ConversationInputReceipt,
) -> (StatusCode, Json<ApiResponse<ConversationInputReceipt>>) {
    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            ok: true,
            data: Some(receipt),
            error: None,
        }),
    )
}

fn store_error<T: Serialize>(
    error: ConversationInputStoreError,
) -> (StatusCode, Json<ApiResponse<T>>) {
    let status = match &error {
        ConversationInputStoreError::InvalidRequest => StatusCode::BAD_REQUEST,
        ConversationInputStoreError::IdempotencyConflict
        | ConversationInputStoreError::TargetConflict => StatusCode::CONFLICT,
        ConversationInputStoreError::CapacityExceeded => StatusCode::TOO_MANY_REQUESTS,
        ConversationInputStoreError::AuthorizationRevoked => StatusCode::UNAUTHORIZED,
        ConversationInputStoreError::NotFound => StatusCode::NOT_FOUND,
        ConversationInputStoreError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    if matches!(error, ConversationInputStoreError::Database(_)) {
        tracing::error!(%error, "conversation_input_store_failed");
    }
    crate::api_err(status, error_code(error.code()))
}

fn default_agent_id() -> String {
    crate::DEFAULT_AGENT_ID.to_string()
}

const fn default_page_limit() -> u32 {
    50
}

#[cfg(test)]
#[path = "conversation_inputs_tests.rs"]
mod tests;
