use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use crate::{now_ts, AppState};

pub(crate) fn fail_noninteractive_child_approval(
    state: &AppState,
    task_id: &str,
    claim_attempt: i64,
) -> anyhow::Result<bool> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
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
    let Some(mut result) = current_result
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
    else {
        return Ok(false);
    };
    let needs_user = result
        .pointer("/task_lifecycle/state")
        .and_then(Value::as_str)
        == Some("needs_user");
    let approval_pending = result
        .pointer("/resume_context/approval_request/status")
        .and_then(Value::as_str)
        == Some("pending");
    if !needs_user || !approval_pending {
        return Ok(false);
    }
    let now = now_ts();
    if let Some(object) = result.as_object_mut() {
        object.insert("status_code".to_string(), json!("approval_unavailable"));
        object.insert("error_code".to_string(), json!("approval_unavailable"));
        object.insert(
            "message_key".to_string(),
            json!("clawd.child_task.approval_unavailable"),
        );
        object.insert("retryable".to_string(), json!(true));
        object.insert(
            "task_lifecycle".to_string(),
            json!({
                "schema_version": crate::child_task_contract::CHILD_TASK_SCHEMA_VERSION,
                "state": "failed",
                "thread_state": "done",
                "execution_state": "failed",
                "source": "noninteractive_child_approval_gate",
                "waiting_reason": "approval_unavailable",
                "can_cancel": false,
                "can_pause": false,
                "can_resume": false,
                "can_retry": true,
            }),
        );
    }
    let changed = db.execute(
        "UPDATE tasks
         SET status = 'failed', result_json = ?2,
             error_text = 'approval_unavailable', updated_at = ?3,
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
    Ok(changed == 1)
}
