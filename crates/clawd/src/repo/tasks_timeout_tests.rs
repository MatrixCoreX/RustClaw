use serde_json::json;
use uuid::Uuid;

use super::{get_task_query_record, update_task_timeout};

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

fn insert_task(
    state: &crate::AppState,
    task_id: &str,
    status: &str,
    result_json: Option<&serde_json::Value>,
    updated_at: i64,
) {
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at,
            lease_owner, lease_expires_at, claim_attempt, claimed_at
        )
        VALUES (
            ?1, 42, 7, 'test-key', 'ui', 'ask', ?2, ?3, ?4, NULL, ?5, ?5,
            ?6, 9223372036854775807, 1, ?5
        )",
        rusqlite::params![
            task_id,
            json!({"text": "long task"}).to_string(),
            status,
            result_json.map(|value| value.to_string()),
            updated_at.to_string(),
            state.worker.worker_id.as_str(),
        ],
    )
    .expect("insert task");
}

fn stored_result_json(state: &crate::AppState, task_id: &str) -> serde_json::Value {
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

fn stored_status(state: &crate::AppState, task_id: &str) -> String {
    let db = state.core.db.get().expect("get db");
    db.query_row(
        "SELECT status FROM tasks WHERE task_id = ?1",
        rusqlite::params![task_id],
        |row| row.get(0),
    )
    .expect("select task status")
}

#[test]
fn update_task_timeout_records_structured_lifecycle_reason() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4();
    insert_task(&state, &task_id.to_string(), "running", None, 1234);

    let terminal_timeout = update_task_timeout(
        &state,
        &task_id.to_string(),
        "worker_task_timeout reason_code=tool_timeout_without_async_resume",
    )
    .expect("update timeout");

    assert!(terminal_timeout);
    assert_eq!(stored_status(&state, &task_id.to_string()), "timeout");
    let result = stored_result_json(&state, &task_id.to_string());
    assert_eq!(result["status_code"], "worker_task_timeout");
    assert_eq!(result["reason_code"], "tool_timeout_without_async_resume");
    assert_eq!(result["message_key"], "clawd.task.worker_timeout");
    assert_eq!(result["task_lifecycle"]["state"], "failed");
    assert_eq!(result["task_lifecycle"]["source"], "worker_timeout");
    assert_eq!(
        result["task_lifecycle"]["terminal_reason"],
        "tool_timeout_without_async_resume"
    );
    assert_eq!(
        result["task_lifecycle"]["worker_events"][0]["event_type"],
        "tool_timeout"
    );

    let (response, _, _) = get_task_query_record(&state, task_id)
        .expect("query task")
        .expect("task exists");
    let lifecycle = response.lifecycle.expect("lifecycle projection");
    assert_eq!(lifecycle["db_status"], "timeout");
    assert_eq!(
        lifecycle["reason_code"],
        "tool_timeout_without_async_resume"
    );
    assert_eq!(
        lifecycle["terminal_reason"],
        "tool_timeout_without_async_resume"
    );
}

#[test]
fn update_task_timeout_preserves_recoverable_async_checkpoint() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4();
    let now = crate::now_ts_u64() as i64;
    let result_json = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "source": "direct_run_skill_async_start_adapter",
            "resume_reason": "pending_async_job",
            "next_check_after": now + 30,
            "checkpoint_id": "ckpt-async-timeout"
        },
        "task_checkpoint": {
            "schema_version": 1,
            "checkpoint_id": "ckpt-async-timeout",
            "pending_async_job": {
                "job_id": "local_process:async-timeout",
                "status": "running",
                "poll_after_seconds": 1,
                "expires_at": now + 600,
                "cancel_ref": "local_process:/tmp/async-timeout",
                "message_key": "clawd.task.async_job_pending"
            }
        }
    });
    insert_task(
        &state,
        &task_id.to_string(),
        "running",
        Some(&result_json),
        1234,
    );

    let terminal_timeout = update_task_timeout(
        &state,
        &task_id.to_string(),
        "worker_task_timeout reason_code=tool_timeout_without_async_resume",
    )
    .expect("update timeout");

    assert!(!terminal_timeout);
    assert_eq!(stored_status(&state, &task_id.to_string()), "running");
    let stored = stored_result_json(&state, &task_id.to_string());
    assert_eq!(stored["task_lifecycle"]["state"], "waiting");
    assert_eq!(
        stored["task_checkpoint"]["pending_async_job"]["job_id"],
        "local_process:async-timeout"
    );
}
