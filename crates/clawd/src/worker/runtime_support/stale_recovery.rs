use anyhow::anyhow;
use rusqlite::Connection;
use serde_json::{json, Value};
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
    result_json: Option<String>,
    is_child_subagent: bool,
}

fn recovery_should_preserve_recoverable_state(result_json: Option<&str>, now: i64) -> bool {
    let Some(result_json) = result_json.and_then(|raw| serde_json::from_str::<Value>(raw).ok())
    else {
        return false;
    };
    crate::task_lifecycle::paused_checkpoint_recovery_status(&result_json, now)
        .preserve_running_status_for_recovery()
        || crate::task_lifecycle::has_recoverable_resume_execution(&result_json)
        || result_projection_pending_recovery_state(&result_json)
}

fn result_projection_pending_recovery_state(result_json: &Value) -> bool {
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(result_json), None);
    let Some(obj) = lifecycle.as_object() else {
        return false;
    };
    if obj
        .get("resume_executor_result_projection")
        .is_some_and(Value::is_object)
    {
        return false;
    }
    let Some(dispatch_result) = obj
        .get("resume_executor_dispatch_result")
        .filter(|value| value.is_object())
    else {
        return false;
    };
    if dispatch_result.get("text").is_some() || dispatch_result.get("error_text").is_some() {
        return false;
    }
    if matches!(
        dispatch_result
            .get("projection_pending_reason")
            .and_then(Value::as_str)
            .map(str::trim),
        Some("terminal_projection_pending" | "result_projection_pending")
    ) {
        return true;
    }
    let executor_action = dispatch_result
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let executor_result_status = dispatch_result
        .get("executor_result_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    dispatch_result_has_known_projection(executor_action, executor_result_status)
}

fn dispatch_result_has_known_projection(
    executor_action: &str,
    executor_result_status: &str,
) -> bool {
    matches!(
        (executor_action, executor_result_status),
        ("run_seeded_agent_loop", "seeded_loop_completed")
            | ("run_seeded_agent_loop", "seeded_loop_deferred")
            | ("run_seeded_agent_loop", "seeded_loop_failed")
            | ("poll_async_job", "async_poll_completed")
            | ("poll_async_job", "async_poll_rescheduled")
            | ("poll_async_job", "async_poll_failed")
            | ("poll_async_job", "async_poll_cancelled")
            | ("verify_and_finalize", "finalize_completed")
            | ("verify_and_finalize", "finalize_failed")
    )
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
            "SELECT task_id, result_json, lease_owner, payload_json,
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
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                parse_recovery_reason(row.get::<_, String>(4)?.as_str()),
            ))
        })?;
        for row in rows {
            let (task_id, result_json, _lease_owner, payload_json, reason) = row?;
            if recovery_should_preserve_recoverable_state(result_json.as_deref(), now) {
                continue;
            }
            candidates.push(StaleRunningTaskCandidate {
                task_id,
                reason,
                result_json,
                is_child_subagent: child_subagent_payload(&payload_json),
            });
        }
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut changed = 0;
    for candidate in &candidates {
        changed += if candidate.is_child_subagent {
            db.execute(
                "UPDATE tasks
                 SET status = 'queued',
                     error_text = NULL,
                     result_json = ?3,
                     updated_at = ?4,
                     lease_owner = NULL,
                     lease_expires_at = 0,
                     claimed_at = 0
                 WHERE task_id = ?1
                   AND status = 'running'
                   AND (
                        CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?2
                        OR (lease_expires_at > 0 AND lease_expires_at <= ?5)
                   )",
                rusqlite::params![
                    candidate.task_id,
                    stale_before.to_string(),
                    stale_child_requeue_result_json(
                        &candidate.task_id,
                        candidate.result_json.as_deref(),
                        &candidate.reason,
                        now,
                    ),
                    now_ts(),
                    now,
                ],
            )?
        } else {
            db.execute(
                "UPDATE tasks
             SET status = 'timeout',
                 error_text = CASE
                     WHEN error_text IS NULL OR TRIM(error_text) = '' THEN ?3
                     ELSE error_text
                 END,
                 result_json = ?6,
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
                    now,
                    stale_running_timeout_result_json(
                        &candidate.task_id,
                        candidate.result_json.as_deref(),
                        &candidate.reason,
                        now,
                    )
                ],
            )?
        };
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

