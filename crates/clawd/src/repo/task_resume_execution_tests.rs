use serde_json::json;

use super::{checkpoint_json, insert_task, state_with_tasks_table, stored_result_json};
use crate::repo::task_resume_execution::record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal;
use crate::repo::{
    claim_dispatched_paused_checkpoint_resume_execution_internal,
    claim_handoff_paused_checkpoint_resume_execution_internal,
    claim_ready_paused_checkpoint_resume_executor_internal,
    claim_recorded_paused_checkpoint_resume_dispatch_result_internal,
    list_dispatched_paused_checkpoint_resume_executions_internal,
    list_handoff_paused_checkpoint_resume_executions_internal,
    list_planned_paused_checkpoint_resume_executions_internal,
    list_ready_paused_checkpoint_resume_executors_internal,
    list_recorded_paused_checkpoint_resume_dispatch_results_internal,
    record_claimed_handoff_paused_checkpoint_resume_dispatch_internal,
    record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal,
    record_paused_checkpoint_resume_execution_plan_internal,
    record_planned_paused_checkpoint_resume_handoff_internal,
};

fn terminal_projection_seed(
    task_id: &str,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    dispatch_state: &str,
    executor_result_status: &str,
    now: i64,
) -> serde_json::Value {
    json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "running",
            "checkpoint_id": checkpoint_id,
            "resume_execution_plan": {
                "schema_version": 1,
                "task_id": task_id,
                "checkpoint_id": checkpoint_id,
                "executor_state": executor_state,
                "executor_action": executor_action
            },
            "resume_executor_handoff": {
                "schema_version": 1,
                "checkpoint_id": checkpoint_id,
                "executor_state": executor_state,
                "executor_action": executor_action,
                "executor_status": executor_status
            },
            "resume_executor_handoff_dispatch": {
                "schema_version": 1,
                "checkpoint_id": checkpoint_id,
                "executor_state": executor_state,
                "executor_action": executor_action,
                "executor_status": executor_status,
                "dispatch_state": dispatch_state
            },
            "resume_executor_dispatch_result": {
                "schema_version": 1,
                "checkpoint_id": checkpoint_id,
                "executor_state": executor_state,
                "executor_action": executor_action,
                "executor_status": executor_status,
                "dispatch_state": dispatch_state,
                "executor_result_status": executor_result_status,
                "recorded_at": now
            }
        },
        "task_checkpoint": checkpoint_json(checkpoint_id, vec!["write_file:tmp/report.txt"]),
        "task_journal": {
            "schema_version": 1,
            "summary": {
                "final_status": "waiting",
                "task_metrics": {
                    "llm_calls_per_task": 2,
                    "llm_elapsed_ms_per_task": 100,
                    "tool_calls": 1,
                    "prompt_truncation_count": 0
                }
            },
            "trace": {
                "step_results": [{
                    "step_id": "step_1",
                    "status": "ok",
                    "executed_skill": "run_cmd",
                    "skill": "run_cmd",
                    "requested_action_ref": "system.run_cmd",
                    "requested_capability": "system.run_cmd"
                }]
            }
        }
    })
}

