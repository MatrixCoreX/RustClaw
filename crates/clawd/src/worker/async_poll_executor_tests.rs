use serde_json::json;

struct TempDirGuard {
    path: std::path::PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw_{prefix}_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn async_poll_claimed_dispatch(
    adapter_result: Option<serde_json::Value>,
) -> crate::repo::ClaimedDispatchedPausedCheckpointResumeExecution {
    let task = crate::ClaimedTask {
        task_id: "task-async-poll-adapter".to_string(),
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
        checkpoint_id: "ckpt-async-poll-adapter".to_string(),
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
            job_id: "job-async-poll-adapter".to_string(),
            status: crate::task_lifecycle::AsyncJobStatus::Running,
            poll_after_seconds: 7,
            expires_at: 2_000,
            cancel_ref: "cancel:job-async-poll-adapter".to_string(),
            message_key: "tool.msg.job.running".to_string(),
        }),
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob,
    };
    let mut dispatch_payload = json!({
        "dispatch_state": "ready_to_poll_async_job",
        "checkpoint_id": "ckpt-async-poll-adapter",
        "job_id": "job-async-poll-adapter",
        "expires_at": 2_000,
    });
    if let Some(adapter_result) = adapter_result {
        dispatch_payload["async_poll_adapter_result"] = adapter_result;
    }
    crate::repo::ClaimedDispatchedPausedCheckpointResumeExecution {
        task,
        task_id: "task-async-poll-adapter".to_string(),
        checkpoint_id: "ckpt-async-poll-adapter".to_string(),
        executor_state: "executing_async_poll".to_string(),
        executor_action: "poll_async_job".to_string(),
        executor_status: "async_poll_adapter_pending".to_string(),
        dispatch_state: "ready_to_poll_async_job".to_string(),
        dispatch_execution_state: "claimed_to_poll_async_job".to_string(),
        resume_trigger: "worker_recovery".to_string(),
        resume_directive: "poll_async_job".to_string(),
        lease_expires_at: 1_500,
        handoff_claim_expires_at: 1_400,
        dispatch_claim_expires_at: 1_300,
        execution_plan: json!({
            "executor_action": "poll_async_job",
            "checkpoint_id": "ckpt-async-poll-adapter",
            "job_id": "job-async-poll-adapter",
            "poll_after_seconds": 7,
            "expires_at": 2_000,
            "cancel_ref": "cancel:job-async-poll-adapter",
            "message_key": "tool.msg.job.running",
        }),
        dispatch_payload,
        dispatch_claim: json!({
            "dispatch_execution_state": "claimed_to_poll_async_job",
            "checkpoint_id": "ckpt-async-poll-adapter"
        }),
        task_checkpoint: checkpoint,
    }
}

#[test]
fn async_poll_local_process_job_dir_becomes_terminal_result() {
    let dir = TempDirGuard::new("async_poll_local_process");
    std::fs::write(dir.path.join("exit_code"), "0\n").expect("write exit code");
    std::fs::write(dir.path.join("stdout"), "async-ok\n").expect("write stdout");
    std::fs::write(dir.path.join("stderr"), "").expect("write stderr");

    let mut claimed = async_poll_claimed_dispatch(None);
    let job_id = "local_process:test-job";
    claimed.execution_plan["job_id"] = json!(job_id);
    claimed.dispatch_payload["job_id"] = json!(job_id);
    if let Some(job) = claimed.task_checkpoint.pending_async_job.as_mut() {
        job.job_id = job_id.to_string();
        job.cancel_ref = format!("local_process:{}", dir.path.display());
    }

    let payload = super::execute_async_poll_dispatch_result(&claimed, 1_000, 30)
        .expect("local process poll completed payload");

    assert_eq!(payload["executor_result_status"], "async_poll_completed");
    assert_eq!(payload["adapter_status"], "succeeded");
    assert_eq!(
        payload["final_result_json"]["source"],
        "local_process_async_job"
    );
    assert_eq!(payload["final_result_json"]["exit_code"], 0);
    assert_eq!(payload["final_result_json"]["stdout"], "async-ok\n");
    assert!(payload.get("text").is_none());
    assert!(payload.get("error_text").is_none());
}

