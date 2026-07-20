use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{
    now_ts, now_ts_u64, parse_task_status, ActiveTaskItem, AppState, ClaimedTask, TaskQueryResponse,
};

mod lifecycle_projection;

use lifecycle_projection::{
    append_checkpoint_resume_directive_lifecycle_fields, append_task_lease_lifecycle_fields,
    async_poll_terminal_projection_without_visible_reply, executing_resume_executor_state,
    expired_resume_claim_recovery_metadata, normalized_optional_task_id,
    ready_paused_checkpoint_resume_executor_from_result_json, resume_entrypoint_token,
    summarize_active_task_payload, worker_failure_result_json,
    worker_timeout_preserves_recoverable_checkpoint, worker_timeout_result_json,
};

pub(crate) const WORKER_LEASE_LOST_STATUS_CODE: &str = "worker_lease_lost";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerTaskWriteRejected {
    pub(crate) status_code: &'static str,
    pub(crate) operation: &'static str,
    pub(crate) task_id: String,
    pub(crate) expected_claim_attempt: i64,
    pub(crate) task_status: Option<String>,
    pub(crate) lease_owner: Option<String>,
    pub(crate) active_claim_attempt: Option<i64>,
}

impl std::fmt::Display for WorkerTaskWriteRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "status_code={} operation={} task_id={} expected_claim_attempt={} task_status={} lease_owner={} active_claim_attempt={}",
            self.status_code,
            self.operation,
            self.task_id,
            self.expected_claim_attempt,
            self.task_status.as_deref().unwrap_or("missing"),
            self.lease_owner.as_deref().unwrap_or("none"),
            self.active_claim_attempt
                .map(|value| value.to_string())
                .as_deref()
                .unwrap_or("none")
        )
    }
}

impl std::error::Error for WorkerTaskWriteRejected {}

pub(crate) fn worker_task_write_rejection(
    db: &rusqlite::Connection,
    state: &AppState,
    task_id: &str,
    expected_claim_attempt: i64,
    operation: &'static str,
    expected_statuses: &[&str],
) -> anyhow::Error {
    let row = match db
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
        .optional()
    {
        Ok(row) => row,
        Err(error) => return anyhow::Error::new(error),
    };
    let (task_status, lease_owner, active_claim_attempt) = match row {
        Some((status, owner, claim_attempt)) => (Some(status), owner, Some(claim_attempt)),
        None => (None, None, None),
    };
    let status_code = if task_status
        .as_deref()
        .is_some_and(|status| expected_statuses.contains(&status))
        && (lease_owner.as_deref() != Some(state.worker.worker_id.as_str())
            || active_claim_attempt != Some(expected_claim_attempt))
    {
        WORKER_LEASE_LOST_STATUS_CODE
    } else if task_status.is_none() {
        "worker_task_not_found"
    } else if operation == "update_task_progress_result"
        && task_status.as_deref() == Some("running")
        && lease_owner.as_deref() == Some(state.worker.worker_id.as_str())
        && active_claim_attempt == Some(expected_claim_attempt)
    {
        "task_progress_cas_exhausted"
    } else {
        "worker_task_state_conflict"
    };
    anyhow::Error::new(WorkerTaskWriteRejected {
        status_code,
        operation,
        task_id: task_id.to_string(),
        expected_claim_attempt,
        task_status,
        lease_owner,
        active_claim_attempt,
    })
}

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

