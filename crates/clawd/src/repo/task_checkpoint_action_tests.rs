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
    let execution_binding = serde_json::json!({
        "schema_version": 1,
        "skill_name": "run_cmd",
        "registry_generation": 7,
    });
    let approval_binding = serde_json::json!({
        "schema_version": 1,
        "action_fingerprint": "sha256:action",
        "arguments_hash": "sha256:args",
        "action_count": 1,
        "targets": ["run_cmd"],
    });

    upsert_task_checkpoint_action(
        &pool,
        "task-1",
        "checkpoint-1",
        "run_cmd",
        "system.run_command",
        &args,
        Some(&contract),
        Some(&continuation_actions),
        Some(&execution_binding),
        Some(&approval_binding),
        4,
        6,
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
    assert_eq!(stored.execution_binding.as_ref(), Some(&execution_binding));
    assert_eq!(stored.approval_binding.as_ref(), Some(&approval_binding));
    assert_eq!(stored.instruction_revision, 4);
    assert_eq!(stored.execution_epoch, 6);
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
        Some(&serde_json::json!({
            "schema_version": 1,
            "skill_name": "run_cmd",
        })),
        Some(&serde_json::json!({
            "schema_version": 1,
            "action_fingerprint": "sha256:action",
            "arguments_hash": "sha256:args",
            "action_count": 1,
            "targets": ["run_cmd"],
        })),
        1,
        2,
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
