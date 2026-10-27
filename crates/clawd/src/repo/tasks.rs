use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{now_ts, now_ts_u64, parse_task_status, AppState, ClaimedTask, TaskQueryResponse};

mod active;
mod checkpoint_resume;
mod history;
mod lifecycle_projection;
mod success_artifacts;

#[cfg(test)]
pub(crate) use active::{
    list_active_tasks_for_user_internal, list_active_tasks_internal, list_all_active_tasks_internal,
};
pub(crate) use active::{
    list_active_tasks_for_user_page_internal, list_active_tasks_page_internal,
    list_all_active_tasks_page_internal,
};
pub(crate) use checkpoint_resume::{
    claim_due_paused_checkpoint_task_internal,
    claim_ready_paused_checkpoint_resume_executor_internal,
    list_due_paused_checkpoint_tasks_internal,
    list_ready_paused_checkpoint_resume_executors_internal,
    record_paused_checkpoint_resume_execution_plan_internal,
    record_paused_checkpoint_resume_executor_state_internal,
    record_paused_checkpoint_resume_work_item_internal, ClaimedPausedCheckpointResumeExecutor,
    DuePausedCheckpointTask, ReadyPausedCheckpointResumeExecutor,
};
pub(crate) use history::{
    list_all_task_history_internal, list_task_history_for_user_internal, list_task_history_internal,
};
use lifecycle_projection::{
    append_checkpoint_resume_directive_lifecycle_fields, append_task_lease_lifecycle_fields,
    async_poll_terminal_projection_without_visible_reply, normalized_optional_task_id,
    summarize_active_task_payload, worker_failure_result_json,
};
pub(crate) use success_artifacts::prepare_succeeded_result_json;

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

pub(crate) fn claim_next_task(state: &AppState) -> anyhow::Result<Option<ClaimedTask>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;

    let mut stmt = db.prepare(
        "SELECT task.task_id, task.user_id, task.chat_id, task.user_key, task.channel,
                task.external_user_id, task.external_chat_id, task.kind, task.payload_json,
                COALESCE(claim_attempt, 0)
         FROM tasks task
         LEFT JOIN child_task_graph_nodes graph_node
           ON graph_node.child_task_id = task.task_id
         WHERE task.status = 'queued'
           AND COALESCE(
                 CAST(json_extract(task.result_json, '$.task_lifecycle.next_check_after') AS INTEGER),
                 0
               ) <= CAST(strftime('%s', 'now') AS INTEGER)
           AND (
               graph_node.child_task_id IS NULL
               OR graph_node.readiness IN ('ready', 'running')
           )
         ORDER BY
           COALESCE((
             SELECT MAX(recent.claimed_at)
             FROM tasks recent
             WHERE recent.user_id = task.user_id AND recent.claimed_at > 0
           ), 0) ASC,
           task.created_at ASC
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
    crate::repo::child_task_graph::mark_child_graph_node_running(&db, &task.task_id, &now_text)?;

    debug!(
        "claim_next_task: worker_id={} claimed task_id={} user_id={} chat_id={} kind={}",
        state.worker.worker_id, task.task_id, task.user_id, task.chat_id, task.kind
    );
    Ok(Some(task))
}

pub(crate) fn requeue_task_after_resource_wait(
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
         SET status = 'queued',
             result_json = ?2,
             error_text = NULL,
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
            "requeue_task_after_resource_wait",
            &["running"],
        ));
    }
    Ok(())
}

pub(crate) fn update_task_success(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
    result_json: &str,
) -> anyhow::Result<()> {
    // Materialization writes delivery files, so fence stale claims before that
    // side effect as well as at the final compare-and-swap below.
    if !is_task_claim_active_or_pending_ask_success_projection(state, task_id, claim_attempt)? {
        let db = state
            .core
            .db
            .get()
            .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
        return Err(worker_task_write_rejection(
            &db,
            state,
            task_id,
            claim_attempt,
            "update_task_success",
            &["running", "succeeded"],
        ));
    }
    let result_json = success_artifacts::prepare_succeeded_result_json(state, task_id, result_json);
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
    let graph_snapshot = crate::repo::child_task_graph::terminate_parent_graph_children(
        state,
        &db,
        task_id,
        "failed",
        &now_ts(),
    )?;
    drop(db);
    publish_parent_graph_terminal_event(state, task_id, graph_snapshot);
    Ok(())
}

