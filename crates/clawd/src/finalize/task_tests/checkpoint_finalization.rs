use serde_json::json;

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

fn claimed_ask_task(task_id: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({"text": "start long task"}).to_string(),
    }
}

fn insert_running_task(state: &crate::AppState, task: &crate::ClaimedTask) {
    let db = state.core.db.get().expect("get db");
    let now = crate::now_ts();
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', NULL, NULL, ?8, ?8)",
        rusqlite::params![
            task.task_id,
            task.user_id,
            task.chat_id,
            task.user_key,
            task.channel,
            task.kind,
            task.payload_json,
            now,
        ],
    )
    .expect("insert running task");
}

fn task_status_and_result(state: &crate::AppState, task_id: &str) -> (String, serde_json::Value) {
    let db = state.core.db.get().expect("get db");
    let (status, raw_result): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("select task result");
    (
        status,
        serde_json::from_str(&raw_result).expect("parse result_json"),
    )
}

#[tokio::test]
async fn checkpointed_ask_finalization_overrides_failure_metric() {
    let state = state_with_tasks_table();
    let task = claimed_ask_task("task-checkpoint-finalize");
    insert_running_task(&state, &task);
    {
        let db = state.core.db.get().expect("get db");
        db.execute(
            "UPDATE tasks
             SET lease_owner = 'worker:foreground',
                 lease_expires_at = 1781800300
             WHERE task_id = ?1",
            rusqlite::params![task.task_id],
        )
        .expect("set foreground lease");
    }

    let mut journal =
        crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "start long task");
    journal.record_task_lifecycle(json!({
        "schema_version": 1,
        "state": "waiting",
        "source": "agent_loop_soft_budget",
        "resume_reason": "agent_loop_max_rounds",
        "next_check_after": 1781800060,
        "checkpoint_id": "ckpt-accepted"
    }));
    journal.record_task_checkpoint(json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-accepted",
        "resume_entrypoint": "next_planner_round"
    }));
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);

    super::finalize_ask_checkpointed(
        &state,
        &task,
        r#"{"checkpoint_id":"ckpt-accepted","next_check_after":1781800060}"#,
        &[],
        &mut journal,
    )
    .await
    .expect("checkpoint finalize");

    let (status, result) = task_status_and_result(&state, &task.task_id);
    assert_eq!(status, "running");
    assert_eq!(result["task_journal"]["summary"]["final_status"], "success");
    assert_eq!(
        result["task_journal"]["summary"]["task_lifecycle"]["state"],
        "waiting"
    );
    assert_eq!(
        result["task_journal"]["summary"]["task_lifecycle"]["checkpoint_id"],
        "ckpt-accepted"
    );
    let db = state.core.db.get().expect("get db");
    let (lease_owner, lease_expires_at): (Option<String>, i64) = db
        .query_row(
            "SELECT lease_owner, lease_expires_at
             FROM tasks
             WHERE task_id = ?1",
            rusqlite::params![task.task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("select released lease");
    assert!(lease_owner.is_none());
    assert_eq!(lease_expires_at, 0);
}
