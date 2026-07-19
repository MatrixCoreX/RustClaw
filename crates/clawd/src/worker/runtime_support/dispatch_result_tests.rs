use serde_json::json;

fn seeded_claimed_dispatch() -> crate::repo::ClaimedDispatchedPausedCheckpointResumeExecution {
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-seeded-terminal".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("test-key".to_string()),
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({"text": "continue"}).to_string(),
    };
    let checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "ckpt-seeded-terminal".to_string(),
        boundary_context: json!({"route_gate_kind": "execute"}),
        last_successful_round: Some(1),
        last_successful_step: Some("step_1".to_string()),
        pending_action: None,
        observations: Vec::new(),
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: vec!["write_file:tmp/report.txt".to_string()],
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 1,
            step: 1,
            llm_calls: 2,
            tool_calls: 1,
            elapsed_ms: 100,
            llm_elapsed_ms: 100,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
    };
    crate::repo::ClaimedDispatchedPausedCheckpointResumeExecution {
        task,
        task_id: "task-seeded-terminal".to_string(),
        checkpoint_id: "ckpt-seeded-terminal".to_string(),
        executor_state: "executing_planner_resume".to_string(),
        executor_action: "run_seeded_agent_loop".to_string(),
        executor_status: "seeded_loop_requires_provider_window".to_string(),
        dispatch_state: "ready_to_run_seeded_agent_loop".to_string(),
        dispatch_execution_state: "claimed_to_run_seeded_agent_loop".to_string(),
        resume_trigger: "worker_recovery".to_string(),
        resume_directive: "run_next_planner_round".to_string(),
        lease_expires_at: 100,
        handoff_claim_expires_at: 90,
        dispatch_claim_expires_at: 80,
        execution_plan: json!({
            "executor_action": "run_seeded_agent_loop",
            "checkpoint_id": "ckpt-seeded-terminal"
        }),
        dispatch_payload: json!({
            "dispatch_state": "ready_to_run_seeded_agent_loop",
            "checkpoint_id": "ckpt-seeded-terminal"
        }),
        dispatch_claim: json!({
            "dispatch_execution_state": "claimed_to_run_seeded_agent_loop",
            "checkpoint_id": "ckpt-seeded-terminal"
        }),
        task_checkpoint: checkpoint,
    }
}

fn async_poll_claimed_dispatch(
    expires_at: Option<i64>,
) -> crate::repo::ClaimedDispatchedPausedCheckpointResumeExecution {
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-async-poll".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("test-key".to_string()),
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({"text": "continue"}).to_string(),
    };
    let checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "ckpt-async-poll".to_string(),
        boundary_context: json!({"route_gate_kind": "execute"}),
        last_successful_round: Some(1),
        last_successful_step: Some("step_1".to_string()),
        pending_action: None,
        observations: Vec::new(),
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: Vec::new(),
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 1,
            step: 1,
            llm_calls: 2,
            tool_calls: 1,
            elapsed_ms: 100,
            llm_elapsed_ms: 100,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: Some(crate::task_lifecycle::AsyncJobRef {
            job_id: "job-async-poll".to_string(),
            status: crate::task_lifecycle::AsyncJobStatus::Running,
            poll_after_seconds: 7,
            expires_at: expires_at.unwrap_or(2_000),
            cancel_ref: "cancel:job-async-poll".to_string(),
            message_key: "tool.msg.job.running".to_string(),
        }),
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob,
    };
    let mut execution_plan = json!({
        "executor_action": "poll_async_job",
        "checkpoint_id": "ckpt-async-poll",
        "job_id": "job-async-poll",
        "poll_after_seconds": 7,
        "cancel_ref": "cancel:job-async-poll",
        "message_key": "tool.msg.job.running"
    });
    if let Some(expires_at) = expires_at {
        execution_plan["expires_at"] = json!(expires_at);
    }
    crate::repo::ClaimedDispatchedPausedCheckpointResumeExecution {
        task,
        task_id: "task-async-poll".to_string(),
        checkpoint_id: "ckpt-async-poll".to_string(),
        executor_state: "executing_async_poll".to_string(),
        executor_action: "poll_async_job".to_string(),
        executor_status: "async_poll_adapter_pending".to_string(),
        dispatch_state: "ready_to_poll_async_job".to_string(),
        dispatch_execution_state: "claimed_to_poll_async_job".to_string(),
        resume_trigger: "worker_recovery".to_string(),
        resume_directive: "poll_async_job".to_string(),
        lease_expires_at: 100,
        handoff_claim_expires_at: 90,
        dispatch_claim_expires_at: 80,
        execution_plan,
        dispatch_payload: json!({
            "dispatch_state": "ready_to_poll_async_job",
            "checkpoint_id": "ckpt-async-poll",
            "job_id": "job-async-poll",
        }),
        dispatch_claim: json!({
            "dispatch_execution_state": "claimed_to_poll_async_job",
            "checkpoint_id": "ckpt-async-poll"
        }),
        task_checkpoint: checkpoint,
    }
}

