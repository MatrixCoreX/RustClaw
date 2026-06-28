use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{
    now_ts, now_ts_u64, parse_task_status, truncate_for_log, ActiveTaskItem, AppState, ClaimedTask,
    TaskQueryResponse,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DuePausedCheckpointTask {
    pub(crate) task_id: String,
    pub(crate) lifecycle_state: String,
    pub(crate) checkpoint_id: String,
    pub(crate) task_checkpoint: crate::task_lifecycle::TaskCheckpoint,
    pub(crate) resume_entrypoint: String,
    pub(crate) resume_wait_seconds: i64,
    pub(crate) completed_side_effect_count: usize,
    pub(crate) requires_idempotency_guard: bool,
    pub(crate) checkpoint_resume_directive: crate::task_lifecycle::CheckpointResumeDirective,
    pub(crate) resume_directive: String,
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
        "SELECT task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, kind, payload_json
         FROM tasks
         WHERE status = 'queued'
         ORDER BY created_at ASC
         LIMIT 1",
    )?;

    let candidate = stmt
        .query_row([], |row| {
            Ok(ClaimedTask {
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
    let lease_expires_at = task_worker_lease_expires_at(state, claimed_at);
    let changed = db.execute(
        "UPDATE tasks
         SET status = 'running',
             updated_at = ?2,
             lease_owner = ?3,
             claimed_at = ?4,
             lease_expires_at = ?5,
             claim_attempt = COALESCE(claim_attempt, 0) + 1
         WHERE task_id = ?1
           AND status = 'queued'",
        params![
            task.task_id,
            now_text,
            state.worker.worker_id,
            claimed_at,
            lease_expires_at
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
         WHERE task_id = ?1 AND status = 'running'",
        params![task_id, result_json, now_ts()],
    )?;
    if changed == 0 {
        let existing = db
            .query_row(
                "SELECT status, result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
                params![task_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        if let Some((status, Some(existing_result_json))) = existing {
            if status == "succeeded"
                && async_poll_terminal_projection_without_visible_reply(&existing_result_json)
            {
                let changed = db.execute(
                    "UPDATE tasks
                     SET result_json = ?2, error_text = NULL, updated_at = ?3
                     WHERE task_id = ?1 AND status = 'succeeded' AND result_json = ?4",
                    params![task_id, result_json, now_ts(), existing_result_json],
                )?;
                if changed > 0 {
                    return Ok(());
                }
            }
        }
        warn!(
            "update_task_success skipped: task_id={} is no longer running",
            task_id
        );
    }
    Ok(())
}

pub(crate) fn touch_running_task(state: &AppState, task_id: &str) -> anyhow::Result<bool> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let heartbeat_at = now_ts_u64() as i64;
    let changed = db.execute(
        "UPDATE tasks
         SET updated_at = ?2,
             lease_owner = ?3,
             lease_expires_at = ?4
         WHERE task_id = ?1
           AND status = 'running'",
        params![
            task_id,
            heartbeat_at.to_string(),
            state.worker.worker_id,
            task_worker_lease_expires_at(state, heartbeat_at)
        ],
    )?;
    Ok(changed > 0)
}

fn task_worker_lease_expires_at(state: &AppState, now_ts: i64) -> i64 {
    let lease_seconds = state
        .worker
        .worker_task_heartbeat_seconds
        .saturating_mul(4)
        .max(60);
    now_ts.saturating_add(lease_seconds as i64)
}

fn append_task_lease_lifecycle_fields(
    lifecycle: &mut Value,
    lease_owner: Option<&str>,
    lease_expires_at: i64,
    claim_attempt: i64,
    claimed_at: i64,
) {
    let Some(obj) = lifecycle.as_object_mut() else {
        return;
    };
    if let Some(owner) = lease_owner.map(str::trim).filter(|value| !value.is_empty()) {
        obj.insert("lease_owner".to_string(), serde_json::json!(owner));
    }
    if lease_expires_at > 0 {
        obj.insert(
            "lease_expires_at".to_string(),
            serde_json::json!(lease_expires_at),
        );
    }
    if claim_attempt > 0 {
        obj.insert(
            "claim_attempt".to_string(),
            serde_json::json!(claim_attempt),
        );
        obj.entry("attempt_id".to_string())
            .or_insert(serde_json::json!(claim_attempt));
    }
    if claimed_at > 0 {
        obj.insert("claimed_at".to_string(), serde_json::json!(claimed_at));
    }
}

fn expired_resume_claim_recovery_metadata(
    lifecycle: &Value,
    checkpoint_id: &str,
    now_ts: i64,
) -> Option<(Option<String>, i64)> {
    let claim = lifecycle.get("resume_claim")?;
    let claim_checkpoint_id = claim
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if claim_checkpoint_id != checkpoint_id {
        return None;
    }
    let expires_at = claim.get("expires_at").and_then(Value::as_i64)?;
    if expires_at <= 0 || expires_at > now_ts {
        return None;
    }
    let owner = claim
        .get("owner")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Some((owner, expires_at))
}

pub(crate) fn is_task_still_running(state: &AppState, task_id: &str) -> anyhow::Result<bool> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let status = db
        .query_row(
            "SELECT status FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(matches!(status.as_deref(), Some("running")))
}

pub(crate) fn is_task_still_running_or_pending_ask_success_projection(
    state: &AppState,
    task_id: &str,
) -> anyhow::Result<bool> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let row = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((status, result_json)) = row else {
        return Ok(false);
    };
    if status == "running" {
        return Ok(true);
    }
    Ok(status == "succeeded"
        && result_json
            .as_deref()
            .is_some_and(async_poll_terminal_projection_without_visible_reply))
}

fn async_poll_terminal_projection_without_visible_reply(raw_result_json: &str) -> bool {
    let Ok(result_json) = serde_json::from_str::<Value>(raw_result_json) else {
        return false;
    };
    if result_has_visible_reply(&result_json) {
        return false;
    }
    let Some(lifecycle) = result_json.get("task_lifecycle") else {
        return false;
    };
    lifecycle
        .get("terminal_executor_action")
        .and_then(Value::as_str)
        == Some("poll_async_job")
        && lifecycle
            .get("terminal_executor_result_status")
            .and_then(Value::as_str)
            == Some("async_poll_completed")
        && lifecycle
            .get("resume_executor_result_projection")
            .and_then(|value| value.get("final_result_json"))
            .is_some()
}

fn result_has_visible_reply(result_json: &Value) -> bool {
    result_json
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || result_json
            .get("messages")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.as_str()
                        .map(str::trim)
                        .is_some_and(|value| !value.is_empty())
                })
            })
}

pub(crate) fn update_task_progress_result(
    state: &AppState,
    task_id: &str,
    result_json: &str,
) -> anyhow::Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    db.execute(
        "UPDATE tasks SET result_json = ?2, updated_at = ?3 WHERE task_id = ?1 AND status IN ('queued','running')",
        params![task_id, result_json, now_ts()],
    )?;
    Ok(())
}