fn stored_task_status_error_result(
    state: &crate::AppState,
    task_id: &str,
) -> (String, Option<String>, serde_json::Value) {
    let db = state.core.db.get().expect("get db");
    let (status, error_text, raw_result): (String, Option<String>, String) = db
        .query_row(
            "SELECT status, error_text, result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("select task status/result");
    (
        status,
        error_text,
        serde_json::from_str(&raw_result).expect("parse result_json"),
    )
}

#[test]
fn record_paused_checkpoint_resume_execution_plan_requires_active_executor_claim() {
    let state = state_with_tasks_table();
    let now = 5_000;
    let ready_planner = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "resume_due": true,
            "resume_wait_seconds": 0,
            "next_check_after": now,
            "checkpoint_id": "ckpt-plan",
            "resume_claim": {
                "schema_version": 1,
                "owner": "worker_recovery",
                "checkpoint_id": "ckpt-plan",
                "executor_state": "ready_for_planner_resume"
            },
            "resume_work_item": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-plan",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round",
                "executor_state": "ready_for_planner_resume"
            },
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-plan",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-plan", vec!["write_file:tmp/report.txt"])
    });
    insert_task(&state, "ready-plan", "running", Some(&ready_planner), 10);

    let claimed = claim_ready_paused_checkpoint_resume_executor_internal(
        &state,
        "ready-plan",
        "ckpt-plan",
        "ready_for_planner_resume",
        now + 1,
        30,
    )
    .expect("claim ready executor")
    .expect("executor claimed");
    let plan_payload = json!({
        "schema_version": 1,
        "task_id": claimed.task_id,
        "checkpoint_id": claimed.checkpoint_id,
        "executor_action": "run_seeded_agent_loop",
        "executor_state": claimed.executor_state,
        "resume_directive": claimed.resume_directive,
        "resume_trigger": claimed.resume_trigger
    });

    assert!(
        !record_paused_checkpoint_resume_execution_plan_internal(
            &state,
            "ready-plan",
            "ckpt-other",
            "executing_planner_resume",
            &plan_payload,
            now + 2,
        )
        .expect("record wrong checkpoint"),
        "checkpoint mismatch must not persist execution plan"
    );
    assert!(
        record_paused_checkpoint_resume_execution_plan_internal(
            &state,
            "ready-plan",
            "ckpt-plan",
            "executing_planner_resume",
            &plan_payload,
            now + 2,
        )
        .expect("record execution plan"),
        "active executor claim should accept execution plan"
    );

    let stored = stored_result_json(&state, "ready-plan");
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&stored), None);
    assert_eq!(
        lifecycle["resume_execution_plan"]["executor_action"],
        "run_seeded_agent_loop"
    );
    assert_eq!(lifecycle["resume_execution_plan"]["planned_at"], now + 2);
    assert_eq!(
        lifecycle["resume_execution_plan"]["completed_side_effect_count"],
        1
    );
    assert_eq!(
        lifecycle["resume_execution_plan"]["completed_side_effect_refs"],
        json!(["write_file:tmp/report.txt"])
    );
    assert_eq!(
        lifecycle["resume_execution_plan"]["requires_idempotency_guard"],
        true
    );
    assert_eq!(
        lifecycle["resume_executor"]["execution_plan_action"],
        "run_seeded_agent_loop"
    );
    assert_eq!(
        lifecycle["resume_executor_claim"]["execution_plan_action"],
        "run_seeded_agent_loop"
    );
    assert!(lifecycle["resume_execution_plan"].get("text").is_none());
    assert!(lifecycle["resume_execution_plan"]
        .get("error_text")
        .is_none());
}

#[test]
fn list_planned_paused_checkpoint_resume_executions_requires_active_machine_plan() {
    let state = state_with_tasks_table();
    let now = 6_000;
    let ready_planner = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "resume_due": true,
            "resume_wait_seconds": 0,
            "next_check_after": now,
            "checkpoint_id": "ckpt-planned",
            "resume_claim": {
                "schema_version": 1,
                "owner": "worker_recovery",
                "checkpoint_id": "ckpt-planned",
                "executor_state": "ready_for_planner_resume"
            },
            "resume_work_item": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-planned",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round",
                "executor_state": "ready_for_planner_resume"
            },
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-planned",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-planned", vec!["write_file:tmp/report.txt"])
    });
    let invalid_text_plan = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "running",
            "checkpoint_id": "ckpt-text-plan",
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-text-plan",
                "executor_state": "executing_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round",
                "execution_plan_action": "run_seeded_agent_loop"
            },
            "resume_executor_claim": {
                "schema_version": 1,
                "owner": "worker_recovery_executor",
                "checkpoint_id": "ckpt-text-plan",
                "executor_state": "executing_planner_resume",
                "expires_at": now + 30
            },
            "resume_execution_plan": {
                "schema_version": 1,
                "task_id": "invalid-text-plan",
                "checkpoint_id": "ckpt-text-plan",
                "executor_state": "executing_planner_resume",
                "executor_action": "run_seeded_agent_loop",
                "text": "not a machine-only plan"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-text-plan", vec![])
    });
    insert_task(&state, "ready-planned", "running", Some(&ready_planner), 10);
    insert_task(
        &state,
        "invalid-text-plan",
        "running",
        Some(&invalid_text_plan),
        20,
    );

    let claimed = claim_ready_paused_checkpoint_resume_executor_internal(
        &state,
        "ready-planned",
        "ckpt-planned",
        "ready_for_planner_resume",
        now + 1,
        30,
    )
    .expect("claim ready executor")
    .expect("executor claimed");
    let plan_payload = json!({
        "schema_version": 1,
        "task_id": claimed.task_id,
        "checkpoint_id": claimed.checkpoint_id,
        "executor_action": "run_seeded_agent_loop",
        "executor_state": claimed.executor_state,
        "resume_directive": claimed.resume_directive,
        "resume_trigger": claimed.resume_trigger,
        "completed_side_effect_count": 1,
        "requires_idempotency_guard": true
    });
    assert!(record_paused_checkpoint_resume_execution_plan_internal(
        &state,
        "ready-planned",
        "ckpt-planned",
        "executing_planner_resume",
        &plan_payload,
        now + 2,
    )
    .expect("record execution plan"));

    let planned = list_planned_paused_checkpoint_resume_executions_internal(&state, now + 3, 10)
        .expect("list planned executions");
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].task_id, "ready-planned");
    assert_eq!(planned[0].task.task_id, "ready-planned");
    assert_eq!(planned[0].checkpoint_id, "ckpt-planned");
    assert_eq!(planned[0].executor_state, "executing_planner_resume");
    assert_eq!(planned[0].executor_action, "run_seeded_agent_loop");
    assert_eq!(planned[0].resume_trigger, "worker_recovery");
    assert_eq!(planned[0].resume_directive, "run_next_planner_round");
    assert_eq!(planned[0].lease_expires_at, now + 31);
    assert_eq!(
        planned[0].task_checkpoint.completed_side_effect_refs.len(),
        1
    );
    assert!(planned[0].execution_plan.get("text").is_none());
    assert!(planned[0].execution_plan.get("error_text").is_none());

    let expired = list_planned_paused_checkpoint_resume_executions_internal(&state, now + 31, 10)
        .expect("list after lease expiry");
    assert!(
        expired.is_empty(),
        "expired executor plans must be reclaimed through the ready queue before execution"
    );
}