pub(crate) fn claim_next_task(state: &AppState) -> anyhow::Result<Option<ClaimedTask>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;

    let mut stmt = db.prepare(
        "SELECT task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, kind, payload_json,
                COALESCE(claim_attempt, 0)
         FROM tasks
         WHERE status = 'queued'
         ORDER BY created_at ASC
         LIMIT 1",
    )?;

    let candidate = stmt
        .query_row([], |row| {
            let previous_claim_attempt = row.get::<_, i64>(9)?;
            let claim_attempt = previous_claim_attempt.checked_add(1).ok_or_else(|| {
                rusqlite::Error::IntegralValueOutOfRange(9, previous_claim_attempt)
            })?;
            Ok(ClaimedTask {
                claim_attempt,
                task_id: row.get(0)?,
                user_id: row.get(1)?,
                chat_id: row.get(2)?,
                user_key: row.get(3)?,
                channel: row.get(4)?,
                external_user_id: row.get(5)?,
                external_chat_id: row.get(6)?,
                kind: row.get(7)?,
                payload_json: row.get(8)?,
            })
        })
        .optional()?;

    let Some(task) = candidate else {
        return Ok(None);
    };

    let now_text = now_ts();
    let claimed_at = now_ts_u64() as i64;
    let lease_expires_at = worker_task_lease_expires_at(state, claimed_at);
    let changed = db.execute(
        "UPDATE tasks
         SET status = 'running',
             updated_at = ?2,
             lease_owner = ?3,
             claimed_at = ?4,
             lease_expires_at = ?5,
             claim_attempt = ?6
         WHERE task_id = ?1
           AND status = 'queued'
           AND COALESCE(claim_attempt, 0) = ?7",
        params![
            task.task_id,
            now_text,
            state.worker.worker_id,
            claimed_at,
            lease_expires_at,
            task.claim_attempt,
            task.claim_attempt - 1
        ],
    )?;

    if changed == 0 {
        debug!(
            "claim_next_task: worker_id={} race lost for task_id={}, another worker took it",
            state.worker.worker_id, task.task_id
        );
        return Ok(None);
    }

    debug!(
        "claim_next_task: worker_id={} claimed task_id={} user_id={} chat_id={} kind={}",
        state.worker.worker_id, task.task_id, task.user_id, task.chat_id, task.kind
    );
    Ok(Some(task))
}

pub(crate) fn update_task_success(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
    result_json: &str,
) -> anyhow::Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let changed = db.execute(
        "UPDATE tasks
         SET status = 'succeeded', result_json = ?2, error_text = NULL, updated_at = ?3
         WHERE task_id = ?1
           AND status = 'running'
           AND lease_owner = ?4
           AND claim_attempt = ?5",
        params![
            task_id,
            result_json,
            now_ts(),
            state.worker.worker_id.as_str(),
            claim_attempt
        ],
    )?;
    if changed == 0 {
        let existing = db
            .query_row(
                "SELECT status, result_json, lease_owner
                 FROM tasks
                 WHERE task_id = ?1
                 LIMIT 1",
                params![task_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((status, Some(existing_result_json), lease_owner)) = existing {
            if status == "succeeded"
                && lease_owner.as_deref() == Some(state.worker.worker_id.as_str())
                && async_poll_terminal_projection_without_visible_reply(&existing_result_json)
            {
                let changed = db.execute(
                    "UPDATE tasks
                     SET result_json = ?2, error_text = NULL, updated_at = ?3
                     WHERE task_id = ?1
                       AND status = 'succeeded'
                       AND result_json = ?4
                       AND lease_owner = ?5
                       AND claim_attempt = ?6",
                    params![
                        task_id,
                        result_json,
                        now_ts(),
                        existing_result_json,
                        state.worker.worker_id.as_str(),
                        claim_attempt
                    ],
                )?;
                if changed > 0 {
                    return Ok(());
                }
            }
        }
        return Err(worker_task_write_rejection(
            &db,
            state,
            task_id,
            claim_attempt,
            "update_task_success",
            &["running", "succeeded"],
        ));
    }
    Ok(())
}

pub(crate) fn touch_running_task(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
) -> anyhow::Result<bool> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let heartbeat_at = now_ts_u64() as i64;
    let current_result_json = db
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
        .optional()?;
    let Some(current_result_json) = current_result_json else {
        return Ok(false);
    };
    if current_result_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .is_some_and(|result| {
            paused_lifecycle_owned_by_other_executor(&result, state.worker.worker_id.as_str())
        })
    {
        return Ok(false);
    }
    let changed = db.execute(
        "UPDATE tasks
         SET updated_at = ?2,
             lease_expires_at = ?3
         WHERE task_id = ?1
           AND status = 'running'
           AND lease_owner = ?4
           AND claim_attempt = ?5
           AND result_json IS ?6",
        params![
            task_id,
            heartbeat_at.to_string(),
            worker_task_lease_expires_at(state, heartbeat_at),
            state.worker.worker_id.as_str(),
            claim_attempt,
            current_result_json
        ],
    )?;
    Ok(changed > 0)
}

