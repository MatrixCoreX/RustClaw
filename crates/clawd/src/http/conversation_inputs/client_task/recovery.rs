use std::time::Duration;

use axum::{extract::State, http::HeaderMap, Json};
use claw_core::{
    channel_ingress::ChannelIngressAttachment,
    conversation_input::{
        ConversationInputClientTaskRequest, ConversationInputSubmission,
        OwnedConversationInputScope, CONVERSATION_INPUT_SCHEMA_VERSION,
    },
    types::{AuthIdentity, ChannelKind},
};
use rusqlite::OptionalExtension;
use tracing::{debug, warn};

use super::prepare_creator_task;
use crate::{repo::conversation_inputs::ConversationInputTaskClaimOutcome, AppState};

const RECOVERY_BATCH_SIZE: u32 = 16;
const RECOVERY_FALLBACK_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) fn spawn_task_creation_recovery_worker(state: AppState) {
    tokio::spawn(async move {
        let mut receiver = state
            .metrics
            .conversation_input_event_notifier
            .subscribe(crate::conversation_input_event_transport::GLOBAL_CONVERSATION_EVENT_KEY);
        loop {
            if let Err(error) = recover_task_creations_once(&state).await {
                warn!(%error, "conversation_input_task_recovery_scan_failed");
            }
            tokio::select! {
                _ = tokio::time::sleep(RECOVERY_FALLBACK_INTERVAL) => {}
                _ = receiver.recv() => {}
            }
        }
    });
}

pub(super) async fn recover_task_creations_once(state: &AppState) -> anyhow::Result<usize> {
    let candidates = crate::repo::conversation_inputs::list_recoverable_conversation_input_tasks(
        &state.core.db,
        RECOVERY_BATCH_SIZE,
    )?;
    let mut settled = 0;
    for candidate in candidates {
        let input_id = candidate.record.receipt.input_id;
        let claim = crate::repo::conversation_inputs::claim_or_bind_conversation_input_task(
            &state.core.db,
            &candidate.scope,
            input_id,
        )?;
        let (record, claim_token) = match claim {
            ConversationInputTaskClaimOutcome::Bound(_) => {
                crate::conversation_input_event_transport::notify(state);
                settled += 1;
                continue;
            }
            ConversationInputTaskClaimOutcome::Waiting => continue,
            ConversationInputTaskClaimOutcome::Creator {
                record,
                claim_token,
            } => (record, claim_token),
        };
        if record.receipt.input_id != input_id {
            let _ = crate::repo::conversation_inputs::release_conversation_input_task_claim(
                &state.core.db,
                &candidate.scope,
                record.receipt.input_id,
                claim_token,
            );
            continue;
        }
        let Some(identity) = recovery_identity(state, &candidate.scope, &candidate.template)?
        else {
            if crate::repo::conversation_inputs::reject_unbound_conversation_input_authorization(
                &state.core.db,
                &candidate.scope,
                input_id,
                claim_token,
            )? {
                crate::conversation_input_event_transport::notify(state);
                settled += 1;
            }
            continue;
        };
        match submit_recovered_task(
            state,
            &identity,
            &candidate.scope,
            record,
            candidate.template,
        )
        .await
        {
            Ok(task_id) => {
                match crate::repo::conversation_inputs::complete_conversation_input_task_claim(
                    &state.core.db,
                    &candidate.scope,
                    input_id,
                    claim_token,
                    task_id,
                ) {
                    Ok(_) => {
                        crate::conversation_input_event_transport::notify(state);
                        settled += 1;
                    }
                    Err(error) => {
                        debug!(%input_id, %error, "conversation_input_task_recovery_binding_raced");
                    }
                }
            }
            Err(error_code) => {
                let _ = crate::repo::conversation_inputs::defer_conversation_input_task_recovery(
                    &state.core.db,
                    &candidate.scope,
                    input_id,
                    claim_token,
                    error_code,
                );
                let _ = crate::repo::conversation_inputs::release_conversation_input_task_claim(
                    &state.core.db,
                    &candidate.scope,
                    input_id,
                    claim_token,
                );
            }
        }
    }
    Ok(settled)
}