#[test]
fn record_planned_paused_checkpoint_resume_handoff_requires_active_machine_plan() {
    let state = state_with_tasks_table();
    let now = 7_000;
    let ready_planner = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "resume_due": true,
            "resume_wait_seconds": 0,
            "next_check_after": now,
            "checkpoint_id": "ckpt-handoff",
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-handoff",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-handoff", vec![])
    });
    insert_task(&state, "ready-handoff", "running", Some(&ready_planner), 10);

    let claimed = claim_ready_paused_checkpoint_resume_executor_internal(
        &state,
        "ready-handoff",
        "ckpt-handoff",
        "ready_for_planner_resume",
        now + 1,
        30,
    )
    .expect("claim ready executor")
    .expect("executor claimed");
    let plan_payload = json!({
        "schema_version": 1,
        "task_id": claimed.task_id,
        "checkpoint_id": claimed.checkpoint_id,
        "executor_action": "run_seeded_agent_loop",
        "executor_state": claimed.executor_state,
        "resume_directive": claimed.resume_directive,
        "resume_trigger": claimed.resume_trigger
    });
    assert!(record_paused_checkpoint_resume_execution_plan_internal(
        &state,
        "ready-handoff",
        "ckpt-handoff",
        "executing_planner_resume",
        &plan_payload,
        now + 2,
    )
    .expect("record execution plan"));

    let bad_handoff = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-other",
        "executor_state": "executing_planner_resume",
        "executor_action": "run_seeded_agent_loop",
        "executor_status": "seeded_loop_requires_provider_window"
    });
    assert!(
        !record_planned_paused_checkpoint_resume_handoff_internal(
            &state,
            "ready-handoff",
            "ckpt-handoff",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            &bad_handoff,
            now + 3,
        )
        .expect("record mismatched handoff"),
        "checkpoint mismatch must not persist handoff"
    );

    let handoff = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-handoff",
        "executor_state": "executing_planner_resume",
        "executor_action": "run_seeded_agent_loop",
        "executor_status": "seeded_loop_requires_provider_window"
    });
    assert!(
        record_planned_paused_checkpoint_resume_handoff_internal(
            &state,
            "ready-handoff",
            "ckpt-handoff",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            &handoff,
            now + 3,
        )
        .expect("record handoff"),
        "active machine plan should accept matching handoff"
    );
    let stored = stored_result_json(&state, "ready-handoff");
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&stored), None);
    assert_eq!(
        lifecycle["resume_executor_handoff"]["executor_status"],
        "seeded_loop_requires_provider_window"
    );
    assert_eq!(lifecycle["resume_executor_handoff"]["handoff_at"], now + 3);
    assert_eq!(
        lifecycle["resume_executor"]["executor_status"],
        "seeded_loop_requires_provider_window"
    );
    assert_eq!(
        lifecycle["resume_executor_claim"]["executor_status"],
        "seeded_loop_requires_provider_window"
    );
    assert!(lifecycle["resume_executor_handoff"].get("text").is_none());
    assert!(lifecycle["resume_executor_handoff"]
        .get("error_text")
        .is_none());
    let planned_after_handoff =
        list_planned_paused_checkpoint_resume_executions_internal(&state, now + 4, 10)
            .expect("list planned after handoff");
    assert!(
        planned_after_handoff.is_empty(),
        "planned queue should hand off ownership after handoff is recorded"
    );
    let handoff_queue =
        list_handoff_paused_checkpoint_resume_executions_internal(&state, now + 4, 10)
            .expect("list handoff queue");
    assert_eq!(handoff_queue.len(), 1);
    assert_eq!(handoff_queue[0].task_id, "ready-handoff");
    assert_eq!(handoff_queue[0].checkpoint_id, "ckpt-handoff");
    assert_eq!(
        handoff_queue[0].executor_status,
        "seeded_loop_requires_provider_window"
    );
    assert_eq!(handoff_queue[0].executor_action, "run_seeded_agent_loop");
    assert_eq!(handoff_queue[0].executor_state, "executing_planner_resume");
    assert_eq!(
        handoff_queue[0].handoff_payload["executor_status"],
        "seeded_loop_requires_provider_window"
    );
    assert!(handoff_queue[0].handoff_payload.get("text").is_none());
    assert!(handoff_queue[0].handoff_payload.get("error_text").is_none());

    let text_handoff = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-handoff",
        "executor_state": "executing_planner_resume",
        "executor_action": "run_seeded_agent_loop",
        "executor_status": "seeded_loop_requires_provider_window",
        "text": "not machine-only"
    });
    assert!(
        !record_planned_paused_checkpoint_resume_handoff_internal(
            &state,
            "ready-handoff",
            "ckpt-handoff",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            &text_handoff,
            now + 4,
        )
        .expect("record text handoff"),
        "handoff payloads with user-visible text must be rejected"
    );
    assert!(
        !record_planned_paused_checkpoint_resume_handoff_internal(
            &state,
            "ready-handoff",
            "ckpt-handoff",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            &handoff,
            now + 31,
        )
        .expect("record expired handoff"),
        "expired executor leases must be reclaimed before handoff updates"
    );
    let expired_handoff_queue =
        list_handoff_paused_checkpoint_resume_executions_internal(&state, now + 31, 10)
            .expect("list expired handoff queue");
    assert!(
        expired_handoff_queue.is_empty(),
        "expired executor handoff leases must not remain executable"
    );
}

