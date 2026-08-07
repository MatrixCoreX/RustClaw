use anyhow::{anyhow, Context};
use rusqlite::{params, OptionalExtension};

use crate::db_init::DbPool;

const INIT_CHANNEL_DELIVERY_OUTBOX_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS channel_terminal_delivery_outbox (
    task_id             TEXT PRIMARY KEY,
    state               TEXT NOT NULL CHECK (state IN ('pending', 'dispatching', 'completed', 'failed')),
    lease_token         TEXT,
    lease_expires_at_ts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at_ts  INTEGER NOT NULL DEFAULT 0,
    attempt_count       INTEGER NOT NULL DEFAULT 0,
    last_error_code     TEXT,
    created_at_ts       INTEGER NOT NULL,
    updated_at_ts       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_channel_terminal_delivery_outbox_due
    ON channel_terminal_delivery_outbox(state, next_attempt_at_ts, lease_expires_at_ts);

CREATE TRIGGER IF NOT EXISTS trg_channel_terminal_delivery_on_update
AFTER UPDATE OF status ON tasks
WHEN NEW.status IN ('succeeded', 'failed', 'canceled', 'timeout')
 AND OLD.status NOT IN ('succeeded', 'failed', 'canceled', 'timeout')
 AND NEW.channel != 'ui'
 AND NEW.user_key IS NOT NULL AND TRIM(NEW.user_key) != ''
 AND json_valid(NEW.payload_json)
 AND json_extract(NEW.payload_json, '$.channel_ingress.schema_version') IS NOT NULL
BEGIN
    INSERT OR IGNORE INTO channel_terminal_delivery_outbox (
        task_id, state, next_attempt_at_ts, created_at_ts, updated_at_ts
    ) VALUES (
        NEW.task_id, 'pending', 0, CAST(strftime('%s', 'now') AS INTEGER),
        CAST(strftime('%s', 'now') AS INTEGER)
    );
END;

CREATE TRIGGER IF NOT EXISTS trg_channel_terminal_delivery_on_insert
AFTER INSERT ON tasks
WHEN NEW.status IN ('succeeded', 'failed', 'canceled', 'timeout')
 AND NEW.channel != 'ui'
 AND NEW.user_key IS NOT NULL AND TRIM(NEW.user_key) != ''
 AND json_valid(NEW.payload_json)
 AND json_extract(NEW.payload_json, '$.channel_ingress.schema_version') IS NOT NULL
BEGIN
    INSERT OR IGNORE INTO channel_terminal_delivery_outbox (
        task_id, state, next_attempt_at_ts, created_at_ts, updated_at_ts
    ) VALUES (
        NEW.task_id, 'pending', 0, CAST(strftime('%s', 'now') AS INTEGER),
        CAST(strftime('%s', 'now') AS INTEGER)
    );
END;
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaimedChannelTerminalDelivery {
    pub(crate) task_id: String,
    pub(crate) lease_token: String,
    pub(crate) attempt_count: u32,
}

pub(crate) fn ensure_channel_delivery_outbox_schema(
    db: &rusqlite::Connection,
) -> anyhow::Result<()> {
    db.execute_batch(INIT_CHANNEL_DELIVERY_OUTBOX_SQL)?;
    if sqlite_table_exists(db, "channel_delivery_dispatch_claims")?
        && sqlite_table_exists(db, "channel_delivery_receipts")?
    {
        db.execute(
            "INSERT OR IGNORE INTO channel_terminal_delivery_outbox (
                 task_id, state, next_attempt_at_ts, created_at_ts, updated_at_ts
             )
             SELECT task.task_id, 'pending', 0,
                    CAST(strftime('%s', 'now') AS INTEGER),
                    CAST(strftime('%s', 'now') AS INTEGER)
             FROM tasks AS task
             JOIN channel_delivery_dispatch_claims AS claim
               ON claim.delivery_id = 'delivery:' || task.task_id || ':terminal'
             LEFT JOIN channel_delivery_receipts AS receipt
               ON receipt.idempotency_key = claim.idempotency_key
             WHERE task.status IN ('succeeded', 'failed', 'canceled', 'timeout')
               AND task.channel != 'ui'
               AND json_valid(task.payload_json)
               AND json_extract(task.payload_json, '$.channel_ingress.schema_version') IS NOT NULL
               AND (
                    receipt.idempotency_key IS NULL
                    OR (receipt.status IN ('failed', 'partial')
                        AND json_extract(receipt.receipt_json, '$.retryable') = 1)
               )",
            [],
        )?;
    }
    Ok(())
}