fn recovery_identity(
    state: &AppState,
    scope: &OwnedConversationInputScope,
    template: &crate::repo::conversation_inputs::ConversationInputTaskTemplate,
) -> anyhow::Result<Option<AuthIdentity>> {
    let channel = template.task.channel;
    let identity = if channel == Some(ChannelKind::Ui) {
        let db = state.core.db.get()?;
        let user_key = db
            .query_row(
                "SELECT user_key FROM auth_keys
                 WHERE principal_id = ?1 AND enabled = 1
                 ORDER BY rowid DESC LIMIT 1",
                [&scope.owner_principal_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        drop(db);
        match user_key {
            Some(user_key) => crate::resolve_auth_identity_by_key(state, &user_key)?,
            None => None,
        }
    } else {
        crate::resolve_channel_binding_identity(
            state,
            &scope.conversation.channel,
            template.task.external_user_id.as_deref(),
            template.task.external_chat_id.as_deref(),
        )?
    };
    Ok(identity.filter(|identity| identity.principal_id == scope.owner_principal_id))
}

async fn submit_recovered_task(
    state: &AppState,
    identity: &AuthIdentity,
    scope: &OwnedConversationInputScope,
    record: claw_core::conversation_input::ConversationInputRecord,
    template: crate::repo::conversation_inputs::ConversationInputTaskTemplate,
) -> Result<uuid::Uuid, &'static str> {
    let bindings = crate::repo::conversation_inputs::conversation_input_attachments(
        &state.core.db,
        record.receipt.input_id,
    )
    .map_err(|_| "conversation_input_attachment_recovery_failed")?;
    let attachments = bindings
        .into_iter()
        .map(|binding| ChannelIngressAttachment {
            kind: binding.kind,
            path: binding.workspace_rel_path,
            mime_type: binding.mime_type,
            size: Some(binding.size_bytes),
        })
        .collect::<Vec<_>>();
    let mut request = ConversationInputClientTaskRequest {
        input: ConversationInputSubmission {
            schema_version: CONVERSATION_INPUT_SCHEMA_VERSION,
            client_message_id: record.receipt.client_message_id.clone(),
            scope: scope.conversation.clone(),
            content: record.content.clone(),
            delivery_mode: record.delivery_mode,
            expected_task_id: record.expected_task_id,
            expected_instruction_revision: record.expected_instruction_revision,
            source: record.source.clone(),
        },
        task: template.task,
    };
    if let Some(ingress) = request.task.ingress.as_mut() {
        ingress.attachments = attachments.clone();
    }
    if let Some(payload) = request.task.payload.as_object_mut() {
        if !attachments.is_empty() {
            payload.insert(
                "attachments".to_string(),
                serde_json::to_value(&attachments)
                    .map_err(|_| "conversation_input_attachment_recovery_failed")?,
            );
        }
    }
    let creator_text = super::client_task_text_content(&record.content);
    prepare_creator_task(
        &mut request,
        identity,
        record.receipt.input_id,
        &creator_text,
        record.source.provider_message_id.as_deref(),
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        identity
            .user_key
            .parse()
            .map_err(|_| "conversation_input_recovery_auth_header_invalid")?,
    );
    if let Some(client_origin) = template.client_origin {
        headers.insert(
            crate::task_execution_policy::CLIENT_ORIGIN_HEADER,
            client_origin
                .parse()
                .map_err(|_| "conversation_input_recovery_header_invalid")?,
        );
    }
    if let Some(execution_mode) = template.execution_mode {
        headers.insert(
            crate::task_execution_policy::EXECUTION_MODE_HEADER,
            execution_mode
                .parse()
                .map_err(|_| "conversation_input_recovery_header_invalid")?,
        );
    }
    let (_, Json(response)) =
        crate::submit_task(State(state.clone()), headers, Json(request.task)).await;
    response
        .data
        .map(|task| task.task_id)
        .ok_or("conversation_input_task_submit_failed")
}
