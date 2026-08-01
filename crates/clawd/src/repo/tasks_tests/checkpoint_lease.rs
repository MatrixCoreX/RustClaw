use super::*;

#[test]
fn foreground_heartbeat_cannot_reclaim_a_published_checkpoint() {
    let state = state_with_tasks_table();
    let checkpoint = json!({
        "task_lifecycle": {
            "state": "waiting",
            "resume_reason": "task_budget_slice_exhausted",
            "next_check_after": 1,
            "checkpoint_id": "ckpt-paused"
        },
        "task_checkpoint": checkpoint_json("ckpt-paused", vec![])
    });
    insert_task(&state, "paused-heartbeat", "running", Some(&checkpoint), 1);

    assert!(
        !touch_running_task(&state, "paused-heartbeat", 0).expect("touch paused checkpoint"),
        "a foreground heartbeat must not reacquire a handed-off checkpoint"
    );
}

#[test]
fn claim_due_paused_checkpoint_task_marks_expired_checkpoint_lease_takeover() {
    let state = state_with_tasks_table();
    let now = 7_000;
    let due = json!({
        "task_lifecycle": {
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "next_check_after": now - 5,
            "checkpoint_id": "ckpt-expired-claim",
            "resume_claim": {
                "schema_version": 1,
                "owner": "worker:previous",
                "owner_layer": "worker_recovery",
                "checkpoint_id": "ckpt-expired-claim",
                "claimed_at": now - 70,
                "expires_at": now - 1
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-expired-claim", vec!["external_call:job-1"])
    });
    insert_task(
        &state,
        "expired-checkpoint-claim",
        "running",
        Some(&due),
        100,
    );

    let claimed = claim_due_paused_checkpoint_task_internal(
        &state,
        "expired-checkpoint-claim",
        "ckpt-expired-claim",
        now,
        45,
    )
    .expect("claim expired checkpoint lease")
    .expect("expired checkpoint lease should be reclaimable");

    assert_eq!(claimed.task_id, "expired-checkpoint-claim");
    assert_eq!(claimed.completed_side_effect_count, 1);
    assert!(claimed.requires_idempotency_guard);

    let stored = stored_result_json(&state, "expired-checkpoint-claim");
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&stored), None);
    assert_eq!(
        lifecycle["resume_claim"]["checkpoint_id"],
        "ckpt-expired-claim"
    );
    assert_eq!(lifecycle["resume_claim"]["owner"], state.worker.worker_id);
    assert_eq!(
        lifecycle["resume_claim"]["recovery_reason"],
        "checkpoint_lease_expired"
    );
    assert_eq!(
        lifecycle["resume_claim"]["previous_claim_owner"],
        "worker:previous"
    );
    assert_eq!(
        lifecycle["resume_claim"]["previous_claim_expires_at"],
        now - 1
    );
    assert_eq!(lifecycle["resume_claim"]["expires_at"], now + 45);
}
