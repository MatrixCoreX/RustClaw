use serde_json::json;

use super::*;

#[test]
fn list_ready_paused_checkpoint_resume_executors_filters_machine_states() {
    let state = state_with_tasks_table();
    let now = 3_000;
    let ready_planner = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "next_check_after": now,
            "checkpoint_id": "ckpt-ready",
            "resume_work_item": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-ready",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            },
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-ready",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-ready", vec!["write_file:tmp/report.txt"])
    });
    let due_poll = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "async_job_poll",
            "next_check_after": now - 1,
            "checkpoint_id": "ckpt-poll",
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-poll",
                "executor_state": "poll_scheduled",
                "resume_trigger": "worker_recovery",
                "resume_directive": "poll_async_job",
                "job_id": "job-1"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-poll", vec![])
    });
    let future = json!({
        "task_lifecycle": {
            "state": "background",
            "next_check_after": now + 60,
            "checkpoint_id": "ckpt-future",
            "resume_executor": {
                "checkpoint_id": "ckpt-future",
                "executor_state": "ready_to_finalize",
                "resume_trigger": "worker_recovery",
                "resume_directive": "verify_and_finalize"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-future", vec![])
    });
    let awaiting_user = json!({
        "task_lifecycle": {
            "state": "needs_user",
            "checkpoint_id": "ckpt-user",
            "resume_executor": {
                "checkpoint_id": "ckpt-user",
                "executor_state": "awaiting_user",
                "resume_trigger": "worker_recovery",
                "resume_directive": "await_user_input"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-user", vec![])
    });
    let mismatched_executor = json!({
        "task_lifecycle": {
            "state": "background",
            "next_check_after": now,
            "checkpoint_id": "ckpt-mismatch",
            "resume_executor": {
                "checkpoint_id": "ckpt-other",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-mismatch", vec![])
    });
    let missing_directive = json!({
        "task_lifecycle": {
            "state": "background",
            "next_check_after": now,
            "checkpoint_id": "ckpt-missing-directive",
            "resume_executor": {
                "checkpoint_id": "ckpt-missing-directive",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-missing-directive", vec![])
    });
    let checkpoint_mismatch = json!({
        "task_lifecycle": {
            "state": "background",
            "next_check_after": now,
            "checkpoint_id": "ckpt-lifecycle",
            "resume_executor": {
                "checkpoint_id": "ckpt-lifecycle",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-task", vec![])
    });

    insert_task(&state, "ready-planner", "running", Some(&ready_planner), 10);
    insert_task(&state, "due-poll", "running", Some(&due_poll), 20);
    insert_task(&state, "future", "running", Some(&future), 30);
    insert_task(&state, "awaiting-user", "running", Some(&awaiting_user), 40);
    insert_task(
        &state,
        "mismatched-executor",
        "running",
        Some(&mismatched_executor),
        50,
    );
    insert_task(
        &state,
        "missing-directive",
        "running",
        Some(&missing_directive),
        60,
    );
    insert_task(
        &state,
        "checkpoint-mismatch",
        "running",
        Some(&checkpoint_mismatch),
        70,
    );

    let first =
        list_ready_paused_checkpoint_resume_executors_internal(&state, now, 1).expect("list first");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].task_id, "ready-planner");
    assert_eq!(first[0].checkpoint_id, "ckpt-ready");
    assert_eq!(first[0].executor_state, "ready_for_planner_resume");
    assert_eq!(first[0].resume_trigger, "worker_recovery");
    assert_eq!(first[0].resume_directive, "run_next_planner_round");
    assert_eq!(first[0].next_check_after, Some(now));
    assert_eq!(first[0].task_checkpoint.completed_side_effect_refs.len(), 1);
    assert_eq!(
        first[0]
            .resume_work_item
            .as_ref()
            .and_then(|value| value.get("resume_trigger"))
            .and_then(serde_json::Value::as_str),
        Some("worker_recovery")
    );

    let all = list_ready_paused_checkpoint_resume_executors_internal(&state, now, 10)
        .expect("list ready executors");
    let task_ids: Vec<_> = all.iter().map(|task| task.task_id.as_str()).collect();
    assert_eq!(task_ids, vec!["ready-planner", "due-poll"]);
    assert_eq!(all[1].executor_state, "poll_scheduled");
    assert_eq!(all[1].resume_directive, "poll_async_job");
    assert_eq!(all[1].resume_executor["job_id"], "job-1");
}

#[test]
fn claim_ready_paused_checkpoint_resume_executor_sets_machine_lease() {
    let state = state_with_tasks_table();
    let now = 4_000;
    let ready_planner = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "resume_due": true,
            "resume_wait_seconds": 0,
            "next_check_after": now,
            "checkpoint_id": "ckpt-ready",
            "resume_claim": {
                "schema_version": 1,
                "owner": "worker_recovery",
                "checkpoint_id": "ckpt-ready",
                "executor_state": "ready_for_planner_resume"
            },
            "resume_work_item": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-ready",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round",
                "executor_state": "ready_for_planner_resume"
            },
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-ready",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-ready", vec!["write_file:tmp/report.txt"])
    });
    insert_task(&state, "ready-planner", "running", Some(&ready_planner), 10);
    set_task_lease(
        &state,
        "ready-planner",
        &state.worker.worker_id,
        now + 30,
        1,
        now,
    );

    assert!(
        claim_ready_paused_checkpoint_resume_executor_internal(
            &state,
            "ready-planner",
            "ckpt-other",
            "ready_for_planner_resume",
            now + 1,
            30,
        )
        .expect("claim wrong checkpoint")
        .is_none(),
        "checkpoint mismatch must not claim"
    );
    assert!(
        claim_ready_paused_checkpoint_resume_executor_internal(
            &state,
            "ready-planner",
            "ckpt-ready",
            "ready_to_finalize",
            now + 1,
            30,
        )
        .expect("claim wrong state")
        .is_none(),
        "executor-state mismatch must not claim"
    );

    let claimed = claim_ready_paused_checkpoint_resume_executor_internal(
        &state,
        "ready-planner",
        "ckpt-ready",
        "ready_for_planner_resume",
        now + 1,
        30,
    )
    .expect("claim ready executor")
    .expect("executor claimed");
    assert_eq!(claimed.task_id, "ready-planner");
    assert_eq!(claimed.task.task_id, "ready-planner");
    assert_eq!(claimed.task.user_id, 42);
    assert_eq!(claimed.task.chat_id, 7);
    assert_eq!(claimed.task.channel, "ui");
    assert_eq!(claimed.task.kind, "ask");
    assert_eq!(claimed.task.claim_attempt, 1);
    assert!(
        claimed.task.payload_json.contains("long task"),
        "claimed executor must carry original task payload for seeded replay"
    );
    assert_eq!(claimed.checkpoint_id, "ckpt-ready");
    assert_eq!(claimed.previous_executor_state, "ready_for_planner_resume");
    assert_eq!(claimed.executor_state, "executing_planner_resume");
    assert_eq!(claimed.resume_trigger, "worker_recovery");
    assert_eq!(claimed.resume_directive, "run_next_planner_round");
    assert_eq!(claimed.lease_expires_at, now + 31);
    assert_eq!(
        claimed.resume_executor["executor_state"],
        "executing_planner_resume"
    );
    assert_eq!(
        claimed
            .resume_work_item
            .as_ref()
            .and_then(|value| value.get("executor_state"))
            .and_then(serde_json::Value::as_str),
        Some("executing_planner_resume")
    );
    assert_eq!(claimed.task_checkpoint.completed_side_effect_refs.len(), 1);

    let stored = stored_result_json(&state, "ready-planner");
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&stored), None);
    assert_eq!(lifecycle["state"], "running");
    assert_eq!(lifecycle["resume_due"], false);
    assert_eq!(lifecycle["resume_wait_seconds"], 0);
    assert_eq!(
        lifecycle["resume_executor"]["previous_executor_state"],
        "ready_for_planner_resume"
    );
    assert_eq!(
        lifecycle["resume_executor"]["executor_state"],
        "executing_planner_resume"
    );
    assert_eq!(lifecycle["resume_executor"]["executor_state_at"], now + 1);
    assert_eq!(
        lifecycle["resume_executor"]["executor_claim_expires_at"],
        now + 31
    );
    assert_eq!(
        lifecycle["resume_executor_claim"]["owner"],
        "worker_recovery_executor"
    );
    assert_eq!(
        lifecycle["resume_executor_claim"]["previous_executor_state"],
        "ready_for_planner_resume"
    );
    assert_eq!(
        lifecycle["resume_executor_claim"]["executor_state"],
        "executing_planner_resume"
    );
    assert_eq!(lifecycle["resume_executor_claim"]["expires_at"], now + 31);
    assert_eq!(
        lifecycle["resume_claim"]["executor_state"],
        "executing_planner_resume"
    );
    assert_eq!(
        lifecycle["resume_work_item"]["executor_state"],
        "executing_planner_resume"
    );

    let during_lease = list_ready_paused_checkpoint_resume_executors_internal(&state, now + 10, 10)
        .expect("list during executor lease");
    assert!(
        during_lease.is_empty(),
        "active executor lease must suppress duplicate ready claims"
    );

    let after_expiry = list_ready_paused_checkpoint_resume_executors_internal(&state, now + 31, 10)
        .expect("list after executor lease expiry");
    assert_eq!(after_expiry.len(), 1);
    assert_eq!(after_expiry[0].task_id, "ready-planner");
    assert_eq!(after_expiry[0].executor_state, "executing_planner_resume");

    let reclaimed = claim_ready_paused_checkpoint_resume_executor_internal(
        &state,
        "ready-planner",
        "ckpt-ready",
        "executing_planner_resume",
        now + 32,
        20,
    )
    .expect("reclaim expired executor")
    .expect("expired executor should be reclaimable");
    assert_eq!(
        reclaimed.previous_executor_state,
        "executing_planner_resume"
    );
    assert_eq!(reclaimed.executor_state, "executing_planner_resume");
    assert_eq!(reclaimed.lease_expires_at, now + 52);
}
