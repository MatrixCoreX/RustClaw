use rusqlite::params;

use super::summarize_active_task_payload;
use crate::{AppState, TaskHistoryItem};

pub(crate) fn list_task_history_internal(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    limit: usize,
    offset: usize,
) -> anyhow::Result<(Vec<TaskHistoryItem>, usize)> {
    list_task_history_scoped_internal(state, Some(user_id), Some(chat_id), limit, offset)
}

pub(crate) fn list_task_history_for_user_internal(
    state: &AppState,
    user_id: i64,
    limit: usize,
    offset: usize,
) -> anyhow::Result<(Vec<TaskHistoryItem>, usize)> {
    list_task_history_scoped_internal(state, Some(user_id), None, limit, offset)
}

pub(crate) fn list_all_task_history_internal(
    state: &AppState,
    limit: usize,
    offset: usize,
) -> anyhow::Result<(Vec<TaskHistoryItem>, usize)> {
    list_task_history_scoped_internal(state, None, None, limit, offset)
}

fn list_task_history_scoped_internal(
    state: &AppState,
    user_id: Option<i64>,
    chat_id: Option<i64>,
    limit: usize,
    offset: usize,
) -> anyhow::Result<(Vec<TaskHistoryItem>, usize)> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    let total = db.query_row(
        "SELECT COUNT(*)
         FROM tasks
         WHERE (?1 IS NULL OR user_id = ?1)
           AND (?2 IS NULL OR chat_id = ?2)
           AND status IN ('succeeded', 'failed', 'canceled', 'timeout')",
        params![user_id, chat_id],
        |row| row.get::<_, i64>(0),
    )?;
    let mut stmt = db.prepare(
        "SELECT task_id, kind, payload_json, status,
                channel, user_id, external_user_id,
                CAST(COALESCE(NULLIF(created_at, ''), '0') AS INTEGER) AS created_ts,
                CAST(COALESCE(NULLIF(updated_at, ''), created_at, '0') AS INTEGER) AS updated_ts
         FROM tasks
         WHERE (?1 IS NULL OR user_id = ?1)
           AND (?2 IS NULL OR chat_id = ?2)
           AND status IN ('succeeded', 'failed', 'canceled', 'timeout')
         ORDER BY created_ts DESC, task_id DESC
         LIMIT ?3 OFFSET ?4",
    )?;
    let rows = stmt.query_map(
        params![user_id, chat_id, limit as i64, offset as i64],
        |row| {
            let kind = row.get::<_, String>(1)?;
            let payload_json = row.get::<_, String>(2)?;
            let created_at_ts = row.get::<_, i64>(7)?;
            let updated_at_ts = row.get::<_, i64>(8)?;
            Ok(TaskHistoryItem {
                task_id: row.get(0)?,
                summary: summarize_active_task_payload(&kind, &payload_json),
                kind,
                status: row.get(3)?,
                channel: row.get(4)?,
                source_user_id: row.get::<_, i64>(5)?.to_string(),
                external_user_id: row
                    .get::<_, Option<String>>(6)?
                    .filter(|value| !value.trim().is_empty()),
                created_at_ts,
                updated_at_ts,
                duration_seconds: (updated_at_ts - created_at_ts).max(0),
            })
        },
    )?;
    let tasks = rows.collect::<Result<Vec<_>, _>>()?;
    Ok((tasks, total.max(0) as usize))
}
