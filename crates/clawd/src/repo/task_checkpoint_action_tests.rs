use super::{load_task_checkpoint_action, upsert_task_checkpoint_action};

fn pool() -> crate::db_init::DbPool {
    let pool = crate::db_init::test_pool();
    let db = pool.get().expect("test db");
    db.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE tasks (task_id TEXT PRIMARY KEY);",
    )
    .expect("create tasks");
    db.execute("INSERT INTO tasks (task_id) VALUES ('task-1')", [])
        .expect("insert task");
    drop(db);
    pool
}

#[test]
fn checkpoint_action_round_trips_exact_private_args_and_contract() {
    let pool = pool();
    let args = serde_json::json!({
        "command": "printf checkpoint_ok > run/checkpoint.txt",
        "cwd": "/workspace"
    });
    let contract = serde_json::json!({
        "response_shape": "strict",
        "semantic_kind": "raw_command_output",
        "selection": {
            "structured_field_selector": "command,created_path,status"
        }
    });
    let continuation_actions = serde_json::json!([
        {
            "type": "call_capability",
            "capability": "filesystem.write_text",
            "args": {"path": "run/result.txt", "content": "ok"}
        },
        {"type": "synthesize_answer", "evidence_refs": []}
    ]);

    upsert_task_checkpoint_action(
        &pool,
        "task-1",
        "checkpoint-1",
        "run_cmd",
        "system.run_command",
        &args,
        Some(&contract),
        Some(&continuation_actions),
    )
    .expect("store action");

    let stored = load_task_checkpoint_action(&pool, "task-1", "checkpoint-1")
        .expect("load action")
        .expect("stored action");
    assert_eq!(stored.task_id, "task-1");
    assert_eq!(stored.checkpoint_id, "checkpoint-1");
    assert_eq!(stored.tool_or_skill, "run_cmd");
    assert_eq!(stored.action_ref, "system.run_command");
    assert_eq!(stored.args, args);
    assert_eq!(stored.output_contract.as_ref(), Some(&contract));
    assert_eq!(
        stored.continuation_actions.as_ref(),
        Some(&continuation_actions)
    );
    assert!(
        load_task_checkpoint_action(&pool, "task-1", "other-checkpoint")
            .expect("load other checkpoint")
            .is_none()
    );
}

#[test]
fn checkpoint_action_rejects_integrity_mismatch() {
    let pool = pool();
    upsert_task_checkpoint_action(
        &pool,
        "task-1",
        "checkpoint-1",
        "run_cmd",
        "system.run_command",
        &serde_json::json!({"command": "printf original"}),
        None,
        Some(&serde_json::json!([
            {"type": "synthesize_answer", "evidence_refs": []}
        ])),
    )
    .expect("store action");
    pool.get()
        .expect("test db")
        .execute(
            "UPDATE task_checkpoint_actions
             SET continuation_actions_json = '[{\"type\":\"respond\",\"content\":\"tampered\"}]'
             WHERE task_id = 'task-1' AND checkpoint_id = 'checkpoint-1'",
            [],
        )
        .expect("tamper action");

    let error = load_task_checkpoint_action(&pool, "task-1", "checkpoint-1")
        .expect_err("integrity mismatch");
    assert_eq!(error.to_string(), "checkpoint_action_integrity_mismatch");
}
