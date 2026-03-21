use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{
    main_flow_rules, now_ts, parse_task_status_with_rules, truncate_for_log, ActiveTaskItem,
    AppState, ClaimedTask, TaskQueryResponse,
};

pub(crate) fn claim_next_task(state: &AppState) -> anyhow::Result<Option<ClaimedTask>> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

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

    let changed = db.execute(
        "UPDATE tasks SET status = 'running', updated_at = ?2 WHERE task_id = ?1 AND status = 'queued'",
        params![task.task_id, now_ts()],
    )?;

    if changed == 0 {
        debug!(
            "claim_next_task: race lost for task_id={}, another worker took it",
            task.task_id
        );
        return Ok(None);
    }

    debug!(
        "claim_next_task: claimed task_id={} user_id={} chat_id={} kind={}",
        task.task_id, task.user_id, task.chat_id, task.kind
    );
    Ok(Some(task))
}

pub(crate) fn update_task_success(
    state: &AppState,
    task_id: &str,
    result_json: &str,
) -> anyhow::Result<()> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let changed = db.execute(
        "UPDATE tasks
         SET status = 'succeeded', result_json = ?2, error_text = NULL, updated_at = ?3
         WHERE task_id = ?1 AND status = 'running'",
        params![task_id, result_json, now_ts()],
    )?;
    if changed == 0 {
        warn!(
            "update_task_success skipped: task_id={} is no longer running",
            task_id
        );
    }
    Ok(())
}

pub(crate) fn touch_running_task(state: &AppState, task_id: &str) -> anyhow::Result<bool> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let changed = db.execute(
        "UPDATE tasks SET updated_at = ?2 WHERE task_id = ?1 AND status = 'running'",
        params![task_id, now_ts()],
    )?;
    Ok(changed > 0)
}

pub(crate) fn is_task_still_running(state: &AppState, task_id: &str) -> anyhow::Result<bool> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let status = db
        .query_row(
            "SELECT status FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(matches!(status.as_deref(), Some("running")))
}

pub(crate) fn update_task_progress_result(
    state: &AppState,
    task_id: &str,
    result_json: &str,
) -> anyhow::Result<()> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
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
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let changed = db.execute(
        "UPDATE tasks
         SET status = 'failed', result_json = NULL, error_text = ?2, updated_at = ?3
         WHERE task_id = ?1 AND status = 'running'",
        params![task_id, error_text, now_ts()],
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
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
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
) -> anyhow::Result<()> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let changed = db.execute(
        "UPDATE tasks
         SET status = 'timeout', result_json = NULL, error_text = ?2, updated_at = ?3
         WHERE task_id = ?1 AND status = 'running'",
        params![task_id, error_text, now_ts()],
    )?;
    if changed == 0 {
        warn!(
            "update_task_timeout skipped: task_id={} is no longer running",
            task_id
        );
    }
    Ok(())
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
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, kind, payload_json, status,
                CAST(COALESCE(NULLIF(created_at, ''), '0') AS INTEGER) AS created_ts,
                CAST(COALESCE(NULLIF(updated_at, ''), created_at, '0') AS INTEGER) AS updated_ts
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
            let created_ts: i64 = row.get(4)?;
            let updated_ts: i64 = row.get(5)?;
            Ok((task_id, kind, payload_json, status, created_ts, updated_ts))
        },
    )?;
    let mut out = Vec::new();
    for (idx, row) in rows.enumerate() {
        let (task_id, kind, payload_json, status, created_ts, updated_ts) = row?;
        let ref_ts = if updated_ts > 0 {
            updated_ts
        } else {
            created_ts
        };
        let age_seconds = if ref_ts > 0 { (now - ref_ts).max(0) } else { 0 };
        let summary = summarize_active_task_payload(&kind, &payload_json);
        out.push(ActiveTaskItem {
            index: idx + 1,
            task_id,
            kind,
            status,
            summary,
            age_seconds,
        });
    }
    Ok(out)
}

pub(crate) fn cancel_tasks_for_user_chat(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<&str>,
) -> anyhow::Result<i64> {
    let now = now_ts();
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

    let mut stmt = db.prepare(
        "UPDATE tasks
         SET status = 'canceled',
             error_text = COALESCE(error_text, 'Canceled by user'),
             updated_at = ?1
         WHERE user_id = ?2
           AND chat_id = ?3
           AND status IN ('queued', 'running')
           AND (?4 IS NULL OR task_id <> ?4)",
    )?;
    let exclude_task_id = normalized_optional_task_id(exclude_task_id);
    let affected = stmt.execute(params![now, user_id, chat_id, exclude_task_id.as_deref()])?;
    Ok(affected as i64)
}

pub(crate) fn cancel_one_task_for_user_chat(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    task_id: &str,
) -> anyhow::Result<i64> {
    let now = now_ts();
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let mut stmt = db.prepare(
        "UPDATE tasks
         SET status = 'canceled',
             error_text = COALESCE(error_text, 'Canceled by user'),
             updated_at = ?1
         WHERE user_id = ?2
           AND chat_id = ?3
           AND task_id = ?4
           AND status IN ('queued', 'running')",
    )?;
    let affected = stmt.execute(params![now, user_id, chat_id, task_id])?;
    Ok(affected as i64)
}

pub(crate) fn get_task_query_record(
    state: &AppState,
    task_id: Uuid,
) -> anyhow::Result<Option<(TaskQueryResponse, Option<String>, String)>> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

    let mut stmt = db.prepare(
        "SELECT status, result_json, error_text, user_key, channel
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

            let status = parse_task_status_with_rules(main_flow_rules(state), &status_str);

            let result_json = result_json_str
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());

            Ok((
                TaskQueryResponse {
                    task_id,
                    status,
                    result_json,
                    error_text,
                },
                task_user_key,
                channel,
            ))
        })
        .optional()?;

    Ok(row)
}

pub(crate) fn channel_allows_shared_ui_task_access(channel: &str) -> bool {
    matches!(channel, "telegram" | "whatsapp" | "feishu" | "lark")
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