/// Transfers durable resume work from the previous local process generation
/// to the worker generation created by this startup. A task that already has
/// a machine resume executor must not wait for the old in-process lease to
/// expire: that owner cannot still exist after this process has restarted.
/// Ordinary running work is deliberately excluded and remains governed by
/// stale/no-progress recovery.
pub(crate) fn adopt_recoverable_resume_executions_on_startup(
    db: &Connection,
    worker_id: &str,
    lease_seconds: i64,
) -> anyhow::Result<Vec<String>> {
    let worker_id = worker_id.trim();
    if worker_id.is_empty() {
        anyhow::bail!("startup_resume_worker_id_missing");
    }
    let now = now_ts_u64() as i64;
    let lease_expires_at = now.saturating_add(lease_seconds.max(1));
    let mut stmt = db.prepare(
        "SELECT task_id, result_json, lease_owner, COALESCE(claim_attempt, 0)
         FROM tasks WHERE status = 'running' AND result_json IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let candidates = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let mut adopted = Vec::new();
    for (task_id, raw_result, previous_owner, previous_claim_attempt) in candidates {
        let mut result = match serde_json::from_str::<Value>(&raw_result) {
            Ok(value) if crate::task_lifecycle::has_recoverable_resume_execution(&value) => value,
            _ => continue,
        };
        let lifecycle = result
            .get_mut("task_lifecycle")
            .and_then(Value::as_object_mut);
        let Some(lifecycle) = lifecycle else {
            continue;
        };
        let has_resume_claim = lifecycle.get("resume_claim").is_some_and(Value::is_object);
        let has_resume_executor = [
            "resume_executor",
            "resume_executor_claim",
            "resume_executor_handoff",
            "resume_executor_handoff_claim",
            "resume_executor_dispatch",
            "resume_executor_dispatch_claim",
            "resume_executor_dispatch_result",
            "resume_executor_result_projection_claim",
        ]
        .into_iter()
        .any(|key| lifecycle.get(key).is_some_and(Value::is_object));
        if !has_resume_claim && !has_resume_executor {
            // The checkpoint has not yet been claimed by the old process.
            // Release only the dead process lease; the ordinary recovery path
            // will create the first resume claim when next_check_after is due.
            let changed = db.execute(
                "UPDATE tasks
                 SET updated_at = ?2, lease_owner = NULL, lease_expires_at = 0, claimed_at = 0
                 WHERE task_id = ?1 AND status = 'running' AND result_json = ?3
                   AND COALESCE(claim_attempt, 0) = ?4",
                rusqlite::params![task_id, now.to_string(), raw_result, previous_claim_attempt,],
            )?;
            if changed == 1 {
                adopted.push(task_id);
            }
            continue;
        }
        let checkpoint_id = lifecycle
            .get("checkpoint_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let Some(checkpoint_id) = checkpoint_id else {
            continue;
        };
        if has_resume_claim && !has_resume_executor {
            if let Some(claim) = lifecycle
                .get_mut("resume_claim")
                .and_then(Value::as_object_mut)
            {
                claim.insert("expires_at".to_string(), json!(now));
                claim.insert("recovery_reason".to_string(), json!("service_restart"));
            }
            lifecycle.insert("resume_due".to_string(), json!(true));
            lifecycle.insert("resume_wait_seconds".to_string(), json!(0));
            let changed = db.execute(
                "UPDATE tasks
                 SET result_json = ?2, updated_at = ?3, lease_owner = NULL,
                     lease_expires_at = 0, claimed_at = 0
                 WHERE task_id = ?1 AND status = 'running' AND result_json = ?4
                   AND COALESCE(claim_attempt, 0) = ?5",
                rusqlite::params![
                    task_id,
                    result.to_string(),
                    now.to_string(),
                    raw_result,
                    previous_claim_attempt,
                ],
            )?;
            if changed == 1 {
                adopted.push(task_id);
            }
            continue;
        }
        let claim = lifecycle
            .entry("resume_claim".to_string())
            .or_insert_with(|| json!({}))
            .as_object_mut();
        let Some(claim) = claim else {
            continue;
        };
        claim.insert("schema_version".to_string(), json!(1));
        claim.insert("checkpoint_id".to_string(), json!(checkpoint_id.clone()));
        claim.insert("owner".to_string(), json!(worker_id));
        claim.insert("owner_layer".to_string(), json!("startup_resume_adoption"));
        claim.insert("claimed_at".to_string(), json!(now));
        claim.insert("expires_at".to_string(), json!(lease_expires_at));
        claim.insert("recovery_reason".to_string(), json!("service_restart"));
        if let Some(owner) = previous_owner
            .as_deref()
            .filter(|owner| *owner != worker_id)
        {
            claim.insert("previous_claim_owner".to_string(), json!(owner));
        }
        for key in [
            "resume_executor_claim",
            "resume_executor_handoff_claim",
            "resume_executor_dispatch_claim",
            "resume_executor_result_projection_claim",
        ] {
            if let Some(stale_claim) = lifecycle.get_mut(key).and_then(Value::as_object_mut) {
                stale_claim.insert("expires_at".to_string(), json!(now));
                stale_claim.insert("recovery_reason".to_string(), json!("service_restart"));
            }
        }
        lifecycle.insert("resume_due".to_string(), json!(true));
        lifecycle.insert("resume_wait_seconds".to_string(), json!(0));
        lifecycle.insert(
            "startup_resume_adoption".to_string(),
            json!({
                "schema_version": 1,
                "checkpoint_id": checkpoint_id,
                "worker_id": worker_id,
                "adopted_at": now,
                "previous_claim_attempt": previous_claim_attempt,
            }),
        );
        let next_claim_attempt = previous_claim_attempt
            .checked_add(1)
            .ok_or_else(|| anyhow!("task claim attempt overflow: task_id={task_id}"))?;
        let changed = db.execute(
            "UPDATE tasks
             SET result_json = ?2, updated_at = ?3, lease_owner = ?4,
                 lease_expires_at = ?5, claimed_at = ?3, claim_attempt = ?6
             WHERE task_id = ?1 AND status = 'running' AND result_json = ?7
               AND COALESCE(claim_attempt, 0) = ?8",
            rusqlite::params![
                task_id,
                result.to_string(),
                now.to_string(),
                worker_id,
                lease_expires_at,
                next_claim_attempt,
                raw_result,
                previous_claim_attempt,
            ],
        )?;
        if changed == 1 {
            adopted.push(task_id);
        }
    }
    Ok(adopted)
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
    let mut active_current_worker_task_ids = Vec::new();
    {
        let mut stmt = db.prepare(
            "SELECT task_id, result_json, lease_owner, payload_json,
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
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                parse_recovery_reason(row.get::<_, String>(4)?.as_str()),
            ))
        })?;
        for row in rows {
            let (task_id, result_json, lease_owner, payload_json, reason) = row?;
            if recovery_should_preserve_recoverable_state(result_json.as_deref(), now) {
                continue;
            }
            if lease_owner.as_deref() == Some(state.worker.worker_id.as_str())
                && state.worker.is_task_active(&task_id)
            {
                active_current_worker_task_ids.push(task_id);
                continue;
            }
            candidates.push(StaleRunningTaskCandidate {
                task_id,
                reason,
                result_json,
                is_child_subagent: child_subagent_payload(&payload_json),
            });
        }
    }

    for task_id in active_current_worker_task_ids {
        if let Err(err) = refresh_active_current_worker_task_lease(&db, state, &task_id) {
            warn!(
                "runtime stale-running active-task lease refresh failed: worker_id={} task_id={} err={}",
                state.worker.worker_id, task_id, err
            );
        }
    }

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut changed = 0;
    for candidate in &candidates {
        changed += if candidate.is_child_subagent {
            db.execute(
                "UPDATE tasks
                 SET status = 'queued',
                     error_text = NULL,
                     result_json = ?3,
                     updated_at = ?4,
                     lease_owner = NULL,
                     lease_expires_at = 0,
                     claimed_at = 0
                 WHERE task_id = ?1
                   AND status = 'running'
                   AND (
                        CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?2
                        OR (lease_expires_at > 0 AND lease_expires_at <= ?5)
                   )",
                rusqlite::params![
                    candidate.task_id,
                    stale_before.to_string(),
                    stale_child_requeue_result_json(
                        &candidate.task_id,
                        candidate.result_json.as_deref(),
                        &candidate.reason,
                        now,
                    ),
                    now_ts(),
                    now,
                ],
            )?
        } else {
            db.execute(
                "UPDATE tasks
             SET status = 'timeout',
                 error_text = CASE
                     WHEN error_text IS NULL OR TRIM(error_text) = '' THEN ?3
                     ELSE error_text
                 END,
                 result_json = ?6,
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
                    now,
                    stale_running_timeout_result_json(
                        &candidate.task_id,
                        candidate.result_json.as_deref(),
                        &candidate.reason,
                        now,
                    )
                ],
            )?
        };
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