pub(crate) fn update_task_failure(
    state: &AppState,
    task_id: &str,
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
         WHERE task_id = ?1 AND status = 'running'",
        params![task_id, result_json, error_text, now_ts()],
    )?;
    if changed == 0 {
        warn!(
            "update_task_failure skipped: task_id={} is no longer running",
            task_id
        );
    }
    Ok(())
}

pub(crate) fn update_task_failure_with_result(
    state: &AppState,
    task_id: &str,
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
         WHERE task_id = ?1 AND status = 'running'",
        params![task_id, result_json, error_text, now_ts()],
    )?;
    if changed == 0 {
        warn!(
            "update_task_failure_with_result skipped: task_id={} is no longer running",
            task_id
        );
    }
    Ok(())
}

pub(crate) fn update_task_timeout(
    state: &AppState,
    task_id: &str,
    error_text: &str,
) -> anyhow::Result<bool> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let existing_result_json = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1 AND status = 'running' LIMIT 1",
            params![task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    if worker_timeout_preserves_recoverable_checkpoint(existing_result_json.as_deref()) {
        let changed = db.execute(
            "UPDATE tasks SET updated_at = ?2 WHERE task_id = ?1 AND status = 'running'",
            params![task_id, now_ts()],
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
         WHERE task_id = ?1 AND status = 'running'",
        params![task_id, result_json, error_text, now_ts()],
    )?;
    if changed == 0 {
        warn!(
            "update_task_timeout skipped: task_id={} is no longer running",
            task_id
        );
        return Ok(false);
    }
    Ok(true)
}

fn worker_timeout_preserves_recoverable_checkpoint(result_json: Option<&str>) -> bool {
    let Some(raw) = result_json else {
        return false;
    };
    let Ok(result_json) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    matches!(
        crate::task_lifecycle::paused_checkpoint_recovery_status(&result_json, now_ts_u64() as i64),
        crate::task_lifecycle::PausedCheckpointRecoveryStatus::Waiting { .. }
    )
}

fn worker_timeout_result_json(task_id: &str) -> String {
    let reason_code =
        crate::task_lifecycle::TerminalFailureReason::ToolTimeoutWithoutAsyncResume.status_code();
    json!({
        "schema_version": 1,
        "status_code": "worker_task_timeout",
        "reason_code": reason_code,
        "message_key": "clawd.task.worker_timeout",
        "task_lifecycle": {
            "schema_version": 1,
            "state": "failed",
            "source": "worker_timeout",
            "terminal_reason": reason_code,
            "reason_code": reason_code,
            "worker_events": [
                {
                    "event_type": "tool_timeout",
                    "owner_layer": "worker_runtime",
                    "task_id": task_id,
                    "state_from": "running",
                    "state_to": "timeout",
                    "reason_code": reason_code,
                }
            ]
        }
    })
    .to_string()
}

fn worker_failure_result_json(task_id: &str, error_text: &str) -> String {
    let reason_code = worker_failure_reason_code(error_text);
    let failure_attribution = worker_failure_attribution(reason_code);
    let message_key = worker_failure_message_key(reason_code);
    json!({
        "schema_version": 1,
        "status_code": "worker_task_failed",
        "reason_code": reason_code,
        "message_key": message_key,
        "failure_attribution": failure_attribution,
        "task_lifecycle": {
            "schema_version": 1,
            "state": "failed",
            "source": "worker_failure",
            "terminal_reason": reason_code,
            "reason_code": reason_code,
            "failure_attribution": failure_attribution,
            "worker_events": [
                {
                    "event_type": "worker_failure",
                    "owner_layer": "worker_runtime",
                    "task_id": task_id,
                    "state_from": "running",
                    "state_to": "failed",
                    "reason_code": reason_code,
                }
            ]
        }
    })
    .to_string()
}

fn worker_failure_reason_code(error_text: &str) -> &'static str {
    let Some(structured) = crate::skills::parse_structured_skill_error(error_text.trim()) else {
        return "worker_runtime_error";
    };
    let error_kind = structured.error_kind.trim().to_ascii_lowercase();
    if matches!(error_kind.as_str(), "timeout" | "idle_timeout") {
        return crate::task_lifecycle::TerminalFailureReason::ToolTimeoutWithoutAsyncResume
            .status_code();
    }
    if error_kind == "confirmation_timeout" {
        return crate::task_lifecycle::TerminalFailureReason::ConfirmationTimeout.status_code();
    }
    if worker_failure_kind_is_provider_window(&error_kind) {
        return crate::task_lifecycle::TerminalFailureReason::ProviderWindowExhausted.status_code();
    }
    "worker_runtime_error"
}