#[test]
fn async_poll_local_process_cancel_marker_becomes_cancelled_result() {
    let dir = TempDirGuard::new("async_poll_local_process_cancelled");
    std::fs::write(dir.path.join("cancel_requested_at"), "1000\n").expect("write cancel marker");

    let mut claimed = async_poll_claimed_dispatch(None);
    let job_id = "local_process:cancelled-job";
    claimed.execution_plan["job_id"] = json!(job_id);
    claimed.dispatch_payload["job_id"] = json!(job_id);
    if let Some(job) = claimed.task_checkpoint.pending_async_job.as_mut() {
        job.job_id = job_id.to_string();
        job.cancel_ref = format!("local_process:{}", dir.path.display());
    }

    let payload = super::execute_async_poll_dispatch_result(&claimed, 1_000, 30)
        .expect("local process poll cancelled payload");

    assert_eq!(payload["executor_result_status"], "async_poll_cancelled");
    assert_eq!(payload["adapter_status"], "cancelled");
    assert_eq!(payload["reason_code"], "async_poll_cancelled");
    assert_eq!(payload["message_key"], "clawd.task.cancelled");
    assert_eq!(
        payload["cancellation_result_json"]["source"],
        "local_process_async_job"
    );
    assert!(payload.get("text").is_none());
    assert!(payload.get("error_text").is_none());
}

#[test]
fn async_poll_adapter_success_becomes_machine_terminal_result() {
    let claimed = async_poll_claimed_dispatch(Some(json!({
        "job_id": "job-async-poll-adapter",
        "status": "succeeded",
        "final_result_json": {
            "status": "ok",
            "result_ref": "artifact:job-async-poll-adapter"
        }
    })));

    let payload = super::execute_async_poll_dispatch_result(&claimed, 1_000, 30)
        .expect("async poll completed payload");
    assert_eq!(payload["executor_result_status"], "async_poll_completed");
    assert_eq!(payload["reason_code"], "async_poll_completed");
    assert_eq!(payload["adapter_status"], "succeeded");
    assert_eq!(
        payload["final_result_json"]["result_ref"],
        "artifact:job-async-poll-adapter"
    );
    assert!(payload.get("text").is_none());
    assert!(payload.get("error_text").is_none());
}

#[test]
fn async_poll_adapter_running_becomes_machine_reschedule() {
    let claimed = async_poll_claimed_dispatch(Some(json!({
        "job_id": "job-async-poll-adapter",
        "status": "running",
        "poll_after_seconds": 13,
        "expires_at": 1_050
    })));

    let payload = super::execute_async_poll_dispatch_result(&claimed, 1_000, 30)
        .expect("async poll rescheduled payload");
    assert_eq!(payload["executor_result_status"], "async_poll_rescheduled");
    assert_eq!(payload["reason_code"], "async_poll_running");
    assert_eq!(payload["defer_reason_code"], "async_poll_running");
    assert_eq!(payload["next_check_after"], 1_013);
    assert_eq!(payload["expires_at"], 1_050);
    assert!(payload.get("text").is_none());
    assert!(payload.get("error_text").is_none());
}

#[test]
fn async_poll_adapter_accepted_becomes_machine_reschedule() {
    let claimed = async_poll_claimed_dispatch(Some(json!({
        "job_id": "job-async-poll-adapter",
        "status": "accepted",
        "poll_after_seconds": 11,
        "expires_at": 1_080
    })));

    let payload = super::execute_async_poll_dispatch_result(&claimed, 1_000, 30)
        .expect("async poll accepted payload");
    assert_eq!(payload["executor_result_status"], "async_poll_rescheduled");
    assert_eq!(payload["reason_code"], "async_poll_accepted");
    assert_eq!(payload["defer_reason_code"], "async_poll_accepted");
    assert_eq!(payload["retry_after_seconds"], 11);
    assert_eq!(payload["next_check_after"], 1_011);
    assert_eq!(payload["expires_at"], 1_080);
    assert_eq!(payload["message_key"], "tool.msg.job.running");
    assert!(payload.get("text").is_none());
    assert!(payload.get("error_text").is_none());
}