fn refresh_active_current_worker_task_lease(
    db: &Connection,
    state: &AppState,
    task_id: &str,
) -> anyhow::Result<()> {
    let heartbeat_at = now_ts_u64() as i64;
    db.execute(
        "UPDATE tasks
         SET updated_at = ?2,
             lease_owner = ?3,
             lease_expires_at = ?4
         WHERE task_id = ?1
           AND status = 'running'",
        rusqlite::params![
            task_id,
            heartbeat_at.to_string(),
            state.worker.worker_id,
            crate::repo::worker_task_lease_expires_at(state, heartbeat_at)
        ],
    )?;
    Ok(())
}

fn parse_recovery_reason(raw: &str) -> StaleRunningRecoveryReason {
    if raw == "worker_lease_expired" {
        StaleRunningRecoveryReason::WorkerLeaseExpired
    } else {
        StaleRunningRecoveryReason::WorkerHeartbeatStale
    }
}

fn child_subagent_payload(raw_payload_json: &str) -> bool {
    serde_json::from_str::<Value>(raw_payload_json)
        .ok()
        .is_some_and(|payload| crate::repo::child_tasks::is_child_subagent_payload(&payload))
}

fn stale_child_requeue_result_json(
    task_id: &str,
    raw_result_json: Option<&str>,
    reason: &StaleRunningRecoveryReason,
    recovered_at: i64,
) -> String {
    let mut result_json = raw_result_json
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let reason_code = reason.error_token();
    if let Some(object) = result_json.as_object_mut() {
        object.insert("status_code".to_string(), json!("child_claim_requeued"));
        object.insert("reason_code".to_string(), json!(reason_code));
        object.insert(
            "task_lifecycle".to_string(),
            json!({
                "schema_version": 1,
                "state": "queued",
                "source": "worker_stale_recovery",
                "reason_code": reason_code,
                "recovered_at": recovered_at,
                "worker_events": [{
                    "event_type": "child_claim_requeued",
                    "owner_layer": "worker_runtime",
                    "task_id": task_id,
                    "state_from": "running",
                    "state_to": "queued",
                    "reason_code": reason_code,
                    "recovered_at": recovered_at,
                }],
            }),
        );
    }
    result_json.to_string()
}

