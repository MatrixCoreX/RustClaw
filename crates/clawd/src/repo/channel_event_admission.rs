use anyhow::Context;
use claw_core::channel_event_admission::{
    ChannelEventClaimRequest, ChannelEventFinishOutcome, ChannelEventFinishRequest,
};
use claw_core::types::ChannelKind;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::db_init::DbPool;

const RECEIPT_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;

const INIT_CHANNEL_EVENT_ADMISSION_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS channel_ingress_event_receipts (
    channel             TEXT NOT NULL,
    account_id          TEXT NOT NULL,
    provider_event_id   TEXT NOT NULL,
    payload_sha256      TEXT NOT NULL,
    state               TEXT NOT NULL CHECK (state IN ('processing', 'completed', 'retryable_failed')),
    lease_token         TEXT,
    lease_expires_at_ts INTEGER NOT NULL DEFAULT 0,
    attempt_count       INTEGER NOT NULL DEFAULT 0,
    first_seen_at_ts    INTEGER NOT NULL,
    updated_at_ts       INTEGER NOT NULL,
    completed_at_ts     INTEGER,
    PRIMARY KEY (channel, account_id, provider_event_id)
);
CREATE INDEX IF NOT EXISTS idx_channel_ingress_event_receipts_state_lease
    ON channel_ingress_event_receipts(state, lease_expires_at_ts);
CREATE INDEX IF NOT EXISTS idx_channel_ingress_event_receipts_updated
    ON channel_ingress_event_receipts(updated_at_ts);

