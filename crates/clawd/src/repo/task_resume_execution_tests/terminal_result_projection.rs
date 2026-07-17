use super::*;

#[test]
fn async_poll_retry_plan_clears_stale_projection_before_terminal_poll() {
    let state = state_with_tasks_table();
    let now = 10_000;
    let checkpoint_id = "ckpt-async-retry";
    let task_id = "async-retry-terminal";
    let stale_retry = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "checkpoint_id": checkpoint_id,
            "resume_reason": "async_poll_rescheduled",
            "resume_due": true,
            "resume_wait_seconds": 0,
            "next_check_after": now,
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": checkpoint_id,
                "executor_state": "poll_scheduled",
                "resume_trigger": "worker_recovery",
                "resume_directive": "poll_async_job",
                "job_id": "local_process:retry-job",
                "poll_after_seconds": 1,
                "expires_at": now + 300,
                "cancel_ref": "local_process:/tmp/retry-job",
                "message_key": "clawd.task.async_job_pending",
                "executor_result_status": "async_poll_rescheduled",
                "result_projection_state": "project_async_poll_rescheduled"
            },
            "resume_execution_plan": {
                "schema_version": 1,
                "task_id": task_id,
                "checkpoint_id": checkpoint_id,
                "executor_state": "executing_async_poll",
                "executor_action": "poll_async_job"
            },
            "resume_executor_handoff": {
                "schema_version": 1,
                "checkpoint_id": checkpoint_id,
                "executor_state": "executing_async_poll",
                "executor_action": "poll_async_job",
                "executor_status": "async_poll_adapter_pending"
            },
            "resume_executor_handoff_dispatch": {
                "schema_version": 1,
                "checkpoint_id": checkpoint_id,
                "executor_state": "executing_async_poll",
                "executor_action": "poll_async_job",
                "executor_status": "async_poll_adapter_pending",
                "dispatch_state": "ready_to_poll_async_job"
            },
            "resume_executor_dispatch_result": {
                "schema_version": 1,
                "task_id": task_id,
                "checkpoint_id": checkpoint_id,
                "executor_state": "executing_async_poll",
                "executor_action": "poll_async_job",
                "executor_status": "async_poll_adapter_pending",
                "dispatch_state": "ready_to_poll_async_job",
                "executor_result_status": "async_poll_rescheduled",
                "result_projection_state": "project_async_poll_rescheduled",
                "recorded_at": now - 1
            },
            "resume_executor_result_projection": {
                "schema_version": 1,
                "task_id": task_id,
                "checkpoint_id": checkpoint_id,
                "executor_state": "executing_async_poll",
                "executor_action": "poll_async_job",
                "executor_status": "async_poll_adapter_pending",
                "dispatch_state": "ready_to_poll_async_job",
                "executor_result_status": "async_poll_rescheduled",
                "result_projection_state": "project_async_poll_rescheduled",
                "projection_result_status": "rescheduled",
                "projected_at": now - 1
            }
        },
        "task_checkpoint": checkpoint_json(
            checkpoint_id,
            vec!["run_skill:run_cmd:async_job:local_process:retry-job"]
        )
    });
    insert_task(&state, task_id, "running", Some(&stale_retry), now - 10);
    activate_resume_owner(&state, task_id, checkpoint_id, now, now + 31);

    let claimed_executor = claim_ready_paused_checkpoint_resume_executor_internal(
        &state,
        task_id,
        checkpoint_id,
        "poll_scheduled",
        now + 1,
        30,
    )
    .expect("claim retry executor")
    .expect("retry executor claimed");
    let plan_payload = json!({
        "schema_version": 1,
        "task_id": task_id,
        "checkpoint_id": checkpoint_id,
        "executor_action": "poll_async_job",
        "executor_state": claimed_executor.executor_state,
        "resume_directive": claimed_executor.resume_directive,
        "resume_trigger": claimed_executor.resume_trigger,
        "job_id": "local_process:retry-job",
        "poll_after_seconds": 1,
        "expires_at": now + 300,
        "cancel_ref": "local_process:/tmp/retry-job",
        "message_key": "clawd.task.async_job_pending"
    });
    assert!(record_paused_checkpoint_resume_execution_plan_internal(
        &state,
        task_id,
        checkpoint_id,
        "executing_async_poll",
        &plan_payload,
        now + 2,
    )
    .expect("record retry plan"));
    let plan_recorded = stored_result_json(&state, task_id);
    let plan_lifecycle = crate::task_lifecycle::task_query_lifecycle_projection(
        "running",
        Some(&plan_recorded),
        None,
    );
    for key in [
        "resume_executor_handoff",
        "resume_executor_handoff_dispatch",
        "resume_executor_dispatch_result",
        "resume_executor_result_projection",
    ] {
        assert!(
            plan_lifecycle.get(key).is_none(),
            "new execution plan must clear stale {key}"
        );
    }
    assert!(plan_lifecycle["resume_executor"]
        .get("executor_result_status")
        .is_none());

    let handoff_payload = json!({
        "schema_version": 1,
        "checkpoint_id": checkpoint_id,
        "executor_state": "executing_async_poll",
        "executor_action": "poll_async_job",
        "executor_status": "async_poll_adapter_pending"
    });
    assert!(record_planned_paused_checkpoint_resume_handoff_internal(
        &state,
        task_id,
        checkpoint_id,
        "executing_async_poll",
        "poll_async_job",
        &handoff_payload,
        now + 3,
    )
    .expect("record retry handoff"));
    claim_handoff_paused_checkpoint_resume_execution_internal(
        &state,
        task_id,
        checkpoint_id,
        "executing_async_poll",
        "poll_async_job",
        "async_poll_adapter_pending",
        now + 4,
        20,
    )
    .expect("claim retry handoff")
    .expect("retry handoff claimed");
    let dispatch_payload = json!({
        "schema_version": 1,
        "task_id": task_id,
        "checkpoint_id": checkpoint_id,
        "executor_state": "executing_async_poll",
        "executor_action": "poll_async_job",
        "executor_status": "async_poll_adapter_pending",
        "dispatch_state": "ready_to_poll_async_job"
    });
    assert!(
        record_claimed_handoff_paused_checkpoint_resume_dispatch_internal(
            &state,
            task_id,
            checkpoint_id,
            "executing_async_poll",
            "poll_async_job",
            "async_poll_adapter_pending",
            &dispatch_payload,
            now + 5,
        )
        .expect("record retry dispatch")
    );
    claim_dispatched_paused_checkpoint_resume_execution_internal(
        &state,
        task_id,
        checkpoint_id,
        "executing_async_poll",
        "poll_async_job",
        "async_poll_adapter_pending",
        "ready_to_poll_async_job",
        now + 6,
        20,
    )
    .expect("claim retry dispatch")
    .expect("retry dispatch claimed");
    let completed_payload = json!({
        "schema_version": 1,
        "task_id": task_id,
        "checkpoint_id": checkpoint_id,
        "executor_state": "executing_async_poll",
        "executor_action": "poll_async_job",
        "executor_status": "async_poll_adapter_pending",
        "dispatch_state": "ready_to_poll_async_job",
        "executor_result_status": "async_poll_completed",
        "result_projection_state": "project_async_poll_completed",
        "final_result_json": {
            "status": "ok",
            "output": "RUSTCLAW_ASYNC_RETRY_DONE"
        }
    });
    assert!(
        record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal(
            &state,
            task_id,
            checkpoint_id,
            "executing_async_poll",
            "poll_async_job",
            "async_poll_adapter_pending",
            "ready_to_poll_async_job",
            &completed_payload,
            now + 7,
        )
        .expect("record terminal retry result")
    );
    claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
        &state,
        task_id,
        checkpoint_id,
        "executing_async_poll",
        "poll_async_job",
        "async_poll_adapter_pending",
        "ready_to_poll_async_job",
        "async_poll_completed",
        now + 8,
        20,
    )
    .expect("claim terminal projection")
    .expect("terminal projection claimed");
    assert!(
        record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
            &state,
            task_id,
            checkpoint_id,
            "executing_async_poll",
            "poll_async_job",
            "async_poll_adapter_pending",
            "ready_to_poll_async_job",
            "async_poll_completed",
            &completed_payload,
            now + 9,
        )
        .expect("record terminal projection")
    );

    let (status, error_text, result) = stored_task_status_error_result(&state, task_id);
    assert_eq!(status, "succeeded");
    assert_eq!(error_text, None);
    assert_eq!(result["output"], "RUSTCLAW_ASYNC_RETRY_DONE");
    assert_eq!(result["task_lifecycle"]["state"], "succeeded");
}

