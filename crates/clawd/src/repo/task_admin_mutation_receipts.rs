use anyhow::Context;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::AppState;

const INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS task_admin_mutation_receipts (
    actor_digest TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    action TEXT NOT NULL,
    target_id TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN ('in_progress', 'completed')),
    lease_token TEXT NOT NULL,
    response_json TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (actor_digest, idempotency_key)
);
CREATE INDEX IF NOT EXISTS idx_task_admin_mutation_receipts_updated
ON task_admin_mutation_receipts(updated_at);
"#;

#[derive(Clone, Debug)]
pub(crate) struct TaskAdminMutationLease {
    actor_digest: String,
    idempotency_key: String,
    lease_token: String,
}

#[derive(Clone, Debug)]
pub(crate) enum TaskAdminMutationClaim {
    Acquired(TaskAdminMutationLease),
    Replay(Value),
    InProgress,
    Conflict,
}

pub(crate) fn claim_task_admin_mutation(
    state: &AppState,
    actor_key: &str,
    idempotency_key: &str,
    action: &str,
    target_id: &str,
    request_payload: &Value,
) -> anyhow::Result<TaskAdminMutationClaim> {
    let idempotency_key = machine_token(idempotency_key, 200)
        .filter(|value| value.len() >= 8)
        .ok_or_else(|| anyhow::anyhow!("task_mutation_idempotency_key_invalid"))?;
    let action =
        machine_token(action, 64).ok_or_else(|| anyhow::anyhow!("task_mutation_action_invalid"))?;
    let target_id = machine_token(target_id, 200)
        .ok_or_else(|| anyhow::anyhow!("task_mutation_target_invalid"))?;
    let actor_digest = digest_token(actor_key.trim());
    let request_json = serde_json::to_vec(request_payload)?;
    let request_digest = digest_token_bytes(&request_json);
    let lease_token = uuid::Uuid::new_v4().to_string();
    let now = crate::now_ts_u64() as i64;

    let mut db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    db.execute_batch(INIT_SQL)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT OR IGNORE INTO task_admin_mutation_receipts(
             actor_digest, idempotency_key, action, target_id, request_digest,
             phase, lease_token, response_json, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'in_progress', ?6, NULL, ?7, ?7)",
        params![
            actor_digest,
            idempotency_key,
            action,
            target_id,
            request_digest,
            lease_token,
            now
        ],
    )?;
    let row = tx
        .query_row(
            "SELECT action, target_id, request_digest, phase, lease_token, response_json
             FROM task_admin_mutation_receipts
             WHERE actor_digest = ?1 AND idempotency_key = ?2",
            params![actor_digest, idempotency_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()?
        .context("task_admin_mutation_receipt_missing")?;
    tx.commit()?;

    if row.0 != action || row.1 != target_id || row.2 != request_digest {
        return Ok(TaskAdminMutationClaim::Conflict);
    }
    if row.3 == "completed" {
        let response = row
            .5
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .context("task_admin_mutation_response_missing")?;
        return Ok(TaskAdminMutationClaim::Replay(response));
    }
    if row.4 != lease_token {
        return Ok(TaskAdminMutationClaim::InProgress);
    }
    Ok(TaskAdminMutationClaim::Acquired(TaskAdminMutationLease {
        actor_digest,
        idempotency_key,
        lease_token,
    }))
}

pub(crate) fn complete_task_admin_mutation(
    state: &AppState,
    lease: &TaskAdminMutationLease,
    response: &Value,
) -> anyhow::Result<()> {
    let response_json = serde_json::to_string(response)?;
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    db.execute_batch(INIT_SQL)?;
    let changed = db.execute(
        "UPDATE task_admin_mutation_receipts
         SET phase = 'completed', response_json = ?4, updated_at = ?5
         WHERE actor_digest = ?1 AND idempotency_key = ?2
           AND lease_token = ?3 AND phase = 'in_progress'",
        params![
            lease.actor_digest,
            lease.idempotency_key,
            lease.lease_token,
            response_json,
            crate::now_ts_u64() as i64
        ],
    )?;
    if changed != 1 {
        anyhow::bail!("task_admin_mutation_receipt_not_completable");
    }
    Ok(())
}

pub(crate) fn release_task_admin_mutation(
    state: &AppState,
    lease: &TaskAdminMutationLease,
) -> anyhow::Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    db.execute_batch(INIT_SQL)?;
    db.execute(
        "DELETE FROM task_admin_mutation_receipts
         WHERE actor_digest = ?1 AND idempotency_key = ?2
           AND lease_token = ?3 AND phase = 'in_progress'",
        params![lease.actor_digest, lease.idempotency_key, lease.lease_token],
    )?;
    Ok(())
}

fn digest_token(value: &str) -> String {
    digest_token_bytes(value.as_bytes())
}

fn digest_token_bytes(value: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(value))
}

fn machine_token(value: &str, max_len: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > max_len
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return None;
    }
    Some(value.to_string())
}

#[cfg(test)]
#[path = "task_admin_mutation_receipts_tests.rs"]
mod tests;
