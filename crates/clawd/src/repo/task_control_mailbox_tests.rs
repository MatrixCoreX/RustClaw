use super::*;

fn state_with_task() -> AppState {
    let state = AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().unwrap();
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY, status TEXT NOT NULL
         );
         INSERT INTO tasks(task_id, status) VALUES ('task-control', 'running');",
    )
    .unwrap();
    drop(db);
    state
}

fn input(action: &str, key: &str) -> EnqueueTaskControl {
    EnqueueTaskControl {
        task_id: "task-control".to_string(),
        action: action.to_string(),
        issued_by: "admin:test".to_string(),
        payload: serde_json::json!({"message": "keep the completed receipt"}),
        idempotency_key: Some(key.to_string()),
        expected_control_seq: None,
    }
}

#[test]
fn mailbox_is_monotonic_idempotent_and_restart_durable() {
    let state = state_with_task();
    let first = enqueue_task_control(&state, input("steer", "same"))
        .unwrap()
        .unwrap();
    let duplicate = enqueue_task_control(&state, input("steer", "same"))
        .unwrap()
        .unwrap();
    let second = enqueue_task_control(&state, input("pause", "next"))
        .unwrap()
        .unwrap();
    assert_eq!(first.control_id, duplicate.control_id);
    assert_eq!(first.control_seq, 1);
    assert_eq!(second.control_seq, 2);

    let pending = pending_task_control_directives(&state, "task-control", 10).unwrap();
    assert_eq!(pending.len(), 2);
    assert!(apply_task_control_directive(&state, "task-control", 1, "steering_applied").unwrap());
    assert!(!apply_task_control_directive(&state, "task-control", 1, "duplicate").unwrap());
    let pending = pending_task_control_directives(&state, "task-control", 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].action, "pause");
}

#[test]
fn expected_sequence_rejects_conflicting_steering() {
    let state = state_with_task();
    enqueue_task_control(&state, input("steer", "first")).unwrap();
    let mut conflicting = input("steer", "second");
    conflicting.expected_control_seq = Some(0);
    assert!(enqueue_task_control(&state, conflicting)
        .unwrap_err()
        .to_string()
        .contains("task_control_version_conflict"));
}