CREATE TABLE IF NOT EXISTS channel_ingress_provider_nonces (
    channel             TEXT NOT NULL,
    account_id          TEXT NOT NULL,
    nonce_sha256        TEXT NOT NULL,
    provider_event_id   TEXT NOT NULL,
    payload_sha256      TEXT NOT NULL,
    consumed_at_ts      INTEGER NOT NULL,
    expires_at_ts       INTEGER NOT NULL,
    PRIMARY KEY (channel, account_id, nonce_sha256)
);
CREATE INDEX IF NOT EXISTS idx_channel_ingress_provider_nonces_expiry
    ON channel_ingress_provider_nonces(expires_at_ts);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClaimChannelEventOutcome {
    Acquired {
        lease_token: String,
        lease_expires_at_ts: u64,
    },
    InProgress {
        lease_expires_at_ts: u64,
    },
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FinishChannelEventOutcome {
    Completed,
    Released,
    AlreadyCompleted,
}

#[derive(Debug, Error)]
pub(crate) enum ChannelEventAdmissionError {
    #[error("channel_event_admission_request_invalid")]
    InvalidRequest,
    #[error("channel_event_admission_payload_conflict")]
    PayloadConflict,
    #[error("channel_event_admission_nonce_conflict")]
    NonceConflict,
    #[error("channel_event_admission_receipt_not_found")]
    ReceiptNotFound,
    #[error("channel_event_admission_lease_mismatch")]
    LeaseMismatch,
    #[error("channel_event_admission_lease_expired")]
    LeaseExpired,
    #[error("channel_event_admission_database_failed")]
    Database(#[source] anyhow::Error),
}

pub(crate) fn ensure_channel_event_admission_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(INIT_CHANNEL_EVENT_ADMISSION_SQL)?;
    Ok(())
}

pub(crate) fn claim_channel_event(
    pool: &DbPool,
    request: &ChannelEventClaimRequest,
) -> Result<ClaimChannelEventOutcome, ChannelEventAdmissionError> {
    request
        .validate()
        .map_err(|_| ChannelEventAdmissionError::InvalidRequest)?;
    let mut db = pool
        .get()
        .context("channel_event_admission_db_pool_failed")
        .map_err(ChannelEventAdmissionError::Database)?;
    ensure_channel_event_admission_schema(&db).map_err(ChannelEventAdmissionError::Database)?;
    claim_channel_event_in_db(&mut db, request, crate::now_ts_u64())
}

pub(crate) fn finish_channel_event(
    pool: &DbPool,
    request: &ChannelEventFinishRequest,
) -> Result<FinishChannelEventOutcome, ChannelEventAdmissionError> {
    request
        .validate()
        .map_err(|_| ChannelEventAdmissionError::InvalidRequest)?;
    let mut db = pool
        .get()
        .context("channel_event_admission_db_pool_failed")
        .map_err(ChannelEventAdmissionError::Database)?;
    ensure_channel_event_admission_schema(&db).map_err(ChannelEventAdmissionError::Database)?;
    finish_channel_event_in_db(&mut db, request, crate::now_ts_u64())
}

fn claim_channel_event_in_db(
    db: &mut Connection,
    request: &ChannelEventClaimRequest,
    now_ts: u64,
) -> Result<ClaimChannelEventOutcome, ChannelEventAdmissionError> {
    let channel = channel_token(request.channel);
    let account_id = request.account_id.trim();
    let provider_event_id = request.provider_event_id.trim();
    let payload_sha256 = request.payload_sha256.to_ascii_lowercase();
    let now = to_i64(now_ts)?;
    let lease_expires_at_ts = now_ts
        .checked_add(request.lease_seconds)
        .ok_or(ChannelEventAdmissionError::InvalidRequest)?;
    let lease_expires = to_i64(lease_expires_at_ts)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("channel_event_admission_transaction_failed")
        .map_err(ChannelEventAdmissionError::Database)?;

    prune_expired_records(&tx, now);

    let existing = tx
        .query_row(
            "SELECT payload_sha256, state, lease_expires_at_ts
             FROM channel_ingress_event_receipts
             WHERE channel = ?1 AND account_id = ?2 AND provider_event_id = ?3",
            params![channel, account_id, provider_event_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .context("channel_event_admission_receipt_lookup_failed")
        .map_err(ChannelEventAdmissionError::Database)?;

    if let Some((existing_digest, state, existing_expiry)) = existing {
        if !existing_digest.eq_ignore_ascii_case(&payload_sha256) {
            return Err(ChannelEventAdmissionError::PayloadConflict);
        }
        if state == "completed" {
            tx.commit()
                .context("channel_event_admission_commit_failed")
                .map_err(ChannelEventAdmissionError::Database)?;
            return Ok(ClaimChannelEventOutcome::Completed);
        }
        if state == "processing" && existing_expiry > now {
            tx.commit()
                .context("channel_event_admission_commit_failed")
                .map_err(ChannelEventAdmissionError::Database)?;
            return Ok(ClaimChannelEventOutcome::InProgress {
                lease_expires_at_ts: u64::try_from(existing_expiry)
                    .map_err(|_| ChannelEventAdmissionError::InvalidRequest)?,
            });
        }
    }

    claim_provider_nonce(
        &tx,
        channel,
        account_id,
        provider_event_id,
        &payload_sha256,
        request.provider_nonce.as_deref(),
        now_ts,
    )?;

    let lease_token = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO channel_ingress_event_receipts (
             channel, account_id, provider_event_id, payload_sha256, state, lease_token,
             lease_expires_at_ts, attempt_count, first_seen_at_ts, updated_at_ts, completed_at_ts
         ) VALUES (?1, ?2, ?3, ?4, 'processing', ?5, ?6, 1, ?7, ?7, NULL)
         ON CONFLICT(channel, account_id, provider_event_id) DO UPDATE SET
             state = 'processing',
             lease_token = excluded.lease_token,
             lease_expires_at_ts = excluded.lease_expires_at_ts,
             attempt_count = channel_ingress_event_receipts.attempt_count + 1,
             updated_at_ts = excluded.updated_at_ts,
             completed_at_ts = NULL",
        params![
            channel,
            account_id,
            provider_event_id,
            payload_sha256,
            lease_token,
            lease_expires,
            now
        ],
    )
    .context("channel_event_admission_receipt_write_failed")
    .map_err(ChannelEventAdmissionError::Database)?;
    tx.commit()
        .context("channel_event_admission_commit_failed")
        .map_err(ChannelEventAdmissionError::Database)?;
    Ok(ClaimChannelEventOutcome::Acquired {
        lease_token,
        lease_expires_at_ts,
    })
}

fn finish_channel_event_in_db(
    db: &mut Connection,
    request: &ChannelEventFinishRequest,
    now_ts: u64,
) -> Result<FinishChannelEventOutcome, ChannelEventAdmissionError> {
    let channel = channel_token(request.channel);
    let account_id = request.account_id.trim();
    let provider_event_id = request.provider_event_id.trim();
    let payload_sha256 = request.payload_sha256.to_ascii_lowercase();
    let now = to_i64(now_ts)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("channel_event_admission_transaction_failed")
        .map_err(ChannelEventAdmissionError::Database)?;
    let existing = tx
        .query_row(
            "SELECT payload_sha256, state, lease_token, lease_expires_at_ts
             FROM channel_ingress_event_receipts
             WHERE channel = ?1 AND account_id = ?2 AND provider_event_id = ?3",
            params![channel, account_id, provider_event_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .context("channel_event_admission_receipt_lookup_failed")
        .map_err(ChannelEventAdmissionError::Database)?
        .ok_or(ChannelEventAdmissionError::ReceiptNotFound)?;
    if !existing.0.eq_ignore_ascii_case(&payload_sha256) {
        return Err(ChannelEventAdmissionError::PayloadConflict);
    }
    if existing.1 == "completed" {
        tx.commit()
            .context("channel_event_admission_commit_failed")
            .map_err(ChannelEventAdmissionError::Database)?;
        return Ok(FinishChannelEventOutcome::AlreadyCompleted);
    }
    if existing.2.as_deref() != Some(request.lease_token.trim()) {
        return Err(ChannelEventAdmissionError::LeaseMismatch);
    }
    if existing.3 < now {
        return Err(ChannelEventAdmissionError::LeaseExpired);
    }
    let (state, completed_at_ts, outcome) = match request.outcome {
        ChannelEventFinishOutcome::Completed => {
            ("completed", Some(now), FinishChannelEventOutcome::Completed)
        }
        ChannelEventFinishOutcome::RetryableFailure => (
            "retryable_failed",
            None,
            FinishChannelEventOutcome::Released,
        ),
    };
    tx.execute(
        "UPDATE channel_ingress_event_receipts
         SET state = ?1, lease_token = NULL, lease_expires_at_ts = 0,
             updated_at_ts = ?2, completed_at_ts = ?3
         WHERE channel = ?4 AND account_id = ?5 AND provider_event_id = ?6",
        params![
            state,
            now,
            completed_at_ts,
            channel,
            account_id,
            provider_event_id
        ],
    )
    .context("channel_event_admission_receipt_finish_failed")
    .map_err(ChannelEventAdmissionError::Database)?;
    tx.commit()
        .context("channel_event_admission_commit_failed")
        .map_err(ChannelEventAdmissionError::Database)?;
    Ok(outcome)
}

fn claim_provider_nonce(
    tx: &rusqlite::Transaction<'_>,
    channel: &str,
    account_id: &str,
    provider_event_id: &str,
    payload_sha256: &str,
    provider_nonce: Option<&str>,
    now_ts: u64,
) -> Result<(), ChannelEventAdmissionError> {
    let Some(provider_nonce) = provider_nonce
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let nonce_sha256 = format!("{:x}", Sha256::digest(provider_nonce.as_bytes()));
    let existing = tx
        .query_row(
            "SELECT provider_event_id, payload_sha256
             FROM channel_ingress_provider_nonces
             WHERE channel = ?1 AND account_id = ?2 AND nonce_sha256 = ?3",
            params![channel, account_id, nonce_sha256],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .context("channel_event_admission_nonce_lookup_failed")
        .map_err(ChannelEventAdmissionError::Database)?;
    if let Some((existing_event_id, existing_digest)) = existing {
        if existing_event_id != provider_event_id
            || !existing_digest.eq_ignore_ascii_case(payload_sha256)
        {
            return Err(ChannelEventAdmissionError::NonceConflict);
        }
        return Ok(());
    }
    let consumed = to_i64(now_ts)?;
    let expires = to_i64(now_ts.saturating_add(RECEIPT_RETENTION_SECS))?;
    tx.execute(
        "INSERT INTO channel_ingress_provider_nonces (
             channel, account_id, nonce_sha256, provider_event_id, payload_sha256,
             consumed_at_ts, expires_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            channel,
            account_id,
            nonce_sha256,
            provider_event_id,
            payload_sha256,
            consumed,
            expires
        ],
    )
    .context("channel_event_admission_nonce_write_failed")
    .map_err(ChannelEventAdmissionError::Database)?;
    Ok(())
}

fn prune_expired_records(tx: &rusqlite::Transaction<'_>, now: i64) {
    let retention_cutoff = now.saturating_sub(RECEIPT_RETENTION_SECS as i64);
    let _ = tx.execute(
        "DELETE FROM channel_ingress_provider_nonces WHERE expires_at_ts < ?1",
        params![now],
    );
    let _ = tx.execute(
        "DELETE FROM channel_ingress_event_receipts
         WHERE state IN ('completed', 'retryable_failed') AND updated_at_ts < ?1",
        params![retention_cutoff],
    );
}

fn channel_token(channel: ChannelKind) -> &'static str {
    match channel {
        ChannelKind::Telegram => "telegram",
        ChannelKind::Whatsapp => "whatsapp",
        ChannelKind::Ui => "ui",
        ChannelKind::Wechat => "wechat",
        ChannelKind::Feishu => "feishu",
        ChannelKind::Lark => "lark",
    }
}

fn to_i64(value: u64) -> Result<i64, ChannelEventAdmissionError> {
    i64::try_from(value).map_err(|_| ChannelEventAdmissionError::InvalidRequest)
}

#[cfg(test)]
#[path = "channel_event_admission_tests.rs"]
mod tests;