#[test]
fn terminal_dispatch_result_projection_updates_task_status_with_machine_payload() {
    let state = state_with_tasks_table();
    let now = 9_000;
    let completed_seed = terminal_projection_seed(
        "terminal-completed",
        "ckpt-terminal-completed",
        "executing_finalize",
        "verify_and_finalize",
        "checkpoint_finalize_executor_pending",
        "ready_to_verify_and_finalize",
        "finalize_completed",
        now,
    );
    let failed_seed = terminal_projection_seed(
        "terminal-failed",
        "ckpt-terminal-failed",
        "executing_async_poll",
        "poll_async_job",
        "async_poll_adapter_pending",
        "ready_to_poll_async_job",
        "async_poll_failed",
        now,
    );
    let cancelled_seed = terminal_projection_seed(
        "terminal-cancelled",
        "ckpt-terminal-cancelled",
        "executing_async_poll",
        "poll_async_job",
        "async_poll_adapter_pending",
        "ready_to_poll_async_job",
        "async_poll_cancelled",
        now,
    );
    insert_task(
        &state,
        "terminal-completed",
        "running",
        Some(&completed_seed),
        now,
    );
    insert_task(
        &state,
        "terminal-failed",
        "running",
        Some(&failed_seed),
        now,
    );
    insert_task(
        &state,
        "terminal-cancelled",
        "running",
        Some(&cancelled_seed),
        now,
    );

    claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
        &state,
        "terminal-completed",
        "ckpt-terminal-completed",
        "executing_finalize",
        "verify_and_finalize",
        "checkpoint_finalize_executor_pending",
        "ready_to_verify_and_finalize",
        "finalize_completed",
        now + 1,
        10,
    )
    .expect("claim completed projection")
    .expect("completed projection claimed");
    let completed_claimed = stored_result_json(&state, "terminal-completed");
    let completed_claimed_lifecycle = crate::task_lifecycle::task_query_lifecycle_projection(
        "running",
        Some(&completed_claimed),
        None,
    );
    assert_eq!(
        completed_claimed_lifecycle["resume_executor_dispatch_result"]["projection_pending_reason"],
        "terminal_projection_pending"
    );
    assert_eq!(
        completed_claimed_lifecycle["resume_executor_result_projection_claim"]
            ["projection_pending_reason"],
        "terminal_projection_pending"
    );
    assert!(
        !record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
            &state,
            "terminal-completed",
            "ckpt-terminal-completed",
            "executing_finalize",
            "verify_and_finalize",
            "checkpoint_finalize_executor_pending",
            "ready_to_verify_and_finalize",
            "finalize_completed",
            &json!({
                "schema_version": 1,
                "task_id": "terminal-completed",
                "checkpoint_id": "ckpt-terminal-completed",
                "executor_state": "executing_finalize",
                "executor_action": "verify_and_finalize",
                "executor_status": "checkpoint_finalize_executor_pending",
                "dispatch_state": "ready_to_verify_and_finalize",
                "executor_result_status": "finalize_completed",
                "result_projection_state": "project_finalize_completed"
            }),
            now + 2,
        )
        .expect("record incomplete completed projection"),
        "completed terminal projection must carry final_result_json"
    );
    assert!(
        record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
            &state,
            "terminal-completed",
            "ckpt-terminal-completed",
            "executing_finalize",
            "verify_and_finalize",
            "checkpoint_finalize_executor_pending",
            "ready_to_verify_and_finalize",
            "finalize_completed",
            &json!({
                "schema_version": 1,
                "task_id": "terminal-completed",
                "checkpoint_id": "ckpt-terminal-completed",
                "executor_state": "executing_finalize",
                "executor_action": "verify_and_finalize",
                "executor_status": "checkpoint_finalize_executor_pending",
                "dispatch_state": "ready_to_verify_and_finalize",
                "executor_result_status": "finalize_completed",
                "result_projection_state": "project_finalize_completed",
                "final_result_json": {
                    "answer_text": "ok",
                    "answer_messages": []
                }
            }),
            now + 3,
        )
        .expect("record completed projection")
    );
    let (completed_status, completed_error, completed_result) =
        stored_task_status_error_result(&state, "terminal-completed");
    assert_eq!(completed_status, "succeeded");
    assert_eq!(completed_error, None);
    assert_eq!(completed_result["answer_text"], "ok");
    assert_eq!(completed_result["task_lifecycle"]["state"], "succeeded");
    assert_eq!(
        completed_result["task_lifecycle"]["resume_executor_result_projection"]
            ["projection_result_status"],
        "terminal_completed"
    );

    claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
        &state,
        "terminal-failed",
        "ckpt-terminal-failed",
        "executing_async_poll",
        "poll_async_job",
        "async_poll_adapter_pending",
        "ready_to_poll_async_job",
        "async_poll_failed",
        now + 4,
        10,
    )
    .expect("claim failed projection")
    .expect("failed projection claimed");
    assert!(
        record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
            &state,
            "terminal-failed",
            "ckpt-terminal-failed",
            "executing_async_poll",
            "poll_async_job",
            "async_poll_adapter_pending",
            "ready_to_poll_async_job",
            "async_poll_failed",
            &json!({
                "schema_version": 1,
                "task_id": "terminal-failed",
                "checkpoint_id": "ckpt-terminal-failed",
                "executor_state": "executing_async_poll",
                "executor_action": "poll_async_job",
                "executor_status": "async_poll_adapter_pending",
                "dispatch_state": "ready_to_poll_async_job",
                "executor_result_status": "async_poll_failed",
                "result_projection_state": "project_async_poll_failed",
                "error_code": "async_poll_expired",
                "message_key": "clawd.task.async_poll_expired"
            }),
            now + 5,
        )
        .expect("record failed projection")
    );
    let (failed_status, failed_error, failed_result) =
        stored_task_status_error_result(&state, "terminal-failed");
    assert_eq!(failed_status, "failed");
    assert_eq!(failed_error.as_deref(), Some("async_poll_expired"));
    assert_eq!(failed_result["status"], "error");
    assert_eq!(failed_result["error_code"], "async_poll_expired");
    assert_eq!(
        failed_result["message_key"],
        "clawd.task.async_poll_expired"
    );
    assert_eq!(failed_result["task_lifecycle"]["state"], "failed");
    assert_eq!(
        failed_result["task_lifecycle"]["resume_executor_result_projection"]
            ["projection_result_status"],
        "terminal_failed"
    );

    claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
        &state,
        "terminal-cancelled",
        "ckpt-terminal-cancelled",
        "executing_async_poll",
        "poll_async_job",
        "async_poll_adapter_pending",
        "ready_to_poll_async_job",
        "async_poll_cancelled",
        now + 6,
        10,
    )
    .expect("claim cancelled projection")
    .expect("cancelled projection claimed");
    assert!(
        record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
            &state,
            "terminal-cancelled",
            "ckpt-terminal-cancelled",
            "executing_async_poll",
            "poll_async_job",
            "async_poll_adapter_pending",
            "ready_to_poll_async_job",
            "async_poll_cancelled",
            &json!({
                "schema_version": 1,
                "task_id": "terminal-cancelled",
                "checkpoint_id": "ckpt-terminal-cancelled",
                "executor_state": "executing_async_poll",
                "executor_action": "poll_async_job",
                "executor_status": "async_poll_adapter_pending",
                "dispatch_state": "ready_to_poll_async_job",
                "executor_result_status": "async_poll_cancelled",
                "result_projection_state": "project_async_poll_cancelled",
                "message_key": "clawd.task.cancelled",
                "cancellation_result_json": {
                    "schema_version": 1,
                    "adapter_kind": "local_process_poll",
                    "status": "accepted",
                    "cancel_ref": "local_process:/tmp/cancelled-job"
                }
            }),
            now + 7,
        )
        .expect("record cancelled projection")
    );
    let (cancelled_status, cancelled_error, cancelled_result) =
        stored_task_status_error_result(&state, "terminal-cancelled");
    assert_eq!(cancelled_status, "canceled");
    assert_eq!(cancelled_error.as_deref(), Some("user_cancelled"));
    assert_eq!(cancelled_result["status"], "cancelled");
    assert_eq!(cancelled_result["message_key"], "clawd.task.cancelled");
    assert_eq!(cancelled_result["task_lifecycle"]["state"], "cancelled");
    assert_eq!(
        cancelled_result["task_lifecycle"]["resume_executor_result_projection"]
            ["projection_result_status"],
        "terminal_cancelled"
    );
}