fn paused_lifecycle_owned_by_other_executor(result_json: &Value, worker_id: &str) -> bool {
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(result_json), None);
    let state = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(state, "waiting" | "background" | "needs_user") {
        return false;
    }
    lifecycle
        .pointer("/resume_claim/owner")
        .and_then(Value::as_str)
        .map(str::trim)
        != Some(worker_id)
}

pub(crate) fn worker_task_lease_expires_at(state: &AppState, now_ts: i64) -> i64 {
    let lease_seconds = state
        .worker
        .worker_task_heartbeat_seconds
        .saturating_mul(4)
        .max(300);
    now_ts.saturating_add(lease_seconds as i64)
}

pub(crate) fn is_task_claim_active(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
) -> anyhow::Result<bool> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let active = db
        .query_row(
            "SELECT 1
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
               AND lease_owner = ?2
               AND claim_attempt = ?3
             LIMIT 1",
            params![task_id, state.worker.worker_id.as_str(), claim_attempt],
            |_| Ok(()),
        )
        .optional()?;
    Ok(active.is_some())
}

pub(crate) fn is_task_claim_active_or_pending_ask_success_projection(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
) -> anyhow::Result<bool> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let row = db
        .query_row(
            "SELECT status, result_json, lease_owner, COALESCE(claim_attempt, 0)
             FROM tasks
             WHERE task_id = ?1
             LIMIT 1",
            params![task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((status, result_json, lease_owner, active_claim_attempt)) = row else {
        return Ok(false);
    };
    if lease_owner.as_deref() != Some(state.worker.worker_id.as_str())
        || active_claim_attempt != claim_attempt
    {
        return Ok(false);
    }
    if status == "running" {
        return Ok(true);
    }
    Ok(status == "succeeded"
        && result_json
            .as_deref()
            .is_some_and(async_poll_terminal_projection_without_visible_reply))
}

pub(crate) fn update_task_progress_result(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
    result_json: &str,
) -> anyhow::Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let progress_result = serde_json::from_str::<Value>(result_json).ok();
    for _ in 0..3 {
        let current_result_json = db
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
            .optional()?;
        let Some(current_result_json) = current_result_json else {
            return Err(worker_task_write_rejection(
                &db,
                state,
                task_id,
                claim_attempt,
                "update_task_progress_result",
                &["running"],
            ));
        };
        let now = now_ts().parse::<i64>().unwrap_or_default();
        let merged_result_json = current_result_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .zip(progress_result.as_ref())
            .and_then(|(current, progress)| {
                crate::repo::task_resume_execution::merge_progress_with_active_resume_coordination(
                    &current, progress, now,
                )
            })
            .map(|value| value.to_string())
            .unwrap_or_else(|| result_json.to_string());
        let changed = db.execute(
            "UPDATE tasks
             SET result_json = ?2,
                 updated_at = ?3
             WHERE task_id = ?1
               AND status IN ('queued','running')
               AND lease_owner = ?5
               AND claim_attempt = ?6
               AND result_json IS ?4",
            params![
                task_id,
                merged_result_json,
                now,
                current_result_json,
                state.worker.worker_id.as_str(),
                claim_attempt
            ],
        )?;
        if changed > 0 {
            return Ok(());
        }
    }
    Err(worker_task_write_rejection(
        &db,
        state,
        task_id,
        claim_attempt,
        "update_task_progress_result",
        &["running"],
    ))
}

