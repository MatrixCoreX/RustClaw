use super::*;
use crate::task_result_retention::test_support::Fixture;

#[test]
fn forged_payload_flags_do_not_change_ordinary_ask_or_skill_retention() {
    let fixture = Fixture::new();
    for kind in ["ask", "run_skill"] {
        let id = uuid::Uuid::new_v4().to_string();
        let payload = fixture.payload();
        fixture.insert(&id, kind, &format!("ordinary-{id}"), &payload);
        let before = fixture.row(&id);
        assert!(retains_shared_outcome(&fixture.state, &id, &payload));
        assert_eq!(before, fixture.row(&id));
    }
}

#[test]
fn host_enrollment_is_effective_before_current_state_contains_task_id() {
    let fixture = Fixture::new();
    fixture.enroll();
    let payload = fixture.payload();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &payload,
    );
    assert!(!fixture.root.join("data/nni/work/current.json").exists());
    assert!(!retains_shared_outcome(
        &fixture.state,
        &fixture.task_id,
        &payload
    ));
    assert!(retains_shared_outcome(
        &fixture.state,
        &fixture.task_id,
        &serde_json::json!({"_task_source":"ordinary"})
    ));
}

#[test]
fn private_enrollment_does_not_authorize_wrong_idempotency_or_kind() {
    let fixture = Fixture::new();
    fixture.enroll();
    let payload = fixture.payload();
    fixture.insert(&fixture.task_id, "run_skill", "ordinary", &payload);
    assert!(retains_shared_outcome(
        &fixture.state,
        &fixture.task_id,
        &payload
    ));
    assert!(crate::repo::distributed_task_cleanup::retire_result(
        &fixture.state,
        &fixture.task_id,
        &fixture.enrollment
    )
    .is_err());
}

#[test]
fn database_unavailability_does_not_copy_an_enrolled_result_to_shared_memory_or_logs() {
    let fixture = Fixture::new();
    fixture.enroll();
    let payload = fixture.payload();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &payload,
    );
    fixture
        .state
        .core
        .db
        .get()
        .unwrap()
        .execute("DROP TABLE tasks", [])
        .unwrap();
    assert!(!retains_shared_outcome(
        &fixture.state,
        &fixture.task_id,
        &payload
    ));
    assert!(retains_shared_outcome(
        &fixture.state,
        &uuid::Uuid::new_v4().to_string(),
        &payload
    ));
}

#[tokio::test]
async fn database_failure_blocks_enrolled_late_artifact_writers() {
    let fixture = Fixture::new();
    fixture.enroll();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &fixture.payload(),
    );
    fixture
        .state
        .core
        .db
        .get()
        .unwrap()
        .execute("DROP TABLE tasks", [])
        .unwrap();
    assert!(write_gate::begin_writing(&fixture.state, &fixture.task_id)
        .await
        .is_err());
    assert!(write_gate::try_begin_writing(&fixture.state, &fixture.task_id).is_err());
}

#[test]
fn malformed_database_payload_cannot_restore_shared_plaintext_for_private_enrollment() {
    let fixture = Fixture::new();
    fixture.enroll();
    let payload = fixture.payload();
    fixture.insert(
        &fixture.task_id,
        "run_skill",
        &fixture.enrollment.idempotency_key(),
        &payload,
    );
    fixture
        .state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET payload_json='{' WHERE task_id=?1",
            [&fixture.task_id],
        )
        .unwrap();
    assert!(!retains_shared_outcome(
        &fixture.state,
        &fixture.task_id,
        &payload
    ));
}
