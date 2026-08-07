use super::*;

#[test]
fn execution_workspace_projection_survives_running_and_terminal_updates() {
    let state = AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().unwrap();
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY, status TEXT NOT NULL,
            result_json TEXT, updated_at TEXT NOT NULL
         );
         INSERT INTO tasks(task_id, status, result_json, updated_at)
         VALUES ('workspace-task', 'running', '{\"task_lifecycle\":{\"state\":\"waiting\"}}', '1');",
    )
    .unwrap();
    drop(db);
    assert!(record_task_execution_workspace(
        &state,
        "workspace-task",
        &json!({"binding": "local/worktree", "worktree_id": "workspace-task"}),
    )
    .unwrap());
    let db = state.core.db.get().unwrap();
    let raw: String = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = 'workspace-task'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let result: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(result["task_lifecycle"]["state"], "waiting");
    assert_eq!(result["execution_workspace"]["binding"], "local/worktree");
}