pub(crate) fn update_task_checkpointed_result(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
    result_json: &str,
) -> anyhow::Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let changed = db.execute(
        "UPDATE tasks
         SET result_json = ?2,
             updated_at = ?3,
             lease_owner = NULL,
             lease_expires_at = 0
         WHERE task_id = ?1
           AND status = 'running'
           AND lease_owner = ?4
           AND claim_attempt = ?5",
        params![
            task_id,
            result_json,
            now_ts(),
            state.worker.worker_id.as_str(),
            claim_attempt
        ],
    )?;
    if changed == 0 {
        return Err(worker_task_write_rejection(
            &db,
            state,
            task_id,
            claim_attempt,
            "update_task_checkpointed_result",
            &["running"],
        ));
    }
    Ok(())
}

pub(crate) fn update_task_failure(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
    error_text: &str,
) -> anyhow::Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let result_json = worker_failure_result_json(task_id, error_text);
    let changed = db.execute(
        "UPDATE tasks
         SET status = 'failed', result_json = ?2, error_text = ?3, updated_at = ?4
         WHERE task_id = ?1
           AND status = 'running'
           AND lease_owner = ?5
           AND claim_attempt = ?6",
        params![
            task_id,
            result_json,
            error_text,
            now_ts(),
            state.worker.worker_id.as_str(),
            claim_attempt
        ],
    )?;
    if changed == 0 {
        return Err(worker_task_write_rejection(
            &db,
            state,
            task_id,
            claim_attempt,
            "update_task_failure",
            &["running"],
        ));
    }
    Ok(())
}

pub(crate) fn update_task_failure_with_result(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
    result_json: &str,
    error_text: &str,
) -> anyhow::Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let changed = db.execute(
        "UPDATE tasks
         SET status = 'failed', result_json = ?2, error_text = ?3, updated_at = ?4
         WHERE task_id = ?1
           AND status = 'running'
           AND lease_owner = ?5
           AND claim_attempt = ?6",
        params![
            task_id,
            result_json,
            error_text,
            now_ts(),
            state.worker.worker_id.as_str(),
            claim_attempt
        ],
    )?;
    if changed == 0 {
        return Err(worker_task_write_rejection(
            &db,
            state,
            task_id,
            claim_attempt,
            "update_task_failure_with_result",
            &["running"],
        ));
    }
    Ok(())
}

pub(crate) fn update_task_timeout(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
    error_text: &str,
) -> anyhow::Result<bool> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let existing_result_json = db
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
        .optional()?;
    let Some(existing_result_json) = existing_result_json else {
        return Err(worker_task_write_rejection(
            &db,
            state,
            task_id,
            claim_attempt,
            "update_task_timeout",
            &["running"],
        ));
    };
    if worker_timeout_preserves_recoverable_checkpoint(existing_result_json.as_deref()) {
        let changed = db.execute(
            "UPDATE tasks
             SET updated_at = ?2
             WHERE task_id = ?1
               AND status = 'running'
               AND lease_owner = ?3
               AND claim_attempt = ?4",
            params![
                task_id,
                now_ts(),
                state.worker.worker_id.as_str(),
                claim_attempt
            ],
        )?;
        if changed > 0 {
            warn!(
                "update_task_timeout preserved recoverable checkpoint: task_id={}",
                task_id
            );
            return Ok(false);
        }
    }
    let result_json = worker_timeout_result_json(task_id);
    let changed = db.execute(
        "UPDATE tasks
         SET status = 'timeout', result_json = ?2, error_text = ?3, updated_at = ?4
         WHERE task_id = ?1
           AND status = 'running'
           AND lease_owner = ?5
           AND claim_attempt = ?6",
        params![
            task_id,
            result_json,
            error_text,
            now_ts(),
            state.worker.worker_id.as_str(),
            claim_attempt
        ],
    )?;
    if changed == 0 {
        return Err(worker_task_write_rejection(
            &db,
            state,
            task_id,
            claim_attempt,
            "update_task_timeout",
            &["running"],
        ));
    }
    Ok(true)
}