fn finalize_claimed_dispatch(
    final_result_json: Option<serde_json::Value>,
) -> crate::repo::ClaimedDispatchedPausedCheckpointResumeExecution {
    let mut claimed = seeded_claimed_dispatch();
    claimed.executor_state = "executing_finalize".to_string();
    claimed.executor_action = "verify_and_finalize".to_string();
    claimed.executor_status = "checkpoint_finalize_executor_pending".to_string();
    claimed.dispatch_state = "ready_to_verify_and_finalize".to_string();
    claimed.dispatch_execution_state = "claimed_to_verify_and_finalize".to_string();
    claimed.resume_directive = "verify_and_finalize".to_string();
    claimed.task_checkpoint.resume_entrypoint =
        crate::task_lifecycle::ResumeEntrypoint::VerifyAndFinalize;
    claimed.execution_plan = json!({
        "executor_action": "verify_and_finalize",
        "checkpoint_id": "ckpt-seeded-terminal"
    });
    if let Some(final_result_json) = final_result_json {
        claimed.execution_plan["final_result_json"] = final_result_json;
    }
    claimed.dispatch_payload = json!({
        "dispatch_state": "ready_to_verify_and_finalize",
        "checkpoint_id": "ckpt-seeded-terminal"
    });
    claimed.dispatch_claim = json!({
        "dispatch_execution_state": "claimed_to_verify_and_finalize",
        "checkpoint_id": "ckpt-seeded-terminal"
    });
    claimed
}

#[test]
fn seeded_agent_loop_terminal_payload_is_machine_only_for_success_and_failure() {
    let claimed = seeded_claimed_dispatch();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-seeded-terminal", "ask", "continue");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    let success = crate::AskReply::non_llm("ok".to_string())
        .with_messages(vec!["observed_step".to_string()])
        .with_task_journal(journal);

    let payload = super::dispatch_result::seeded_agent_loop_terminal_dispatch_result_payload(
        &claimed,
        Ok(success),
    )
    .expect("seeded loop success payload");
    assert_eq!(payload["executor_result_status"], "seeded_loop_completed");
    assert!(payload.get("text").is_none());
    assert!(payload.get("error_text").is_none());
    assert_eq!(payload["final_result_json"]["text"], "ok");
    assert_eq!(payload["final_result_json"]["messages"][0], "observed_step");
    assert!(
        payload["final_result_json"].get("task_journal").is_some(),
        "success payload should preserve journal evidence for projection"
    );

    let failed = super::dispatch_result::seeded_agent_loop_terminal_dispatch_result_payload(
        &claimed,
        Err("provider raw failure must not be copied".to_string()),
    )
    .expect("seeded loop failure payload");
    assert_eq!(failed["executor_result_status"], "seeded_loop_failed");
    assert_eq!(failed["error_code"], "seeded_loop_runtime_error");
    assert_eq!(
        failed["message_key"],
        "clawd.task.seeded_loop_runtime_error"
    );
    assert!(failed.get("text").is_none());
    assert!(failed.get("error_text").is_none());
    assert!(failed.get("raw_error").is_none());
}

