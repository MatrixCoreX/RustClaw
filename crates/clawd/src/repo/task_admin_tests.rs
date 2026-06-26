use serde_json::{json, Value};
use uuid::Uuid;

use super::*;

fn state_with_tasks_table() -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("get db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT,
            channel TEXT NOT NULL,
            external_user_id TEXT,
            external_chat_id TEXT,
            message_id INTEGER,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL,
            result_json TEXT,
            error_text TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            lease_owner TEXT,
            lease_expires_at INTEGER NOT NULL DEFAULT 0,
            claim_attempt INTEGER NOT NULL DEFAULT 0,
            claimed_at INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("create tasks table");
    drop(db);
    state
}

fn insert_running_task(state: &crate::AppState, task_id: &str, result_json: &Value) {
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES (?1, 42, 7, 'test-key', 'ui', 'ask', ?2, 'running', ?3, NULL, '1', '1')",
        rusqlite::params![
            task_id,
            json!({"text": "visible request"}).to_string(),
            result_json.to_string(),
        ],
    )
    .expect("insert task");
}

fn stored_result_json(state: &crate::AppState, task_id: &str) -> Value {
    let db = state.core.db.get().expect("get db");
    let raw: String = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| row.get(0),
        )
        .expect("select result_json");
    serde_json::from_str(&raw).expect("parse result_json")
}

#[test]
fn cancel_task_by_id_records_provider_cancel_contract_without_text_fields() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    insert_running_task(
        &state,
        &task_id,
        &json!({
            "task_checkpoint": {
                "pending_async_job": {
                    "job_id": "provider:video_generate:minimax:task-1",
                    "status": "running",
                    "poll_after_seconds": 30,
                    "expires_at": 9_999,
                    "cancel_ref": "provider:video_generate:minimax:task-1",
                    "message_key": "clawd.task.async_job_pending",
                    "poll_adapter": {
                        "adapter_kind": "media_job_poll",
                        "skill_name": "video_generate"
                    }
                }
            }
        }),
    );

    let canceled = cancel_task_by_id(&state, &task_id).expect("cancel task");

    assert_eq!(canceled, 1);
    let result = stored_result_json(&state, &task_id);
    assert_eq!(
        result["cancel_adapter_result"]["adapter_kind"],
        "media_job_poll"
    );
    assert_eq!(
        result["cancel_adapter_result"]["status"],
        "requires_provider_adapter"
    );
    assert_eq!(
        result["cancel_adapter_result"]["error_code"],
        "provider_cancel_adapter_missing"
    );
    assert_eq!(
        result["cancel_adapter_result"]["provider_cancel_contract"]["provider"],
        "minimax"
    );
    assert_eq!(
        result["cancel_adapter_result"]["provider_cancel_contract"]["job_id"],
        "task-1"
    );
    assert_eq!(
        result["task_lifecycle"]["cancel_adapter_kind"],
        "media_job_poll"
    );
    assert!(result["cancel_adapter_result"].get("text").is_none());
    assert!(result["cancel_adapter_result"].get("error_text").is_none());
}
