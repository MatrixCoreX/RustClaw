use anyhow::{anyhow, Context};
use claw_core::channel_delivery::{
    ChannelDeliveryEnvelope, ChannelDeliveryReceipt, ChannelDeliveryStatus,
};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::db_init::DbPool;

const INIT_CHANNEL_DELIVERY_RECEIPT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS channel_delivery_receipts (
    idempotency_key TEXT PRIMARY KEY,
    delivery_id     TEXT NOT NULL,
    channel         TEXT NOT NULL,
    adapter         TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('accepted', 'delivered', 'read', 'failed', 'partial')),
    receipt_json    TEXT NOT NULL,
    receipt_digest  TEXT NOT NULL,
    created_at_ts   INTEGER NOT NULL,
    updated_at_ts   INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_channel_delivery_receipts_delivery_id
    ON channel_delivery_receipts(delivery_id);
CREATE INDEX IF NOT EXISTS idx_channel_delivery_receipts_status_updated
    ON channel_delivery_receipts(status, updated_at_ts);

CREATE TABLE IF NOT EXISTS channel_delivery_receipt_events (
    event_id        INTEGER PRIMARY KEY AUTOINCREMENT,
    idempotency_key TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('accepted', 'delivered', 'read', 'failed', 'partial')),
    receipt_json    TEXT NOT NULL,
    receipt_digest  TEXT NOT NULL,
    recorded_at_ts  INTEGER NOT NULL,
    UNIQUE (idempotency_key, receipt_digest),
    FOREIGN KEY (idempotency_key) REFERENCES channel_delivery_receipts(idempotency_key) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_channel_delivery_receipt_events_key_event
    ON channel_delivery_receipt_events(idempotency_key, event_id);

