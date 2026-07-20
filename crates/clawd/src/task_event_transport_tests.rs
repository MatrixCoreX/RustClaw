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
fn claimed_event_rejects_stale_generation_from_same_worker() {
    let state = state();
    let task_id = "task-event-generation";
    let db = state.core.db.get().expect("get db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT,
            channel TEXT NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            lease_owner TEXT,
            lease_expires_at INTEGER NOT NULL DEFAULT 0,
            claim_attempt INTEGER NOT NULL DEFAULT 0,
            claimed_at INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("create task table");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, created_at, updated_at, lease_owner, lease_expires_at,
            claim_attempt, claimed_at
         ) VALUES (
            ?1, 42, 7, 'test-key', 'ui', 'ask', '{}', 'running',
            '1', '1', ?2, 9223372036854775807, 1, 1
         )",
        rusqlite::params![task_id, state.worker.worker_id],
    )
    .expect("insert claimed task");
    drop(db);
    let task = crate::ClaimedTask {
        claim_attempt: 1,
        task_id: task_id.to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };

    publish_claimed_event(&state, &task, "tool_started", json!({"step_id":"one"}))
        .expect("publish active claim event");
    let db = state.core.db.get().expect("get db");
    db.execute(
        "UPDATE tasks SET claim_attempt = 2 WHERE task_id = ?1",
        rusqlite::params![task_id],
    )
    .expect("advance task generation");
    drop(db);

    let error = publish_claimed_event(&state, &task, "tool_finished", json!({"step_id":"one"}))
        .expect_err("stale generation event must be fenced");
    let rejection = error
        .downcast_ref::<crate::repo::WorkerTaskWriteRejected>()
        .expect("typed worker event rejection");
    assert_eq!(
        rejection.status_code,
        crate::repo::WORKER_LEASE_LOST_STATUS_CODE
    );
    assert_eq!(rejection.expected_claim_attempt, 1);
    assert_eq!(rejection.active_claim_attempt, Some(2));
    assert_eq!(
        replay_events_after(&state, task_id, 0)
            .unwrap()
            .events
            .len(),
        1
    );
}

#[test]
fn coding_projection_appends_a_new_authoritative_event_after_green_verification() {
    let state = state();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-red-green-events", "ask", "fix tests");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_red".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(
            "exit=1 command=python3 -m unittest test_calc.py\n\
             stderr_ref=artifact:stderr:step_red"
                .to_string(),
        ),
        started_at: 1,
        finished_at: 2,
    });
    publish_journal_snapshot(&state, &journal).unwrap();

    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_fix".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "write_text",
                    "path": "calc.py"
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_green".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("exit=0 command=python3 -m unittest test_calc.py".to_string()),
        error: None,
        started_at: 5,
        finished_at: 6,
    });
    publish_journal_snapshot(&state, &journal).unwrap();

    let coding_events = replay_events_after(&state, "task-red-green-events", 0)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| event["event_type"] == "coding_evidence")
        .collect::<Vec<_>>();
    assert_eq!(coding_events.len(), 2);
    assert_eq!(coding_events[0]["payload"]["verification_status"], "failed");
    assert_eq!(
        coding_events[1]["payload"]["verification_status"],
        "verified"
    );
    assert_eq!(
        coding_events[1]["payload"]["latest_verification_step_ref"],
        "step_green"
    );
    assert!(
        coding_events[1]["payload"]["projection_revision"]
            .as_u64()
            .unwrap()
            > coding_events[0]["payload"]["projection_revision"]
                .as_u64()
                .unwrap()
    );
    assert!(coding_events[1]["seq"].as_u64().unwrap() > coding_events[0]["seq"].as_u64().unwrap());
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
