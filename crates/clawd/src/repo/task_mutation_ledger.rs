use anyhow::{anyhow, Context};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::db_init::DbPool;

const INIT_TASK_MUTATION_LEDGER_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS task_mutation_ledger (
    task_id            TEXT NOT NULL,
    fingerprint_hash   TEXT NOT NULL,
    action_ref         TEXT NOT NULL,
    status             TEXT NOT NULL CHECK (status IN ('started', 'completed', 'uncertain')),
    execution_token    TEXT NOT NULL,
    lease_owner        TEXT NOT NULL,
    claim_attempt      INTEGER NOT NULL,
    outcome_hash       TEXT,
    outcome_json       TEXT,
    started_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    completed_at       INTEGER,
    PRIMARY KEY (task_id, fingerprint_hash)
);
CREATE INDEX IF NOT EXISTS idx_task_mutation_ledger_status_updated
    ON task_mutation_ledger(status, updated_at);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskMutationRecord {
    pub(crate) task_id: String,
    pub(crate) fingerprint_hash: String,
    pub(crate) action_ref: String,
    pub(crate) status: String,
    pub(crate) outcome: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskMutationLease {
    pub(crate) record: TaskMutationRecord,
    pub(crate) execution_token: String,
    pub(crate) lease_owner: String,
    pub(crate) claim_attempt: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskMutationClaimRejected {
    pub(crate) status_code: &'static str,
    pub(crate) task_id: String,
    pub(crate) expected_lease_owner: String,
    pub(crate) expected_claim_attempt: i64,
    pub(crate) task_status: Option<String>,
    pub(crate) active_lease_owner: Option<String>,
    pub(crate) active_claim_attempt: Option<i64>,
}

impl std::fmt::Display for TaskMutationClaimRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "status_code={} task_id={} expected_lease_owner={} expected_claim_attempt={} task_status={} active_lease_owner={} active_claim_attempt={}",
            self.status_code,
            self.task_id,
            self.expected_lease_owner,
            self.expected_claim_attempt,
            self.task_status.as_deref().unwrap_or("missing"),
            self.active_lease_owner.as_deref().unwrap_or("none"),
            self.active_claim_attempt
                .map(|value| value.to_string())
                .as_deref()
                .unwrap_or("none")
        )
    }
}