CREATE TABLE IF NOT EXISTS channel_delivery_dispatch_claims (
    idempotency_key TEXT PRIMARY KEY,
    delivery_id     TEXT NOT NULL,
    channel         TEXT NOT NULL,
    adapter         TEXT NOT NULL,
    state           TEXT NOT NULL CHECK (state IN ('dispatching', 'receipt_recorded', 'query_required')),
    lease_token     TEXT,
    lease_expires_at_ts INTEGER NOT NULL DEFAULT 0,
    attempt_count   INTEGER NOT NULL DEFAULT 0,
    created_at_ts   INTEGER NOT NULL,
    updated_at_ts   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_channel_delivery_dispatch_claims_state_lease
    ON channel_delivery_dispatch_claims(state, lease_expires_at_ts);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordChannelDeliveryReceiptOutcome {
    Inserted,
    Updated,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClaimChannelDeliveryDispatchOutcome {
    Acquired { lease_token: String },
    ExistingReceipt(ChannelDeliveryReceipt),
    InProgress,
    QueryRequired,
}

pub(crate) fn ensure_channel_delivery_receipt_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(INIT_CHANNEL_DELIVERY_RECEIPT_SQL)?;
    Ok(())
}

pub(crate) fn record_channel_delivery_receipt(
    pool: &DbPool,
    receipt: &ChannelDeliveryReceipt,
) -> anyhow::Result<RecordChannelDeliveryReceiptOutcome> {
    let db = pool
        .get()
        .context("channel_delivery_receipt_db_pool_failed")?;
    ensure_channel_delivery_receipt_schema(&db)?;
    record_channel_delivery_receipt_in_db(&db, receipt)
}

pub(crate) fn claim_channel_delivery_dispatch(
    pool: &DbPool,
    envelope: &ChannelDeliveryEnvelope,
    lease_seconds: u64,
) -> anyhow::Result<ClaimChannelDeliveryDispatchOutcome> {
    let db = pool
        .get()
        .context("channel_delivery_receipt_db_pool_failed")?;
    ensure_channel_delivery_receipt_schema(&db)?;
    claim_channel_delivery_dispatch_in_db(&db, envelope, crate::now_ts_u64(), lease_seconds)
}

pub(crate) fn complete_channel_delivery_dispatch(
    pool: &DbPool,
    idempotency_key: &str,
    lease_token: &str,
) -> anyhow::Result<()> {
    let db = pool
        .get()
        .context("channel_delivery_receipt_db_pool_failed")?;
    ensure_channel_delivery_receipt_schema(&db)?;
    complete_channel_delivery_dispatch_in_db(&db, idempotency_key, lease_token, crate::now_ts_u64())
}

fn load_channel_delivery_receipt_from_db(
    db: &Connection,
    idempotency_key: &str,
) -> anyhow::Result<Option<ChannelDeliveryReceipt>> {
    let idempotency_key = required_idempotency_key(idempotency_key)?;
    let stored = db
        .query_row(
            "SELECT receipt_json, receipt_digest
             FROM channel_delivery_receipts
             WHERE idempotency_key = ?1
             LIMIT 1",
            params![idempotency_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    stored
        .map(|(receipt_json, receipt_digest)| decode_stored_receipt(&receipt_json, &receipt_digest))
        .transpose()
}

fn record_channel_delivery_receipt_in_db(
    db: &Connection,
    receipt: &ChannelDeliveryReceipt,
) -> anyhow::Result<RecordChannelDeliveryReceiptOutcome> {
    receipt.validate().map_err(|err| anyhow!(err.to_string()))?;
    let idempotency_key = required_idempotency_key(&receipt.idempotency_key)?;
    let receipt_json =
        serde_json::to_string(receipt).context("channel_delivery_receipt_serialize_failed")?;
    let receipt_digest = receipt_digest(&receipt_json);
    let channel = channel_token(receipt)?;
    let status = status_token(receipt.status);
    let updated_at_ts = i64::try_from(receipt.updated_at_ts)
        .map_err(|_| anyhow!("channel_delivery_receipt_timestamp_out_of_range"))?;

    let tx = db.unchecked_transaction()?;
    let existing = tx
        .query_row(
            "SELECT receipt_json, receipt_digest
             FROM channel_delivery_receipts
             WHERE idempotency_key = ?1
             LIMIT 1",
            params![idempotency_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let outcome = if let Some((existing_json, existing_digest)) = existing {
        if existing_digest == receipt_digest {
            tx.commit()?;
            return Ok(RecordChannelDeliveryReceiptOutcome::Unchanged);
        }
        let existing_receipt = decode_stored_receipt(&existing_json, &existing_digest)?;
        ensure_same_delivery_identity(&existing_receipt, receipt)?;
        ensure_receipt_transition(&existing_receipt, receipt)?;
        tx.execute(
            "UPDATE channel_delivery_receipts
             SET status = ?2,
                 receipt_json = ?3,
                 receipt_digest = ?4,
                 updated_at_ts = ?5
             WHERE idempotency_key = ?1",
            params![
                idempotency_key,
                status,
                receipt_json,
                receipt_digest,
                updated_at_ts,
            ],
        )?;
        RecordChannelDeliveryReceiptOutcome::Updated
    } else {
        tx.execute(
            "INSERT INTO channel_delivery_receipts (
                 idempotency_key, delivery_id, channel, adapter, status,
                 receipt_json, receipt_digest, created_at_ts, updated_at_ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                idempotency_key,
                receipt.delivery_id,
                channel,
                receipt.adapter,
                status,
                receipt_json,
                receipt_digest,
                updated_at_ts,
            ],
        )?;
        RecordChannelDeliveryReceiptOutcome::Inserted
    };
    tx.execute(
        "INSERT INTO channel_delivery_receipt_events (
             idempotency_key, status, receipt_json, receipt_digest, recorded_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            idempotency_key,
            status,
            receipt_json,
            receipt_digest,
            updated_at_ts,
        ],
    )?;
    tx.commit()?;
    Ok(outcome)
}

fn claim_channel_delivery_dispatch_in_db(
    db: &Connection,
    envelope: &ChannelDeliveryEnvelope,
    now_ts: u64,
    lease_seconds: u64,
) -> anyhow::Result<ClaimChannelDeliveryDispatchOutcome> {
    envelope
        .validate()
        .map_err(|err| anyhow!(err.to_string()))?;
    let idempotency_key = required_idempotency_key(&envelope.idempotency_key)?;
    if let Some(receipt) = load_channel_delivery_receipt_from_db(db, idempotency_key)? {
        ensure_envelope_receipt_identity(envelope, &receipt)?;
        return Ok(ClaimChannelDeliveryDispatchOutcome::ExistingReceipt(
            receipt,
        ));
    }
    let now_ts = i64::try_from(now_ts)
        .map_err(|_| anyhow!("channel_delivery_dispatch_timestamp_out_of_range"))?;
    let lease_seconds = i64::try_from(lease_seconds.max(1))
        .map_err(|_| anyhow!("channel_delivery_dispatch_lease_out_of_range"))?;
    let lease_expires_at_ts = now_ts.saturating_add(lease_seconds);
    let channel = envelope_channel_token(envelope);
    let tx = db.unchecked_transaction()?;
    let existing = tx
        .query_row(
            "SELECT delivery_id, channel, adapter, state, lease_expires_at_ts
             FROM channel_delivery_dispatch_claims
             WHERE idempotency_key = ?1
             LIMIT 1",
            params![idempotency_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?;
    let outcome =
        if let Some((delivery_id, stored_channel, adapter, state, lease_expires)) = existing {
            if delivery_id != envelope.delivery_id
                || stored_channel != channel
                || adapter != envelope.adapter
            {
                return Err(anyhow!("channel_delivery_dispatch_identity_conflict"));
            }
            match state.as_str() {
                "dispatching" if lease_expires > now_ts => {
                    ClaimChannelDeliveryDispatchOutcome::InProgress
                }
                "dispatching" => {
                    tx.execute(
                        "UPDATE channel_delivery_dispatch_claims
                     SET state = 'query_required', lease_token = NULL,
                         lease_expires_at_ts = 0, updated_at_ts = ?2
                     WHERE idempotency_key = ?1",
                        params![idempotency_key, now_ts],
                    )?;
                    ClaimChannelDeliveryDispatchOutcome::QueryRequired
                }
                "query_required" | "receipt_recorded" => {
                    ClaimChannelDeliveryDispatchOutcome::QueryRequired
                }
                _ => return Err(anyhow!("channel_delivery_dispatch_state_invalid")),
            }
        } else {
            let lease_token = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO channel_delivery_dispatch_claims (
                 idempotency_key, delivery_id, channel, adapter, state, lease_token,
                 lease_expires_at_ts, attempt_count, created_at_ts, updated_at_ts
             ) VALUES (?1, ?2, ?3, ?4, 'dispatching', ?5, ?6, 1, ?7, ?7)",
                params![
                    idempotency_key,
                    envelope.delivery_id,
                    channel,
                    envelope.adapter,
                    lease_token,
                    lease_expires_at_ts,
                    now_ts,
                ],
            )?;
            ClaimChannelDeliveryDispatchOutcome::Acquired { lease_token }
        };
    tx.commit()?;
    Ok(outcome)
}

fn complete_channel_delivery_dispatch_in_db(
    db: &Connection,
    idempotency_key: &str,
    lease_token: &str,
    now_ts: u64,
) -> anyhow::Result<()> {
    let idempotency_key = required_idempotency_key(idempotency_key)?;
    if lease_token.trim().is_empty() {
        return Err(anyhow!("channel_delivery_dispatch_lease_token_required"));
    }
    let now_ts = i64::try_from(now_ts)
        .map_err(|_| anyhow!("channel_delivery_dispatch_timestamp_out_of_range"))?;
    let receipt_exists: i64 = db.query_row(
        "SELECT COUNT(*) FROM channel_delivery_receipts WHERE idempotency_key = ?1",
        params![idempotency_key],
        |row| row.get(0),
    )?;
    if receipt_exists != 1 {
        return Err(anyhow!("channel_delivery_dispatch_receipt_required"));
    }
    let changed = db.execute(
        "UPDATE channel_delivery_dispatch_claims
         SET state = 'receipt_recorded', lease_token = NULL,
             lease_expires_at_ts = 0, updated_at_ts = ?3
         WHERE idempotency_key = ?1
           AND state = 'dispatching'
           AND lease_token = ?2",
        params![idempotency_key, lease_token, now_ts],
    )?;
    if changed != 1 {
        return Err(anyhow!("channel_delivery_dispatch_lease_mismatch"));
    }
    Ok(())
}

fn decode_stored_receipt(
    receipt_json: &str,
    expected_digest: &str,
) -> anyhow::Result<ChannelDeliveryReceipt> {
    if receipt_digest(receipt_json) != expected_digest {
        return Err(anyhow!("channel_delivery_receipt_integrity_mismatch"));
    }
    let receipt = serde_json::from_str::<ChannelDeliveryReceipt>(receipt_json)
        .context("channel_delivery_receipt_parse_failed")?;
    receipt
        .validate()
        .map_err(|err| anyhow!(format!("stored_{}", err)))?;
    Ok(receipt)
}

fn ensure_same_delivery_identity(
    existing: &ChannelDeliveryReceipt,
    next: &ChannelDeliveryReceipt,
) -> anyhow::Result<()> {
    if existing.delivery_id != next.delivery_id
        || existing.idempotency_key != next.idempotency_key
        || existing.channel != next.channel
        || existing.adapter != next.adapter
    {
        return Err(anyhow!("channel_delivery_receipt_identity_conflict"));
    }
    Ok(())
}

fn ensure_envelope_receipt_identity(
    envelope: &ChannelDeliveryEnvelope,
    receipt: &ChannelDeliveryReceipt,
) -> anyhow::Result<()> {
    if envelope.delivery_id != receipt.delivery_id
        || envelope.idempotency_key != receipt.idempotency_key
        || envelope.channel != receipt.channel
        || envelope.adapter != receipt.adapter
    {
        return Err(anyhow!(
            "channel_delivery_envelope_receipt_identity_conflict"
        ));
    }
    Ok(())
}

fn ensure_receipt_transition(
    existing: &ChannelDeliveryReceipt,
    next: &ChannelDeliveryReceipt,
) -> anyhow::Result<()> {
    if next.updated_at_ts < existing.updated_at_ts {
        return Err(anyhow!("channel_delivery_receipt_timestamp_regression"));
    }
    let allowed = match existing.status {
        ChannelDeliveryStatus::Accepted => true,
        ChannelDeliveryStatus::Delivered => matches!(
            next.status,
            ChannelDeliveryStatus::Delivered | ChannelDeliveryStatus::Read
        ),
        ChannelDeliveryStatus::Read => matches!(next.status, ChannelDeliveryStatus::Read),
        ChannelDeliveryStatus::Failed => {
            matches!(next.status, ChannelDeliveryStatus::Failed)
                || (existing.retryable
                    && matches!(
                        next.status,
                        ChannelDeliveryStatus::Accepted
                            | ChannelDeliveryStatus::Partial
                            | ChannelDeliveryStatus::Delivered
                            | ChannelDeliveryStatus::Read
                    ))
        }
        ChannelDeliveryStatus::Partial => {
            matches!(next.status, ChannelDeliveryStatus::Partial)
                || matches!(
                    next.status,
                    ChannelDeliveryStatus::Delivered | ChannelDeliveryStatus::Read
                )
                || (existing.retryable
                    && matches!(
                        next.status,
                        ChannelDeliveryStatus::Accepted | ChannelDeliveryStatus::Failed
                    ))
        }
    };
    if !allowed {
        return Err(anyhow!("channel_delivery_receipt_transition_invalid"));
    }
    Ok(())
}

fn required_idempotency_key(value: &str) -> anyhow::Result<&str> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 200
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.' | b'/')
        })
    {
        return Err(anyhow!("channel_delivery_idempotency_key_invalid"));
    }
    Ok(value)
}

fn channel_token(receipt: &ChannelDeliveryReceipt) -> anyhow::Result<&str> {
    match receipt.channel {
        claw_core::types::ChannelKind::Telegram => Ok("telegram"),
        claw_core::types::ChannelKind::Whatsapp => Ok("whatsapp"),
        claw_core::types::ChannelKind::Ui => Ok("ui"),
        claw_core::types::ChannelKind::Wechat => Ok("wechat"),
        claw_core::types::ChannelKind::Feishu => Ok("feishu"),
        claw_core::types::ChannelKind::Lark => Ok("lark"),
    }
}

fn envelope_channel_token(envelope: &ChannelDeliveryEnvelope) -> &'static str {
    match envelope.channel {
        claw_core::types::ChannelKind::Telegram => "telegram",
        claw_core::types::ChannelKind::Whatsapp => "whatsapp",
        claw_core::types::ChannelKind::Ui => "ui",
        claw_core::types::ChannelKind::Wechat => "wechat",
        claw_core::types::ChannelKind::Feishu => "feishu",
        claw_core::types::ChannelKind::Lark => "lark",
    }
}

fn status_token(status: ChannelDeliveryStatus) -> &'static str {
    match status {
        ChannelDeliveryStatus::Accepted => "accepted",
        ChannelDeliveryStatus::Delivered => "delivered",
        ChannelDeliveryStatus::Read => "read",
        ChannelDeliveryStatus::Failed => "failed",
        ChannelDeliveryStatus::Partial => "partial",
    }
}

fn receipt_digest(receipt_json: &str) -> String {
    format!("{:x}", Sha256::digest(receipt_json.as_bytes()))
}

#[cfg(test)]
#[path = "channel_delivery_receipt_tests.rs"]
mod tests;
