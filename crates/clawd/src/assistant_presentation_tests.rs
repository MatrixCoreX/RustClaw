use super::{
    abort_active_answer, publish_provisional_answer, publish_terminal_answer,
    terminal_answer_events,
};
use crate::ClaimedTask;
use sha2::{Digest, Sha256};

fn task(payload_json: &str) -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 2,
        task_id: "task-presentation".to_string(),
        user_id: 1,
        chat_id: 9,
        user_key: None,
        channel: "web".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: payload_json.to_string(),
    }
}

#[test]
fn terminal_events_preserve_utf8_offsets_sequence_and_digest() {
    let events = terminal_answer_events(
        &task(r#"{"conversation_id":"conversation-1","turn_id":"turn-1"}"#),
        "A你好B",
        4,
    );

    assert_eq!(events[0].0, "assistant_output_started");
    assert_eq!(events[0].1["sequence"], 0);
    assert_eq!(events[1].1["content"], "A你");
    assert_eq!(events[1].1["content_offset_bytes"], 0);
    assert_eq!(events[2].1["content"], "好B");
    assert_eq!(events[2].1["content_offset_bytes"], 4);
    let completed = &events[3].1;
    assert_eq!(completed["sequence"], 3);
    assert_eq!(completed["total_content_bytes"], 8);
    assert_eq!(completed["conversation_id"], "conversation-1");
    assert_eq!(completed["turn_id"], "turn-1");
    assert_eq!(
        completed["content_sha256"],
        format!("sha256:{:x}", Sha256::digest("A你好B".as_bytes()))
    );
}

#[test]
fn terminal_events_use_sanitized_public_content_only() {
    let events = terminal_answer_events(&task("{}"), "token=sk-secret-value-1234567890", 4096);
    let delta = events
        .iter()
        .find(|(kind, _)| *kind == "assistant_output_delta")
        .expect("delta");
    let content = delta.1["content"].as_str().expect("content");

    assert!(!content.contains("sk-secret-value"));
    assert!(content.contains("[REDACTED]"));
    assert_eq!(delta.1["publication_mode"], "terminal_only");
    assert_eq!(delta.1["fallback_reason"], "terminal_safe_point");
}

#[test]
fn empty_answer_still_has_started_and_completed_events() {
    let events = terminal_answer_events(&task("{}"), "", 16);

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].0, "assistant_output_started");
    assert_eq!(events[1].0, "assistant_output_completed");
    assert_eq!(events[1].1["sequence"], 1);
    assert_eq!(events[1].1["total_content_bytes"], 0);
}

#[test]
fn event_schema_is_identical_across_answer_languages() {
    let shapes = ["plain answer", "中文回答", "respuesta en espanol"]
        .into_iter()
        .map(|text| {
            terminal_answer_events(&task("{}"), text, 4096)
                .into_iter()
                .map(|(kind, payload)| {
                    let mut keys = payload
                        .as_object()
                        .expect("payload")
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>();
                    keys.sort();
                    (kind, keys)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    assert_eq!(shapes[0], shapes[1]);
    assert_eq!(shapes[1], shapes[2]);
}

fn claimed_task(state: &crate::AppState, task_id: &str) -> ClaimedTask {
    {
        let db = state.core.db.get().expect("task db");
        db.execute_batch(crate::INIT_SQL)
            .expect("initialize task schema");
        crate::db_init::ensure_task_lease_schema(&db).expect("initialize task lease schema");
        db.execute("ALTER TABLE tasks ADD COLUMN user_key TEXT", [])
            .expect("initialize task identity column");
    }
    state.seed_ask_task_row(
        task_id,
        7,
        11,
        r#"{"conversation_id":"conversation-live","turn_id":"turn-live"}"#,
    );
    crate::repo::claim_next_task(state)
        .expect("claim query")
        .expect("claimed task")
}

fn presentation_events(state: &crate::AppState, task_id: &str) -> Vec<serde_json::Value> {
    crate::task_event_transport::replay_events_after(state, task_id, 0)
        .expect("replay presentation events")
        .events
        .into_iter()
        .filter(|event| {
            event
                .get("event_kind")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind.starts_with("assistant_output_"))
        })
        .collect()
}

#[test]
fn matching_provisional_answer_completes_without_terminal_duplicate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = claimed_task(&state, "task-presentation-live-match");
    publish_provisional_answer(
        &state,
        &task,
        "assistant:provisional-match",
        "llm:1:provider:1",
        "A你好B",
        1_000,
        1_020,
    );
    publish_terminal_answer(&state, &task, "A你好B");

    let events = presentation_events(&state, &task.task_id);
    let kinds = events
        .iter()
        .filter_map(|event| event.get("event_kind").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            "assistant_output_started",
            "assistant_output_delta",
            "assistant_output_completed"
        ]
    );
    assert_eq!(
        events[0]["payload"]["publication_mode"],
        "provisional_low_latency"
    );
    assert_eq!(events[0]["payload"]["provider_first_byte_elapsed_ms"], 20);
    assert_eq!(events[2]["payload"]["total_content_bytes"], 8);
}

#[test]
fn divergent_final_answer_aborts_replaces_and_uses_terminal_fallback() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = claimed_task(&state, "task-presentation-live-mismatch");
    publish_provisional_answer(
        &state,
        &task,
        "assistant:provisional-mismatch",
        "llm:1:provider:1",
        "draft",
        1_000,
        1_010,
    );
    publish_terminal_answer(&state, &task, "verified");

    let events = presentation_events(&state, &task.task_id);
    let kinds = events
        .iter()
        .filter_map(|event| event.get("event_kind").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            "assistant_output_started",
            "assistant_output_delta",
            "assistant_output_aborted",
            "assistant_output_replaced",
            "assistant_output_started",
            "assistant_output_delta",
            "assistant_output_completed",
        ]
    );
    assert_eq!(
        events[2]["payload"]["error_code"],
        "assistant_output_final_mismatch"
    );
    assert_eq!(events[2]["payload"]["stream_abort_count"], 1);
    assert_eq!(
        events[3]["payload"]["old_stream_id"],
        "assistant:provisional-mismatch"
    );
    assert_eq!(events[3]["payload"]["stream_replacement_count"], 1);
    assert_eq!(events[4]["payload"]["publication_mode"], "terminal_only");
    assert_eq!(events[5]["payload"]["content"], "verified");
}

#[test]
fn retry_abort_links_the_next_provisional_attempt_as_replacement() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = claimed_task(&state, "task-presentation-live-retry");
    publish_provisional_answer(
        &state,
        &task,
        "assistant:attempt-one",
        "llm:1:provider:1",
        "first",
        1_000,
        1_010,
    );
    abort_active_answer(
        &state,
        &task,
        "answer_verifier_retry",
        "assistant.output.verifier_retry",
        true,
    );
    publish_provisional_answer(
        &state,
        &task,
        "assistant:attempt-two",
        "llm:2:provider:1",
        "second",
        2_000,
        2_010,
    );

    let events = presentation_events(&state, &task.task_id);
    let replacement = events
        .iter()
        .find(|event| event["event_kind"] == "assistant_output_replaced")
        .expect("replacement event");
    assert_eq!(
        replacement["payload"]["old_stream_id"],
        "assistant:attempt-one"
    );
    assert_eq!(
        replacement["payload"]["new_stream_id"],
        "assistant:attempt-two"
    );
    assert_eq!(replacement["payload"]["stream_replacement_count"], 1);
    assert_eq!(
        events.last().expect("last event")["payload"]["content"],
        "second"
    );
}