#[test]
fn terminal_async_poll_projection_preserves_visible_ask_reply() {
    let state = state_with_tasks_table();
    let now = 9_500;
    let mut seed = terminal_projection_seed(
        "ask-visible-terminal",
        "ckpt-ask-visible-terminal",
        "executing_async_poll",
        "poll_async_job",
        "async_poll_adapter_pending",
        "ready_to_poll_async_job",
        "async_poll_completed",
        now,
    );
    seed["text"] = json!("checkpoint_id=ckpt-ask-visible-terminal");
    seed["messages"] = json!(["checkpoint_id=ckpt-ask-visible-terminal"]);
    insert_task(&state, "ask-visible-terminal", "running", Some(&seed), now);

    claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
        &state,
        "ask-visible-terminal",
        "ckpt-ask-visible-terminal",
        "executing_async_poll",
        "poll_async_job",
        "async_poll_adapter_pending",
        "ready_to_poll_async_job",
        "async_poll_completed",
        now + 1,
        10,
    )
    .expect("claim completed projection")
    .expect("completed projection claimed");
    assert!(
        record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
            &state,
            "ask-visible-terminal",
            "ckpt-ask-visible-terminal",
            "executing_async_poll",
            "poll_async_job",
            "async_poll_adapter_pending",
            "ready_to_poll_async_job",
            "async_poll_completed",
            &json!({
                "schema_version": 1,
                "task_id": "ask-visible-terminal",
                "checkpoint_id": "ckpt-ask-visible-terminal",
                "executor_state": "executing_async_poll",
                "executor_action": "poll_async_job",
                "executor_status": "async_poll_adapter_pending",
                "dispatch_state": "ready_to_poll_async_job",
                "executor_result_status": "async_poll_completed",
                "result_projection_state": "project_async_poll_completed",
                "final_result_json": {
                    "status": "ok",
                    "output": "RUSTCLAW_ASYNC_SMOKE"
                }
            }),
            now + 2,
        )
        .expect("record completed projection")
    );

    let (status, error_text, result) =
        stored_task_status_error_result(&state, "ask-visible-terminal");
    assert_eq!(status, "succeeded");
    assert_eq!(error_text, None);
    assert_eq!(
        result["messages"][0],
        "checkpoint_id=ckpt-ask-visible-terminal"
    );
    assert_eq!(result.get("output"), None);
    assert_eq!(result["task_lifecycle"]["state"], "succeeded");
    assert_eq!(
        result["task_lifecycle"]["resume_executor_result_projection"]["final_result_json"]
            ["output"],
        "RUSTCLAW_ASYNC_SMOKE"
    );
}