#[test]
fn seeded_agent_loop_with_new_checkpoint_is_deferred_not_terminal() {
    let claimed = seeded_claimed_dispatch();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-seeded-terminal", "ask", "continue");
    journal.record_task_lifecycle(json!({
        "schema_version": 1,
        "state": "background",
        "checkpoint_id": "ckpt-seeded-next",
        "resume_reason": "provider_window",
        "next_check_after": 9_999_999_999_i64
    }));
    let mut next_checkpoint = claimed.task_checkpoint.clone();
    next_checkpoint.checkpoint_id = "ckpt-seeded-next".to_string();
    journal.record_task_checkpoint(next_checkpoint.to_machine_json());
    let reply = crate::AskReply::non_llm("waiting".to_string()).with_task_journal(journal);

    let payload = super::dispatch_result::seeded_agent_loop_terminal_dispatch_result_payload(
        &claimed,
        Ok(reply),
    )
    .expect("seeded loop deferred payload");
    assert_eq!(payload["executor_result_status"], "seeded_loop_deferred");
    assert_eq!(payload["deferred_checkpoint_id"], "ckpt-seeded-next");
    assert_eq!(payload["deferred_lifecycle_state"], "background");
    assert_eq!(
        payload["final_result_json"]["task_journal"]["summary"]["task_checkpoint"]["checkpoint_id"],
        "ckpt-seeded-next"
    );
    assert!(payload.get("text").is_none());
    assert!(payload.get("error_text").is_none());
}

#[test]
fn seeded_agent_loop_terminal_payload_rejects_text_leaks_in_claim_chain() {
    let mut claimed = seeded_claimed_dispatch();
    claimed.dispatch_claim = json!({"text": "leak"});
    assert!(
        super::dispatch_result::seeded_agent_loop_terminal_dispatch_result_payload(
            &claimed,
            Ok(crate::AskReply::non_llm("ok".to_string())),
        )
        .is_none()
    );
}

#[test]
fn async_poll_dispatch_result_reschedules_before_expiry_and_fails_after_expiry() {
    let active = async_poll_claimed_dispatch(Some(1_050));
    let rescheduled = super::dispatch_result::paused_checkpoint_resume_dispatch_result_payload(
        &active, 1_000, 90,
    )
    .expect("active async poll payload");
    assert_eq!(
        rescheduled["executor_result_status"],
        "async_poll_rescheduled"
    );
    assert_eq!(rescheduled["next_check_after"], 1_050);
    assert_eq!(
        rescheduled["defer_reason_code"],
        "async_poll_adapter_pending"
    );
    assert!(rescheduled.get("text").is_none());
    assert!(rescheduled.get("error_text").is_none());

    let expired = async_poll_claimed_dispatch(Some(999));
    let failed = super::dispatch_result::paused_checkpoint_resume_dispatch_result_payload(
        &expired, 1_000, 90,
    )
    .expect("expired async poll payload");
    assert_eq!(failed["executor_result_status"], "async_poll_failed");
    assert_eq!(failed["error_code"], "async_poll_expired");
    assert_eq!(failed["message_key"], "clawd.task.async_poll_expired");
    assert!(failed.get("next_check_after").is_none());
    assert!(failed.get("text").is_none());
    assert!(failed.get("error_text").is_none());
}

