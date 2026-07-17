use rusqlite::params;
use serde_json::json;

fn running_task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: "worker-runtime-error-task".to_string(),
        user_id: 7,
        chat_id: 9,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({"text": "test"}).to_string(),
    }
}

#[test]
fn worker_runtime_error_immediately_transitions_running_task_to_failed() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = running_task();
    {
        let db = state.core.db.get().expect("get db");
        db.execute_batch(
            "CREATE TABLE tasks (
                task_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                result_json TEXT,
                error_text TEXT,
                updated_at TEXT NOT NULL
            );",
        )
        .expect("create tasks table");
        db.execute(
            "INSERT INTO tasks (task_id, status, updated_at)
             VALUES (?1, 'running', '0')",
            params![task.task_id],
        )
        .expect("insert running task");
    }

    super::finalize_worker_runtime_error(
        &state,
        &task,
        Some(&json!({"text": "test"})),
        &anyhow::anyhow!("context_prompt_overhead_budget_exceeded"),
    )
    .expect("finalize runtime error");

    let db = state.core.db.get().expect("get db");
    let (status, result_json, error_text): (String, String, String) = db
        .query_row(
            "SELECT status, result_json, error_text FROM tasks WHERE task_id = ?1",
            params![task.task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("load task");
    let result: serde_json::Value =
        serde_json::from_str(&result_json).expect("parse structured failure");

    assert_eq!(status, "failed");
    assert_eq!(error_text, "context_prompt_overhead_budget_exceeded");
    assert_eq!(result["status_code"], "worker_task_failed");
    assert_eq!(result["reason_code"], "worker_runtime_error");
    assert_eq!(result["task_lifecycle"]["state"], "failed");
}