fn sqlite_table_exists(db: &rusqlite::Connection, table_name: &str) -> anyhow::Result<bool> {
    Ok(db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            params![table_name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

pub(crate) fn claim_due_channel_terminal_delivery(
    pool: &DbPool,
    now_ts: u64,
    lease_seconds: u64,
) -> anyhow::Result<Option<ClaimedChannelTerminalDelivery>> {
    let mut db = pool
        .get()
        .context("channel_delivery_outbox_db_pool_failed")?;
    let now_ts = i64::try_from(now_ts).map_err(|_| anyhow!("outbox_timestamp_out_of_range"))?;
    let lease_expires_at_ts = now_ts.saturating_add(
        i64::try_from(lease_seconds.max(1)).map_err(|_| anyhow!("outbox_lease_out_of_range"))?,
    );
    let tx = db.transaction()?;
    let task_id = tx
        .query_row(
            "SELECT task_id
             FROM channel_terminal_delivery_outbox
             WHERE (state = 'pending' AND next_attempt_at_ts <= ?1)
                OR (state = 'dispatching' AND lease_expires_at_ts <= ?1)
             ORDER BY next_attempt_at_ts ASC, created_at_ts ASC
             LIMIT 1",
            params![now_ts],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(task_id) = task_id else {
        tx.commit()?;
        return Ok(None);
    };
    let lease_token = uuid::Uuid::new_v4().to_string();
    let changed = tx.execute(
        "UPDATE channel_terminal_delivery_outbox
         SET state = 'dispatching', lease_token = ?2, lease_expires_at_ts = ?3,
             attempt_count = attempt_count + 1, updated_at_ts = ?1
         WHERE task_id = ?4
           AND ((state = 'pending' AND next_attempt_at_ts <= ?1)
             OR (state = 'dispatching' AND lease_expires_at_ts <= ?1))",
        params![now_ts, lease_token, lease_expires_at_ts, task_id],
    )?;
    if changed != 1 {
        tx.commit()?;
        return Ok(None);
    }
    let attempt_count = tx.query_row(
        "SELECT attempt_count FROM channel_terminal_delivery_outbox WHERE task_id = ?1",
        params![task_id],
        |row| row.get::<_, u32>(0),
    )?;
    tx.commit()?;
    Ok(Some(ClaimedChannelTerminalDelivery {
        task_id,
        lease_token,
        attempt_count,
    }))
}

pub(crate) fn finish_channel_terminal_delivery(
    pool: &DbPool,
    claim: &ClaimedChannelTerminalDelivery,
    completed: bool,
    retry_after_seconds: Option<u64>,
    error_code: Option<&str>,
    now_ts: u64,
) -> anyhow::Result<()> {
    let db = pool
        .get()
        .context("channel_delivery_outbox_db_pool_failed")?;
    let now_ts = i64::try_from(now_ts).map_err(|_| anyhow!("outbox_timestamp_out_of_range"))?;
    let (state, next_attempt_at_ts) = if completed {
        ("completed", 0)
    } else if let Some(delay) = retry_after_seconds {
        (
            "pending",
            now_ts.saturating_add(i64::try_from(delay).unwrap_or(i64::MAX)),
        )
    } else {
        ("failed", 0)
    };
    let changed = db.execute(
        "UPDATE channel_terminal_delivery_outbox
         SET state = ?3, lease_token = NULL, lease_expires_at_ts = 0,
             next_attempt_at_ts = ?4, last_error_code = ?5, updated_at_ts = ?6
         WHERE task_id = ?1 AND state = 'dispatching' AND lease_token = ?2",
        params![
            claim.task_id,
            claim.lease_token,
            state,
            next_attempt_at_ts,
            error_code,
            now_ts,
        ],
    )?;
    if changed != 1 {
        return Err(anyhow!("channel_delivery_outbox_lease_mismatch"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "channel_delivery_outbox_tests.rs"]
mod tests;
