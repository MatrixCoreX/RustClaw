use serde_json::json;

use super::super::{
    checkpoint_json, insert_task, set_task_lease, state_with_tasks_table, stored_result_json,
};
use crate::repo::{
    claim_recorded_paused_checkpoint_resume_dispatch_result_internal,
    record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal,
    record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal,
    renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal,
    update_task_progress_result, ClaimedDispatchedPausedCheckpointResumeExecution,
};

fn claimed_dispatch_fixture(
    task_id: &str,
    checkpoint_id: &str,
    now: i64,
) -> (
    crate::AppState,
    ClaimedDispatchedPausedCheckpointResumeExecution,
) {
    let state = state_with_tasks_table();
    let expires_at = now + 60;
    let executor_state = "executing_planner_resume";
    let executor_action = "run_seeded_agent_loop";
    let executor_status = "seeded_loop_requires_provider_window";
    let dispatch_state = "ready_to_run_seeded_agent_loop";
    let lifecycle = json!({
        "schema_version": 1,
        "state": "running",
        "checkpoint_id": checkpoint_id,
        "resume_claim": {
            "schema_version": 1,
            "owner": state.worker.worker_id,
            "checkpoint_id": checkpoint_id,
            "claimed_at": now,
            "expires_at": expires_at
        },
        "resume_work_item": {
            "schema_version": 1,
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state
        },
        "resume_executor": {
            "schema_version": 1,
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state,
            "resume_trigger": "worker_recovery",
            "resume_directive": "run_next_planner_round",
            "executor_claim_expires_at": expires_at,
            "handoff_claim_expires_at": expires_at,
            "dispatch_claim_expires_at": expires_at
        },
        "resume_executor_claim": {
            "schema_version": 1,
            "owner": "worker_recovery_executor",
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state,
            "expires_at": expires_at
        },
        "resume_execution_plan": {
            "schema_version": 1,
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state,
            "executor_action": executor_action
        },
        "resume_executor_handoff": {
            "schema_version": 1,
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state,
            "executor_action": executor_action,
            "executor_status": executor_status,
            "claim_expires_at": expires_at,
            "dispatch_claim_expires_at": expires_at
        },
        "resume_executor_handoff_claim": {
            "schema_version": 1,
            "owner": "worker_recovery_handoff_executor",
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state,
            "executor_action": executor_action,
            "executor_status": executor_status,
            "expires_at": expires_at
        },
        "resume_executor_handoff_dispatch": {
            "schema_version": 1,
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state,
            "executor_action": executor_action,
            "executor_status": executor_status,
            "dispatch_state": dispatch_state,
            "claim_expires_at": expires_at
        },
        "resume_executor_dispatch_claim": {
            "schema_version": 1,
            "owner": "worker_recovery_dispatch_executor",
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state,
            "executor_action": executor_action,
            "executor_status": executor_status,
            "dispatch_state": dispatch_state,
            "dispatch_execution_state": "claimed_to_run_seeded_agent_loop",
            "expires_at": expires_at
        }
    });
    let checkpoint = checkpoint_json(checkpoint_id, vec![]);
    insert_task(
        &state,
        task_id,
        "running",
        Some(&json!({
            "task_lifecycle": lifecycle,
            "task_checkpoint": checkpoint
        })),
        now,
    );
    set_task_lease(&state, task_id, &state.worker.worker_id, expires_at, 1, now);
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
        payload_json: json!({"text": "long task"}).to_string(),
    };
    let claimed = ClaimedDispatchedPausedCheckpointResumeExecution {
        task,
        task_id: task_id.to_string(),
        checkpoint_id: checkpoint_id.to_string(),
        executor_state: executor_state.to_string(),
        executor_action: executor_action.to_string(),
        executor_status: executor_status.to_string(),
        dispatch_state: dispatch_state.to_string(),
        dispatch_execution_state: "claimed_to_run_seeded_agent_loop".to_string(),
        resume_trigger: "worker_recovery".to_string(),
        resume_directive: "run_next_planner_round".to_string(),
        lease_expires_at: expires_at,
        handoff_claim_expires_at: expires_at,
        dispatch_claim_expires_at: expires_at,
        execution_plan: json!({
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state,
            "executor_action": executor_action
        }),
        dispatch_payload: json!({
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state,
            "executor_action": executor_action,
            "executor_status": executor_status,
            "dispatch_state": dispatch_state
        }),
        dispatch_claim: json!({
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state,
            "executor_action": executor_action,
            "executor_status": executor_status,
            "dispatch_state": dispatch_state
        }),
        task_checkpoint: crate::task_lifecycle::task_checkpoint_from_result_json(
            &stored_result_json(&state, task_id),
        )
        .expect("stored checkpoint"),
    };
    (state, claimed)
}

