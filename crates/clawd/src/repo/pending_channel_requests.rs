use claw_core::types::{
    PendingChannelRequestStatus, PendingChannelRequestStoreRequest, SubmitTaskRequest, TaskKind,
};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{now_ts_u64, AppState};

const DEFAULT_TTL_SECONDS: u64 = 300;
const MIN_TTL_SECONDS: u64 = 60;
const MAX_TTL_SECONDS: u64 = 600;
const MAX_REQUEST_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_IDEMPOTENCY_KEY_CHARS: usize = 512;

#[derive(Debug)]
pub(crate) struct PendingChannelResumeCandidate {
    pub(crate) status: PendingChannelRequestStatus,
    pub(crate) request: Option<SubmitTaskRequest>,
}

pub(crate) fn store_pending_channel_request(
    state: &AppState,
    input: &PendingChannelRequestStoreRequest,
) -> anyhow::Result<PendingChannelRequestStatus> {
    validate_store_request(input)?;
    let mut stored_request = input.request.clone();
    stored_request.idempotency_key = Some(input.idempotency_key.trim().to_string());
    let request_json = serde_json::to_string(&stored_request)?;
    anyhow::ensure!(
        request_json.len() <= MAX_REQUEST_JSON_BYTES,
        "pending_channel_request_too_large"
    );
    let ingress = stored_request.ingress.as_ref().expect("validated ingress");
    let now = now_ts_u64() as i64;
    let ttl = input
        .expires_in_seconds
        .unwrap_or(DEFAULT_TTL_SECONDS)
        .clamp(MIN_TTL_SECONDS, MAX_TTL_SECONDS) as i64;
    let expires_at = now.saturating_add(ttl);
    let request_id = Uuid::new_v4();
    let content_digest = hex::encode(Sha256::digest(request_json.as_bytes()));
    let attachment_refs_json = serde_json::to_string(&ingress.attachments)?;
    let channel = crate::repo::submit::channel_kind_name(ingress.channel);
    let external_user_id = ingress.external_user_id.as_deref();
    let external_chat_id = ingress.external_chat_id.as_deref();
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;

    if let Some(existing) = pending_status_by_idempotency_key(&db, &input.idempotency_key)? {
        return Ok(existing);
    }

    let tx = db.unchecked_transaction()?;
    tx.execute(
        "UPDATE pending_channel_requests
         SET status = 'invalid', error_code = 'pending_request_superseded', updated_at = ?1
         WHERE channel = ?2
           AND COALESCE(external_user_id, '') = COALESCE(?3, '')
           AND COALESCE(external_chat_id, '') = COALESCE(?4, '')
           AND status = 'pending'",
        params![now, channel, external_user_id, external_chat_id],
    )?;
    tx.execute(
        "INSERT INTO pending_channel_requests (
             pending_request_id, channel, adapter, external_user_id, external_chat_id,
             message_id, content_digest, attachment_refs_json, context_token, request_json,
             idempotency_key, status, task_id, error_code, created_at, updated_at, expires_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending', NULL, NULL, ?12, ?12, ?13)",
        params![
            request_id.to_string(),
            channel,
            ingress.adapter,
            external_user_id,
            external_chat_id,
            ingress.message_id,
            content_digest,
            attachment_refs_json,
            ingress.context_token,
            request_json,
            input.idempotency_key.trim(),
            now,
            expires_at,
        ],
    )?;
    tx.commit()?;
    Ok(PendingChannelRequestStatus {
        pending_request_id: request_id,
        status: "pending".to_string(),
        expires_at,
        external_user_id: external_user_id.map(ToString::to_string),
        external_chat_id: external_chat_id.map(ToString::to_string),
        context_token: ingress.context_token.clone(),
        task_id: None,
        error_code: None,
    })
}

pub(crate) fn pending_channel_resume_candidate(
    state: &AppState,
    channel: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
) -> anyhow::Result<Option<PendingChannelResumeCandidate>> {
    let now = now_ts_u64() as i64;
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    let row = db
        .query_row(
            "SELECT pending_request_id, status, expires_at, task_id, error_code, request_json,
                    external_user_id, external_chat_id, context_token, content_digest
             FROM pending_channel_requests
             WHERE channel = ?1
               AND ((?2 IS NOT NULL AND external_user_id = ?2)
                    OR (?2 IS NULL AND external_chat_id = ?3))
               AND status IN ('pending', 'submitted', 'expired')
             ORDER BY CASE
                        WHEN COALESCE(external_chat_id, '') = COALESCE(?3, '') THEN 0
                        ELSE 1
                      END,
                      created_at DESC
             LIMIT 1",
            params![channel, external_user_id, external_chat_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        request_id,
        mut status,
        expires_at,
        task_id,
        mut error_code,
        request_json,
        stored_external_user_id,
        stored_external_chat_id,
        stored_context_token,
        content_digest,
    )) = row
    else {
        return Ok(None);
    };
    if status == "pending" && expires_at <= now {
        status = "expired".to_string();
        error_code = Some("pending_request_expired".to_string());
        db.execute(
            "UPDATE pending_channel_requests
             SET status = 'expired', error_code = ?2, updated_at = ?3
             WHERE pending_request_id = ?1 AND status = 'pending'",
            params![request_id, error_code, now],
        )?;
    }
    if status == "pending" && hex::encode(Sha256::digest(request_json.as_bytes())) != content_digest
    {
        status = "invalid".to_string();
        error_code = Some("pending_request_digest_mismatch".to_string());
        db.execute(
            "UPDATE pending_channel_requests
             SET status = 'invalid', error_code = ?2, updated_at = ?3
             WHERE pending_request_id = ?1 AND status = 'pending'",
            params![request_id, error_code, now],
        )?;
    }
    let status_value = PendingChannelRequestStatus {
        pending_request_id: Uuid::parse_str(&request_id)?,
        status: status.clone(),
        expires_at,
        external_user_id: stored_external_user_id,
        external_chat_id: stored_external_chat_id,
        context_token: stored_context_token,
        task_id: task_id.as_deref().map(Uuid::parse_str).transpose()?,
        error_code,
    };
    let request = if status == "pending" {
        Some(serde_json::from_str(&request_json)?)
    } else {
        None
    };
    Ok(Some(PendingChannelResumeCandidate {
        status: status_value,
        request,
    }))
}

