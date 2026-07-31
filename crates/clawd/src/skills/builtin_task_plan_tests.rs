use super::*;

fn active_task() -> (crate::AppState, crate::ClaimedTask) {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task_id = "task-plan-builtin";
    state.seed_ask_task_row(task_id, 42, 7, r#"{"text":"implement"}"#);
    let db = state.core.db.get().expect("task plan db");
    db.execute(
        "UPDATE tasks
         SET status = 'running',
             lease_owner = ?2,
             lease_expires_at = 9223372036854775807,
             claim_attempt = 1,
             claimed_at = 1
         WHERE task_id = ?1",
        rusqlite::params![task_id, state.worker.worker_id],
    )
    .expect("claim task");
    drop(db);
    (
        state,
        crate::ClaimedTask {
            claim_attempt: 1,
            task_id: task_id.to_string(),
            user_id: 42,
            chat_id: 7,
            user_key: Some("anon:42:7".to_string()),
            channel: "ui".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: r#"{"text":"implement"}"#.to_string(),
        },
    )
}

#[test]
fn task_plan_builtin_persists_and_publishes_data_only_event() {
    let (state, task) = active_task();
    let output = execute_task_plan(
        &state,
        &task,
        json!({
            "action": "set_plan",
            "plan_revision": 0,
            "steps": [
                {"step_id":"inspect","title":"Inspect","status":"completed"},
                {"step_id":"implement","title":"Implement","status":"in_progress"}
            ]
        })
        .as_object()
        .expect("args object"),
    )
    .expect("set task plan");
    let result: Value = serde_json::from_str(&output).expect("task plan result");
    assert_eq!(result["plan_revision"], 1);
    assert_eq!(result["checkpoint"]["kind"], "task_plan");

    let replay = crate::task_event_transport::replay_events_after(&state, &task.task_id, 0)
        .expect("task event replay");
    let event = replay
        .events
        .iter()
        .find(|event| event["event_kind"] == "task_plan_updated")
        .expect("task_plan_updated event");
    assert_eq!(event["payload"]["data_only"], true);
    assert_eq!(
        event["payload"]["render_owner"],
        "ui_cli_channel_projection"
    );
    assert_eq!(event["payload"]["plan_revision"], 1);

    let restored = crate::repo::read_task_plan(&state, &task.task_id, "read_plan")
        .expect("restore plan after event");
    assert_eq!(restored["steps"], result["steps"]);
}

#[test]
fn task_plan_builtin_returns_structured_revision_conflict() {
    let (state, task) = active_task();
    execute_task_plan(
        &state,
        &task,
        json!({
            "action": "set_plan",
            "steps": [{"step_id":"one","title":"One","status":"in_progress"}]
        })
        .as_object()
        .expect("args object"),
    )
    .expect("set task plan");

    let error = execute_task_plan(
        &state,
        &task,
        json!({
            "action": "update_steps",
            "plan_revision": 0,
            "updates": [{"step_id":"one","status":"completed"}]
        })
        .as_object()
        .expect("args object"),
    )
    .expect_err("stale revision must fail");
    let structured = crate::skills::parse_structured_skill_error(&error).expect("structured error");
    assert_eq!(
        structured.extra.as_ref().expect("error extra")["error_code"],
        "task_plan_revision_conflict"
    );
    assert_eq!(
        structured.extra.as_ref().expect("error extra")["current_plan_revision"],
        1
    );
    assert_eq!(
        structured.extra.as_ref().expect("error extra")["retryable"],
        true
    );
}