#[test]
fn claim_handoff_paused_checkpoint_resume_execution_uses_active_machine_lease() {
    let state = state_with_tasks_table();
    let now = 8_000;
    let ready_planner = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "resume_due": true,
            "resume_wait_seconds": 0,
            "next_check_after": now,
            "checkpoint_id": "ckpt-handoff-claim",
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-handoff-claim",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-handoff-claim", vec!["write_file:tmp/report.txt"])
    });
    insert_task(&state, "handoff-claim", "running", Some(&ready_planner), 10);

    let claimed_executor = claim_ready_paused_checkpoint_resume_executor_internal(
        &state,
        "handoff-claim",
        "ckpt-handoff-claim",
        "ready_for_planner_resume",
        now + 1,
        30,
    )
    .expect("claim ready executor")
    .expect("executor claimed");
    let plan_payload = json!({
        "schema_version": 1,
        "task_id": claimed_executor.task_id,
        "checkpoint_id": claimed_executor.checkpoint_id,
        "executor_action": "run_seeded_agent_loop",
        "executor_state": claimed_executor.executor_state,
        "resume_directive": claimed_executor.resume_directive,
        "resume_trigger": claimed_executor.resume_trigger
    });
    assert!(record_paused_checkpoint_resume_execution_plan_internal(
        &state,
        "handoff-claim",
        "ckpt-handoff-claim",
        "executing_planner_resume",
        &plan_payload,
        now + 2,
    )
    .expect("record execution plan"));
    let handoff_payload = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-handoff-claim",
        "executor_state": "executing_planner_resume",
        "executor_action": "run_seeded_agent_loop",
        "executor_status": "seeded_loop_requires_provider_window"
    });
    assert!(record_planned_paused_checkpoint_resume_handoff_internal(
        &state,
        "handoff-claim",
        "ckpt-handoff-claim",
        "executing_planner_resume",
        "run_seeded_agent_loop",
        &handoff_payload,
        now + 3,
    )
    .expect("record handoff"));

    assert!(
        claim_handoff_paused_checkpoint_resume_execution_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "verify_and_finalize",
            "seeded_loop_requires_provider_window",
            now + 4,
            20,
        )
        .expect("claim wrong action")
        .is_none(),
        "wrong handoff action must not claim"
    );
    assert!(
        claim_handoff_paused_checkpoint_resume_execution_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            "async_poll_adapter_pending",
            now + 4,
            20,
        )
        .expect("claim wrong status")
        .is_none(),
        "wrong handoff status must not claim"
    );
    let dispatch_payload = json!({
        "schema_version": 1,
        "task_id": "handoff-claim",
        "checkpoint_id": "ckpt-handoff-claim",
        "executor_state": "executing_planner_resume",
        "executor_action": "run_seeded_agent_loop",
        "executor_status": "seeded_loop_requires_provider_window",
        "dispatch_state": "ready_to_run_seeded_agent_loop"
    });
    assert!(
        !record_claimed_handoff_paused_checkpoint_resume_dispatch_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            &dispatch_payload,
            now + 4,
        )
        .expect("record unclaimed dispatch"),
        "handoff dispatch must require an active handoff claim"
    );

    let claimed_handoff = claim_handoff_paused_checkpoint_resume_execution_internal(
        &state,
        "handoff-claim",
        "ckpt-handoff-claim",
        "executing_planner_resume",
        "run_seeded_agent_loop",
        "seeded_loop_requires_provider_window",
        now + 5,
        20,
    )
    .expect("claim handoff")
    .expect("handoff claimed");
    assert_eq!(claimed_handoff.task_id, "handoff-claim");
    assert_eq!(claimed_handoff.task.task_id, "handoff-claim");
    assert_eq!(claimed_handoff.checkpoint_id, "ckpt-handoff-claim");
    assert_eq!(claimed_handoff.executor_state, "executing_planner_resume");
    assert_eq!(claimed_handoff.executor_action, "run_seeded_agent_loop");
    assert_eq!(
        claimed_handoff.executor_status,
        "seeded_loop_requires_provider_window"
    );
    assert_eq!(claimed_handoff.lease_expires_at, now + 31);
    assert_eq!(claimed_handoff.handoff_claim_expires_at, now + 25);
    assert_eq!(
        claimed_handoff.handoff_claim["owner"],
        "worker_recovery_handoff_executor"
    );
    assert!(claimed_handoff.handoff_payload.get("text").is_none());
    assert!(claimed_handoff.handoff_payload.get("error_text").is_none());

    let stored = stored_result_json(&state, "handoff-claim");
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&stored), None);
    assert_eq!(
        lifecycle["resume_executor_handoff_claim"]["owner"],
        "worker_recovery_handoff_executor"
    );
    assert_eq!(
        lifecycle["resume_executor_handoff_claim"]["expires_at"],
        now + 25
    );
    assert_eq!(
        lifecycle["resume_executor_handoff"]["claim_state"],
        "claimed"
    );
    assert_eq!(
        lifecycle["resume_executor"]["handoff_claim_expires_at"],
        now + 25
    );
    assert!(
        !record_claimed_handoff_paused_checkpoint_resume_dispatch_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            &json!({
                "checkpoint_id": "ckpt-handoff-claim",
                "executor_state": "executing_planner_resume",
                "executor_action": "run_seeded_agent_loop",
                "executor_status": "seeded_loop_requires_provider_window",
                "dispatch_state": "ready_to_run_seeded_agent_loop",
                "text": "not machine-only"
            }),
            now + 6,
        )
        .expect("record text dispatch"),
        "dispatch payloads with user-visible text must be rejected"
    );
    assert!(
        record_claimed_handoff_paused_checkpoint_resume_dispatch_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            &dispatch_payload,
            now + 6,
        )
        .expect("record claimed dispatch"),
        "active handoff claim should accept matching dispatch payload"
    );
    let dispatched = stored_result_json(&state, "handoff-claim");
    let dispatched_lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&dispatched), None);
    assert_eq!(
        dispatched_lifecycle["resume_executor_handoff_dispatch"]["dispatch_state"],
        "ready_to_run_seeded_agent_loop"
    );
    assert_eq!(
        dispatched_lifecycle["resume_executor_handoff_dispatch"]["dispatched_at"],
        now + 6
    );
    assert_eq!(
        dispatched_lifecycle["resume_executor"]["dispatch_state"],
        "ready_to_run_seeded_agent_loop"
    );

    let dispatched_queue =
        list_dispatched_paused_checkpoint_resume_executions_internal(&state, now + 7, 10)
            .expect("list dispatched execution queue");
    assert_eq!(dispatched_queue.len(), 1);
    assert_eq!(dispatched_queue[0].task_id, "handoff-claim");
    assert_eq!(
        dispatched_queue[0].dispatch_state,
        "ready_to_run_seeded_agent_loop"
    );
    assert_eq!(
        dispatched_queue[0].dispatch_execution_state,
        "claimed_to_run_seeded_agent_loop"
    );
    assert_eq!(
        dispatched_queue[0].handoff_claim_expires_at,
        now + 25,
        "dispatch execution must stay bounded by the handoff claim lease"
    );
    assert!(
        claim_dispatched_paused_checkpoint_resume_execution_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            "ready_to_poll_async_job",
            now + 7,
            20,
        )
        .expect("claim wrong dispatch state")
        .is_none(),
        "wrong dispatch state must not claim"
    );
    let claimed_dispatch = claim_dispatched_paused_checkpoint_resume_execution_internal(
        &state,
        "handoff-claim",
        "ckpt-handoff-claim",
        "executing_planner_resume",
        "run_seeded_agent_loop",
        "seeded_loop_requires_provider_window",
        "ready_to_run_seeded_agent_loop",
        now + 7,
        5,
    )
    .expect("claim dispatched execution")
    .expect("dispatch execution claimed");
    assert_eq!(claimed_dispatch.task_id, "handoff-claim");
    assert_eq!(claimed_dispatch.task.task_id, "handoff-claim");
    assert_eq!(
        claimed_dispatch.dispatch_execution_state,
        "claimed_to_run_seeded_agent_loop"
    );
    assert_eq!(
        claimed_dispatch.dispatch_claim_expires_at,
        now + 12,
        "dispatch claim lease is capped by the shorter dispatch lease"
    );
    assert_eq!(
        claimed_dispatch.dispatch_claim["owner"],
        "worker_recovery_dispatch_executor"
    );
    assert!(claimed_dispatch.dispatch_payload.get("text").is_none());
    assert!(claimed_dispatch
        .dispatch_payload
        .get("error_text")
        .is_none());
    let dispatch_claimed = stored_result_json(&state, "handoff-claim");
    let dispatch_claimed_lifecycle = crate::task_lifecycle::task_query_lifecycle_projection(
        "running",
        Some(&dispatch_claimed),
        None,
    );
    assert_eq!(
        dispatch_claimed_lifecycle["resume_executor_dispatch_claim"]["owner"],
        "worker_recovery_dispatch_executor"
    );
    assert_eq!(
        dispatch_claimed_lifecycle["resume_executor_handoff_dispatch"]["claim_state"],
        "claimed"
    );
    assert_eq!(
        dispatch_claimed_lifecycle["resume_executor_handoff_dispatch"]["dispatch_execution_state"],
        "claimed_to_run_seeded_agent_loop"
    );
    assert!(
        list_dispatched_paused_checkpoint_resume_executions_internal(&state, now + 8, 10)
            .expect("list active dispatch claim")
            .is_empty(),
        "active dispatch claims must suppress duplicate executor ownership"
    );
    let result_payload = json!({
        "schema_version": 1,
        "task_id": "handoff-claim",
        "checkpoint_id": "ckpt-handoff-claim",
        "executor_state": "executing_planner_resume",
        "executor_action": "run_seeded_agent_loop",
        "executor_status": "seeded_loop_requires_provider_window",
        "dispatch_state": "ready_to_run_seeded_agent_loop",
        "executor_result_status": "seeded_loop_deferred",
        "retry_after_seconds": 60
    });
    assert!(
        !record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            "ready_to_run_seeded_agent_loop",
            &json!({
                "checkpoint_id": "ckpt-handoff-claim",
                "executor_state": "executing_planner_resume",
                "executor_action": "run_seeded_agent_loop",
                "executor_status": "seeded_loop_requires_provider_window",
                "dispatch_state": "ready_to_run_seeded_agent_loop",
                "executor_result_status": "seeded_loop_deferred",
                "text": "not machine-only"
            }),
            now + 8,
        )
        .expect("record text result"),
        "dispatch execution results with user-visible text must be rejected"
    );
    assert!(
        record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            "ready_to_run_seeded_agent_loop",
            &result_payload,
            now + 8,
        )
        .expect("record dispatch execution result"),
        "active dispatch claim should accept matching machine-only result payload"
    );
    let result_recorded = stored_result_json(&state, "handoff-claim");
    let result_recorded_lifecycle = crate::task_lifecycle::task_query_lifecycle_projection(
        "running",
        Some(&result_recorded),
        None,
    );
    assert_eq!(
        result_recorded_lifecycle["resume_executor_dispatch_result"]["executor_result_status"],
        "seeded_loop_deferred"
    );
    assert_eq!(
        result_recorded_lifecycle["resume_executor_dispatch_result"]["projection_pending_reason"],
        "result_projection_pending"
    );
    assert_eq!(
        result_recorded_lifecycle["resume_executor_dispatch_result"]["recorded_at"],
        now + 8
    );
    assert_eq!(
        result_recorded_lifecycle["resume_executor"]["executor_result_status"],
        "seeded_loop_deferred"
    );
    let result_queue =
        list_recorded_paused_checkpoint_resume_dispatch_results_internal(&state, now + 9, 10)
            .expect("list recorded dispatch results");
    assert_eq!(result_queue.len(), 1);
    assert_eq!(result_queue[0].task_id, "handoff-claim");
    assert_eq!(result_queue[0].task.task_id, "handoff-claim");
    assert_eq!(result_queue[0].checkpoint_id, "ckpt-handoff-claim");
    assert_eq!(result_queue[0].executor_state, "executing_planner_resume");
    assert_eq!(result_queue[0].executor_action, "run_seeded_agent_loop");
    assert_eq!(
        result_queue[0].executor_status,
        "seeded_loop_requires_provider_window"
    );
    assert_eq!(
        result_queue[0].dispatch_state,
        "ready_to_run_seeded_agent_loop"
    );
    assert_eq!(
        result_queue[0].executor_result_status,
        "seeded_loop_deferred"
    );
    assert_eq!(
        result_queue[0].result_projection_state,
        "project_seeded_loop_deferred"
    );
    assert_eq!(result_queue[0].recorded_at, now + 8);
    assert!(result_queue[0]
        .execution_result_payload
        .get("text")
        .is_none());
    assert!(result_queue[0]
        .execution_result_payload
        .get("error_text")
        .is_none());
    assert!(
        claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            "ready_to_run_seeded_agent_loop",
            "seeded_loop_completed",
            now + 9,
            5,
        )
        .expect("claim wrong result status")
        .is_none(),
        "wrong executor result status must not claim projection ownership"
    );
    let claimed_result_projection =
        claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            "ready_to_run_seeded_agent_loop",
            "seeded_loop_deferred",
            now + 9,
            5,
        )
        .expect("claim recorded dispatch result")
        .expect("dispatch result projection claimed");
    assert_eq!(claimed_result_projection.task_id, "handoff-claim");
    assert_eq!(
        claimed_result_projection.executor_result_status,
        "seeded_loop_deferred"
    );
    assert_eq!(
        claimed_result_projection.result_projection_state,
        "project_seeded_loop_deferred"
    );
    assert_eq!(
        claimed_result_projection.result_projection_claim_expires_at,
        now + 14
    );
    assert_eq!(
        claimed_result_projection.result_projection_claim["owner"],
        "worker_recovery_result_projector"
    );
    let result_projection_claimed = stored_result_json(&state, "handoff-claim");
    let result_projection_claimed_lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection(
            "running",
            Some(&result_projection_claimed),
            None,
        );
    assert_eq!(
        result_projection_claimed_lifecycle["resume_executor_result_projection_claim"]["owner"],
        "worker_recovery_result_projector"
    );
    assert_eq!(
        result_projection_claimed_lifecycle["resume_executor_dispatch_result"]
            ["projection_claim_state"],
        "claimed"
    );
    assert_eq!(
        result_projection_claimed_lifecycle["resume_executor_dispatch_result"]
            ["result_projection_state"],
        "project_seeded_loop_deferred"
    );
    assert_eq!(
        result_projection_claimed_lifecycle["resume_executor_dispatch_result"]
            ["projection_pending_reason"],
        "result_projection_pending"
    );
    assert_eq!(
        result_projection_claimed_lifecycle["resume_executor_result_projection_claim"]
            ["projection_pending_reason"],
        "result_projection_pending"
    );
    assert!(
        list_recorded_paused_checkpoint_resume_dispatch_results_internal(&state, now + 10, 10)
            .expect("list active result projection claim")
            .is_empty(),
        "active result projection claims must suppress duplicate projection ownership"
    );
    assert_eq!(
        list_recorded_paused_checkpoint_resume_dispatch_results_internal(&state, now + 15, 10)
            .expect("list expired result projection claim")
            .len(),
        1,
        "expired result projection claims can be reclaimed until a projection result is recorded"
    );
    assert!(
        list_dispatched_paused_checkpoint_resume_executions_internal(&state, now + 13, 10)
            .expect("list after dispatch claim expiry with result")
            .is_empty(),
        "recorded dispatch results must suppress duplicate dispatch ownership"
    );
    let redispatched =
        list_dispatched_paused_checkpoint_resume_executions_internal(&state, now + 26, 10)
            .expect("list expired dispatch claim");
    assert!(
        redispatched.is_empty(),
        "dispatch queue must not outlive the parent handoff claim lease"
    );

    let suppressed = list_handoff_paused_checkpoint_resume_executions_internal(&state, now + 6, 10)
        .expect("list active claimed handoff");
    assert!(
        suppressed.is_empty(),
        "active handoff claims must suppress duplicate executor ownership"
    );

    let expired = list_handoff_paused_checkpoint_resume_executions_internal(&state, now + 26, 10)
        .expect("list expired handoff claim");
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].task_id, "handoff-claim");

    let reclaimed = claim_handoff_paused_checkpoint_resume_execution_internal(
        &state,
        "handoff-claim",
        "ckpt-handoff-claim",
        "executing_planner_resume",
        "run_seeded_agent_loop",
        "seeded_loop_requires_provider_window",
        now + 27,
        20,
    )
    .expect("reclaim handoff")
    .expect("expired handoff claim can be reclaimed");
    assert_eq!(
        reclaimed.handoff_claim_expires_at,
        now + 31,
        "handoff claim lease is capped by the active executor lease"
    );

    let reclaimed_projection = claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
        &state,
        "handoff-claim",
        "ckpt-handoff-claim",
        "executing_planner_resume",
        "run_seeded_agent_loop",
        "seeded_loop_requires_provider_window",
        "ready_to_run_seeded_agent_loop",
        "seeded_loop_deferred",
        now + 28,
        5,
    )
    .expect("reclaim recorded dispatch result")
    .expect("expired result projection claim can be reclaimed");
    assert_eq!(
        reclaimed_projection.result_projection_state,
        "project_seeded_loop_deferred"
    );
    let projection_payload = json!({
        "schema_version": 1,
        "task_id": "handoff-claim",
        "checkpoint_id": "ckpt-handoff-claim",
        "executor_state": "executing_planner_resume",
        "executor_action": "run_seeded_agent_loop",
        "executor_status": "seeded_loop_requires_provider_window",
        "dispatch_state": "ready_to_run_seeded_agent_loop",
        "executor_result_status": "seeded_loop_deferred",
        "result_projection_state": "project_seeded_loop_deferred",
        "retry_after_seconds": 60
    });
    assert!(
        !record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            "ready_to_run_seeded_agent_loop",
            "seeded_loop_deferred",
            &json!({
                "checkpoint_id": "ckpt-handoff-claim",
                "executor_state": "executing_planner_resume",
                "executor_action": "run_seeded_agent_loop",
                "executor_status": "seeded_loop_requires_provider_window",
                "dispatch_state": "ready_to_run_seeded_agent_loop",
                "executor_result_status": "seeded_loop_deferred",
                "result_projection_state": "project_seeded_loop_deferred",
                "retry_after_seconds": 60,
                "text": "not machine-only"
            }),
            now + 29,
        )
        .expect("record text projection"),
        "result projection payloads with user-visible text must be rejected"
    );
    assert!(
        record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
            &state,
            "handoff-claim",
            "ckpt-handoff-claim",
            "executing_planner_resume",
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            "ready_to_run_seeded_agent_loop",
            "seeded_loop_deferred",
            &projection_payload,
            now + 29,
        )
        .expect("record reschedule projection"),
        "active projection claim should accept matching reschedule projection"
    );
    let projection_recorded = stored_result_json(&state, "handoff-claim");
    let projection_recorded_lifecycle = crate::task_lifecycle::task_query_lifecycle_projection(
        "running",
        Some(&projection_recorded),
        None,
    );
    assert_eq!(projection_recorded_lifecycle["state"], "background");
    assert_eq!(
        projection_recorded_lifecycle["resume_reason"],
        "seeded_loop_deferred"
    );
    assert_eq!(projection_recorded_lifecycle["next_check_after"], now + 89);
    assert_eq!(
        projection_recorded_lifecycle["resume_executor"]["executor_state"],
        "ready_for_planner_resume"
    );
    assert_eq!(
        projection_recorded_lifecycle["resume_executor_result_projection"]
            ["projection_result_status"],
        "rescheduled"
    );
    assert!(
        projection_recorded_lifecycle
            .get("resume_executor_dispatch_result")
            .is_none(),
        "old dispatch result should be folded into the projection record before retry"
    );
    assert!(
        projection_recorded_lifecycle
            .get("resume_executor_handoff")
            .is_none(),
        "old handoff state should not block a rescheduled retry"
    );
    assert!(
        list_ready_paused_checkpoint_resume_executors_internal(&state, now + 88, 10)
            .expect("list before reschedule due")
            .is_empty(),
        "rescheduled executor must not be ready before next_check_after"
    );
    let ready_after_projection =
        list_ready_paused_checkpoint_resume_executors_internal(&state, now + 89, 10)
            .expect("list after reschedule due");
    assert_eq!(ready_after_projection.len(), 1);
    assert_eq!(ready_after_projection[0].task_id, "handoff-claim");
    assert_eq!(
        ready_after_projection[0].executor_state,
        "ready_for_planner_resume"
    );
    assert!(
        list_recorded_paused_checkpoint_resume_dispatch_results_internal(&state, now + 89, 10)
            .expect("list after projection recorded")
            .is_empty(),
        "recorded projection should suppress duplicate result projection ownership"
    );
}

#[path = "task_resume_execution_tests/terminal_result_projection.rs"]
mod terminal_result_projection;
