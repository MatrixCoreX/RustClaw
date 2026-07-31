use anyhow::{anyhow, Context};
use claw_core::channel_delivery::{
    ChannelConversationWindow, ChannelConversationWindowState, ChannelDeliveryEnvelope,
    ChannelDeliveryReceipt, ChannelDeliveryStatus,
};
use claw_core::channel_whatsapp_cloud::{
    customer_service_window_expires_at, customer_service_window_is_open,
    WhatsappDeliveryEventStatus,
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

CREATE TABLE IF NOT EXISTS channel_delivery_provider_messages (
    provider_message_id TEXT PRIMARY KEY,
    idempotency_key     TEXT NOT NULL,
    status              TEXT NOT NULL CHECK (status IN ('accepted', 'delivered', 'read', 'failed')),
    event_at_ts         INTEGER NOT NULL,
    provider_error_code TEXT,
    diagnostic_id       TEXT,
    updated_at_ts       INTEGER NOT NULL,
    FOREIGN KEY (idempotency_key) REFERENCES channel_delivery_receipts(idempotency_key) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_channel_delivery_provider_messages_key
    ON channel_delivery_provider_messages(idempotency_key, status);

CREATE TABLE IF NOT EXISTS whatsapp_cloud_conversation_windows (
    phone_number_id    TEXT NOT NULL,
    external_user_id  TEXT NOT NULL,
    last_inbound_at_ts INTEGER NOT NULL,
    expires_at_ts      INTEGER NOT NULL,
    updated_at_ts      INTEGER NOT NULL,
    PRIMARY KEY (phone_number_id, external_user_id)
);
CREATE INDEX IF NOT EXISTS idx_whatsapp_cloud_conversation_windows_expiry
    ON whatsapp_cloud_conversation_windows(expires_at_ts);

CREATE TABLE IF NOT EXISTS whatsapp_cloud_pending_provider_statuses (
    provider_message_id TEXT PRIMARY KEY,
    status              TEXT NOT NULL CHECK (status IN ('accepted', 'delivered', 'read', 'failed')),
    event_at_ts         INTEGER NOT NULL,
    provider_error_code TEXT,
    updated_at_ts       INTEGER NOT NULL
);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordWhatsappProviderStatusOutcome {
    Updated,
    Unchanged,
    UnknownMessage,
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
    let outcome = record_channel_delivery_receipt_in_db(&db, receipt)?;
    replay_pending_whatsapp_statuses(&db, &receipt.provider_message_ids)?;
    Ok(outcome)
}

pub(crate) fn record_whatsapp_cloud_inbound(
    pool: &DbPool,
    phone_number_id: &str,
    external_user_id: &str,
    received_at_ts: u64,
) -> anyhow::Result<()> {
    let phone_number_id = required_provider_identity(phone_number_id)?;
    let external_user_id = required_provider_identity(external_user_id)?;
    if received_at_ts == 0 {
        return Err(anyhow!("whatsapp_cloud_inbound_timestamp_invalid"));
    }
    let expires_at_ts = customer_service_window_expires_at(received_at_ts);
    let received_at_ts = i64::try_from(received_at_ts)
        .map_err(|_| anyhow!("whatsapp_cloud_inbound_timestamp_out_of_range"))?;
    let expires_at_ts = i64::try_from(expires_at_ts)
        .map_err(|_| anyhow!("whatsapp_cloud_window_expiry_out_of_range"))?;
    let db = pool.get().context("whatsapp_cloud_window_db_pool_failed")?;
    ensure_channel_delivery_receipt_schema(&db)?;
    db.execute(
        "INSERT INTO whatsapp_cloud_conversation_windows (
             phone_number_id, external_user_id, last_inbound_at_ts, expires_at_ts, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?3)
         ON CONFLICT(phone_number_id, external_user_id) DO UPDATE SET
             last_inbound_at_ts = excluded.last_inbound_at_ts,
             expires_at_ts = excluded.expires_at_ts,
             updated_at_ts = excluded.updated_at_ts
         WHERE excluded.last_inbound_at_ts > whatsapp_cloud_conversation_windows.last_inbound_at_ts",
        params![phone_number_id, external_user_id, received_at_ts, expires_at_ts],
    )?;
    Ok(())
}

pub(crate) fn whatsapp_cloud_conversation_window(
    pool: &DbPool,
    phone_number_id: &str,
    external_user_id: &str,
    now_ts: u64,
) -> anyhow::Result<ChannelConversationWindow> {
    let phone_number_id = required_provider_identity(phone_number_id)?;
    let external_user_id = required_provider_identity(external_user_id)?;
    let db = pool.get().context("whatsapp_cloud_window_db_pool_failed")?;
    ensure_channel_delivery_receipt_schema(&db)?;
    let stored = db
        .query_row(
            "SELECT last_inbound_at_ts, expires_at_ts
             FROM whatsapp_cloud_conversation_windows
             WHERE phone_number_id = ?1 AND external_user_id = ?2
             LIMIT 1",
            params![phone_number_id, external_user_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((last_inbound_at_ts, expires_at_ts)) = stored else {
        return Ok(ChannelConversationWindow {
            state: ChannelConversationWindowState::Unknown,
            expires_at_ts: None,
            context_token: None,
        });
    };
    let last_inbound_at_ts = u64::try_from(last_inbound_at_ts)
        .map_err(|_| anyhow!("whatsapp_cloud_window_timestamp_invalid"))?;
    let expires_at_ts = u64::try_from(expires_at_ts)
        .map_err(|_| anyhow!("whatsapp_cloud_window_expiry_invalid"))?;
    Ok(ChannelConversationWindow {
        state: if customer_service_window_is_open(last_inbound_at_ts, now_ts) {
            ChannelConversationWindowState::Open
        } else {
            ChannelConversationWindowState::Closed
        },
        expires_at_ts: Some(expires_at_ts),
        context_token: None,
    })
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
            sync_provider_message_ids(
                &tx,
                idempotency_key,
                updated_at_ts,
                &receipt.provider_message_ids,
            )?;
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
    sync_provider_message_ids(
        &tx,
        idempotency_key,
        updated_at_ts,
        &receipt.provider_message_ids,
    )?;
    tx.commit()?;
    Ok(outcome)
}

fn sync_provider_message_ids(
    db: &Connection,
    idempotency_key: &str,
    updated_at_ts: i64,
    provider_message_ids: &[String],
) -> anyhow::Result<()> {
    for provider_message_id in provider_message_ids {
        let provider_message_id = required_provider_identity(provider_message_id)?;
        let existing_key = db
            .query_row(
                "SELECT idempotency_key
                 FROM channel_delivery_provider_messages
                 WHERE provider_message_id = ?1
                 LIMIT 1",
                params![provider_message_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if existing_key
            .as_deref()
            .is_some_and(|existing| existing != idempotency_key)
        {
            return Err(anyhow!(
                "channel_delivery_provider_message_identity_conflict"
            ));
        }
        db.execute(
            "INSERT OR IGNORE INTO channel_delivery_provider_messages (
                 provider_message_id, idempotency_key, status, event_at_ts,
                 provider_error_code, diagnostic_id, updated_at_ts
             ) VALUES (?1, ?2, 'accepted', ?3, NULL, NULL, ?3)",
            params![provider_message_id, idempotency_key, updated_at_ts],
        )?;
    }
    Ok(())
}

pub(crate) fn record_whatsapp_cloud_provider_status(
    pool: &DbPool,
    provider_message_id: &str,
    status: WhatsappDeliveryEventStatus,
    event_at_ts: u64,
    provider_error_code: Option<&str>,
) -> anyhow::Result<RecordWhatsappProviderStatusOutcome> {
    let provider_message_id = required_provider_identity(provider_message_id)?;
    if event_at_ts == 0 {
        return Err(anyhow!("whatsapp_cloud_status_timestamp_invalid"));
    }
    let db = pool
        .get()
        .context("channel_delivery_receipt_db_pool_failed")?;
    ensure_channel_delivery_receipt_schema(&db)?;
    record_whatsapp_cloud_provider_status_in_db(
        &db,
        provider_message_id,
        status,
        event_at_ts,
        provider_error_code,
        true,
    )
}

fn record_whatsapp_cloud_provider_status_in_db(
    db: &Connection,
    provider_message_id: &str,
    status: WhatsappDeliveryEventStatus,
    event_at_ts: u64,
    provider_error_code: Option<&str>,
    store_if_unknown: bool,
) -> anyhow::Result<RecordWhatsappProviderStatusOutcome> {
    let provider_message_id = required_provider_identity(provider_message_id)?;
    let stored = db
        .query_row(
            "SELECT idempotency_key, status, event_at_ts
             FROM channel_delivery_provider_messages
             WHERE provider_message_id = ?1
             LIMIT 1",
            params![provider_message_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((idempotency_key, existing_status, existing_event_at_ts)) = stored else {
        if store_if_unknown {
            let event_at_ts_i64 = i64::try_from(event_at_ts)
                .map_err(|_| anyhow!("whatsapp_cloud_status_timestamp_out_of_range"))?;
            db.execute(
                "INSERT INTO whatsapp_cloud_pending_provider_statuses (
                     provider_message_id, status, event_at_ts, provider_error_code, updated_at_ts
                 ) VALUES (?1, ?2, ?3, ?4, ?3)
                 ON CONFLICT(provider_message_id) DO UPDATE SET
                     status = excluded.status,
                     event_at_ts = excluded.event_at_ts,
                     provider_error_code = excluded.provider_error_code,
                     updated_at_ts = excluded.updated_at_ts
                 WHERE excluded.event_at_ts >= whatsapp_cloud_pending_provider_statuses.event_at_ts",
                params![
                    provider_message_id,
                    status.as_str(),
                    event_at_ts_i64,
                    provider_error_code,
                ],
            )?;
        }
        return Ok(RecordWhatsappProviderStatusOutcome::UnknownMessage);
    };
    let existing = provider_event_status(&existing_status)?;
    let incoming_event_at_ts = i64::try_from(event_at_ts)
        .map_err(|_| anyhow!("whatsapp_cloud_status_timestamp_out_of_range"))?;
    if provider_status_is_regression(existing, status)
        || (incoming_event_at_ts < existing_event_at_ts && existing != status)
    {
        return Ok(RecordWhatsappProviderStatusOutcome::Unchanged);
    }
    let provider_error = (status == WhatsappDeliveryEventStatus::Failed).then(|| {
        claw_core::channel_provider_error::ChannelProviderError::from_machine_failure(
            "whatsapp_cloud",
            "delivery_status",
            claw_core::channel_provider_error::ChannelProviderFailureClass::PayloadRejected,
            None,
            provider_error_code,
            None,
            &format!("{provider_message_id}:{event_at_ts}"),
        )
    });
    db.execute(
        "UPDATE channel_delivery_provider_messages
         SET status = ?2, event_at_ts = MAX(event_at_ts, ?3),
             provider_error_code = ?4, diagnostic_id = ?5, updated_at_ts = ?3
         WHERE provider_message_id = ?1",
        params![
            provider_message_id,
            status.as_str(),
            incoming_event_at_ts,
            provider_error
                .as_ref()
                .and_then(|error| error.provider_error_code.as_deref()),
            provider_error
                .as_ref()
                .map(|error| error.diagnostic_id.as_str()),
        ],
    )?;
    let current_receipt = load_channel_delivery_receipt_from_db(&db, &idempotency_key)?
        .ok_or_else(|| anyhow!("channel_delivery_provider_receipt_missing"))?;
    let aggregate = aggregate_provider_statuses(&db, &idempotency_key)?;
    let mut next = current_receipt.clone();
    next.status = aggregate.status;
    next.updated_at_ts = current_receipt.updated_at_ts.max(event_at_ts);
    next.error_code = aggregate.error_code;
    next.message_key = aggregate.message_key;
    next.diagnostic_id = aggregate.diagnostic_id;
    next.provider_error_code = aggregate.provider_error_code;
    next.retryable = false;
    if next == current_receipt {
        return Ok(RecordWhatsappProviderStatusOutcome::Unchanged);
    }
    record_channel_delivery_receipt_in_db(&db, &next)?;
    Ok(RecordWhatsappProviderStatusOutcome::Updated)
}

fn replay_pending_whatsapp_statuses(
    db: &Connection,
    provider_message_ids: &[String],
) -> anyhow::Result<()> {
    for provider_message_id in provider_message_ids {
        let pending = db
            .query_row(
                "SELECT status, event_at_ts, provider_error_code
                 FROM whatsapp_cloud_pending_provider_statuses
                 WHERE provider_message_id = ?1
                 LIMIT 1",
                params![provider_message_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((status, event_at_ts, provider_error_code)) = pending else {
            continue;
        };
        let event_at_ts = u64::try_from(event_at_ts)
            .map_err(|_| anyhow!("whatsapp_cloud_status_timestamp_invalid"))?;
        record_whatsapp_cloud_provider_status_in_db(
            db,
            provider_message_id,
            provider_event_status(&status)?,
            event_at_ts,
            provider_error_code.as_deref(),
            false,
        )?;
        db.execute(
            "DELETE FROM whatsapp_cloud_pending_provider_statuses
             WHERE provider_message_id = ?1",
            params![provider_message_id],
        )?;
    }
    Ok(())
}

struct ProviderStatusAggregate {
    status: ChannelDeliveryStatus,
    error_code: Option<String>,
    message_key: Option<String>,
    diagnostic_id: Option<String>,
    provider_error_code: Option<String>,
}

fn aggregate_provider_statuses(
    db: &Connection,
    idempotency_key: &str,
) -> anyhow::Result<ProviderStatusAggregate> {
    let mut statement = db.prepare(
        "SELECT status, provider_error_code, diagnostic_id
         FROM channel_delivery_provider_messages
         WHERE idempotency_key = ?1
         ORDER BY provider_message_id ASC",
    )?;
    let rows = statement
        .query_map(params![idempotency_key], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.is_empty() {
        return Err(anyhow!("channel_delivery_provider_messages_missing"));
    }
    if let Some((_, provider_error_code, diagnostic_id)) =
        rows.iter().find(|(status, _, _)| status == "failed")
    {
        return Ok(ProviderStatusAggregate {
            status: ChannelDeliveryStatus::Failed,
            error_code: Some("channel.provider.payload_rejected".to_string()),
            message_key: Some("channel.error.provider_payload_rejected".to_string()),
            diagnostic_id: diagnostic_id.clone(),
            provider_error_code: provider_error_code.clone(),
        });
    }
    let all_read = rows.iter().all(|(status, _, _)| status == "read");
    let all_delivered = rows
        .iter()
        .all(|(status, _, _)| matches!(status.as_str(), "delivered" | "read"));
    Ok(ProviderStatusAggregate {
        status: if all_read {
            ChannelDeliveryStatus::Read
        } else if all_delivered {
            ChannelDeliveryStatus::Delivered
        } else {
            ChannelDeliveryStatus::Accepted
        },
        error_code: None,
        message_key: None,
        diagnostic_id: None,
        provider_error_code: None,
    })
}

fn provider_event_status(value: &str) -> anyhow::Result<WhatsappDeliveryEventStatus> {
    match value {
        "accepted" => Ok(WhatsappDeliveryEventStatus::Accepted),
        "delivered" => Ok(WhatsappDeliveryEventStatus::Delivered),
        "read" => Ok(WhatsappDeliveryEventStatus::Read),
        "failed" => Ok(WhatsappDeliveryEventStatus::Failed),
        _ => Err(anyhow!("channel_delivery_provider_status_invalid")),
    }
}

fn provider_status_is_regression(
    existing: WhatsappDeliveryEventStatus,
    incoming: WhatsappDeliveryEventStatus,
) -> bool {
    if existing == WhatsappDeliveryEventStatus::Failed {
        return incoming != WhatsappDeliveryEventStatus::Failed;
    }
    if incoming == WhatsappDeliveryEventStatus::Failed {
        return false;
    }
    provider_status_rank(incoming) < provider_status_rank(existing)
}

fn provider_status_rank(status: WhatsappDeliveryEventStatus) -> u8 {
    match status {
        WhatsappDeliveryEventStatus::Accepted => 0,
        WhatsappDeliveryEventStatus::Delivered => 1,
        WhatsappDeliveryEventStatus::Read => 2,
        WhatsappDeliveryEventStatus::Failed => 3,
    }
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

fn required_provider_identity(value: &str) -> anyhow::Result<&str> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 512
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(anyhow!("channel_delivery_provider_identity_invalid"));
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
