use serde_json::json;

#[test]
fn noninteractive_child_approval_returns_stable_machine_failure() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let worker_id = state.worker.worker_id.clone();
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, channel, kind, payload_json, status,
            result_json, error_text, created_at, updated_at, lease_owner,
            lease_expires_at, claim_attempt, claimed_at
         ) VALUES (
            'approval-child', 42, 7, 'ui', 'ask', '{}', 'running', ?1,
            NULL, '1', '1', ?2, 9999999999, 2, 1
         )",
        rusqlite::params![
            json!({
                "task_lifecycle": {"state": "needs_user"},
                "resume_context": {
                    "approval_request": {"status": "pending", "request_id": "approval-1"}
                }
            })
            .to_string(),
            worker_id
        ],
    )
    .expect("insert approval waiting child");
    drop(db);

    assert!(
        crate::repo::fail_noninteractive_child_approval(&state, "approval-child", 2)
            .expect("fail unavailable approval")
    );
    let db = state.core.db.get().expect("get db after failure");
    let (status, raw_result): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = 'approval-child'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read approval failure");
    let result: serde_json::Value = serde_json::from_str(&raw_result).expect("parse result");
    assert_eq!(status, "failed");
    assert_eq!(result["error_code"], "approval_unavailable");
    assert_eq!(
        result["message_key"],
        "clawd.child_task.approval_unavailable"
    );
    assert_eq!(result["retryable"], true);
    assert_eq!(
        result["task_lifecycle"]["waiting_reason"],
        "approval_unavailable"
    );
    assert!(
        !crate::repo::fail_noninteractive_child_approval(&state, "approval-child", 2)
            .expect("repeat unavailable approval")
    );
}

#[test]
fn noninteractive_child_approval_gate_uses_only_stamped_machine_field() {
    assert!(super::child_requires_noninteractive_approval_failure(
        &json!({
            "child_execution": {"interactive_approval_available": false}
        })
    ));
    assert!(!super::child_requires_noninteractive_approval_failure(
        &json!({
            "schedule_triggered": true
        })
    ));
    assert!(!super::child_requires_noninteractive_approval_failure(
        &json!({
            "child_execution": {"interactive_approval_available": true}
        })
    ));
}
