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

#[test]
fn stale_lease_needs_bound_evidence_and_never_repeats_an_unknown_mutation() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let claim = || {
        claim_task_admin_mutation(
            &state,
            "actor",
            "lost-response",
            "steer",
            "target",
            &json!({}),
        )
        .unwrap()
    };
    assert!(matches!(claim(), TaskAdminMutationClaim::Acquired(_)));
    let db = state.core.db.get().unwrap();
    db.execute("UPDATE task_admin_mutation_receipts SET updated_at=1", [])
        .unwrap();
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_control_mailbox(task_id TEXT, action TEXT, issued_by TEXT,
        control_id TEXT, control_seq INTEGER, status TEXT, payload_digest TEXT)",
    )
    .unwrap();
    drop(db);
    assert!(matches!(claim(), TaskAdminMutationClaim::OutcomeUnknown));
    let digest = digest_token("payload");
    let id = format!(
        "ctl:{:x}",
        Sha256::digest(format!("target\nsteer\nlost-response\n{digest}").as_bytes())
    );
    let db = state.core.db.get().unwrap();
    db.execute("INSERT INTO task_control_mailbox(task_id,action,issued_by,control_id,control_seq,status,payload_digest)
        VALUES ('target','steer','task_control_api',?1,1,'applied',?2)", params![id,digest]).unwrap();
    drop(db);
    for _ in 0..2 {
        assert!(
            matches!(claim(), TaskAdminMutationClaim::Replay(value) if value["control_seq"] == 1)
        );
    }
}
