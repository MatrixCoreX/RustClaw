use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MIGRATION_ID: &str = "029_memory_job_completion_v1";
const MIGRATION_SQL: &str = include_str!("../../../../migrations/029_memory_job_completion.sql");
const COMPLETED_JOB_COST: i64 = 1000; // Existing logical units, not token/USD billing.

#[derive(Clone, Copy)]
pub(super) struct GenerationLease<'a> {
    pub(super) job_id: &'a str,
    pub(super) worker_id: &'a str,
    pub(super) principal_id: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JobPhase {
    Unstarted,
    SourceVerified,
    ResourceWait,
    ProviderStarted,
    ResponsePersisted,
    Committed,
    Invalid,
}

pub(super) fn phase(checkpoint: &str) -> JobPhase {
    let Ok(value) = serde_json::from_str::<Value>(checkpoint) else {
        return JobPhase::Invalid;
    };
    if value.as_object().is_some_and(|object| object.is_empty()) {
        return JobPhase::Unstarted;
    }
    if value.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return JobPhase::Invalid;
    }
    match value.get("phase").and_then(Value::as_str) {
        Some("source_range_verified") => JobPhase::SourceVerified,
        Some("resource_wait") => JobPhase::ResourceWait,
        Some("provider_request_started") => JobPhase::ProviderStarted,
        Some("provider_response_persisted") => JobPhase::ResponsePersisted,
        Some("commit_verified") => JobPhase::Committed,
        _ => JobPhase::Invalid,
    }
}

pub(super) fn ensure_schema(db: &Connection) -> anyhow::Result<()> {
    let digest = format!("sha256:{:x}", Sha256::digest(MIGRATION_SQL.as_bytes()));
    let applied: Option<String> = db
        .query_row(
            "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id = ?1",
            [MIGRATION_ID],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(applied) = applied {
        anyhow::ensure!(
            applied == digest,
            "runtime_schema_migration_digest_mismatch:{MIGRATION_ID}"
        );
        return Ok(());
    }
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_completion_migration_requires_transaction"
    );
    db.execute_batch(MIGRATION_SQL)?;
    db.execute(
        "INSERT INTO runtime_schema_migrations(migration_id, schema_digest, applied_at)
        VALUES (?1, ?2, ?3)",
        params![MIGRATION_ID, digest, crate::now_ts()],
    )?;
    Ok(())
}

pub(super) fn require_schema(db: &Connection) -> anyhow::Result<()> {
    let digest = format!("sha256:{:x}", Sha256::digest(MIGRATION_SQL.as_bytes()));
    super::scope::require_applied_migration(db, MIGRATION_ID, &digest)
}

/// Called inside the same transaction as canonical writes. A later failed
/// terminal/charge write cannot erase this marker or replay those effects.
pub(super) fn mark_committed(db: &Connection, lease: GenerationLease<'_>) -> anyhow::Result<()> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_job_commit_requires_transaction"
    );
    let changed = db.execute(
        "UPDATE memory_jobs SET checkpoint_json = ?1,
        progress_current = 2, progress_total = 2, updated_at_ts = ?2,
        background_cost_pending_microunits = CASE
            WHEN background_cost_settled_at_ts IS NULL THEN ?3 ELSE 0 END
        WHERE job_id = ?4 AND principal_id = ?5 AND status = 'running'
          AND lease_owner = ?6 AND cancel_requested = 0",
        params![
            json!({"schema_version":1,"phase":"commit_verified"}).to_string(),
            crate::now_ts_u64() as i64,
            COMPLETED_JOB_COST,
            lease.job_id,
            lease.principal_id,
            lease.worker_id
        ],
    )?;
    anyhow::ensure!(changed == 1, "memory_job_commit_lease_lost");
    db.execute(
        "DELETE FROM memory_generation_outputs WHERE job_id = ?1 AND principal_id = ?2",
        params![lease.job_id, lease.principal_id],
    )?;
    Ok(())
}

pub(super) fn complete(
    state: &crate::AppState,
    job_id: &str,
    worker_id: &str,
) -> anyhow::Result<()> {
    let mut db = state.core.db.get()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let checkpoint: Option<String> = tx
        .query_row(
            "SELECT checkpoint_json FROM memory_jobs
        WHERE job_id = ?1 AND status = 'running' AND lease_owner = ?2",
            params![job_id, worker_id],
            |row| row.get(0),
        )
        .optional()?;
    anyhow::ensure!(
        checkpoint.as_deref().map(phase) == Some(JobPhase::Committed),
        "memory_job_completion_lease_or_commit_lost"
    );
    let now = crate::now_ts_u64() as i64;
    let changed = tx.execute(
        "UPDATE memory_jobs
        SET status = CASE WHEN cancel_requested = 1 THEN 'cancelled' ELSE 'completed' END,
            lease_owner = NULL, lease_expires_at_ts = NULL, retryable = 0,
            updated_at_ts = ?1, finished_at_ts = ?1,
            background_cost_pending_microunits = CASE
                WHEN background_cost_settled_at_ts IS NULL THEN ?2 ELSE 0 END
        WHERE job_id = ?3 AND status = 'running' AND lease_owner = ?4",
        params![now, COMPLETED_JOB_COST, job_id, worker_id],
    )?;
    anyhow::ensure!(changed == 1, "memory_job_completion_lease_lost");
    tx.commit()?;
    try_settle(&mut db, job_id, false);
    Ok(())
}

