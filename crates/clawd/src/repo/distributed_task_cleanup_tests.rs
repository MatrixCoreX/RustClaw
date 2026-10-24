use super::*;
use crate::task_result_retention::test_support::Fixture;

#[test]
fn retirement_is_idempotent_removes_result_projections_and_preserves_other_tasks_exactly() {
    let fixture = Fixture::new();
    fixture.enroll();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
    let other = uuid::Uuid::new_v4().to_string();
    fixture.insert(
        &other,
        "ask",
        "ordinary",
        &json!({"text":"ordinary-request"}),
    );
    let before = fixture.row(&other);
    crate::task_event_transport::publish_event(
        &fixture.state,
        &fixture.task_id,
        "tool_finished",
        json!({"output":"sensitive result","large":"x".repeat(70_000)}),
    )
    .unwrap();
    crate::task_event_transport::publish_event(
        &fixture.state,
        &other,
        "tool_finished",
        json!({"output":"ordinary result"}),
    )
    .unwrap();
    let other_events = crate::task_event_transport::replay_events_after(&fixture.state, &other, 0)
        .unwrap()
        .events;
    retire_result(&fixture.state, &fixture.task_id, &fixture.enrollment).unwrap();
    let after = fixture.row(&fixture.task_id);
    assert!(!after.0.contains("private-fixture-share"));
    assert!(!after.1.contains("private-article"));
    assert_eq!(after.2, None);
    let tombstone: Value = serde_json::from_str(&after.1).unwrap();
    assert_eq!(tombstone["result_retention"], "removed_after_delivery_ack");
    assert_eq!(tombstone["retired_event_count"], 1);
    assert!(tombstone["final_event_hash"].as_str().is_some());
    retire_result(&fixture.state, &fixture.task_id, &fixture.enrollment).unwrap();
    assert_eq!(after, fixture.row(&fixture.task_id));
    assert_eq!(before, fixture.row(&other));
    assert_eq!(
        other_events,
        crate::task_event_transport::replay_events_after(&fixture.state, &other, 0)
            .unwrap()
            .events
    );
    // Simulate a late worker with a result read before the retirement commit.
    assert!(crate::task_event_transport::publish_event(
        &fixture.state,
        &fixture.task_id,
        "task_final",
        json!({"output":"late result","large":"z".repeat(70_000)})
    )
    .unwrap()
    .is_none());
    for table in [
        "task_event_stream",
        "task_event_archive",
        "task_event_snapshots",
        "task_event_artifacts",
    ] {
        let count: usize = fixture
            .state
            .core
            .db
            .get()
            .unwrap()
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE task_id=?1"),
                [&fixture.task_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn no_host_enrollment_or_wrong_origin_binding_never_changes_results() {
    let fixture = Fixture::new();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
    let before = fixture.row(&fixture.task_id);
    assert!(retire_result(&fixture.state, &fixture.task_id, &fixture.enrollment).is_err());
    fixture.enroll();
    let mut wrong = fixture.enrollment.clone();
    wrong.origin = "https://other.example.org".to_string();
    assert!(retire_result(&fixture.state, &fixture.task_id, &wrong).is_err());
    assert_eq!(before, fixture.row(&fixture.task_id));
}