fn worker_failure_kind_is_provider_window(error_kind: &str) -> bool {
    matches!(
        error_kind,
        "provider_error"
            | "provider_retryable_response"
            | "provider_retryable_business"
            | "provider_non_retryable_business"
            | "provider_response_invalid"
            | "provider_schema_error"
            | "provider_unavailable"
            | "provider_rate_limited"
            | "llm_provider_error"
            | "llm_provider_unavailable"
    )
}

fn worker_failure_attribution(reason_code: &str) -> &'static str {
    match reason_code {
        "provider_window_exhausted" => "provider_error",
        "confirmation_timeout" => "confirmation_wait",
        "tool_timeout_without_async_resume" => "tool_timeout",
        _ => "runtime_error",
    }
}

fn worker_failure_message_key(reason_code: &str) -> &'static str {
    match reason_code {
        "provider_window_exhausted" => "clawd.task.provider_window_exhausted",
        "confirmation_timeout" => "clawd.task.confirmation_timeout",
        "tool_timeout_without_async_resume" => "clawd.task.worker_timeout",
        _ => "clawd.task.worker_failed",
    }
}

fn normalized_optional_task_id(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn summarize_active_task_payload(kind: &str, payload_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(payload_json) else {
        return truncate_for_log(payload_json);
    };
    let summary = match kind {
        "ask" => v
            .get("text")
            .and_then(|x| x.as_str())
            .unwrap_or(payload_json)
            .to_string(),
        "run_skill" => {
            let skill = v
                .get("skill_name")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown");
            let action = v
                .get("args")
                .and_then(|x| x.get("action"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim();
            if action.is_empty() {
                format!("run_skill:{skill}")
            } else {
                format!("run_skill:{skill} action={action}")
            }
        }
        _ => payload_json.to_string(),
    };
    truncate_for_log(summary.trim())
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
        let checkpoint_resume_directive =
            crate::task_lifecycle::checkpoint_resume_directive(&result_json, now_ts);
        let resume_directive = checkpoint_resume_directive.status_code().to_string();
        let Some(task_checkpoint) =
            crate::task_lifecycle::task_checkpoint_from_result_json(&result_json)
        else {
            continue;
        };
        out.push(DuePausedCheckpointTask {
            task_id,
            lifecycle_state: state,
            checkpoint_id,
            task_checkpoint,
            resume_entrypoint: resume_entrypoint_token(resume_entrypoint).to_string(),
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
    let raw_result_json = db
        .query_row(
            "SELECT result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
             LIMIT 1",
            params![task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let Some(raw_result_json) = raw_result_json else {
        return Ok(None);
    };
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
    let Some(task_checkpoint) =
        crate::task_lifecycle::task_checkpoint_from_result_json(&result_json)
    else {
        return Ok(None);
    };
    let checkpoint_resume_directive =
        crate::task_lifecycle::checkpoint_resume_directive(&result_json, now_ts);
    let resume_directive = checkpoint_resume_directive.status_code().to_string();

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
             lease_expires_at = ?6
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?4",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json,
            state.worker.worker_id,
            now_ts.saturating_add(lease_seconds)
        ],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    Ok(Some(DuePausedCheckpointTask {
        task_id: task_id.to_string(),
        lifecycle_state,
        checkpoint_id: ready_checkpoint_id,
        task_checkpoint,
        resume_entrypoint: resume_entrypoint_token(resume_entrypoint).to_string(),
        resume_wait_seconds: 0,
        completed_side_effect_count,
        requires_idempotency_guard,
        checkpoint_resume_directive,
        resume_directive,
    }))
}

pub(crate) fn record_paused_checkpoint_resume_work_item_internal(
    state: &AppState,
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
             LIMIT 1",
            params![task_id],
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
           AND result_json = ?4",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json
        ],
    )?;
    Ok(changed > 0)
}

pub(crate) fn record_paused_checkpoint_resume_executor_state_internal(
    state: &AppState,
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
             LIMIT 1",
            params![task_id],
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
           AND result_json = ?4",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json
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
            "SELECT task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, kind, payload_json, result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
             LIMIT 1",
            params![task_id],
            |row| {
                Ok((
                    ClaimedTask {
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
           AND result_json = ?4",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json
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
             LIMIT 1",
            params![task_id],
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
           AND result_json = ?4",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json
        ],
    )?;
    Ok(changed > 0)
}

fn ready_paused_checkpoint_resume_executor_from_result_json(
    task_id: String,
    result_json: &Value,
    now_ts: i64,
) -> Option<ReadyPausedCheckpointResumeExecutor> {
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(result_json), None);
    let lifecycle_obj = lifecycle.as_object()?;
    let lifecycle_state = lifecycle_obj
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(lifecycle_state, "background" | "waiting" | "running") {
        return None;
    }
    let checkpoint_id = lifecycle_obj
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if checkpoint_id.is_empty() {
        return None;
    }
    let resume_executor = lifecycle_obj
        .get("resume_executor")
        .filter(|value| value.is_object())
        .cloned()?;
    let executor_checkpoint_id = resume_executor
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if executor_checkpoint_id != checkpoint_id {
        return None;
    }
    let executor_state = resume_executor
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !resume_executor_state_is_ready(lifecycle_obj, executor_state, checkpoint_id, now_ts) {
        return None;
    }
    let next_check_after = lifecycle_obj
        .get("next_check_after")
        .and_then(Value::as_i64)
        .or_else(|| {
            resume_executor
                .get("next_check_after")
                .and_then(Value::as_i64)
        });
    if next_check_after.is_some_and(|ts| ts > now_ts) {
        return None;
    }
    let task_checkpoint = crate::task_lifecycle::task_checkpoint_from_result_json(result_json)?;
    if task_checkpoint.checkpoint_id != checkpoint_id {
        return None;
    }
    let resume_work_item = lifecycle_obj
        .get("resume_work_item")
        .filter(|value| value.is_object())
        .cloned();
    if let Some(work_item_checkpoint_id) = resume_work_item
        .as_ref()
        .and_then(|value| value.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
    {
        if work_item_checkpoint_id != checkpoint_id {
            return None;
        }
    }
    let resume_trigger = resume_executor
        .get("resume_trigger")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let resume_directive = resume_executor
        .get("resume_directive")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if resume_directive.is_empty() {
        return None;
    }
    Some(ReadyPausedCheckpointResumeExecutor {
        task_id,
        lifecycle_state: lifecycle_state.to_string(),
        checkpoint_id: checkpoint_id.to_string(),
        executor_state: executor_state.to_string(),
        resume_trigger,
        resume_directive,
        next_check_after,
        resume_executor,
        resume_work_item,
        task_checkpoint,
    })
}

fn resume_executor_state_is_ready(
    lifecycle_obj: &serde_json::Map<String, Value>,
    executor_state: &str,
    checkpoint_id: &str,
    now_ts: i64,
) -> bool {
    if matches!(
        executor_state,
        "ready_for_planner_resume" | "ready_to_finalize" | "poll_scheduled"
    ) {
        return true;
    }
    if !matches!(
        executor_state,
        "executing_planner_resume" | "executing_finalize" | "executing_async_poll"
    ) {
        return false;
    }
    let claim = lifecycle_obj.get("resume_executor_claim");
    let claim_checkpoint_id = claim
        .and_then(|value| value.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if claim_checkpoint_id != checkpoint_id {
        return false;
    }
    claim
        .and_then(|value| value.get("expires_at"))
        .and_then(Value::as_i64)
        .is_some_and(|expires_at| expires_at <= now_ts)
}

fn executing_resume_executor_state(executor_state: &str) -> Option<&'static str> {
    match executor_state {
        "ready_for_planner_resume" | "executing_planner_resume" => Some("executing_planner_resume"),
        "ready_to_finalize" | "executing_finalize" => Some("executing_finalize"),
        "poll_scheduled" | "executing_async_poll" => Some("executing_async_poll"),
        _ => None,
    }
}

fn resume_entrypoint_token(entrypoint: crate::task_lifecycle::ResumeEntrypoint) -> &'static str {
    match entrypoint {
        crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound => "next_planner_round",
        crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob => "poll_async_job",
        crate::task_lifecycle::ResumeEntrypoint::AwaitUserInput => "await_user_input",
        crate::task_lifecycle::ResumeEntrypoint::VerifyAndFinalize => "verify_and_finalize",
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
        "SELECT status, result_json, error_text, user_key, channel,
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
            let result_json_str: Option<String> = row.get(1)?;
            let error_text: Option<String> = row.get(2)?;
            let task_user_key: Option<String> = row.get(3)?;
            let channel: String = row.get(4)?;
            let updated_ts: i64 = row.get(5)?;
            let lease_owner: Option<String> = row.get(6)?;
            let lease_expires_at: i64 = row.get(7)?;
            let claim_attempt: i64 = row.get(8)?;
            let claimed_at: i64 = row.get(9)?;

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
            let execution_state =
                crate::task_lifecycle::task_execution_state_from_lifecycle(&lifecycle);

            Ok((
                TaskQueryResponse {
                    task_id,
                    status,
                    execution_state: Some(execution_state),
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
