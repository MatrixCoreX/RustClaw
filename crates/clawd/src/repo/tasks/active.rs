use rusqlite::params;
use serde_json::Value;

use super::{
    append_checkpoint_resume_directive_lifecycle_fields, append_task_lease_lifecycle_fields,
    normalized_optional_task_id, summarize_active_task_payload,
};
use crate::{now_ts, ActiveTaskItem, AppState};

pub(crate) fn list_active_tasks_internal(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<&str>,
) -> anyhow::Result<Vec<ActiveTaskItem>> {
    list_active_tasks_scoped_internal(state, Some(user_id), Some(chat_id), exclude_task_id)
}

pub(crate) fn list_active_tasks_for_user_internal(
    state: &AppState,
    user_id: i64,
    exclude_task_id: Option<&str>,
) -> anyhow::Result<Vec<ActiveTaskItem>> {
    list_active_tasks_scoped_internal(state, Some(user_id), None, exclude_task_id)
}

pub(crate) fn list_all_active_tasks_internal(
    state: &AppState,
    exclude_task_id: Option<&str>,
) -> anyhow::Result<Vec<ActiveTaskItem>> {
    list_active_tasks_scoped_internal(state, None, None, exclude_task_id)
}

fn list_active_tasks_scoped_internal(
    state: &AppState,
    user_id: Option<i64>,
    chat_id: Option<i64>,
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
         WHERE (?1 IS NULL OR user_id = ?1)
           AND (?2 IS NULL OR chat_id = ?2)
           AND status IN ('running', 'queued')
           AND (?3 IS NULL OR task_id <> ?3)
         ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END,
                  created_ts ASC,
                  task_id ASC",
    )?;
    let rows = stmt.query_map(
        params![user_id, chat_id, exclude_task_id.as_deref()],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
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
