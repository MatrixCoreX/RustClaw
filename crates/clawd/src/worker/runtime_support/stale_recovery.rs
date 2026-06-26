use anyhow::anyhow;
use rusqlite::Connection;
use serde_json::Value;
use tracing::warn;

use crate::{now_ts, now_ts_u64, AppState};

#[derive(Debug, Clone, PartialEq, Eq)]
enum StaleRunningRecoveryReason {
    WorkerHeartbeatStale,
    WorkerLeaseExpired,
}

impl StaleRunningRecoveryReason {
    fn error_token(&self) -> &'static str {
        match self {
            Self::WorkerHeartbeatStale => "worker_heartbeat_stale",
            Self::WorkerLeaseExpired => "worker_lease_expired",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StaleRunningTaskCandidate {
    task_id: String,
    reason: StaleRunningRecoveryReason,
}

fn recovery_should_preserve_paused_checkpoint(result_json: Option<&str>, now: i64) -> bool {
    let Some(result_json) = result_json.and_then(|raw| serde_json::from_str::<Value>(raw).ok())
    else {
        return false;
    };
    crate::task_lifecycle::paused_checkpoint_recovery_status(&result_json, now)
        .preserve_running_status_for_recovery()
}

pub(crate) fn recover_stale_running_tasks_on_startup(
    db: &Connection,
    no_progress_timeout_seconds: u64,
) -> anyhow::Result<Vec<String>> {
    let now = now_ts_u64() as i64;
    let timeout = no_progress_timeout_seconds.max(1) as i64;
    let stale_before = now.saturating_sub(timeout);
    let mut candidates = Vec::new();
    {
        let mut stmt = db.prepare(
            "SELECT task_id, result_json,
                    CASE
                        WHEN lease_expires_at > 0 AND lease_expires_at <= ?2 THEN 'worker_lease_expired'
                        ELSE 'worker_heartbeat_stale'
                    END AS recovery_reason
             FROM tasks
             WHERE status = 'running'
               AND (
                    CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?1
                    OR (lease_expires_at > 0 AND lease_expires_at <= ?2)
               )
             ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![stale_before.to_string(), now], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                parse_recovery_reason(row.get::<_, String>(2)?.as_str()),
            ))
        })?;
        for row in rows {
            let (task_id, result_json, reason) = row?;
            if recovery_should_preserve_paused_checkpoint(result_json.as_deref(), now) {
                continue;
            }
            candidates.push(StaleRunningTaskCandidate { task_id, reason });
        }
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut changed = 0;
    for candidate in &candidates {
        changed += db.execute(
            "UPDATE tasks
             SET status = 'timeout',
                 error_text = CASE
                     WHEN error_text IS NULL OR TRIM(error_text) = '' THEN ?3
                     ELSE error_text
                 END,
                 updated_at = ?4
             WHERE task_id = ?1
               AND status = 'running'
               AND (
                    CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?2
                    OR (lease_expires_at > 0 AND lease_expires_at <= ?5)
               )",
            rusqlite::params![
                candidate.task_id,
                stale_before.to_string(),
                candidate.reason.error_token(),
                now_ts(),
                now
            ],
        )?;
    }
    if changed != candidates.len() {
        warn!(
            "startup stale-running recovery count mismatch: selected={} updated={}",
            candidates.len(),
            changed
        );
    }

    Ok(candidates
        .into_iter()
        .map(|candidate| candidate.task_id)
        .collect())
}

pub(crate) fn recover_stale_running_tasks_by_no_progress(
    state: &AppState,
) -> anyhow::Result<Vec<String>> {
    let timeout_secs = state
        .worker
        .worker_running_no_progress_timeout_seconds
        .max(60);
    let now = now_ts_u64() as i64;
    let stale_before = now.saturating_sub(timeout_secs as i64);
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;

    let mut candidates = Vec::new();
    {
        let mut stmt = db.prepare(
            "SELECT task_id, result_json,
                    CASE
                        WHEN lease_expires_at > 0 AND lease_expires_at <= ?2 THEN 'worker_lease_expired'
                        ELSE 'worker_heartbeat_stale'
                    END AS recovery_reason
             FROM tasks
             WHERE status = 'running'
               AND (
                    CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?1
                    OR (lease_expires_at > 0 AND lease_expires_at <= ?2)
               )
             ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![stale_before.to_string(), now], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                parse_recovery_reason(row.get::<_, String>(2)?.as_str()),
            ))
        })?;
        for row in rows {
            let (task_id, result_json, reason) = row?;
            if recovery_should_preserve_paused_checkpoint(result_json.as_deref(), now) {
                continue;
            }
            candidates.push(StaleRunningTaskCandidate { task_id, reason });
        }
    }

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut changed = 0;
    for candidate in &candidates {
        changed += db.execute(
            "UPDATE tasks
             SET status = 'timeout',
                 error_text = CASE
                     WHEN error_text IS NULL OR TRIM(error_text) = '' THEN ?3
                     ELSE error_text
                 END,
                 updated_at = ?4
             WHERE task_id = ?1
               AND status = 'running'
               AND (
                    CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?2
                    OR (lease_expires_at > 0 AND lease_expires_at <= ?5)
               )",
            rusqlite::params![
                candidate.task_id,
                stale_before.to_string(),
                candidate.reason.error_token(),
                now_ts(),
                now
            ],
        )?;
    }
    if changed != candidates.len() {
        warn!(
            "runtime stale-running recovery count mismatch: selected={} updated={}",
            candidates.len(),
            changed
        );
    }
    Ok(candidates
        .into_iter()
        .map(|candidate| candidate.task_id)
        .collect())
}

fn parse_recovery_reason(raw: &str) -> StaleRunningRecoveryReason {
    if raw == "worker_lease_expired" {
        StaleRunningRecoveryReason::WorkerLeaseExpired
    } else {
        StaleRunningRecoveryReason::WorkerHeartbeatStale
    }
}
