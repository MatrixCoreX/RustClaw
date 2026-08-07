use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use crate::{now_ts, AppState};

pub(crate) fn record_task_execution_workspace(
    state: &AppState,
    task_id: &str,
    projection: &Value,
) -> anyhow::Result<bool> {
    if !projection.is_object() {
        return Ok(false);
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    let row = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((status, raw_result)) = row else {
        return Ok(false);
    };
    let mut result = raw_result
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    result["execution_workspace"] = projection.clone();
    Ok(db.execute(
        "UPDATE tasks SET result_json = ?2, updated_at = ?3
         WHERE task_id = ?1 AND status = ?4
           AND COALESCE(result_json, '') = COALESCE(?5, '')",
        params![task_id, result.to_string(), now_ts(), status, raw_result],
    )? == 1)
}

#[cfg(test)]
#[path = "task_workspace_tests.rs"]
mod tests;
