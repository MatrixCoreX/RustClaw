use serde_json::json;

use super::*;

fn state() -> crate::AppState {
    crate::AppState::test_default_with_fixture_provider()
}

#[test]
fn event_schema_is_ordered_deduplicated_and_replayable() {
    let state = state();
    let first = publish_event(&state, "task-a", "tool_started", json!({"step_id":"one"}))
        .unwrap()
        .unwrap();
    let duplicate = publish_event(&state, "task-a", "tool_started", json!({"step_id":"one"}))
        .unwrap()
        .unwrap();
    let second = publish_event(&state, "task-a", "tool_finished", json!({"step_id":"one"}))
        .unwrap()
        .unwrap();

    assert_eq!(first["schema_version"], 1);
    assert_eq!(first["seq"], 1);
    assert_eq!(duplicate["seq"], 1);
    assert_eq!(second["seq"], 2);
    assert_eq!(second["event_kind"], "tool_finished");
    let replay = replay_events_after(&state, "task-a", 1).unwrap();
    assert!(!replay.cursor_expired);
    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.events[0]["seq"], 2);
}

#[test]
fn secrets_and_raw_teaching_fields_are_redacted_before_persistence() {
    let state = state();
    let event = publish_event(
        &state,
        "task-secret",
        "provider_call",
        json!({
            "api_key": "top-secret",
            "nested": {"authorization": "Bearer abcdefghijklmnop"},
            "raw_llm_response": {"content": "private"},
            "opaque_ref": "rustclaw-secret://v1/12345678-1234-1234-1234-123456789abc",
            "safe": "visible",
        }),
    )
    .unwrap()
    .unwrap();
    let encoded = serde_json::to_string(&event).unwrap();
    assert!(!encoded.contains("top-secret"));
    assert!(!encoded.contains("abcdefghijklmnop"));
    assert!(!encoded.contains("private"));
    assert!(!encoded.contains("rustclaw-secret://"));
    assert_eq!(event["payload"]["safe"], "visible");
    assert_eq!(event["redaction"]["applied"], true);
}

#[test]
fn oversized_payload_is_replaced_with_persisted_artifact_reference() {
    let state = state();
    let event = publish_event(
        &state,
        "task-large",
        "tool_finished",
        json!({"output": "x".repeat(EVENT_MAX_BYTES + 1)}),
    )
    .unwrap()
    .unwrap();
    assert_eq!(event["payload"]["payload_omitted"], true);
    let artifact_id = event["artifact_refs"][0]["artifact_id"].as_str().unwrap();
    let db = state.core.db.get().unwrap();
    let count: u64 = db
        .query_row(
            "SELECT COUNT(*) FROM task_event_artifacts WHERE task_id = ?1 AND artifact_id = ?2",
            rusqlite::params!["task-large", artifact_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    drop(db);
    let payload = read_event_artifact(&state, "task-large", artifact_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        payload["output"].as_str().unwrap().len(),
        EVENT_MAX_BYTES + 1
    );
}

#[test]
fn notifier_wakes_subscriber_with_persisted_sequence() {
    let state = state();
    let mut receiver = state.metrics.task_event_notifier.subscribe("task-notify");
    publish_event(&state, "task-notify", "task_goal", json!({})).unwrap();
    assert_eq!(receiver.try_recv().unwrap(), 1);
}

#[test]
fn event_context_projects_thread_and_child_refs() {
    let state = state();
    let event = publish_event(
        &state,
        "task-parent",
        "subagent",
        json!({
            "thread_ref": "thread-a",
            "session_id": "session-a",
            "parent_task_id": "task-parent",
            "child_run_id": "task-child",
        }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(event["thread_id"], "thread-a");
    assert_eq!(event["session_id"], "session-a");
    assert_eq!(event["parent_task_id"], "task-parent");
    assert_eq!(event["child_task_id"], "task-child");
}

#[test]
fn event_context_falls_back_to_persisted_task_thread_binding() {
    let state = state();
    let db = state.core.db.get().unwrap();
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL
        );",
    )
    .unwrap();
    db.execute(
        "INSERT INTO tasks (task_id, payload_json) VALUES (?1, ?2)",
        rusqlite::params![
            "task-thread-context",
            json!({
                "text": "inspect",
                "thread_id": "cli_thread_a",
                "session_id": "cli_session_a",
                "parent_task_id": "task_parent_a"
            })
            .to_string()
        ],
    )
    .unwrap();
    drop(db);

    let event = publish_event(
        &state,
        "task-thread-context",
        "planner_finished",
        json!({"round_no": 1}),
    )
    .unwrap()
    .unwrap();
    assert_eq!(event["thread_id"], "cli_thread_a");
    assert_eq!(event["session_id"], "cli_session_a");
    assert_eq!(event["parent_task_id"], "task_parent_a");
}

#[test]
fn event_context_rejects_unbounded_or_non_machine_refs() {
    let state = state();
    let event = publish_event(
        &state,
        "task-unsafe-context",
        "task_goal",
        json!({
            "thread_id": "thread with spaces",
            "session_id": "session/with/slashes"
        }),
    )
    .unwrap()
    .unwrap();
    assert!(event["thread_id"].is_null());
    assert!(event["session_id"].is_null());
}

#[test]
fn invalid_event_kind_is_rejected() {
    let state = state();
    assert!(publish_event(&state, "task-a", "Tool Started", json!({})).is_err());
}

#[test]
fn bounded_replay_marks_an_expired_cursor() {
    let state = state();
    for index in 0..EVENT_REPLAY_LIMIT + 2 {
        publish_event(
            &state,
            "task-retained",
            "task_observation",
            json!({"index": index}),
        )
        .unwrap();
    }
    let replay = replay_events_after(&state, "task-retained", 1).unwrap();
    assert!(replay.cursor_expired);
    assert_eq!(replay.oldest_seq, Some(3));
    assert_eq!(replay.events.len(), EVENT_REPLAY_LIMIT as usize);
    assert_eq!(replay.events.first().unwrap()["seq"], 3);
}

#[tokio::test]
async fn lagged_broadcast_consumer_recovers_from_persisted_replay() {
    let state = state();
    let mut receiver = state.metrics.task_event_notifier.subscribe("task-lagged");
    for index in 0..NOTIFIER_CAPACITY + 2 {
        publish_event(
            &state,
            "task-lagged",
            "task_observation",
            json!({"index": index}),
        )
        .unwrap();
    }
    assert!(matches!(
        receiver.recv().await,
        Err(broadcast::error::RecvError::Lagged(_))
    ));
    let replay = replay_events_after(&state, "task-lagged", 0).unwrap();
    assert_eq!(replay.events.len(), NOTIFIER_CAPACITY + 2);
    assert_eq!(
        replay.events.last().unwrap()["seq"],
        (NOTIFIER_CAPACITY + 2) as u64
    );
}
