use serde_json::json;

use super::*;

#[test]
fn completed_mutation_replays_and_payload_reuse_conflicts() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let payload = json!({"pause_seconds": 60});
    let lease = match claim_task_admin_mutation(
        &state,
        "rk-admin",
        "idem-pause-0001",
        "pause",
        "task-1",
        &payload,
    )
    .expect("claim")
    {
        TaskAdminMutationClaim::Acquired(lease) => lease,
        other => panic!("unexpected claim: {other:?}"),
    };
    let response = json!({"status": "task_pause_requested", "task_id": "task-1"});
    complete_task_admin_mutation(&state, &lease, &response).expect("complete");
    assert!(matches!(
        claim_task_admin_mutation(
            &state,
            "rk-admin",
            "idem-pause-0001",
            "pause",
            "task-1",
            &payload,
        )
        .expect("replay"),
        TaskAdminMutationClaim::Replay(value) if value == response
    ));
    assert!(matches!(
        claim_task_admin_mutation(
            &state,
            "rk-admin",
            "idem-pause-0001",
            "pause",
            "task-1",
            &json!({"pause_seconds": 120}),
        )
        .expect("conflict"),
        TaskAdminMutationClaim::Conflict
    ));
}

#[test]
fn in_progress_claim_survives_a_second_caller_and_release_allows_retry() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let payload = json!({"task_id": "task-2"});
    let lease = match claim_task_admin_mutation(
        &state,
        "rk-admin",
        "idem-cancel-0001",
        "cancel",
        "task-2",
        &payload,
    )
    .expect("claim")
    {
        TaskAdminMutationClaim::Acquired(lease) => lease,
        other => panic!("unexpected claim: {other:?}"),
    };
    assert!(matches!(
        claim_task_admin_mutation(
            &state,
            "rk-admin",
            "idem-cancel-0001",
            "cancel",
            "task-2",
            &payload,
        )
        .expect("second claim"),
        TaskAdminMutationClaim::InProgress
    ));
    release_task_admin_mutation(&state, &lease).expect("release");
    assert!(matches!(
        claim_task_admin_mutation(
            &state,
            "rk-admin",
            "idem-cancel-0001",
            "cancel",
            "task-2",
            &payload,
        )
        .expect("retry claim"),
        TaskAdminMutationClaim::Acquired(_)
    ));
}
