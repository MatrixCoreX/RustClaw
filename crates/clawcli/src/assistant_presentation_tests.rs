use super::{decode, AssistantPresentationReducer, PresentationUpdate};
use serde_json::json;
use sha2::{Digest, Sha256};

fn event(kind: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut payload = json!({
        "schema_version": 1,
        "task_id": "task-1",
        "conversation_id": "conversation-1",
        "turn_id": "turn-1",
        "stream_id": "stream-1",
        "attempt_id": "attempt-1",
        "sequence": 0,
        "content_offset_bytes": 0,
        "created_at": 10,
    });
    payload
        .as_object_mut()
        .expect("payload")
        .extend(extra.as_object().expect("extra").clone());
    json!({
        "schema_version": 1,
        "task_id": "task-1",
        "event_kind": kind,
        "payload": payload,
    })
}

#[test]
fn decodes_valid_presentation_delta() {
    let decoded = decode(&event(
        "assistant_output_delta",
        json!({"sequence": 1, "content": "hello"}),
    ))
    .expect("decode")
    .expect("presentation event");

    assert_eq!(decoded.kind, "assistant_output_delta");
    assert_eq!(decoded.stream_id, "stream-1");
    assert_eq!(decoded.sequence, 1);
}

#[test]
fn rejects_identity_and_completion_mismatch() {
    let mut wrong_task = event("assistant_output_started", json!({}));
    wrong_task["task_id"] = json!("task-2");
    assert_eq!(
        decode(&wrong_task).expect_err("identity").to_string(),
        "assistant_presentation_identity_mismatch"
    );

    let completed = event(
        "assistant_output_completed",
        json!({
            "sequence": 2,
            "content_offset_bytes": 3,
            "total_content_bytes": 4,
            "content_sha256": format!("sha256:{}", "a".repeat(64)),
        }),
    );
    assert_eq!(
        decode(&completed).expect_err("size").to_string(),
        "assistant_presentation_completion_size_mismatch"
    );
}

#[test]
fn reducer_validates_sequence_offsets_digest_and_duplicates() {
    let mut reducer = AssistantPresentationReducer::default();
    let started = decode(&event("assistant_output_started", json!({})))
        .unwrap()
        .unwrap();
    assert_eq!(
        reducer.apply(started.clone()).unwrap(),
        PresentationUpdate::Started
    );
    assert_eq!(
        reducer.apply(started).unwrap(),
        PresentationUpdate::Duplicate
    );

    let delta = decode(&event(
        "assistant_output_delta",
        json!({"sequence": 1, "content": "你好"}),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(
        reducer.apply(delta).unwrap(),
        PresentationUpdate::Delta("你好".to_string())
    );
    let digest = format!("sha256:{:x}", Sha256::digest("你好".as_bytes()));
    let completed = decode(&event(
        "assistant_output_completed",
        json!({
            "sequence": 2,
            "content_offset_bytes": 6,
            "total_content_bytes": 6,
            "content_sha256": digest,
        }),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(
        reducer.apply(completed).unwrap(),
        PresentationUpdate::Completed
    );
    assert!(reducer.completed_matches(Some("你好")));
    assert_eq!(reducer.latest_display_content(), Some("你好"));
}

#[test]
fn reducer_rejects_gaps_bad_digests_and_conflicting_duplicates() {
    let mut gap = AssistantPresentationReducer::default();
    gap.apply(
        decode(&event("assistant_output_started", json!({})))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let gap_event = decode(&event(
        "assistant_output_delta",
        json!({"sequence": 2, "content": "x"}),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(
        gap.apply(gap_event).unwrap_err().to_string(),
        "assistant_presentation_sequence_gap"
    );

    let mut digest = AssistantPresentationReducer::default();
    digest
        .apply(
            decode(&event("assistant_output_started", json!({})))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    digest
        .apply(
            decode(&event(
                "assistant_output_delta",
                json!({"sequence": 1, "content": "x"}),
            ))
            .unwrap()
            .unwrap(),
        )
        .unwrap();
    let bad_completion = decode(&event(
        "assistant_output_completed",
        json!({
            "sequence": 2,
            "content_offset_bytes": 1,
            "total_content_bytes": 1,
            "content_sha256": format!("sha256:{}", "0".repeat(64)),
        }),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(
        digest.apply(bad_completion).unwrap_err().to_string(),
        "assistant_presentation_digest_mismatch"
    );

    let conflicting = decode(&event(
        "assistant_output_delta",
        json!({"sequence": 1, "content": "y"}),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(
        digest.apply(conflicting).unwrap_err().to_string(),
        "assistant_presentation_duplicate_conflict"
    );
}

#[test]
fn reducer_tracks_abort_and_replacement_without_merging_attempts() {
    let mut reducer = AssistantPresentationReducer::default();
    reducer
        .apply(
            decode(&event("assistant_output_started", json!({})))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    let aborted = decode(&event(
        "assistant_output_aborted",
        json!({
            "sequence": 1,
            "error_code": "answer_verifier_retry",
            "message_key": "assistant.output.verifier_retry",
            "retryable": true,
        }),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(reducer.apply(aborted).unwrap(), PresentationUpdate::Aborted);
    let replaced = decode(&event(
        "assistant_output_replaced",
        json!({
            "sequence": 2,
            "old_stream_id": "stream-1",
            "new_stream_id": "stream-2",
        }),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(
        reducer.apply(replaced).unwrap(),
        PresentationUpdate::Replaced
    );
    assert_eq!(reducer.latest_display_content(), None);
}