/// One settlement unit per worker tick. Failed principals get a retry delay;
/// their charge cannot prevent another principal's job or lease maintenance.
pub(super) fn settle_due(state: &crate::AppState) -> anyhow::Result<bool> {
    let mut db = state.core.db.get()?;
    let now = crate::now_ts_u64() as i64;
    let id: Option<String> = db
        .query_row(
            "SELECT job_id FROM memory_jobs
        WHERE background_cost_pending_microunits > 0 AND background_cost_retry_at_ts <= ?1
        ORDER BY background_cost_retry_at_ts, updated_at_ts, job_id LIMIT 1",
            [now],
            |row| row.get(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Ok(false);
    };
    try_settle(&mut db, &id, true);
    Ok(true)
}

fn try_settle(db: &mut Connection, id: &str, schedule_retry: bool) {
    if let Err(error) = settle(db, id) {
        tracing::warn!(job_id = id, error = %error, "memory_job_cost_deferred");
        if !schedule_retry {
            return;
        }
        if let Err(error) = db.execute(
            "UPDATE memory_jobs SET background_cost_retry_at_ts = ?1
            WHERE job_id = ?2 AND background_cost_pending_microunits > 0",
            params![(crate::now_ts_u64() as i64).saturating_add(2), id],
        ) {
            tracing::warn!(job_id = id, error = %error, "memory_job_cost_retry_record_failed");
        }
    }
}

fn settle(db: &mut Connection, id: &str) -> anyhow::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row: Option<(String, i64, String)> = tx
        .query_row(
            "SELECT principal_id, background_cost_pending_microunits, checkpoint_json
         FROM memory_jobs WHERE job_id = ?1 AND background_cost_pending_microunits > 0",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((principal, pending, checkpoint)) = row else {
        return Ok(());
    };
    anyhow::ensure!(
        phase(&checkpoint) == JobPhase::Committed,
        "memory_job_cost_commit_missing"
    );
    let used: i64 = tx.query_row(
        "SELECT used_background_cost_microunits
        FROM memory_principal_quotas WHERE principal_id = ?1",
        [&principal],
        |row| row.get(0),
    )?;
    anyhow::ensure!(used >= 0, "memory_job_cost_counter_invalid");
    let next = used
        .checked_add(pending)
        .ok_or_else(|| anyhow::anyhow!("memory_job_cost_counter_overflow"))?;
    let now = crate::now_ts_u64() as i64;
    tx.execute(
        "UPDATE memory_principal_quotas SET used_background_cost_microunits = ?1,
        updated_at_ts = ?2 WHERE principal_id = ?3",
        params![next, now, principal],
    )?;
    let changed = tx.execute(
        "UPDATE memory_jobs SET background_cost_pending_microunits = 0,
        background_cost_settled_at_ts = ?1, background_cost_retry_at_ts = 0
        WHERE job_id = ?2 AND background_cost_pending_microunits = ?3
          AND background_cost_settled_at_ts IS NULL",
        params![now, id, pending],
    )?;
    anyhow::ensure!(changed == 1, "memory_job_cost_settlement_conflict");
    tx.commit()?;
    Ok(())
}

pub(super) fn finalize_cancelled(db: &Connection, now: i64) -> anyhow::Result<()> {
    let mut cursor: Option<String> = None;
    loop {
        let row: Option<(String, String)> = db
            .query_row(
                "SELECT job_id, checkpoint_json FROM memory_jobs
            WHERE cancel_requested = 1 AND (status IN ('queued','retry_wait')
              OR (status = 'running' AND lease_expires_at_ts <= ?1))
              AND (?2 IS NULL OR job_id > ?2) ORDER BY job_id LIMIT 1",
                params![now, cursor],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((id, checkpoint)) = row else {
            break;
        };
        let committed = phase(&checkpoint) == JobPhase::Committed;
        db.execute(
            "UPDATE memory_jobs SET status = 'cancelled', lease_owner = NULL,
            lease_expires_at_ts = NULL, error_code = 'cancel_requested', retryable = 0,
            updated_at_ts = ?1, finished_at_ts = ?1,
            background_cost_pending_microunits = CASE
                WHEN ?2 AND background_cost_settled_at_ts IS NULL THEN ?3
                ELSE background_cost_pending_microunits END WHERE job_id = ?4",
            params![now, committed, COMPLETED_JOB_COST, id],
        )?;
        db.execute(
            "DELETE FROM memory_generation_outputs WHERE job_id = ?1",
            [&id],
        )?;
        cursor = Some(id);
    }
    Ok(())
}

#[cfg(test)]
#[path = "job_completion_tests.rs"]
mod tests;
