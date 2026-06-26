use serde_json::{json, Value};

use crate::repo;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PausedCheckpointResumeWorkItem {
    pub(crate) schema_version: u8,
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) lifecycle_state: String,
    pub(crate) executor_state: &'static str,
    pub(crate) resume_entrypoint: String,
    pub(crate) resume_trigger: &'static str,
    pub(crate) resume_directive: String,
    pub(crate) resume_directive_payload: Value,
    pub(crate) lease_seconds: i64,
    pub(crate) completed_side_effect_count: usize,
    pub(crate) requires_idempotency_guard: bool,
    pub(crate) seed_report: crate::agent_engine::LoopStateCheckpointSeedReport,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PausedCheckpointResumeExecutionDecision {
    pub(crate) executor_state: &'static str,
    pub(crate) lifecycle_state: Option<&'static str>,
    pub(crate) next_check_after: Option<i64>,
    pub(crate) payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaimedPausedCheckpointResumeExecutionPlan {
    pub(crate) task: crate::ClaimedTask,
    pub(crate) executor_action: &'static str,
    pub(crate) executor_state: String,
    pub(crate) resume_directive: String,
    pub(crate) checkpoint_id: String,
    pub(crate) payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedPausedCheckpointResumeExecutorHandoff {
    pub(crate) executor_action: String,
    pub(crate) executor_status: &'static str,
    pub(crate) checkpoint_id: String,
    pub(crate) executor_state: String,
    pub(crate) payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaimedPausedCheckpointResumeHandoffDispatch {
    pub(crate) task: crate::ClaimedTask,
    pub(crate) executor_action: String,
    pub(crate) executor_status: String,
    pub(crate) dispatch_state: &'static str,
    pub(crate) checkpoint_id: String,
    pub(crate) executor_state: String,
    pub(crate) payload: Value,
}

impl PausedCheckpointResumeWorkItem {
    pub(crate) fn to_machine_json(&self) -> Value {
        json!({
            "schema_version": self.schema_version,
            "task_id": self.task_id,
            "checkpoint_id": self.checkpoint_id,
            "lifecycle_state": self.lifecycle_state,
            "executor_state": self.executor_state,
            "resume_entrypoint": self.resume_entrypoint,
            "resume_trigger": self.resume_trigger,
            "resume_directive": self.resume_directive,
            "resume_directive_payload": self.resume_directive_payload,
            "lease_seconds": self.lease_seconds,
            "completed_side_effect_count": self.completed_side_effect_count,
            "requires_idempotency_guard": self.requires_idempotency_guard,
            "seed_report": {
                "checkpoint_id": self.seed_report.checkpoint_id,
                "resume_entrypoint": self.resume_entrypoint,
                "restored_round": self.seed_report.restored_round,
                "restored_step": self.seed_report.restored_step,
                "restored_tool_calls": self.seed_report.restored_tool_calls,
                "completed_side_effect_count": self.seed_report.completed_side_effect_count,
                "observation_count": self.seed_report.observation_count,
            }
        })
    }
}

pub(crate) fn checkpoint_resume_entrypoint_token(
    entrypoint: &crate::task_lifecycle::ResumeEntrypoint,
) -> &'static str {
    match entrypoint {
        crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound => "next_planner_round",
        crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob => "poll_async_job",
        crate::task_lifecycle::ResumeEntrypoint::AwaitUserInput => "await_user_input",
        crate::task_lifecycle::ResumeEntrypoint::VerifyAndFinalize => "verify_and_finalize",
    }
}

pub(crate) fn plan_claimed_paused_checkpoint_resume_execution(
    claimed: &repo::ClaimedPausedCheckpointResumeExecutor,
) -> Option<ClaimedPausedCheckpointResumeExecutionPlan> {
    if claimed.task_checkpoint.checkpoint_id != claimed.checkpoint_id {
        return None;
    }
    let executor_action = match (
        claimed.executor_state.as_str(),
        claimed.resume_directive.as_str(),
    ) {
        ("executing_planner_resume", "run_next_planner_round") => "run_seeded_agent_loop",
        ("executing_async_poll", "poll_async_job") => "poll_async_job",
        ("executing_finalize", "verify_and_finalize") => "verify_and_finalize",
        _ => return None,
    };
    let completed_side_effect_count = claimed.task_checkpoint.completed_side_effect_refs.len();
    let requires_idempotency_guard = claimed
        .resume_executor
        .get("requires_idempotency_guard")
        .and_then(Value::as_bool)
        .unwrap_or(completed_side_effect_count > 0);
    let mut payload = json!({
        "schema_version": 1,
        "task_id": claimed.task_id,
        "checkpoint_id": claimed.checkpoint_id,
        "executor_action": executor_action,
        "executor_state": claimed.executor_state,
        "previous_executor_state": claimed.previous_executor_state,
        "resume_directive": claimed.resume_directive,
        "resume_trigger": claimed.resume_trigger,
        "lease_expires_at": claimed.lease_expires_at,
        "task_kind": claimed.task.kind,
        "task_channel": claimed.task.channel,
        "task_payload_bytes": claimed.task.payload_json.len(),
        "resume_entrypoint": checkpoint_resume_entrypoint_token(&claimed.task_checkpoint.resume_entrypoint),
        "completed_side_effect_count": completed_side_effect_count,
        "requires_idempotency_guard": requires_idempotency_guard,
    });

    if executor_action == "poll_async_job" {
        let job_id = claimed
            .resume_executor
            .get("job_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("job_id".to_string(), json!(job_id));
            for key in ["cancel_ref", "message_key"] {
                if let Some(value) = claimed.resume_executor.get(key).and_then(Value::as_str) {
                    obj.insert(key.to_string(), json!(value));
                }
            }
            for key in ["poll_after_seconds", "expires_at"] {
                if let Some(value) = claimed.resume_executor.get(key).and_then(Value::as_i64) {
                    obj.insert(key.to_string(), json!(value));
                }
            }
        }
    }

    Some(ClaimedPausedCheckpointResumeExecutionPlan {
        task: claimed.task.clone(),
        executor_action,
        executor_state: claimed.executor_state.clone(),
        resume_directive: claimed.resume_directive.clone(),
        checkpoint_id: claimed.checkpoint_id.clone(),
        payload,
    })
}

pub(crate) fn prepare_paused_checkpoint_resume_execution(
    work_item: &PausedCheckpointResumeWorkItem,
    directive: &crate::task_lifecycle::CheckpointResumeDirective,
    now_ts: i64,
) -> PausedCheckpointResumeExecutionDecision {
    match directive {
        crate::task_lifecycle::CheckpointResumeDirective::RunNextPlannerRound {
            completed_side_effect_count,
            requires_idempotency_guard,
            ..
        } => PausedCheckpointResumeExecutionDecision {
            executor_state: "ready_for_planner_resume",
            lifecycle_state: Some("background"),
            next_check_after: Some(now_ts),
            payload: json!({
                "checkpoint_id": work_item.checkpoint_id,
                "resume_directive": directive.status_code(),
                "resume_entrypoint": work_item.resume_entrypoint,
                "resume_trigger": work_item.resume_trigger,
                "completed_side_effect_count": completed_side_effect_count,
                "requires_idempotency_guard": requires_idempotency_guard,
                "seed_checkpoint_id": work_item.seed_report.checkpoint_id,
            }),
        },
        crate::task_lifecycle::CheckpointResumeDirective::PollAsyncJob {
            job_id,
            adapter_kind,
            poll_after_seconds,
            expires_at,
            cancel_ref,
            message_key,
            ..
        } => {
            let poll_after_seconds_i64 = (*poll_after_seconds).min(i64::MAX as u64) as i64;
            PausedCheckpointResumeExecutionDecision {
                executor_state: "poll_scheduled",
                lifecycle_state: Some("background"),
                next_check_after: Some(now_ts.saturating_add(poll_after_seconds_i64)),
                payload: json!({
                    "checkpoint_id": work_item.checkpoint_id,
                    "resume_directive": directive.status_code(),
                    "resume_trigger": work_item.resume_trigger,
                    "job_id": job_id,
                    "adapter_kind": adapter_kind,
                    "poll_after_seconds": poll_after_seconds,
                    "expires_at": expires_at,
                    "cancel_ref": cancel_ref,
                    "message_key": message_key,
                }),
            }
        }
        crate::task_lifecycle::CheckpointResumeDirective::AwaitUserInput { .. } => {
            PausedCheckpointResumeExecutionDecision {
                executor_state: "awaiting_user",
                lifecycle_state: Some("needs_user"),
                next_check_after: None,
                payload: json!({
                    "checkpoint_id": work_item.checkpoint_id,
                    "resume_directive": directive.status_code(),
                    "resume_trigger": work_item.resume_trigger,
                    "awaiting": "user_input",
                }),
            }
        }
        crate::task_lifecycle::CheckpointResumeDirective::VerifyAndFinalize {
            completed_side_effect_count,
            ..
        } => PausedCheckpointResumeExecutionDecision {
            executor_state: "ready_to_finalize",
            lifecycle_state: Some("background"),
            next_check_after: Some(now_ts),
            payload: json!({
                "checkpoint_id": work_item.checkpoint_id,
                "resume_directive": directive.status_code(),
                "resume_trigger": work_item.resume_trigger,
                "completed_side_effect_count": completed_side_effect_count,
            }),
        },
        crate::task_lifecycle::CheckpointResumeDirective::WaitForActiveLease {
            lease_expires_at,
            resume_wait_seconds,
            ..
        } => PausedCheckpointResumeExecutionDecision {
            executor_state: "waiting_for_active_lease",
            lifecycle_state: Some("background"),
            next_check_after: Some(*lease_expires_at),
            payload: json!({
                "checkpoint_id": work_item.checkpoint_id,
                "resume_directive": directive.status_code(),
                "resume_trigger": work_item.resume_trigger,
                "lease_expires_at": lease_expires_at,
                "resume_wait_seconds": resume_wait_seconds,
            }),
        },
        crate::task_lifecycle::CheckpointResumeDirective::NotReady { status_code } => {
            PausedCheckpointResumeExecutionDecision {
                executor_state: "not_ready",
                lifecycle_state: None,
                next_check_after: None,
                payload: json!({
                    "checkpoint_id": work_item.checkpoint_id,
                    "resume_directive": directive.status_code(),
                    "resume_trigger": work_item.resume_trigger,
                    "status_code": status_code,
                }),
            }
        }
    }
}

pub(crate) fn build_paused_checkpoint_resume_work_item(
    claimed: &repo::DuePausedCheckpointTask,
    lease_seconds: i64,
    resume_trigger: crate::task_lifecycle::ResumeTrigger,
    seed_report: crate::agent_engine::LoopStateCheckpointSeedReport,
) -> PausedCheckpointResumeWorkItem {
    PausedCheckpointResumeWorkItem {
        schema_version: 1,
        task_id: claimed.task_id.clone(),
        checkpoint_id: claimed.checkpoint_id.clone(),
        lifecycle_state: claimed.lifecycle_state.clone(),
        executor_state: "prepared",
        resume_entrypoint: claimed.resume_entrypoint.clone(),
        resume_trigger: resume_trigger.status_code(),
        resume_directive: claimed.resume_directive.clone(),
        resume_directive_payload: claimed.checkpoint_resume_directive.to_machine_json(),
        lease_seconds,
        completed_side_effect_count: claimed.completed_side_effect_count,
        requires_idempotency_guard: claimed.requires_idempotency_guard,
        seed_report,
    }
}