impl std::error::Error for TaskMutationClaimRejected {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BeginTaskMutationOutcome {
    Acquired(TaskMutationLease),
    Completed(TaskMutationRecord),
    Uncertain(TaskMutationRecord),
}

pub(crate) fn begin_task_mutation(
    pool: &DbPool,
    lease_owner: &str,
    claim_attempt: i64,
    task_id: &str,
    action_fingerprint: &str,
    action_ref: &str,
) -> anyhow::Result<BeginTaskMutationOutcome> {
    let task_id = required_value(task_id, "task_id")?;
    let lease_owner = required_value(lease_owner, "lease_owner")?;
    let action_fingerprint = required_value(action_fingerprint, "action_fingerprint")?;
    let action_ref = required_value(action_ref, "action_ref")?;
    let fingerprint_hash = sha256_hex(action_fingerprint.as_bytes());
    let execution_token = uuid::Uuid::new_v4().to_string();
    let now = crate::now_ts_u64() as i64;
    let mut db = pool.get().context("task mutation ledger db pool")?;
    ensure_task_mutation_ledger_schema(&db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active_task_claim(&tx, task_id, lease_owner, claim_attempt)?;
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO task_mutation_ledger (
             task_id, fingerprint_hash, action_ref, status, execution_token,
             lease_owner, claim_attempt, outcome_hash, outcome_json,
             started_at, updated_at, completed_at
         ) VALUES (?1, ?2, ?3, 'started', ?4, ?5, ?6, NULL, NULL, ?7, ?7, NULL)",
        params![
            task_id,
            fingerprint_hash,
            action_ref,
            execution_token,
            lease_owner,
            claim_attempt,
            now
        ],
    )?;
    let row = tx
        .query_row(
            "SELECT action_ref, status, execution_token, outcome_json
             FROM task_mutation_ledger
             WHERE task_id = ?1 AND fingerprint_hash = ?2",
            params![task_id, fingerprint_hash],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("task mutation ledger insert was not observable"))?;
    tx.commit()?;

    let record = TaskMutationRecord {
        task_id: task_id.to_string(),
        fingerprint_hash,
        action_ref: row.0,
        status: row.1.clone(),
        outcome: parse_outcome_json(row.3.as_deref())?,
    };
    if inserted == 1 {
        return Ok(BeginTaskMutationOutcome::Acquired(TaskMutationLease {
            record,
            execution_token,
            lease_owner: lease_owner.to_string(),
            claim_attempt,
        }));
    }
    match row.1.as_str() {
        "completed" => Ok(BeginTaskMutationOutcome::Completed(record)),
        "started" | "uncertain" => Ok(BeginTaskMutationOutcome::Uncertain(record)),
        status => Err(anyhow!("unsupported task mutation ledger status: {status}")),
    }
}

pub(crate) fn complete_task_mutation(
    pool: &DbPool,
    lease: &TaskMutationLease,
    outcome_hash_source: &str,
    outcome_projection: Option<&Value>,
) -> anyhow::Result<()> {
    let now = crate::now_ts_u64() as i64;
    let outcome_hash = sha256_hex(outcome_hash_source.as_bytes());
    let outcome_json = outcome_projection
        .map(serde_json::to_string)
        .transpose()
        .context("serialize task mutation outcome projection")?;
    let mut db = pool.get().context("task mutation ledger db pool")?;
    ensure_task_mutation_ledger_schema(&db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active_task_claim(
        &tx,
        &lease.record.task_id,
        &lease.lease_owner,
        lease.claim_attempt,
    )?;
    let changed = tx.execute(
        "UPDATE task_mutation_ledger
         SET status = 'completed',
             outcome_hash = ?4,
             outcome_json = ?5,
             updated_at = ?6,
             completed_at = ?6
         WHERE task_id = ?1
           AND fingerprint_hash = ?2
           AND execution_token = ?3
           AND lease_owner = ?7
           AND claim_attempt = ?8
           AND status = 'started'",
        params![
            lease.record.task_id,
            lease.record.fingerprint_hash,
            lease.execution_token,
            outcome_hash,
            outcome_json,
            now,
            lease.lease_owner,
            lease.claim_attempt
        ],
    )?;
    if changed == 1 {
        tx.commit()?;
        return Ok(());
    }
    let status = task_mutation_status(&tx, lease)?;
    if status.as_deref() == Some("completed") {
        tx.commit()?;
        return Ok(());
    }
    Err(anyhow!("task mutation lease was not completable"))
}

pub(crate) fn mark_task_mutation_uncertain(
    pool: &DbPool,
    lease: &TaskMutationLease,
) -> anyhow::Result<()> {
    let now = crate::now_ts_u64() as i64;
    let mut db = pool.get().context("task mutation ledger db pool")?;
    ensure_task_mutation_ledger_schema(&db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active_task_claim(
        &tx,
        &lease.record.task_id,
        &lease.lease_owner,
        lease.claim_attempt,
    )?;
    tx.execute(
        "UPDATE task_mutation_ledger
         SET status = 'uncertain',
             updated_at = ?4
         WHERE task_id = ?1
           AND fingerprint_hash = ?2
           AND execution_token = ?3
           AND lease_owner = ?5
           AND claim_attempt = ?6
           AND status = 'started'",
        params![
            lease.record.task_id,
            lease.record.fingerprint_hash,
            lease.execution_token,
            now,
            lease.lease_owner,
            lease.claim_attempt
        ],
    )?;
    tx.commit()?;
    Ok(())
}

fn task_mutation_status(
    db: &rusqlite::Connection,
    lease: &TaskMutationLease,
) -> anyhow::Result<Option<String>> {
    db.query_row(
        "SELECT status
         FROM task_mutation_ledger
         WHERE task_id = ?1
           AND fingerprint_hash = ?2
           AND execution_token = ?3
           AND lease_owner = ?4
           AND claim_attempt = ?5",
        params![
            lease.record.task_id,
            lease.record.fingerprint_hash,
            lease.execution_token,
            lease.lease_owner,
            lease.claim_attempt
        ],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn require_active_task_claim(
    db: &rusqlite::Connection,
    task_id: &str,
    expected_lease_owner: &str,
    expected_claim_attempt: i64,
) -> anyhow::Result<()> {
    let row = db
        .query_row(
            "SELECT status, lease_owner, COALESCE(claim_attempt, 0)
             FROM tasks
             WHERE task_id = ?1
             LIMIT 1",
            params![task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let (task_status, active_lease_owner, active_claim_attempt) = match row {
        Some((status, owner, attempt)) => (Some(status), owner, Some(attempt)),
        None => (None, None, None),
    };
    if task_status.as_deref() == Some("running")
        && active_lease_owner.as_deref() == Some(expected_lease_owner)
        && active_claim_attempt == Some(expected_claim_attempt)
    {
        return Ok(());
    }
    Err(anyhow::Error::new(TaskMutationClaimRejected {
        status_code: crate::repo::WORKER_LEASE_LOST_STATUS_CODE,
        task_id: task_id.to_string(),
        expected_lease_owner: expected_lease_owner.to_string(),
        expected_claim_attempt,
        task_status,
        active_lease_owner,
        active_claim_attempt,
    }))
}

fn required_value<'a>(value: &'a str, field: &str) -> anyhow::Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("task mutation ledger missing {field}"));
    }
    Ok(value)
}

fn ensure_task_mutation_ledger_schema(db: &rusqlite::Connection) -> anyhow::Result<()> {
    db.execute_batch(INIT_TASK_MUTATION_LEDGER_SQL)?;
    crate::ensure_column_exists(
        db,
        "task_mutation_ledger",
        "outcome_json",
        "ALTER TABLE task_mutation_ledger ADD COLUMN outcome_json TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "task_mutation_ledger",
        "lease_owner",
        "ALTER TABLE task_mutation_ledger ADD COLUMN lease_owner TEXT NOT NULL DEFAULT ''",
    )?;
    crate::ensure_column_exists(
        db,
        "task_mutation_ledger",
        "claim_attempt",
        "ALTER TABLE task_mutation_ledger ADD COLUMN claim_attempt INTEGER NOT NULL DEFAULT 0",
    )
}

fn parse_outcome_json(raw: Option<&str>) -> anyhow::Result<Option<Value>> {
    raw.map(serde_json::from_str)
        .transpose()
        .context("parse task mutation outcome projection")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "task_mutation_ledger_tests.rs"]
mod tests;