#[test]
fn async_poll_adapter_running_after_expiry_becomes_machine_failure() {
    let claimed = async_poll_claimed_dispatch(Some(json!({
        "job_id": "job-async-poll-adapter",
        "status": "running",
        "poll_after_seconds": 13,
        "expires_at": 999
    })));

    let payload = super::execute_async_poll_dispatch_result(&claimed, 1_000, 30)
        .expect("async poll expired payload");
    assert_eq!(payload["executor_result_status"], "async_poll_failed");
    assert_eq!(payload["error_code"], "async_poll_expired");
    assert_eq!(payload["message_key"], "clawd.task.async_poll_expired");
    assert_eq!(payload["adapter_status"], "running");
    assert!(payload.get("text").is_none());
    assert!(payload.get("error_text").is_none());
}

#[test]
fn async_poll_adapter_failure_keeps_machine_error_contract() {
    let claimed = async_poll_claimed_dispatch(Some(json!({
        "job_id": "job-async-poll-adapter",
        "status": "failed",
        "error_code": "provider_job_failed",
        "message_key": "provider.job.failed",
        "failure_result_json": {
            "status": "error",
            "error_code": "provider_job_failed"
        }
    })));

    let payload = super::execute_async_poll_dispatch_result(&claimed, 1_000, 30)
        .expect("async poll failure payload");
    assert_eq!(payload["executor_result_status"], "async_poll_failed");
    assert_eq!(payload["error_code"], "provider_job_failed");
    assert_eq!(payload["message_key"], "provider.job.failed");
    assert_eq!(payload["failure_result_json"]["status"], "error");
    assert!(payload.get("text").is_none());
    assert!(payload.get("error_text").is_none());
}

#[tokio::test]
async fn skill_poll_adapter_accepts_registry_async_adapter_kinds() {
    let state = crate::AppState::test_default_with_fixture_provider();
    for adapter_kind in [
        "skill_poll",
        "http_job_poll",
        "mcp_job_poll",
        "media_job_poll",
        "browser_job_poll",
        "remote_job_poll",
    ] {
        let mut claimed = async_poll_claimed_dispatch(None);
        claimed.task_checkpoint.boundary_context["async_poll_adapter"] = json!({
            "kind": adapter_kind
        });

        let result =
            super::skill_poll_async_adapter_result(&state, &claimed, "job-async-poll-adapter")
                .await
                .expect("adapter result");

        assert_eq!(
            result["error_code"], "skill_poll_adapter_missing_skill_name",
            "adapter_kind={adapter_kind} should pass kind validation"
        );
        assert!(result.get("text").is_none());
        assert!(result.get("error_text").is_none());
    }
}

#[tokio::test]
async fn skill_poll_adapter_rejects_unknown_adapter_kind() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut claimed = async_poll_claimed_dispatch(None);
    claimed.task_checkpoint.boundary_context["async_poll_adapter"] = json!({
        "kind": "unknown_job_poll"
    });

    let result = super::skill_poll_async_adapter_result(&state, &claimed, "job-async-poll-adapter")
        .await
        .expect("adapter result");

    assert_eq!(result["error_code"], "skill_poll_adapter_kind_unsupported");
    assert!(result.get("text").is_none());
    assert!(result.get("error_text").is_none());
}

#[test]
fn async_poll_adapter_result_rejects_text_leak_and_job_mismatch() {
    let text_leak = async_poll_claimed_dispatch(Some(json!({
        "job_id": "job-async-poll-adapter",
        "status": "succeeded",
        "text": "leak",
        "final_result_json": {"status": "ok"}
    })));
    assert!(super::execute_async_poll_dispatch_result(&text_leak, 1_000, 30).is_none());

    let mismatch = async_poll_claimed_dispatch(Some(json!({
        "job_id": "other-job",
        "status": "succeeded",
        "final_result_json": {"status": "ok"}
    })));
    assert!(super::execute_async_poll_dispatch_result(&mismatch, 1_000, 30).is_none());
}