#[test]
fn active_resume_dispatch_lease_renews_the_complete_claim_chain() {
    let now = 10_000;
    let (state, claimed) = claimed_dispatch_fixture("renew-dispatch", "ckpt-renew-dispatch", now);

    assert!(
        renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal(
            &state,
            &claimed,
            now + 20,
            90,
        )
        .expect("renew active dispatch")
    );
    let stored = stored_result_json(&state, "renew-dispatch");
    let lifecycle = &stored["task_lifecycle"];
    for pointer in [
        "/resume_claim/expires_at",
        "/resume_executor_claim/expires_at",
        "/resume_executor_handoff_claim/expires_at",
        "/resume_executor_dispatch_claim/expires_at",
        "/resume_executor/executor_claim_expires_at",
        "/resume_executor/handoff_claim_expires_at",
        "/resume_executor/dispatch_claim_expires_at",
        "/resume_executor_handoff/claim_expires_at",
        "/resume_executor_handoff_dispatch/claim_expires_at",
    ] {
        assert_eq!(
            lifecycle
                .pointer(pointer)
                .and_then(serde_json::Value::as_i64),
            Some(now + 110)
        );
    }
    assert_eq!(
        lifecycle["resume_executor_dispatch_claim"]["renewal_count"],
        1
    );
    let db = state.core.db.get().expect("get db");
    let task_lease_expires_at: i64 = db
        .query_row(
            "SELECT lease_expires_at FROM tasks WHERE task_id = 'renew-dispatch'",
            [],
            |row| row.get(0),
        )
        .expect("select renewed task lease");
    assert_eq!(task_lease_expires_at, now + 110);
    drop(db);

    assert!(
        !renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal(
            &state,
            &claimed,
            now + 111,
            90,
        )
        .expect("expired claim cannot be resurrected")
    );
}

#[test]
fn stale_resume_generation_cannot_renew_same_worker_lease() {
    let now = 15_000;
    let (state, claimed) =
        claimed_dispatch_fixture("stale-resume-generation", "ckpt-stale-generation", now);
    let before = stored_result_json(&state, "stale-resume-generation");
    set_task_lease(
        &state,
        "stale-resume-generation",
        &state.worker.worker_id,
        now + 120,
        claimed.task.claim_attempt + 1,
        now + 1,
    );

    assert!(
        !renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal(
            &state,
            &claimed,
            now + 20,
            90,
        )
        .expect("stale generation renewal is a rejected claim")
    );
    assert_eq!(
        stored_result_json(&state, "stale-resume-generation"),
        before
    );
}

#[test]
fn resumed_agent_progress_cannot_erase_dispatch_coordination() {
    let now = 20_000;
    let (state, claimed) =
        claimed_dispatch_fixture("progress-dispatch", "ckpt-progress-dispatch", now);
    let progress = json!({
        "progress_messages": ["step complete"],
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "checkpoint_id": "ckpt-newer",
            "next_check_after": now + 120
        },
        "task_checkpoint": checkpoint_json("ckpt-newer", vec![])
    });

    update_task_progress_result(
        &state,
        "progress-dispatch",
        claimed.task.claim_attempt,
        &progress.to_string(),
    )
    .expect("publish resumed progress");
    let stored = stored_result_json(&state, "progress-dispatch");
    assert_eq!(
        stored["task_lifecycle"]["checkpoint_id"],
        "ckpt-progress-dispatch"
    );
    assert_eq!(
        stored["task_lifecycle"]["resume_executor_dispatch_claim"]["owner"],
        "worker_recovery_dispatch_executor"
    );
    assert_eq!(
        stored["resume_execution_progress"]["payload"]["task_lifecycle"]["checkpoint_id"],
        "ckpt-newer"
    );

    let result = json!({
        "schema_version": 1,
        "checkpoint_id": claimed.checkpoint_id,
        "executor_state": claimed.executor_state,
        "executor_action": claimed.executor_action,
        "executor_status": claimed.executor_status,
        "dispatch_state": claimed.dispatch_state,
        "executor_result_status": "seeded_loop_completed",
        "final_result_json": {
            "text": "done"
        }
    });
    assert!(
        record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal(
            &state,
            claimed.task.claim_attempt,
            &claimed.task_id,
            &claimed.checkpoint_id,
            &claimed.executor_state,
            &claimed.executor_action,
            &claimed.executor_status,
            &claimed.dispatch_state,
            &result,
            now + 30,
        )
        .expect("record result after progress")
    );
}