pub(crate) fn list_active_tasks_internal(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<&str>,
) -> anyhow::Result<Vec<ActiveTaskItem>> {
    let exclude_task_id = normalized_optional_task_id(exclude_task_id);
    let now = now_ts().parse::<i64>().unwrap_or_default();
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, kind, payload_json, status, result_json,
                CAST(COALESCE(NULLIF(created_at, ''), '0') AS INTEGER) AS created_ts,
                CAST(COALESCE(NULLIF(updated_at, ''), created_at, '0') AS INTEGER) AS updated_ts,
                lease_owner,
                lease_expires_at,
                claim_attempt,
                claimed_at
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND status IN ('running', 'queued')
           AND (?3 IS NULL OR task_id <> ?3)
         ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END,
                  created_ts ASC,
                  task_id ASC",
    )?;
    let rows = stmt.query_map(
        params![user_id, chat_id, exclude_task_id.as_deref()],
        |row| {
            let task_id: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let payload_json: String = row.get(2)?;
            let status: String = row.get(3)?;
            let result_json_str: Option<String> = row.get(4)?;
            let created_ts: i64 = row.get(5)?;
            let updated_ts: i64 = row.get(6)?;
            let lease_owner: Option<String> = row.get(7)?;
            let lease_expires_at: i64 = row.get(8)?;
            let claim_attempt: i64 = row.get(9)?;
            let claimed_at: i64 = row.get(10)?;
            Ok((
                task_id,
                kind,
                payload_json,
                status,
                result_json_str,
                created_ts,
                updated_ts,
                lease_owner,
                lease_expires_at,
                claim_attempt,
                claimed_at,
            ))
        },
    )?;
    let mut out = Vec::new();
    for (idx, row) in rows.enumerate() {
        let (
            task_id,
            kind,
            payload_json,
            status,
            result_json_str,
            created_ts,
            updated_ts,
            lease_owner,
            lease_expires_at,
            claim_attempt,
            claimed_at,
        ) = row?;
        let ref_ts = if updated_ts > 0 {
            updated_ts
        } else {
            created_ts
        };
        let age_seconds = if ref_ts > 0 { (now - ref_ts).max(0) } else { 0 };
        let summary = summarize_active_task_payload(&kind, &payload_json);
        let result_json = result_json_str
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
        let mut lifecycle = crate::task_lifecycle::task_query_lifecycle_projection(
            &status,
            result_json.as_ref(),
            (updated_ts > 0).then_some(updated_ts),
        );
        append_task_lease_lifecycle_fields(
            &mut lifecycle,
            lease_owner.as_deref(),
            lease_expires_at,
            claim_attempt,
            claimed_at,
        );
        append_checkpoint_resume_directive_lifecycle_fields(&mut lifecycle, result_json.as_ref());
        let execution_state =
            crate::task_lifecycle::task_execution_state_from_lifecycle(&lifecycle);
        out.push(ActiveTaskItem {
            index: idx + 1,
            task_id,
            kind,
            status,
            execution_state: serde_json::to_value(execution_state)
                .ok()
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| "failed".to_string()),
            summary,
            age_seconds,
            lifecycle: Some(lifecycle),
        });
    }
    Ok(out)
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
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, result_json
         FROM tasks
         WHERE status = 'running'
           AND result_json IS NOT NULL
         ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at, '0') AS INTEGER) ASC,
                  task_id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (task_id, result_json) = row?;
        let Some(result_json) =
            result_json.and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        else {
            continue;
        };
        let Some(ready) =
            ready_paused_checkpoint_resume_executor_from_result_json(task_id, &result_json, now_ts)
        else {
            continue;
        };
        out.push(ready);
        if out.len() >= limit {
            break;
        }
    }
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
            task.claim_attempt
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

