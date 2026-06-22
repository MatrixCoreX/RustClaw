use anyhow::anyhow;
use rusqlite::Connection;
use serde_json::Value;
use tracing::warn;

use crate::{now_ts, now_ts_u64, AppState};

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
    let mut task_ids = Vec::new();
    {
        let mut stmt = db.prepare(
            "SELECT task_id, result_json
             FROM tasks
             WHERE status = 'running'
               AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?1
             ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![stale_before.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (task_id, result_json) = row?;
            if recovery_should_preserve_paused_checkpoint(result_json.as_deref(), now) {
                continue;
            }
            task_ids.push(task_id);
        }
    }
    if task_ids.is_empty() {
        return Ok(task_ids);
    }

    let stale_note = format!(
        "auto timeout on startup: no progress heartbeat for {}s while status=running",
        no_progress_timeout_seconds.max(1)
    );

    let mut changed = 0;
    for task_id in &task_ids {
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
               AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?2",
            rusqlite::params![task_id, stale_before.to_string(), stale_note, now_ts()],
        )?;
    }
    if changed != task_ids.len() {
        warn!(
            "startup stale-running recovery count mismatch: selected={} updated={}",
            task_ids.len(),
            changed
        );
    }

    Ok(task_ids)
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
    let stale_note = format!(
        "auto timeout: no progress heartbeat for {}s while status=running",
        timeout_secs
    );
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;

    let mut task_ids = Vec::new();
    {
        let mut stmt = db.prepare(
            "SELECT task_id, result_json
             FROM tasks
             WHERE status = 'running'
               AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?1
             ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![stale_before.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (task_id, result_json) = row?;
            if recovery_should_preserve_paused_checkpoint(result_json.as_deref(), now) {
                continue;
            }
            task_ids.push(task_id);
        }
    }

    if task_ids.is_empty() {
        return Ok(task_ids);
    }

    let mut changed = 0;
    for task_id in &task_ids {
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
               AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?2",
            rusqlite::params![task_id, stale_before.to_string(), stale_note, now_ts()],
        )?;
    }
    if changed != task_ids.len() {
        warn!(
            "runtime stale-running recovery count mismatch: selected={} updated={}",
            task_ids.len(),
            changed
        );
    }
    Ok(task_ids)
}
