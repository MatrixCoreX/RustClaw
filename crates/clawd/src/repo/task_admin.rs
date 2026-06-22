use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::{now_ts, AppState};

const TASK_CANCELLED_SOURCE: &str = "task_admin_cancel";
const TASK_CANCELLED_MESSAGE_KEY: &str = "clawd.task.cancelled";
const TASK_CONTROL_SOURCE: &str = "task_admin_control";
const TASK_PAUSED_MESSAGE_KEY: &str = "clawd.task.pause_requested";
const TASK_RESUMED_MESSAGE_KEY: &str = "clawd.task.resume_requested";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskAdminTarget {
    pub(crate) task_id: String,
    pub(crate) user_id: i64,
    pub(crate) chat_id: i64,
    pub(crate) user_key: Option<String>,
    pub(crate) channel: String,
    pub(crate) status: String,
}

struct CancelTaskRecord {
    task_id: String,
    result_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TaskControlUpdate {
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) lifecycle: Value,
}

pub(crate) fn cancel_tasks_for_user_chat(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<&str>,
) -> anyhow::Result<i64> {
    let now = now_ts();
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let exclude_task_id = normalized_optional_task_id(exclude_task_id);
    let mut stmt = db.prepare(
        "SELECT task_id, result_json
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND status IN ('queued', 'running')
           AND (?3 IS NULL OR task_id <> ?3)",
    )?;
    let records = stmt
        .query_map(
            params![user_id, chat_id, exclude_task_id.as_deref()],
            |row| {
                Ok(CancelTaskRecord {
                    task_id: row.get(0)?,
                    result_json: row.get(1)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    cancel_task_records(&db, records, &now)
}

pub(crate) fn cancel_one_task_for_user_chat(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    task_id: &str,
) -> anyhow::Result<i64> {
    let now = now_ts();
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, result_json
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND task_id = ?3
           AND status IN ('queued', 'running')",
    )?;
    let records = stmt
        .query_map(params![user_id, chat_id, task_id], |row| {
            Ok(CancelTaskRecord {
                task_id: row.get(0)?,
                result_json: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    cancel_task_records(&db, records, &now)
}

pub(crate) fn get_task_admin_target(
    state: &AppState,
    task_id: &str,
) -> anyhow::Result<Option<TaskAdminTarget>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, user_id, chat_id, user_key, channel, status
         FROM tasks
         WHERE task_id = ?1
         LIMIT 1",
    )?;
    let target = stmt
        .query_row(params![task_id], |row| {
            Ok(TaskAdminTarget {
                task_id: row.get(0)?,
                user_id: row.get(1)?,
                chat_id: row.get(2)?,
                user_key: row.get(3)?,
                channel: row.get(4)?,
                status: row.get(5)?,
            })
        })
        .optional()?;
    Ok(target)
}

pub(crate) fn cancel_task_by_id(state: &AppState, task_id: &str) -> anyhow::Result<i64> {
    let now = now_ts();
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, result_json
         FROM tasks
         WHERE task_id = ?1
           AND status IN ('queued', 'running')",
    )?;
    let records = stmt
        .query_map(params![task_id], |row| {
            Ok(CancelTaskRecord {
                task_id: row.get(0)?,
                result_json: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    cancel_task_records(&db, records, &now)
}

pub(crate) fn resume_task_by_id(
    state: &AppState,
    task_id: &str,
) -> anyhow::Result<Option<TaskControlUpdate>> {
    let now_ts = crate::now_ts_u64() as i64;
    update_paused_checkpoint_schedule(state, task_id, now_ts, now_ts, TASK_RESUMED_MESSAGE_KEY)
}

pub(crate) fn pause_task_by_id(
    state: &AppState,
    task_id: &str,
    pause_seconds: u64,
) -> anyhow::Result<Option<TaskControlUpdate>> {
    let now_ts = crate::now_ts_u64() as i64;
    let pause_seconds = pause_seconds.clamp(1, 604_800) as i64;
    update_paused_checkpoint_schedule(
        state,
        task_id,
        now_ts,
        now_ts.saturating_add(pause_seconds),
        TASK_PAUSED_MESSAGE_KEY,
    )
}

fn cancel_task_records(
    db: &Connection,
    records: Vec<CancelTaskRecord>,
    now: &str,
) -> anyhow::Result<i64> {
    let reason = crate::task_lifecycle::TerminalFailureReason::UserCancelled.status_code();
    let now_ts = now.parse::<i64>().unwrap_or_default();
    let mut affected = 0_i64;
    for record in records {
        let result_json = cancelled_task_result_json(record.result_json.as_deref(), reason, now_ts);
        let count = db.execute(
            "UPDATE tasks
             SET status = 'canceled',
                 error_text = ?1,
                 result_json = ?2,
                 updated_at = ?3
             WHERE task_id = ?4
               AND status IN ('queued', 'running')",
            params![reason, result_json.to_string(), now, record.task_id],
        )?;
        affected += count as i64;
    }
    Ok(affected)
}

fn cancelled_task_result_json(raw_result_json: Option<&str>, reason: &str, now_ts: i64) -> Value {
    let mut result = raw_result_json
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = result.as_object_mut() {
        obj.insert("status_code".to_string(), json!(reason));
        obj.insert("error_code".to_string(), json!(reason));
        obj.insert("terminal_reason".to_string(), json!(reason));
        obj.insert("message_key".to_string(), json!(TASK_CANCELLED_MESSAGE_KEY));
        obj.insert(
            "task_lifecycle".to_string(),
            json!({
                "schema_version": 1,
                "state": "cancelled",
                "source": TASK_CANCELLED_SOURCE,
                "terminal_reason": reason,
                "message_key": TASK_CANCELLED_MESSAGE_KEY,
                "can_cancel": false,
                "cancelled_at": now_ts,
            }),
        );
    }
    result
}

fn update_paused_checkpoint_schedule(
    state: &AppState,
    task_id: &str,
    now_ts: i64,
    next_check_after: i64,
    message_key: &str,
) -> anyhow::Result<Option<TaskControlUpdate>> {
    let task_id = task_id.trim();
    if task_id.is_empty() {
        return Ok(None);
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
        return Ok(None);
    };
    let mut result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) if value.is_object() => value,
        _ => return Ok(None),
    };
    let readiness = crate::task_lifecycle::paused_checkpoint_resume_readiness(&result_json, now_ts);
    if matches!(
        &readiness,
        crate::task_lifecycle::PausedCheckpointResumeReadiness::NotPaused
            | crate::task_lifecycle::PausedCheckpointResumeReadiness::InvalidPausedCheckpoint
            | crate::task_lifecycle::PausedCheckpointResumeReadiness::MissingTaskCheckpoint { .. }
            | crate::task_lifecycle::PausedCheckpointResumeReadiness::InvalidTaskCheckpoint { .. }
            | crate::task_lifecycle::PausedCheckpointResumeReadiness::CheckpointMismatch { .. }
            | crate::task_lifecycle::PausedCheckpointResumeReadiness::ActiveResumeLease { .. }
    ) {
        return Ok(None);
    }
    let checkpoint_id = match readiness {
        crate::task_lifecycle::PausedCheckpointResumeReadiness::WaitingNotDue {
            checkpoint_id,
            ..
        }
        | crate::task_lifecycle::PausedCheckpointResumeReadiness::Ready { checkpoint_id, .. } => {
            checkpoint_id
        }
        _ => return Ok(None),
    };
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(obj) = lifecycle.as_object_mut() else {
        return Ok(None);
    };
    obj.insert("source".to_string(), json!(TASK_CONTROL_SOURCE));
    obj.insert("next_check_after".to_string(), json!(next_check_after));
    obj.insert(
        "resume_due".to_string(),
        json!(next_check_after.saturating_sub(now_ts) == 0),
    );
    obj.insert(
        "resume_wait_seconds".to_string(),
        json!(next_check_after.saturating_sub(now_ts).max(0)),
    );
    obj.insert("message_key".to_string(), json!(message_key));
    obj.insert("manual_control_requested_at".to_string(), json!(now_ts));
    result_json["task_lifecycle"] = lifecycle.clone();
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
    Ok(Some(TaskControlUpdate {
        task_id: task_id.to_string(),
        checkpoint_id,
        lifecycle,
    }))
}

fn normalized_optional_task_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