pub(crate) fn get_task_query_record(
    state: &AppState,
    task_id: Uuid,
) -> anyhow::Result<Option<(TaskQueryResponse, Option<String>, String)>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;

    let mut stmt = db.prepare(
        "SELECT status, payload_json, result_json, error_text, user_key, channel,
                CAST(COALESCE(NULLIF(updated_at, ''), '0') AS INTEGER) AS updated_ts,
                lease_owner,
                lease_expires_at,
                claim_attempt,
                claimed_at
         FROM tasks
         WHERE task_id = ?1
         LIMIT 1",
    )?;

    let row = stmt
        .query_row(params![task_id.to_string()], |row| {
            let status_str: String = row.get(0)?;
            let payload_json: String = row.get(1)?;
            let result_json_str: Option<String> = row.get(2)?;
            let error_text: Option<String> = row.get(3)?;
            let task_user_key: Option<String> = row.get(4)?;
            let channel: String = row.get(5)?;
            let updated_ts: i64 = row.get(6)?;
            let lease_owner: Option<String> = row.get(7)?;
            let lease_expires_at: i64 = row.get(8)?;
            let claim_attempt: i64 = row.get(9)?;
            let claimed_at: i64 = row.get(10)?;

            let status = parse_task_status(&status_str);

            let result_json = result_json_str
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
            let mut lifecycle = crate::task_lifecycle::task_query_lifecycle_projection(
                &status_str,
                result_json.as_ref(),
                (updated_ts > 0).then_some(updated_ts),
            );
            append_task_lease_lifecycle_fields(
                &mut lifecycle,
                lease_owner.as_deref(),
                lease_expires_at,
                claim_attempt,
                claimed_at,
            );
            append_checkpoint_resume_directive_lifecycle_fields(
                &mut lifecycle,
                result_json.as_ref(),
            );
            let execution_state =
                crate::task_lifecycle::task_execution_state_from_lifecycle(&lifecycle);
            let goal = crate::repo::task_goal::task_goal_projection(
                task_id,
                &payload_json,
                result_json.as_ref(),
                &lifecycle,
            );

            Ok((
                TaskQueryResponse {
                    task_id,
                    status,
                    execution_state: Some(execution_state),
                    goal,
                    result_json,
                    error_text,
                    lifecycle: Some(lifecycle),
                },
                task_user_key,
                channel,
            ))
        })
        .optional()?;

    Ok(row)
}

pub(crate) fn channel_allows_shared_ui_task_access(channel: &str) -> bool {
    matches!(
        channel,
        "telegram" | "whatsapp" | "wechat" | "feishu" | "lark"
    )
}

pub(crate) enum TaskViewerAccessError {
    AuthLookup(anyhow::Error),
    TaskOwnerMismatch,
    InvalidUserKey,
}

pub(crate) fn check_task_view_access(
    state: &AppState,
    task_user_key: Option<&str>,
    channel: &str,
    provided_key: Option<&str>,
) -> Result<(), TaskViewerAccessError> {
    let expected_key = task_user_key.map(str::trim).filter(|v| !v.is_empty());
    let provided_key = provided_key.map(crate::normalize_user_key);
    let provided_key = provided_key.as_deref().filter(|v| !v.is_empty());
    let viewer_identity = match provided_key {
        Some(key) => crate::resolve_auth_identity_by_key(state, key)
            .map_err(TaskViewerAccessError::AuthLookup)?,
        None => None,
    };
    if !channel_allows_shared_ui_task_access(channel) {
        if let Some(expected_key) = expected_key {
            if provided_key != Some(expected_key) {
                return Err(TaskViewerAccessError::TaskOwnerMismatch);
            }
        }
    } else if provided_key.is_some() && viewer_identity.is_none() {
        return Err(TaskViewerAccessError::InvalidUserKey);
    }
    Ok(())
}

#[cfg(test)]
#[path = "tasks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tasks_timeout_tests.rs"]
mod tasks_timeout_tests;

#[cfg(test)]
#[path = "task_cancel_resume_tests.rs"]
mod task_cancel_resume_tests;