#[test]
fn deferred_seeded_loop_projects_the_new_checkpoint_and_releases_its_lease() {
    let now = 30_000;
    let (state, claimed) = claimed_dispatch_fixture("deferred-dispatch", "ckpt-deferred-old", now);
    let deferred_result = json!({
        "text": "waiting",
        "task_journal": {
            "summary": {
                "task_lifecycle": {
                    "schema_version": 1,
                    "state": "needs_user",
                    "checkpoint_id": "ckpt-deferred-new",
                    "resume_reason": "confirmation_required"
                },
                "task_checkpoint": checkpoint_json("ckpt-deferred-new", vec![])
            },
            "trace": {}
        }
    });
    let execution_result = json!({
        "schema_version": 1,
        "checkpoint_id": claimed.checkpoint_id,
        "executor_state": claimed.executor_state,
        "executor_action": claimed.executor_action,
        "executor_status": claimed.executor_status,
        "dispatch_state": claimed.dispatch_state,
        "executor_result_status": "seeded_loop_deferred",
        "deferred_checkpoint_id": "ckpt-deferred-new",
        "deferred_lifecycle_state": "needs_user",
        "final_result_json": deferred_result
    });
    assert!(
        record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal(
            &state,
            claimed.task.claim_attempt,
            &claimed.task_id,
            &claimed.checkpoint_id,
            &claimed.executor_state,
            &claimed.executor_action,
            &claimed.executor_status,
            &claimed.dispatch_state,
            &execution_result,
            now + 20,
        )
        .expect("record deferred result")
    );
    let projection_claim = claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
        &state,
        &claimed.task_id,
        &claimed.checkpoint_id,
        &claimed.executor_state,
        &claimed.executor_action,
        &claimed.executor_status,
        &claimed.dispatch_state,
        "seeded_loop_deferred",
        now + 21,
        30,
    )
    .expect("claim deferred projection")
    .expect("deferred projection claimed");
    let mut projection = projection_claim.execution_result_payload.clone();
    projection["task_id"] = json!(claimed.task_id);
    projection["result_projection_state"] = json!(projection_claim.result_projection_state);
    assert!(
        record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
            &state,
            claimed.task.claim_attempt,
            &claimed.task_id,
            &claimed.checkpoint_id,
            &claimed.executor_state,
            &claimed.executor_action,
            &claimed.executor_status,
            &claimed.dispatch_state,
            "seeded_loop_deferred",
            &projection,
            now + 22,
        )
        .expect("project deferred checkpoint")
    );

    let stored = stored_result_json(&state, "deferred-dispatch");
    assert_eq!(stored["task_lifecycle"]["state"], "needs_user");
    assert_eq!(
        stored["task_lifecycle"]["checkpoint_id"],
        "ckpt-deferred-new"
    );
    assert_eq!(
        stored["task_checkpoint"]["checkpoint_id"],
        "ckpt-deferred-new"
    );
    assert_eq!(
        stored["task_lifecycle"]["previous_checkpoint_id"],
        "ckpt-deferred-old"
    );
    assert!(stored["task_lifecycle"]
        .get("resume_executor_dispatch_claim")
        .is_none());
    let db = state.core.db.get().expect("get db");
    let (lease_owner, lease_expires_at): (Option<String>, i64) = db
        .query_row(
            "SELECT lease_owner, lease_expires_at
             FROM tasks
             WHERE task_id = 'deferred-dispatch'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("select released lease");
    assert!(lease_owner.is_none());
    assert_eq!(lease_expires_at, 0);
}
