use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use std::sync::atomic::Ordering;

use super::lifecycle_projection::{
    executing_resume_executor_state, expired_resume_claim_recovery_metadata,
    ready_paused_checkpoint_resume_executor_from_result_json, resume_entrypoint_token,
};
use crate::{AppState, ClaimedTask};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DuePausedCheckpointTask {
    pub(crate) claim_attempt: i64,
    pub(crate) task_id: String,
    pub(crate) lifecycle_state: String,
    pub(crate) checkpoint_id: String,
    pub(crate) task_checkpoint: crate::task_lifecycle::TaskCheckpoint,
    pub(crate) resume_entrypoint: String,
    pub(crate) resume_trigger: crate::task_lifecycle::ResumeTrigger,
    pub(crate) resume_wait_seconds: i64,
    pub(crate) completed_side_effect_count: usize,
    pub(crate) requires_idempotency_guard: bool,
    pub(crate) checkpoint_resume_directive: crate::task_lifecycle::CheckpointResumeDirective,
    pub(crate) resume_directive: String,
}

fn automatic_checkpoint_resume_allowed(
    resume_entrypoint: &crate::task_lifecycle::ResumeEntrypoint,
) -> bool {
    !matches!(
        resume_entrypoint,
        crate::task_lifecycle::ResumeEntrypoint::AwaitUserInput
    )
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReadyPausedCheckpointResumeExecutor {
    pub(crate) task_id: String,
    pub(crate) lifecycle_state: String,
    pub(crate) checkpoint_id: String,
    pub(crate) executor_state: String,
    pub(crate) resume_trigger: String,
    pub(crate) resume_directive: String,
    pub(crate) next_check_after: Option<i64>,
    pub(crate) resume_executor: Value,
    pub(crate) resume_work_item: Option<Value>,
    pub(crate) task_checkpoint: crate::task_lifecycle::TaskCheckpoint,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaimedPausedCheckpointResumeExecutor {
    pub(crate) task: ClaimedTask,
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) previous_executor_state: String,
    pub(crate) executor_state: String,
    pub(crate) resume_trigger: String,
    pub(crate) resume_directive: String,
    pub(crate) lease_expires_at: i64,
    pub(crate) resume_executor: Value,
    pub(crate) resume_work_item: Option<Value>,
    pub(crate) task_checkpoint: crate::task_lifecycle::TaskCheckpoint,
}

pub(crate) fn list_due_paused_checkpoint_tasks_internal(
    state: &AppState,
    now_ts: i64,
    limit: usize,
) -> anyhow::Result<Vec<DuePausedCheckpointTask>> {
    let limit = limit.max(1);
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, result_json, COALESCE(claim_attempt, 0)
         FROM tasks
         WHERE status = 'running'
           AND result_json IS NOT NULL
           AND lease_expires_at <= ?1
         ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at, '0') AS INTEGER) ASC,
                  task_id ASC",
    )?;
    let rows = stmt.query_map(params![now_ts], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (task_id, result_json, claim_attempt) = row?;
        let Some(result_json) =
            result_json.and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        else {
            continue;
        };
        let crate::task_lifecycle::PausedCheckpointResumeReadiness::Ready {
            state,
            checkpoint_id,
            resume_entrypoint,
            completed_side_effect_count,
            requires_idempotency_guard,
        } = crate::task_lifecycle::paused_checkpoint_resume_readiness(&result_json, now_ts)
        else {
            continue;
        };
        if !automatic_checkpoint_resume_allowed(&resume_entrypoint) {
            continue;
        }
        let checkpoint_resume_directive =
            crate::task_lifecycle::checkpoint_resume_directive(&result_json, now_ts);
        let resume_directive = checkpoint_resume_directive.status_code().to_string();
        let Some(task_checkpoint) =
            crate::task_lifecycle::task_checkpoint_from_result_json(&result_json)
        else {
            continue;
        };
        let resume_trigger =
            crate::task_lifecycle::checkpoint_resume_trigger(&result_json, &resume_entrypoint);
        out.push(DuePausedCheckpointTask {
            claim_attempt,
            task_id,
            lifecycle_state: state,
            checkpoint_id,
            task_checkpoint,
            resume_entrypoint: resume_entrypoint_token(resume_entrypoint).to_string(),
            resume_trigger,
            resume_wait_seconds: 0,
            completed_side_effect_count,
            requires_idempotency_guard,
            checkpoint_resume_directive,
            resume_directive,
        });
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

pub(crate) fn claim_due_paused_checkpoint_task_internal(
    state: &AppState,
    task_id: &str,
    checkpoint_id: &str,
    now_ts: i64,
    lease_seconds: i64,
) -> anyhow::Result<Option<DuePausedCheckpointTask>> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    if task_id.is_empty() || checkpoint_id.is_empty() {
        return Ok(None);
    }
    let lease_seconds = lease_seconds.max(1);
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let task_row = db
        .query_row(
            "SELECT result_json, lease_expires_at, COALESCE(claim_attempt, 0)
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
             LIMIT 1",
            params![task_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((raw_result_json, task_lease_expires_at, previous_claim_attempt)) = task_row else {
        return Ok(None);
    };
    let claim_attempt = previous_claim_attempt
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("task claim attempt overflow: task_id={task_id}"))?;
    let Some(raw_result_json) = raw_result_json else {
        return Ok(None);
    };
    if task_lease_expires_at > now_ts {
        return Ok(None);
    }
    let mut result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let crate::task_lifecycle::PausedCheckpointResumeReadiness::Ready {
        state: lifecycle_state,
        checkpoint_id: ready_checkpoint_id,
        resume_entrypoint,
        completed_side_effect_count,
        requires_idempotency_guard,
    } = crate::task_lifecycle::paused_checkpoint_resume_readiness(&result_json, now_ts)
    else {
        return Ok(None);
    };
    if ready_checkpoint_id != checkpoint_id {
        return Ok(None);
    }
    if !automatic_checkpoint_resume_allowed(&resume_entrypoint) {
        return Ok(None);
    }
    let Some(task_checkpoint) =
        crate::task_lifecycle::task_checkpoint_from_result_json(&result_json)
    else {
        return Ok(None);
    };
    let checkpoint_resume_directive =
        crate::task_lifecycle::checkpoint_resume_directive(&result_json, now_ts);
    let resume_directive = checkpoint_resume_directive.status_code().to_string();
    let resume_trigger =
        crate::task_lifecycle::checkpoint_resume_trigger(&result_json, &resume_entrypoint);

    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let expired_claim_recovery =
        expired_resume_claim_recovery_metadata(&lifecycle, &ready_checkpoint_id, now_ts);
    if let Some(obj) = lifecycle.as_object_mut() {
        let mut resume_claim = serde_json::json!({
            "schema_version": 1,
            "owner": state.worker.worker_id.clone(),
            "owner_layer": "worker_recovery",
            "checkpoint_id": ready_checkpoint_id.clone(),
            "claim_attempt": claim_attempt,
            "claimed_at": now_ts,
            "expires_at": now_ts.saturating_add(lease_seconds),
        });
        if let Some((previous_owner, previous_expires_at)) = expired_claim_recovery {
            if let Some(claim_obj) = resume_claim.as_object_mut() {
                claim_obj.insert(
                    "recovery_reason".to_string(),
                    serde_json::json!("checkpoint_lease_expired"),
                );
                claim_obj.insert(
                    "previous_claim_expires_at".to_string(),
                    serde_json::json!(previous_expires_at),
                );
                if let Some(previous_owner) = previous_owner {
                    claim_obj.insert(
                        "previous_claim_owner".to_string(),
                        serde_json::json!(previous_owner),
                    );
                }
            }
        }
        obj.insert("resume_claim".to_string(), resume_claim);
        obj.insert("resume_due".to_string(), serde_json::json!(true));
        obj.insert("resume_wait_seconds".to_string(), serde_json::json!(0));
    } else {
        return Ok(None);
    }
    result_json["task_lifecycle"] = lifecycle;
    let updated_result_json = result_json.to_string();
    let changed = db.execute(
        "UPDATE tasks
         SET result_json = ?2,
             updated_at = ?3,
             lease_owner = ?5,
             lease_expires_at = ?6,
             claim_attempt = ?7,
             claimed_at = ?3
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?4
           AND lease_expires_at <= ?3
           AND COALESCE(claim_attempt, 0) = ?8",
        params![
            task_id,
            updated_result_json,
            now_ts,
            raw_result_json,
            state.worker.worker_id,
            now_ts.saturating_add(lease_seconds),
            claim_attempt,
            previous_claim_attempt
        ],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    Ok(Some(DuePausedCheckpointTask {
        claim_attempt,
        task_id: task_id.to_string(),
        lifecycle_state,
        checkpoint_id: ready_checkpoint_id,
        task_checkpoint,
        resume_entrypoint: resume_entrypoint_token(resume_entrypoint).to_string(),
        resume_trigger,
        resume_wait_seconds: 0,
        completed_side_effect_count,
        requires_idempotency_guard,
        checkpoint_resume_directive,
        resume_directive,
    }))
}

pub(crate) fn record_paused_checkpoint_resume_work_item_internal(
    state: &AppState,
    claim_attempt: i64,
    task_id: &str,
    checkpoint_id: &str,
    work_item_json: &Value,
    now_ts: i64,
) -> anyhow::Result<bool> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    if task_id.is_empty() || checkpoint_id.is_empty() || !work_item_json.is_object() {
        return Ok(false);
    }
    let work_item_checkpoint_id = work_item_json
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if work_item_checkpoint_id != checkpoint_id {
        return Ok(false);
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let raw_result_json = db
        .query_row(
            "SELECT result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
               AND lease_owner = ?2
               AND claim_attempt = ?3
             LIMIT 1",
            params![task_id, state.worker.worker_id.as_str(), claim_attempt],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let Some(raw_result_json) = raw_result_json else {
        return Ok(false);
    };
    let mut result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(obj) = lifecycle.as_object_mut() else {
        return Ok(false);
    };
    let claim_checkpoint_id = obj
        .get("resume_claim")
        .and_then(|claim| claim.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if claim_checkpoint_id != checkpoint_id {
        return Ok(false);
    }
    if let Some(claim_obj) = obj
        .get_mut("resume_claim")
        .and_then(serde_json::Value::as_object_mut)
    {
        claim_obj.insert("executor_state".to_string(), serde_json::json!("prepared"));
        claim_obj.insert("prepared_at".to_string(), serde_json::json!(now_ts));
    }
    obj.insert("resume_work_item".to_string(), work_item_json.clone());
    obj.insert("resume_due".to_string(), serde_json::json!(true));
    obj.insert("resume_wait_seconds".to_string(), serde_json::json!(0));

    result_json["task_lifecycle"] = lifecycle;
    let updated_result_json = result_json.to_string();
    let changed = db.execute(
        "UPDATE tasks
         SET result_json = ?2,
             updated_at = ?3
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?4
           AND lease_owner = ?5
           AND claim_attempt = ?6",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json,
            state.worker.worker_id.as_str(),
            claim_attempt
        ],
    )?;
    Ok(changed > 0)
}

pub(crate) fn record_paused_checkpoint_resume_executor_state_internal(
    state: &AppState,
    claim_attempt: i64,
    task_id: &str,
    checkpoint_id: &str,
    executor_state: &str,
    executor_payload: &Value,
    lifecycle_state: Option<&str>,
    next_check_after: Option<i64>,
    now_ts: i64,
) -> anyhow::Result<bool> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    let executor_state = executor_state.trim();
    if task_id.is_empty()
        || checkpoint_id.is_empty()
        || executor_state.is_empty()
        || !executor_payload.is_object()
    {
        return Ok(false);
    }
    let payload_checkpoint_id = executor_payload
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or(checkpoint_id);
    if payload_checkpoint_id != checkpoint_id {
        return Ok(false);
    }

    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let raw_result_json = db
        .query_row(
            "SELECT result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
               AND lease_owner = ?2
               AND claim_attempt = ?3
             LIMIT 1",
            params![task_id, state.worker.worker_id.as_str(), claim_attempt],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let Some(raw_result_json) = raw_result_json else {
        return Ok(false);
    };
    let mut result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(obj) = lifecycle.as_object_mut() else {
        return Ok(false);
    };
    let claim_checkpoint_id = obj
        .get("resume_claim")
        .and_then(|claim| claim.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if claim_checkpoint_id != checkpoint_id {
        return Ok(false);
    }
    if let Some(work_item_checkpoint_id) = obj
        .get("resume_work_item")
        .and_then(|work_item| work_item.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
    {
        if work_item_checkpoint_id != checkpoint_id {
            return Ok(false);
        }
    }

    if let Some(claim_obj) = obj
        .get_mut("resume_claim")
        .and_then(serde_json::Value::as_object_mut)
    {
        claim_obj.insert(
            "executor_state".to_string(),
            serde_json::json!(executor_state),
        );
        claim_obj.insert("executor_state_at".to_string(), serde_json::json!(now_ts));
    }
    if let Some(work_item_obj) = obj
        .get_mut("resume_work_item")
        .and_then(serde_json::Value::as_object_mut)
    {
        work_item_obj.insert(
            "executor_state".to_string(),
            serde_json::json!(executor_state),
        );
    }

    let mut executor_record = executor_payload.clone();
    if let Some(executor_obj) = executor_record.as_object_mut() {
        executor_obj.insert("schema_version".to_string(), serde_json::json!(1));
        executor_obj.insert(
            "checkpoint_id".to_string(),
            serde_json::json!(checkpoint_id),
        );
        executor_obj.insert(
            "executor_state".to_string(),
            serde_json::json!(executor_state),
        );
        executor_obj.insert("recorded_at".to_string(), serde_json::json!(now_ts));
    }
    obj.insert("resume_executor".to_string(), executor_record);

    if let Some(state) = lifecycle_state
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        obj.insert("state".to_string(), serde_json::json!(state));
    }
    if let Some(next_check_after) = next_check_after {
        obj.insert(
            "next_check_after".to_string(),
            serde_json::json!(next_check_after),
        );
        let wait_seconds = next_check_after.saturating_sub(now_ts).max(0);
        obj.insert(
            "resume_due".to_string(),
            serde_json::json!(wait_seconds == 0),
        );
        obj.insert(
            "resume_wait_seconds".to_string(),
            serde_json::json!(wait_seconds),
        );
    } else if lifecycle_state
        .map(str::trim)
        .is_some_and(|state| state == "needs_user")
    {
        obj.insert("resume_due".to_string(), serde_json::json!(false));
        obj.insert("resume_wait_seconds".to_string(), serde_json::json!(0));
    }

    result_json["task_lifecycle"] = lifecycle;
    let updated_result_json = result_json.to_string();
    let changed = db.execute(
        "UPDATE tasks
         SET result_json = ?2,
             updated_at = ?3
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?4
           AND lease_owner = ?5
           AND claim_attempt = ?6",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json,
            state.worker.worker_id.as_str(),
            claim_attempt
        ],
    )?;
    Ok(changed > 0)
}

pub(crate) fn list_ready_paused_checkpoint_resume_executors_internal(
    state: &AppState,
    now_ts: i64,
    limit: usize,
) -> anyhow::Result<Vec<ReadyPausedCheckpointResumeExecutor>> {
    let limit = limit.max(1);
    let candidate_batch = limit.saturating_mul(2).clamp(16, 128);
    let scan_limit = limit.saturating_mul(8).clamp(candidate_batch, 1024);
    let aging_cutoff = now_ts.saturating_sub(60);
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "WITH candidates AS (
             SELECT task_id,
                    result_json,
                    CAST(COALESCE(NULLIF(updated_at, ''), created_at, '0') AS INTEGER) AS update_epoch,
                    json_extract(result_json, '$.task_lifecycle.state') AS lifecycle_state,
                    json_extract(result_json, '$.task_lifecycle.next_check_after') AS lifecycle_next_check,
                    json_extract(result_json, '$.task_lifecycle.resume_executor.next_check_after') AS executor_next_check,
                    json_extract(result_json, '$.task_lifecycle.resume_executor.executor_state') AS executor_state,
                    json_extract(result_json, '$.task_lifecycle.resume_executor_claim.expires_at') AS claim_expires_at
             FROM tasks
             WHERE status = 'running'
               AND result_json IS NOT NULL
               AND json_valid(result_json) = 1
         )
         SELECT task_id, result_json
         FROM candidates
         WHERE lifecycle_state IN ('waiting', 'running', 'background')
           AND (
                COALESCE(lifecycle_next_check, executor_next_check) IS NULL
                OR COALESCE(lifecycle_next_check, executor_next_check) <= ?2
           )
           AND (
                executor_state IN ('ready_for_planner_resume', 'ready_to_finalize', 'poll_scheduled')
                OR (
                    executor_state IN ('executing_planner_resume', 'executing_finalize', 'executing_async_poll')
                    AND COALESCE(claim_expires_at, 0) <= ?2
                )
           )
         ORDER BY CASE
                    WHEN update_epoch <= ?1 THEN 0
                    WHEN lifecycle_state = 'waiting' THEN 1
                    WHEN lifecycle_state = 'running' THEN 2
                    ELSE 3
                  END ASC,
                  update_epoch ASC,
                  task_id ASC
         LIMIT ?3 OFFSET ?4",
    )?;

    let mut out = Vec::new();
    let scan_cursor = &state.worker.resume_executor_scan_offset;
    let start_offset = scan_cursor.load(Ordering::Relaxed);
    let mut offset = start_offset;
    let mut scanned_total = 0usize;
    let mut exhausted = false;
    while scanned_total < scan_limit && out.len() < limit {
        let batch = candidate_batch.min(scan_limit - scanned_total);
        let rows = stmt.query_map(
            params![aging_cutoff, now_ts, batch as i64, offset as i64],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        let mut scanned = 0usize;
        for row in rows {
            scanned = scanned.saturating_add(1);
            let (task_id, raw_result_json) = row?;
            let Ok(result_json) = serde_json::from_str::<Value>(&raw_result_json) else {
                continue;
            };
            let Some(ready) = ready_paused_checkpoint_resume_executor_from_result_json(
                task_id,
                &result_json,
                now_ts,
            ) else {
                continue;
            };
            out.push(ready);
            if out.len() >= limit {
                break;
            }
        }
        scanned_total = scanned_total.saturating_add(scanned);
        offset = offset.saturating_add(scanned);
        if scanned < batch {
            exhausted = true;
            break;
        }
    }
    // A rejected prefix must not consume every subsequent pass. Resume the
    // bounded scan, then wrap at EOF; claims still recheck exact durable ownership.
    let next_offset = if exhausted || out.len() >= limit {
        0
    } else {
        offset
    };
    let _ = scan_cursor.compare_exchange(
        start_offset,
        next_offset,
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
    tracing::debug!(
        scanned_total,
        returned = out.len(),
        next_offset,
        "resume_executor_scan_completed"
    );
    Ok(out)
}

pub(crate) fn claim_ready_paused_checkpoint_resume_executor_internal(
    state: &AppState,
    task_id: &str,
    checkpoint_id: &str,
    expected_executor_state: &str,
    now_ts: i64,
    lease_seconds: i64,
) -> anyhow::Result<Option<ClaimedPausedCheckpointResumeExecutor>> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    let expected_executor_state = expected_executor_state.trim();
    if task_id.is_empty() || checkpoint_id.is_empty() || expected_executor_state.is_empty() {
        return Ok(None);
    }
    let lease_seconds = lease_seconds.max(1);
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let task_row = db
        .query_row(
            "SELECT task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, kind, payload_json, result_json,
                    COALESCE(claim_attempt, 0)
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
               AND lease_owner = ?2
             LIMIT 1",
            params![task_id, state.worker.worker_id.as_str()],
            |row| {
                Ok((
                    ClaimedTask {
                        claim_attempt: row.get(10)?,
                        task_id: row.get(0)?,
                        user_id: row.get(1)?,
                        chat_id: row.get(2)?,
                        user_key: row.get(3)?,
                        channel: row.get(4)?,
                        external_user_id: row.get(5)?,
                        external_chat_id: row.get(6)?,
                        kind: row.get(7)?,
                        payload_json: row.get(8)?,
                    },
                    row.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((task, raw_result_json)) = task_row else {
        return Ok(None);
    };
    let Some(raw_result_json) = raw_result_json else {
        return Ok(None);
    };
    let mut result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let Some(mut ready) = ready_paused_checkpoint_resume_executor_from_result_json(
        task_id.to_string(),
        &result_json,
        now_ts,
    ) else {
        return Ok(None);
    };
    if ready.checkpoint_id != checkpoint_id || ready.executor_state != expected_executor_state {
        return Ok(None);
    }
    let Some(executing_state) = executing_resume_executor_state(&ready.executor_state) else {
        return Ok(None);
    };
    let lease_expires_at = now_ts.saturating_add(lease_seconds);
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(obj) = lifecycle.as_object_mut() else {
        return Ok(None);
    };
    let Some(resume_claim_obj) = obj
        .get_mut("resume_claim")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Ok(None);
    };
    let resume_claim_checkpoint_id = resume_claim_obj
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let resume_claim_owner = resume_claim_obj
        .get("owner")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if resume_claim_checkpoint_id != checkpoint_id
        || resume_claim_owner != state.worker.worker_id.as_str()
    {
        return Ok(None);
    }
    let renewal_count = resume_claim_obj
        .get("renewal_count")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .saturating_add(1);
    resume_claim_obj.insert(
        "expires_at".to_string(),
        serde_json::json!(lease_expires_at),
    );
    resume_claim_obj.insert("renewed_at".to_string(), serde_json::json!(now_ts));
    resume_claim_obj.insert(
        "renewal_count".to_string(),
        serde_json::json!(renewal_count),
    );
    resume_claim_obj.insert(
        "claim_attempt".to_string(),
        serde_json::json!(task.claim_attempt),
    );
    let Some(executor_obj) = obj
        .get_mut("resume_executor")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Ok(None);
    };
    executor_obj.insert(
        "executor_state".to_string(),
        serde_json::json!(executing_state),
    );
    executor_obj.insert(
        "previous_executor_state".to_string(),
        serde_json::json!(ready.executor_state),
    );
    executor_obj.insert("executor_state_at".to_string(), serde_json::json!(now_ts));
    executor_obj.insert(
        "executor_claim_expires_at".to_string(),
        serde_json::json!(lease_expires_at),
    );
    obj.insert("state".to_string(), serde_json::json!("running"));
    obj.insert("resume_due".to_string(), serde_json::json!(false));
    obj.insert("resume_wait_seconds".to_string(), serde_json::json!(0));
    obj.insert(
        "resume_executor_claim".to_string(),
        serde_json::json!({
            "schema_version": 1,
            "owner": "worker_recovery_executor",
            "checkpoint_id": checkpoint_id,
            "claimed_at": now_ts,
            "expires_at": lease_expires_at,
            "previous_executor_state": ready.executor_state,
            "executor_state": executing_state,
        }),
    );
    if let Some(claim_obj) = obj
        .get_mut("resume_claim")
        .and_then(serde_json::Value::as_object_mut)
    {
        claim_obj.insert(
            "executor_state".to_string(),
            serde_json::json!(executing_state),
        );
        claim_obj.insert("executor_state_at".to_string(), serde_json::json!(now_ts));
    }
    if let Some(work_item_obj) = obj
        .get_mut("resume_work_item")
        .and_then(serde_json::Value::as_object_mut)
    {
        work_item_obj.insert(
            "executor_state".to_string(),
            serde_json::json!(executing_state),
        );
    }

    let updated_resume_executor = obj
        .get("resume_executor")
        .cloned()
        .unwrap_or_else(|| ready.resume_executor.clone());
    let updated_resume_work_item = obj
        .get("resume_work_item")
        .filter(|value| value.is_object())
        .cloned();

    result_json["task_lifecycle"] = lifecycle;
    let updated_result_json = result_json.to_string();
    let changed = db.execute(
        "UPDATE tasks
         SET result_json = ?2,
             updated_at = ?3,
             lease_expires_at = ?7
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?4
           AND lease_owner = ?5
           AND claim_attempt = ?6",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json,
            state.worker.worker_id.as_str(),
            task.claim_attempt,
            lease_expires_at
        ],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    ready.resume_executor = updated_resume_executor;
    ready.resume_work_item = updated_resume_work_item;
    Ok(Some(ClaimedPausedCheckpointResumeExecutor {
        task,
        task_id: ready.task_id,
        checkpoint_id: ready.checkpoint_id,
        previous_executor_state: ready.executor_state,
        executor_state: executing_state.to_string(),
        resume_trigger: ready.resume_trigger,
        resume_directive: ready.resume_directive,
        lease_expires_at,
        resume_executor: ready.resume_executor,
        resume_work_item: ready.resume_work_item,
        task_checkpoint: ready.task_checkpoint,
    }))
}

pub(crate) fn record_paused_checkpoint_resume_execution_plan_internal(
    state: &AppState,
    claim_attempt: i64,
    task_id: &str,
    checkpoint_id: &str,
    executor_state: &str,
    execution_plan: &Value,
    now_ts: i64,
) -> anyhow::Result<bool> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    let executor_state = executor_state.trim();
    if task_id.is_empty()
        || checkpoint_id.is_empty()
        || executor_state.is_empty()
        || !execution_plan.is_object()
    {
        return Ok(false);
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let raw_result_json = db
        .query_row(
            "SELECT result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
               AND lease_owner = ?2
               AND claim_attempt = ?3
             LIMIT 1",
            params![task_id, state.worker.worker_id.as_str(), claim_attempt],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let Some(raw_result_json) = raw_result_json else {
        return Ok(false);
    };
    let mut result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(obj) = lifecycle.as_object_mut() else {
        return Ok(false);
    };
    let claim = obj.get("resume_executor_claim");
    let claim_checkpoint_id = claim
        .and_then(|value| value.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let claim_executor_state = claim
        .and_then(|value| value.get("executor_state"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if claim_checkpoint_id != checkpoint_id || claim_executor_state != executor_state {
        return Ok(false);
    }
    let plan_action = execution_plan
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if plan_action.is_empty() {
        return Ok(false);
    }
    let completed_side_effect_refs =
        crate::task_lifecycle::task_checkpoint_from_result_json(&result_json)
            .map(|checkpoint| checkpoint.completed_side_effect_refs)
            .unwrap_or_default();
    let completed_side_effect_count = completed_side_effect_refs.len();
    let mut plan_payload = execution_plan.clone();
    if let Some(plan_obj) = plan_payload.as_object_mut() {
        plan_obj.insert("planned_at".to_string(), serde_json::json!(now_ts));
        plan_obj.insert(
            "checkpoint_id".to_string(),
            serde_json::json!(checkpoint_id),
        );
        plan_obj.insert(
            "executor_state".to_string(),
            serde_json::json!(executor_state),
        );
        plan_obj.insert(
            "completed_side_effect_count".to_string(),
            serde_json::json!(completed_side_effect_count),
        );
        plan_obj.insert(
            "completed_side_effect_refs".to_string(),
            serde_json::json!(completed_side_effect_refs),
        );
        plan_obj.insert(
            "requires_idempotency_guard".to_string(),
            serde_json::json!(completed_side_effect_count > 0),
        );
    }
    obj.insert("resume_execution_plan".to_string(), plan_payload);
    for key in [
        "resume_executor_handoff",
        "resume_executor_handoff_claim",
        "resume_executor_handoff_dispatch",
        "resume_executor_dispatch_claim",
        "resume_executor_dispatch_result",
        "resume_executor_result_projection_claim",
        "resume_executor_result_projection",
    ] {
        obj.remove(key);
    }
    if let Some(executor_obj) = obj
        .get_mut("resume_executor")
        .and_then(serde_json::Value::as_object_mut)
    {
        for key in [
            "dispatch_state",
            "dispatch_execution_state",
            "dispatched_at",
            "dispatch_claimed_at",
            "dispatch_claim_expires_at",
            "handoff_claimed_at",
            "handoff_claim_expires_at",
            "executor_result_status",
            "executor_result_at",
            "result_projection_state",
            "result_projection_claimed_at",
            "result_projection_claim_expires_at",
            "projected_at",
        ] {
            executor_obj.remove(key);
        }
        executor_obj.insert(
            "execution_plan_action".to_string(),
            serde_json::json!(plan_action),
        );
        executor_obj.insert("execution_plan_at".to_string(), serde_json::json!(now_ts));
    }
    if let Some(claim_obj) = obj
        .get_mut("resume_executor_claim")
        .and_then(serde_json::Value::as_object_mut)
    {
        claim_obj.insert(
            "execution_plan_action".to_string(),
            serde_json::json!(plan_action),
        );
        claim_obj.insert("execution_plan_at".to_string(), serde_json::json!(now_ts));
    }

    result_json["task_lifecycle"] = lifecycle;
    let updated_result_json = result_json.to_string();
    let changed = db.execute(
        "UPDATE tasks
         SET result_json = ?2,
             updated_at = ?3
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?4
           AND lease_owner = ?5
           AND claim_attempt = ?6",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json,
            state.worker.worker_id.as_str(),
            claim_attempt
        ],
    )?;
    Ok(changed > 0)
}