fn stale_running_timeout_result_json(
    task_id: &str,
    raw_result_json: Option<&str>,
    reason: &StaleRunningRecoveryReason,
    recovered_at: i64,
) -> String {
    let mut result_json = raw_result_json
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let reason_code = reason.error_token();
    let event_type = match reason {
        StaleRunningRecoveryReason::WorkerHeartbeatStale => "heartbeat_missed",
        StaleRunningRecoveryReason::WorkerLeaseExpired => "lease_reclaimed",
    };
    if let Some(obj) = result_json.as_object_mut() {
        obj.insert("status_code".to_string(), json!(reason_code));
        obj.insert("reason_code".to_string(), json!(reason_code));
        obj.insert(
            "message_key".to_string(),
            json!("clawd.task.stale_running_recovered"),
        );
        obj.insert(
            "task_lifecycle".to_string(),
            json!({
                "schema_version": 1,
                "state": "failed",
                "source": "worker_stale_recovery",
                "terminal_reason": reason_code,
                "reason_code": reason_code,
                "recovered_at": recovered_at,
                "worker_events": [
                    {
                        "event_type": event_type,
                        "owner_layer": "worker_runtime",
                        "task_id": task_id,
                        "state_from": "running",
                        "state_to": "timeout",
                        "reason_code": reason_code,
                        "recovered_at": recovered_at
                    }
                ]
            }),
        );
    }
    result_json.to_string()
}