pub(crate) fn update_task_runtime_timeout(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
    runtime_deadline_ms: u64,
    timeout_source: &str,
) -> anyhow::Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let current_result = db
        .query_row(
            "SELECT result_json FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
               AND lease_owner = ?2
               AND claim_attempt = ?3",
            params![task_id, state.worker.worker_id.as_str(), claim_attempt],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let mut result = current_result
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let now = now_ts();
    let now_unix = now.parse::<i64>().unwrap_or_default();
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "status_code".to_string(),
            serde_json::json!("runtime_deadline_exceeded"),
        );
        object.insert(
            "error_code".to_string(),
            serde_json::json!("runtime_deadline_exceeded"),
        );
        object.insert(
            "message_key".to_string(),
            serde_json::json!("clawd.child_task.runtime_deadline_exceeded"),
        );
        object.insert(
            "task_lifecycle".to_string(),
            serde_json::json!({
                "schema_version": crate::child_task_contract::CHILD_TASK_SCHEMA_VERSION,
                "state": "timed_out",
                "thread_state": "done",
                "execution_state": "timed_out",
                "source": "child_runtime_deadline",
                "timeout_class": "operation",
                "timeout_source": timeout_source,
                "runtime_deadline_ms": runtime_deadline_ms,
                "runtime_deadline_at": now_unix,
                "can_cancel": false,
                "can_pause": false,
                "can_resume": false,
                "can_retry": true,
            }),
        );
    }
    let changed = db.execute(
        "UPDATE tasks
         SET status = 'timeout', result_json = ?2,
             error_text = 'runtime_deadline_exceeded', updated_at = ?3,
             lease_owner = NULL, lease_expires_at = 0
         WHERE task_id = ?1
           AND status = 'running'
           AND lease_owner = ?4
           AND claim_attempt = ?5",
        params![
            task_id,
            result.to_string(),
            now,
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
            "update_task_runtime_timeout",
            &["running"],
        ));
    }
    let graph_snapshot = crate::repo::child_task_graph::terminate_parent_graph_children(
        state,
        &db,
        task_id,
        "timeout",
        &now_ts(),
    )?;
    drop(db);
    publish_parent_graph_terminal_event(state, task_id, graph_snapshot);
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
    let graph_snapshot = crate::repo::child_task_graph::terminate_parent_graph_children(
        state,
        &db,
        task_id,
        "failed",
        &now_ts(),
    )?;
    drop(db);
    publish_parent_graph_terminal_event(state, task_id, graph_snapshot);
    Ok(())
}

fn publish_parent_graph_terminal_event(
    state: &AppState,
    parent_task_id: &str,
    graph_snapshot: Option<Value>,
) {
    let Some(graph_snapshot) = graph_snapshot else {
        return;
    };
    if let Err(error) = crate::task_event_transport::publish_event(
        state,
        parent_task_id,
        "subagent_graph",
        graph_snapshot,
    ) {
        warn!(
            "subagent graph terminal event publish failed: task_id={} error={}",
            parent_task_id, error
        );
    }
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

    let mut row = stmt
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
                    task_plan: None,
                    skill_progress: None,
                    result_json,
                    error_text,
                    lifecycle: Some(lifecycle),
                },
                task_user_key,
                channel,
            ))
        })
        .optional()?;

    drop(stmt);
    drop(db);
    if let Some((task, _, _)) = row.as_mut() {
        task.task_plan =
            crate::repo::task_plan::read_task_plan_query_projection(state, &task_id.to_string())
                .map_err(|error| {
                    anyhow::anyhow!(
                        "read task plan projection failed: error_code={} detail={}",
                        error.error_code,
                        error.detail.as_deref().unwrap_or_default()
                    )
                })?;
        task.skill_progress =
            crate::task_event_transport::latest_skill_progress_event(state, &task_id.to_string())?;
    }

    Ok(row)
}

pub(crate) fn get_task_artifact_result_projection(
    state: &AppState,
    task_id: Uuid,
) -> anyhow::Result<Option<(Option<Value>, Option<String>, String)>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    db.query_row(
        "SELECT CASE
                    WHEN json_valid(result_json)
                    THEN json_extract(result_json, '$.artifacts')
                    ELSE NULL
                END,
                user_key,
                channel
         FROM tasks
         WHERE task_id = ?1
         LIMIT 1",
        params![task_id.to_string()],
        |row| {
            let artifacts_json = row.get::<_, Option<String>>(0)?;
            let result_projection = artifacts_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .filter(Value::is_array)
                .map(|artifacts| serde_json::json!({"artifacts": artifacts}));
            Ok((result_projection, row.get(1)?, row.get(2)?))
        },
    )
    .optional()
    .map_err(Into::into)
}

include!("tasks/view_access.rs");

#[cfg(test)]
#[path = "tasks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "task_cancel_resume_tests.rs"]
mod task_cancel_resume_tests;