pub(crate) fn finish_pending_channel_resume(
    state: &AppState,
    pending_request_id: Uuid,
    task_id: Option<Uuid>,
    error_code: Option<&str>,
) -> anyhow::Result<PendingChannelRequestStatus> {
    let now = now_ts_u64() as i64;
    let (status, stable_error) = if task_id.is_some() {
        ("submitted", None)
    } else {
        ("invalid", error_code.or(Some("pending_request_invalid")))
    };
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    db.execute(
        "UPDATE pending_channel_requests
         SET status = ?2, task_id = ?3, error_code = ?4, updated_at = ?5
         WHERE pending_request_id = ?1 AND status = 'pending'",
        params![
            pending_request_id.to_string(),
            status,
            task_id.map(|value| value.to_string()),
            stable_error,
            now,
        ],
    )?;
    pending_status_by_id(&db, pending_request_id)?
        .ok_or_else(|| anyhow::anyhow!("pending_channel_request_missing_after_resume"))
}

fn validate_store_request(input: &PendingChannelRequestStoreRequest) -> anyhow::Result<()> {
    let idempotency_key = input.idempotency_key.trim();
    anyhow::ensure!(!idempotency_key.is_empty(), "idempotency_key_required");
    anyhow::ensure!(
        idempotency_key.chars().count() <= MAX_IDEMPOTENCY_KEY_CHARS,
        "idempotency_key_too_long"
    );
    anyhow::ensure!(
        input.request.user_key.is_none(),
        "pending_request_user_key_forbidden"
    );
    anyhow::ensure!(
        !matches!(input.request.kind, TaskKind::Admin),
        "pending_request_admin_forbidden"
    );
    let ingress = input
        .request
        .ingress
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("channel_ingress_required"))?;
    anyhow::ensure!(
        input.request.channel.is_none() || input.request.channel == Some(ingress.channel),
        "channel_ingress_channel_conflict"
    );
    anyhow::ensure!(
        !ingress.adapter.trim().is_empty(),
        "channel_ingress_adapter_required"
    );
    anyhow::ensure!(
        ingress
            .external_user_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            && ingress
                .external_chat_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
        "pending_request_external_identity_required"
    );
    anyhow::ensure!(
        ingress
            .message_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        "pending_request_message_id_required"
    );
    Ok(())
}

fn pending_status_by_idempotency_key(
    db: &rusqlite::Connection,
    idempotency_key: &str,
) -> anyhow::Result<Option<PendingChannelRequestStatus>> {
    db.query_row(
        "SELECT pending_request_id, status, expires_at, task_id, error_code,
                external_user_id, external_chat_id, context_token
         FROM pending_channel_requests WHERE idempotency_key = ?1",
        params![idempotency_key.trim()],
        status_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn pending_status_by_id(
    db: &rusqlite::Connection,
    pending_request_id: Uuid,
) -> anyhow::Result<Option<PendingChannelRequestStatus>> {
    db.query_row(
        "SELECT pending_request_id, status, expires_at, task_id, error_code,
                external_user_id, external_chat_id, context_token
         FROM pending_channel_requests WHERE pending_request_id = ?1",
        params![pending_request_id.to_string()],
        status_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn status_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingChannelRequestStatus> {
    let pending_request_id = row.get::<_, String>(0)?;
    let task_id = row.get::<_, Option<String>>(3)?;
    Ok(PendingChannelRequestStatus {
        pending_request_id: Uuid::parse_str(&pending_request_id).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        status: row.get(1)?,
        expires_at: row.get(2)?,
        external_user_id: row.get(5)?,
        external_chat_id: row.get(6)?,
        context_token: row.get(7)?,
        task_id: task_id
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        error_code: row.get(4)?,
    })
}

pub(crate) fn request_attachment_paths(request: &SubmitTaskRequest) -> impl Iterator<Item = &str> {
    request
        .ingress
        .as_ref()
        .into_iter()
        .flat_map(|ingress| ingress.attachments.iter())
        .map(|attachment| attachment.path.as_str())
        .chain(
            request
                .payload
                .get("attachments")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|attachment| attachment.get("path").and_then(Value::as_str)),
        )
}

#[cfg(test)]
#[path = "pending_channel_requests_tests.rs"]
mod tests;