#[test]
fn async_poll_dispatch_result_fails_missing_expiry_as_machine_contract_gap() {
    let invalid = async_poll_claimed_dispatch(None);
    let failed = super::dispatch_result::paused_checkpoint_resume_dispatch_result_payload(
        &invalid, 1_000, 90,
    )
    .expect("invalid async poll payload");
    assert_eq!(failed["executor_result_status"], "async_poll_failed");
    assert_eq!(failed["error_code"], "async_poll_invalid_contract");
    assert_eq!(
        failed["message_key"],
        "clawd.task.async_poll_invalid_contract"
    );
    assert!(failed.get("text").is_none());
    assert!(failed.get("error_text").is_none());
}

#[test]
fn checkpoint_finalize_dispatch_result_uses_precomputed_final_result_or_machine_failure() {
    let completed = finalize_claimed_dispatch(Some(json!({
        "text": "precomputed final",
        "task_journal": {
            "summary": {
                "final_status": "success"
            }
        }
    })));
    let completed_payload =
        super::dispatch_result::paused_checkpoint_resume_dispatch_result_payload(
            &completed, 1_000, 90,
        )
        .expect("finalize completed payload");
    assert_eq!(
        completed_payload["executor_result_status"],
        "finalize_completed"
    );
    assert_eq!(
        completed_payload["reason_code"],
        "checkpoint_finalize_completed"
    );
    assert_eq!(
        completed_payload["final_result_json"]["text"],
        "precomputed final"
    );
    assert!(completed_payload.get("text").is_none());
    assert!(completed_payload.get("error_text").is_none());

    let missing = finalize_claimed_dispatch(None);
    let failed = super::dispatch_result::paused_checkpoint_resume_dispatch_result_payload(
        &missing, 1_000, 90,
    )
    .expect("finalize missing result payload");
    assert_eq!(failed["executor_result_status"], "finalize_failed");
    assert_eq!(
        failed["error_code"],
        "checkpoint_finalize_missing_final_result"
    );
    assert_eq!(
        failed["message_key"],
        "clawd.task.checkpoint_finalize_missing_final_result"
    );
    assert!(failed.get("final_result_json").is_none());
    assert!(failed.get("text").is_none());
    assert!(failed.get("error_text").is_none());
}

#[test]
fn checkpoint_finalize_dispatch_result_projects_structured_observation_answer() {
    let mut from_journal = finalize_claimed_dispatch(None);
    from_journal.task_checkpoint.observations.push(json!({
        "source": "finalizer",
        "task_journal": {
            "summary": {
                "final_status": "success",
                "final_answer": "journal-backed final"
            }
        }
    }));
    let journal_payload = super::dispatch_result::paused_checkpoint_resume_dispatch_result_payload(
        &from_journal,
        1_000,
        90,
    )
    .expect("journal final answer payload");
    assert_eq!(
        journal_payload["executor_result_status"],
        "finalize_completed"
    );
    assert_eq!(
        journal_payload["final_result_json"]["text"],
        "journal-backed final"
    );
    assert!(
        journal_payload["final_result_json"]
            .get("task_journal")
            .is_some(),
        "task journal evidence should be preserved"
    );
    assert!(journal_payload.get("text").is_none());
    assert!(journal_payload.get("error_text").is_none());

    let mut from_answer = finalize_claimed_dispatch(None);
    from_answer.task_checkpoint.observations.push(json!({
        "source": "finalizer",
        "answer": {
            "text": "answer-object final",
            "messages": ["answer-object final"]
        }
    }));
    let answer_payload = super::dispatch_result::paused_checkpoint_resume_dispatch_result_payload(
        &from_answer,
        1_000,
        90,
    )
    .expect("answer object payload");
    assert_eq!(
        answer_payload["executor_result_status"],
        "finalize_completed"
    );
    assert_eq!(
        answer_payload["final_result_json"]["text"],
        "answer-object final"
    );
    assert_eq!(
        answer_payload["final_result_json"]["messages"][0],
        "answer-object final"
    );
    assert!(answer_payload.get("text").is_none());
    assert!(answer_payload.get("error_text").is_none());
}
