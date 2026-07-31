use rusqlite::{params, OptionalExtension};
use serde_json::Value;

use crate::{AppState, ClaimedTask};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TaskDeliveryRecord {
    pub(crate) task: ClaimedTask,
    pub(crate) status: String,
    pub(crate) result_json: Option<Value>,
    pub(crate) error_text: Option<String>,
}

pub(crate) fn get_task_delivery_record(
    state: &AppState,
    task_id: &str,
) -> anyhow::Result<Option<TaskDeliveryRecord>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    db.query_row(
        "SELECT task_id, user_id, chat_id, user_key, channel,
                external_user_id, external_chat_id, kind, payload_json,
                COALESCE(claim_attempt, 0), status, result_json, error_text
         FROM tasks
         WHERE task_id = ?1
         LIMIT 1",
        params![task_id],
        |row| {
            let raw_result: Option<String> = row.get(11)?;
            Ok(TaskDeliveryRecord {
                task: ClaimedTask {
                    task_id: row.get(0)?,
                    user_id: row.get(1)?,
                    chat_id: row.get(2)?,
                    user_key: row.get(3)?,
                    channel: row.get(4)?,
                    external_user_id: row.get(5)?,
                    external_chat_id: row.get(6)?,
                    kind: row.get(7)?,
                    payload_json: row.get(8)?,
                    claim_attempt: row.get(9)?,
                },
                status: row.get(10)?,
                result_json: raw_result
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok()),
                error_text: row.get(12)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

#[cfg(test)]
#[path = "task_delivery_tests.rs"]
mod tests;