#[test]
fn terminal_agent_loop_async_poll_projection_replaces_waiting_visible_ask_reply() {
    let state = state_with_tasks_table();
    let now = 9_700;
    let checkpoint_id =
        "agent-loop:ask-machine-terminal:round-1:step-1:async-job:local_process:poll-1";
    let mut seed = terminal_projection_seed(
        "ask-machine-terminal",
        checkpoint_id,
        "executing_async_poll",
        "poll_async_job",
        "async_poll_adapter_pending",
        "ready_to_poll_async_job",
        "async_poll_completed",
        now,
    );
    seed["task_lifecycle"]["poll_ref"] = json!("local_process:poll-1");
    seed["task_lifecycle"]["next_check_after"] = json!(2);
    seed["task_lifecycle"]["async_job_message_key"] = json!("clawd.task.async_job_pending");
    seed["task_lifecycle"]["async_timeout_policy"] = json!({
        "schema_version": 1,
        "policy_source": "async_job_contract",
        "adapter_kind": "local_process_poll",
        "effective_deadline_ts": now + 600,
        "remaining_seconds": 600,
        "expired": false
    });
    seed["task_checkpoint"]["resume_entrypoint"] = json!("poll_async_job");
    seed["text"] = json!("checkpoint accepted but not terminal");
    seed["messages"] = json!(["checkpoint accepted but not terminal"]);
    insert_task(&state, "ask-machine-terminal", "running", Some(&seed), now);

    claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
        &state,
        "ask-machine-terminal",
        checkpoint_id,
        "executing_async_poll",
        "poll_async_job",
        "async_poll_adapter_pending",
        "ready_to_poll_async_job",
        "async_poll_completed",
        now + 1,
        10,
    )
    .expect("claim completed projection")
    .expect("completed projection claimed");
    assert!(
        record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
            &state,
            "ask-machine-terminal",
            checkpoint_id,
            "executing_async_poll",
            "poll_async_job",
            "async_poll_adapter_pending",
            "ready_to_poll_async_job",
            "async_poll_completed",
            &json!({
                "schema_version": 1,
                "task_id": "ask-machine-terminal",
                "checkpoint_id": checkpoint_id,
                "executor_state": "executing_async_poll",
                "executor_action": "poll_async_job",
                "executor_status": "async_poll_adapter_pending",
                "dispatch_state": "ready_to_poll_async_job",
                "executor_result_status": "async_poll_completed",
                "result_projection_state": "project_async_poll_completed",
                "final_result_json": {
                    "status": "ok",
                    "job_id": "local_process:poll-1",
                    "output": "RUSTCLAW_ASYNC_SMOKE"
                }
            }),
            now + 2,
        )
        .expect("record completed projection")
    );

    let (status, error_text, result) =
        stored_task_status_error_result(&state, "ask-machine-terminal");
    assert_eq!(status, "succeeded");
    assert_eq!(error_text, None);
    let reply = result["messages"][0].as_str().expect("machine reply");
    assert_ne!(reply, "checkpoint accepted but not terminal");
    assert!(reply.contains("checkpoint_id"));
    assert!(reply.contains("poll_ref"));
    assert!(reply.contains("next_check_after"));
    assert_eq!(result["machine_reply"]["checkpoint_id"], checkpoint_id);
    assert_eq!(result["machine_reply"]["poll_ref"], "local_process:poll-1");
    assert_eq!(result["machine_reply"]["next_check_after"], 2);
    assert_eq!(
        result["machine_reply"]["adapter_kind"],
        "local_process_poll"
    );
    assert_eq!(
        result["machine_reply"]["async_timeout_policy"]["adapter_kind"],
        "local_process_poll"
    );
    assert_eq!(
        result["machine_reply"]["final_result_json"]["output"],
        "RUSTCLAW_ASYNC_SMOKE"
    );
    assert_eq!(
        result["task_journal"]["trace"]["step_results"][0]["executed_skill"],
        "run_cmd"
    );
    assert_eq!(
        result["task_journal"]["summary"]["task_metrics"]["tool_calls"],
        1
    );
    assert_eq!(result["task_lifecycle"]["state"], "succeeded");
}
