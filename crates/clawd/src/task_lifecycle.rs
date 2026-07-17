use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use claw_core::types::TaskExecutionState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskLifecycleState {
    Queued,
    Running,
    Waiting,
    Background,
    NeedsUser,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CheckpointBudgetCounters {
    pub(crate) round: u32,
    pub(crate) step: u32,
    pub(crate) llm_calls: u32,
    pub(crate) tool_calls: u32,
    pub(crate) elapsed_ms: u64,
    #[serde(default)]
    pub(crate) llm_elapsed_ms: u64,
    #[serde(default)]
    pub(crate) tool_elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResumeEntrypoint {
    NextPlannerRound,
    PollAsyncJob,
    AwaitUserInput,
    VerifyAndFinalize,
}

impl ResumeEntrypoint {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::NextPlannerRound => "next_planner_round",
            Self::PollAsyncJob => "poll_async_job",
            Self::AwaitUserInput => "await_user_input",
            Self::VerifyAndFinalize => "verify_and_finalize",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResumeTrigger {
    UserFollowup,
    ScheduledWakeup,
    WorkerRecovery,
    AsyncJobPoll,
}

impl ResumeTrigger {
    pub(crate) fn status_code(self) -> &'static str {
        match self {
            Self::UserFollowup => "user_followup",
            Self::ScheduledWakeup => "scheduled_wakeup",
            Self::WorkerRecovery => "worker_recovery",
            Self::AsyncJobPoll => "async_job_poll",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TerminalFailureReason {
    WorkerLeaseLost,
    ToolTimeoutWithoutAsyncResume,
    UserCancelled,
    ConfirmationTimeout,
    ProviderWindowExhausted,
    VerifierUnrecoverable,
}

impl TerminalFailureReason {
    pub(crate) fn status_code(self) -> &'static str {
        match self {
            Self::WorkerLeaseLost => "worker_lease_lost",
            Self::ToolTimeoutWithoutAsyncResume => "tool_timeout_without_async_resume",
            Self::UserCancelled => "user_cancelled",
            Self::ConfirmationTimeout => "confirmation_timeout",
            Self::ProviderWindowExhausted => "provider_window_exhausted",
            Self::VerifierUnrecoverable => "verifier_unrecoverable",
        }
    }
}

pub(crate) fn task_query_lifecycle_projection(
    db_status: &str,
    result_json: Option<&Value>,
    updated_at_ts: Option<i64>,
) -> Value {
    let extracted_lifecycle = result_json.and_then(extract_task_lifecycle_payload_with_source);
    let state_source = extracted_lifecycle
        .as_ref()
        .map(|(_, source)| *source)
        .unwrap_or("db_status_projection");
    let mut lifecycle = extracted_lifecycle
        .map(|(payload, _)| payload)
        .unwrap_or_else(|| fallback_task_lifecycle_payload(db_status));
    if let Some(obj) = lifecycle.as_object_mut() {
        obj.entry("schema_version".to_string()).or_insert(json!(1));
        obj.insert("db_status".to_string(), json!(db_status.trim()));
        obj.entry("state_source".to_string())
            .or_insert(json!(state_source));
        obj.entry("can_poll".to_string()).or_insert(json!(true));
        let state = obj
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let execution_state = task_execution_state_from_lifecycle_state(&state);
        obj.entry("execution_state".to_string())
            .or_insert(json!(execution_state));
        let active_state = lifecycle_state_token_is_active(&state);
        obj.entry("can_cancel".to_string())
            .or_insert(json!(active_state));
        append_pause_resume_due_fields(obj, &state, crate::now_ts_u64() as i64);
        if let Some(result_json) = result_json {
            append_lifecycle_product_contract_fields(obj, result_json, &state);
        }
        append_lifecycle_reason_code_field(obj, &state);
        append_lifecycle_next_action_fields(obj, &state);
        append_lifecycle_recommended_user_action_fields(obj, &state);
        if active_state {
            if let Some(updated_at_ts) = updated_at_ts.filter(|ts| *ts > 0) {
                obj.entry("last_heartbeat_ts".to_string())
                    .or_insert(json!(updated_at_ts));
                obj.entry("heartbeat_at".to_string())
                    .or_insert(json!(updated_at_ts));
            }
        }
    }
    lifecycle
}

pub(crate) fn task_execution_state_from_lifecycle(lifecycle: &Value) -> TaskExecutionState {
    let state = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default();
    task_execution_state_from_lifecycle_state(state)
}

fn task_execution_state_from_lifecycle_state(state: &str) -> TaskExecutionState {
    match state.trim() {
        "queued" => TaskExecutionState::Queued,
        "running" => TaskExecutionState::Running,
        "waiting" => TaskExecutionState::Waiting,
        "background" => TaskExecutionState::Background,
        "needs_confirmation" | "needs_user" => TaskExecutionState::NeedsConfirmation,
        "blocked" => TaskExecutionState::Blocked,
        "cancelled" | "canceled" => TaskExecutionState::Cancelled,
        "succeeded" | "completed" => TaskExecutionState::Completed,
        "failed" | "timeout" => TaskExecutionState::Failed,
        _ => TaskExecutionState::Failed,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PausedCheckpointRecoveryStatus {
    NotPaused,
    InvalidPausedCheckpoint,
    Waiting {
        state: String,
        checkpoint_id: String,
        resume_due: bool,
        resume_wait_seconds: i64,
    },
}

impl PausedCheckpointRecoveryStatus {
    pub(crate) fn preserve_running_status_for_recovery(&self) -> bool {
        matches!(self, Self::Waiting { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PausedCheckpointResumeReadiness {
    NotPaused,
    InvalidPausedCheckpoint,
    WaitingNotDue {
        state: String,
        checkpoint_id: String,
        resume_wait_seconds: i64,
    },
    MissingTaskCheckpoint {
        state: String,
        checkpoint_id: String,
    },
    InvalidTaskCheckpoint {
        state: String,
        checkpoint_id: String,
    },
    CheckpointMismatch {
        state: String,
        lifecycle_checkpoint_id: String,
        task_checkpoint_id: String,
    },
    ActiveResumeLease {
        state: String,
        checkpoint_id: String,
        lease_expires_at: i64,
        resume_wait_seconds: i64,
    },
    Ready {
        state: String,
        checkpoint_id: String,
        resume_entrypoint: ResumeEntrypoint,
        completed_side_effect_count: usize,
        requires_idempotency_guard: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckpointResumeDirective {
    RunNextPlannerRound {
        checkpoint_id: String,
        completed_side_effect_count: usize,
        requires_idempotency_guard: bool,
    },
    PollAsyncJob {
        checkpoint_id: String,
        job_id: String,
        adapter_kind: String,
        poll_after_seconds: u64,
        expires_at: i64,
        cancel_ref: String,
        message_key: String,
    },
    AwaitUserInput {
        checkpoint_id: String,
    },
    VerifyAndFinalize {
        checkpoint_id: String,
        completed_side_effect_count: usize,
    },
    WaitForActiveLease {
        checkpoint_id: String,
        lease_expires_at: i64,
        resume_wait_seconds: i64,
    },
    NotReady {
        status_code: &'static str,
    },
}

impl CheckpointResumeDirective {
    pub(crate) fn status_code(&self) -> &'static str {
        match self {
            Self::RunNextPlannerRound { .. } => "run_next_planner_round",
            Self::PollAsyncJob { .. } => "poll_async_job",
            Self::AwaitUserInput { .. } => "await_user_input",
            Self::VerifyAndFinalize { .. } => "verify_and_finalize",
            Self::WaitForActiveLease { .. } => "wait_for_active_resume_lease",
            Self::NotReady { status_code } => status_code,
        }
    }

    pub(crate) fn to_machine_json(&self) -> Value {
        match self {
            Self::RunNextPlannerRound {
                checkpoint_id,
                completed_side_effect_count,
                requires_idempotency_guard,
            } => json!({
                "status_code": self.status_code(),
                "checkpoint_id": checkpoint_id,
                "completed_side_effect_count": completed_side_effect_count,
                "requires_idempotency_guard": requires_idempotency_guard,
            }),
            Self::PollAsyncJob {
                checkpoint_id,
                job_id,
                adapter_kind,
                poll_after_seconds,
                expires_at,
                cancel_ref,
                message_key,
            } => json!({
                "status_code": self.status_code(),
                "checkpoint_id": checkpoint_id,
                "job_id": job_id,
                "adapter_kind": adapter_kind,
                "poll_after_seconds": poll_after_seconds,
                "expires_at": expires_at,
                "cancel_ref": cancel_ref,
                "message_key": message_key,
            }),
            Self::AwaitUserInput { checkpoint_id } => json!({
                "status_code": self.status_code(),
                "checkpoint_id": checkpoint_id,
            }),
            Self::VerifyAndFinalize {
                checkpoint_id,
                completed_side_effect_count,
            } => json!({
                "status_code": self.status_code(),
                "checkpoint_id": checkpoint_id,
                "completed_side_effect_count": completed_side_effect_count,
            }),
            Self::WaitForActiveLease {
                checkpoint_id,
                lease_expires_at,
                resume_wait_seconds,
            } => json!({
                "status_code": self.status_code(),
                "checkpoint_id": checkpoint_id,
                "lease_expires_at": lease_expires_at,
                "resume_wait_seconds": resume_wait_seconds,
            }),
            Self::NotReady { status_code } => json!({
                "status_code": status_code,
            }),
        }
    }
}

pub(crate) fn paused_checkpoint_recovery_status(
    result_json: &Value,
    now_ts: i64,
) -> PausedCheckpointRecoveryStatus {
    let lifecycle = task_query_lifecycle_projection("running", Some(result_json), None);
    let state = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let needs_user_input = state == "needs_user";
    if !lifecycle_state_token_is_paused(&state) && !needs_user_input {
        return PausedCheckpointRecoveryStatus::NotPaused;
    }
    let Some(checkpoint_id) = lifecycle
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
    else {
        return PausedCheckpointRecoveryStatus::InvalidPausedCheckpoint;
    };
    let Some(next_check_after) = lifecycle
        .get("next_check_after")
        .and_then(Value::as_i64)
        .or_else(|| needs_user_input.then_some(now_ts))
    else {
        return PausedCheckpointRecoveryStatus::InvalidPausedCheckpoint;
    };
    let resume_wait_seconds = next_check_after.saturating_sub(now_ts).max(0);
    PausedCheckpointRecoveryStatus::Waiting {
        state,
        checkpoint_id,
        resume_due: resume_wait_seconds == 0,
        resume_wait_seconds,
    }
}

pub(crate) fn paused_checkpoint_resume_readiness(
    result_json: &Value,
    now_ts: i64,
) -> PausedCheckpointResumeReadiness {
    let (state, checkpoint_id, resume_due, resume_wait_seconds) =
        match paused_checkpoint_recovery_status(result_json, now_ts) {
            PausedCheckpointRecoveryStatus::NotPaused => {
                return PausedCheckpointResumeReadiness::NotPaused;
            }
            PausedCheckpointRecoveryStatus::InvalidPausedCheckpoint => {
                return PausedCheckpointResumeReadiness::InvalidPausedCheckpoint;
            }
            PausedCheckpointRecoveryStatus::Waiting {
                state,
                checkpoint_id,
                resume_due,
                resume_wait_seconds,
            } => (state, checkpoint_id, resume_due, resume_wait_seconds),
        };

    if !resume_due {
        return PausedCheckpointResumeReadiness::WaitingNotDue {
            state,
            checkpoint_id,
            resume_wait_seconds,
        };
    }

    let Some(checkpoint_payload) = extract_task_checkpoint_payload(result_json) else {
        return PausedCheckpointResumeReadiness::MissingTaskCheckpoint {
            state,
            checkpoint_id,
        };
    };
    let Ok(checkpoint) = serde_json::from_value::<TaskCheckpoint>(checkpoint_payload) else {
        return PausedCheckpointResumeReadiness::InvalidTaskCheckpoint {
            state,
            checkpoint_id,
        };
    };
    if checkpoint.checkpoint_id != checkpoint_id {
        return PausedCheckpointResumeReadiness::CheckpointMismatch {
            state,
            lifecycle_checkpoint_id: checkpoint_id,
            task_checkpoint_id: checkpoint.checkpoint_id,
        };
    }
    let lifecycle = task_query_lifecycle_projection("running", Some(result_json), None);
    if let Some(lease_expires_at) =
        active_resume_lease_expires_at(&lifecycle, &checkpoint_id, now_ts)
    {
        return PausedCheckpointResumeReadiness::ActiveResumeLease {
            state,
            checkpoint_id,
            lease_expires_at,
            resume_wait_seconds: lease_expires_at.saturating_sub(now_ts).max(0),
        };
    }
    let completed_side_effect_count = checkpoint.completed_side_effect_refs.len();
    PausedCheckpointResumeReadiness::Ready {
        state,
        checkpoint_id: checkpoint.checkpoint_id,
        resume_entrypoint: checkpoint.resume_entrypoint,
        completed_side_effect_count,
        requires_idempotency_guard: completed_side_effect_count > 0,
    }
}

pub(crate) fn checkpoint_resume_directive(
    result_json: &Value,
    now_ts: i64,
) -> CheckpointResumeDirective {
    match paused_checkpoint_resume_readiness(result_json, now_ts) {
        PausedCheckpointResumeReadiness::Ready {
            checkpoint_id,
            resume_entrypoint,
            completed_side_effect_count,
            requires_idempotency_guard,
            ..
        } => {
            let Some(checkpoint_payload) = extract_task_checkpoint_payload(result_json) else {
                return CheckpointResumeDirective::NotReady {
                    status_code: "missing_task_checkpoint",
                };
            };
            let Ok(checkpoint) = serde_json::from_value::<TaskCheckpoint>(checkpoint_payload)
            else {
                return CheckpointResumeDirective::NotReady {
                    status_code: "invalid_task_checkpoint",
                };
            };
            match resume_entrypoint {
                ResumeEntrypoint::NextPlannerRound => {
                    CheckpointResumeDirective::RunNextPlannerRound {
                        checkpoint_id,
                        completed_side_effect_count,
                        requires_idempotency_guard,
                    }
                }
                ResumeEntrypoint::PollAsyncJob => match checkpoint.pending_async_job.as_ref() {
                    Some(job) => {
                        if matches!(job.status, AsyncJobStatus::Succeeded) {
                            if checkpoint_has_async_job_success_observation(&checkpoint) {
                                return CheckpointResumeDirective::VerifyAndFinalize {
                                    checkpoint_id,
                                    completed_side_effect_count,
                                };
                            }
                            return CheckpointResumeDirective::NotReady {
                                status_code: "async_job_observation_required",
                            };
                        }
                        let effective_expires_at =
                            async_job_effective_expires_at(result_json, job.expires_at);
                        if let Some(status_code) =
                            pending_async_job_resume_blocker(&job, effective_expires_at, now_ts)
                        {
                            CheckpointResumeDirective::NotReady { status_code }
                        } else {
                            let adapter_kind =
                                async_job_adapter_kind(result_json).unwrap_or("unspecified_poll");
                            CheckpointResumeDirective::PollAsyncJob {
                                checkpoint_id,
                                job_id: job.job_id.clone(),
                                adapter_kind: adapter_kind.to_string(),
                                poll_after_seconds: job.poll_after_seconds,
                                expires_at: effective_expires_at,
                                cancel_ref: job.cancel_ref.clone(),
                                message_key: job.message_key.clone(),
                            }
                        }
                    }
                    None => CheckpointResumeDirective::NotReady {
                        status_code: "missing_pending_async_job",
                    },
                },
                ResumeEntrypoint::AwaitUserInput => {
                    CheckpointResumeDirective::AwaitUserInput { checkpoint_id }
                }
                ResumeEntrypoint::VerifyAndFinalize => {
                    CheckpointResumeDirective::VerifyAndFinalize {
                        checkpoint_id,
                        completed_side_effect_count,
                    }
                }
            }
        }
        PausedCheckpointResumeReadiness::ActiveResumeLease {
            checkpoint_id,
            lease_expires_at,
            resume_wait_seconds,
            ..
        } => CheckpointResumeDirective::WaitForActiveLease {
            checkpoint_id,
            lease_expires_at,
            resume_wait_seconds,
        },
        PausedCheckpointResumeReadiness::NotPaused => CheckpointResumeDirective::NotReady {
            status_code: "not_paused",
        },
        PausedCheckpointResumeReadiness::InvalidPausedCheckpoint => {
            CheckpointResumeDirective::NotReady {
                status_code: "invalid_paused_checkpoint",
            }
        }
        PausedCheckpointResumeReadiness::WaitingNotDue { .. } => {
            CheckpointResumeDirective::NotReady {
                status_code: "waiting_not_due",
            }
        }
        PausedCheckpointResumeReadiness::MissingTaskCheckpoint { .. } => {
            CheckpointResumeDirective::NotReady {
                status_code: "missing_task_checkpoint",
            }
        }
        PausedCheckpointResumeReadiness::InvalidTaskCheckpoint { .. } => {
            CheckpointResumeDirective::NotReady {
                status_code: "invalid_task_checkpoint",
            }
        }
        PausedCheckpointResumeReadiness::CheckpointMismatch { .. } => {
            CheckpointResumeDirective::NotReady {
                status_code: "checkpoint_mismatch",
            }
        }
    }
}

fn checkpoint_has_async_job_success_observation(checkpoint: &TaskCheckpoint) -> bool {
    checkpoint
        .pending_action
        .as_ref()
        .is_some_and(value_has_final_result_signal)
        || checkpoint
            .observations
            .iter()
            .any(value_has_final_result_signal)
}

fn value_has_final_result_signal(value: &Value) -> bool {
    value.get("final_result_json").is_some_and(Value::is_object)
        || non_empty_string_at(value, &["final_answer"])
        || value
            .get("answer")
            .filter(|answer| answer.is_object())
            .is_some_and(|answer| {
                non_empty_string_at(answer, &["text"])
                    || non_empty_string_at(answer, &["final_answer"])
            })
        || value
            .get("task_journal")
            .and_then(|task_journal| task_journal.get("summary"))
            .is_some_and(|summary| non_empty_string_at(summary, &["final_answer"]))
}

fn non_empty_string_at(value: &Value, path: &[&str]) -> bool {
    let Some((last, parents)) = path.split_last() else {
        return false;
    };
    let mut current = value;
    for key in parents {
        let Some(next) = current.get(*key) else {
            return false;
        };
        current = next;
    }
    current
        .get(*last)
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|item| !item.is_empty())
}

fn pending_async_job_resume_blocker(
    job: &AsyncJobRef,
    _effective_expires_at: i64,
    _now_ts: i64,
) -> Option<&'static str> {
    if !job.missing_required_fields().is_empty() {
        return Some("invalid_pending_async_job");
    }
    match job.status {
        AsyncJobStatus::Accepted | AsyncJobStatus::Running | AsyncJobStatus::Expired => None,
        AsyncJobStatus::Succeeded => Some("async_job_observation_required"),
        AsyncJobStatus::Failed => Some("async_job_failed"),
    }
}

fn async_job_effective_expires_at(result_json: &Value, job_expires_at: i64) -> i64 {
    let policy_deadline = result_json
        .pointer("/task_lifecycle/async_timeout_policy/effective_deadline_ts")
        .and_then(Value::as_i64)
        .filter(|deadline| *deadline > 0);
    policy_deadline
        .map(|deadline| deadline.min(job_expires_at))
        .unwrap_or(job_expires_at)
}

fn async_job_adapter_kind(result_json: &Value) -> Option<&str> {
    result_json
        .pointer("/task_lifecycle/async_timeout_policy/adapter_kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn active_resume_lease_expires_at(
    lifecycle: &Value,
    checkpoint_id: &str,
    now_ts: i64,
) -> Option<i64> {
    let claim = lifecycle.get("resume_claim")?;
    let claim_checkpoint_id = claim
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if claim_checkpoint_id != checkpoint_id {
        return None;
    }
    let expires_at = claim.get("expires_at").and_then(Value::as_i64)?;
    (expires_at > now_ts).then_some(expires_at)
}

fn extract_task_lifecycle_payload_with_source(
    result_json: &Value,
) -> Option<(Value, &'static str)> {
    result_json
        .get("task_lifecycle")
        .filter(|value| value.is_object())
        .cloned()
        .map(|payload| (payload, "task_lifecycle_payload"))
        .or_else(|| {
            result_json
                .pointer("/task_journal/summary/task_lifecycle")
                .filter(|value| value.is_object())
                .cloned()
                .map(|payload| (payload, "task_journal_summary"))
        })
}

fn extract_task_checkpoint_payload(result_json: &Value) -> Option<Value> {
    result_json
        .get("task_checkpoint")
        .filter(|value| value.is_object())
        .cloned()
        .or_else(|| {
            result_json
                .pointer("/task_journal/summary/task_checkpoint")
                .filter(|value| value.is_object())
                .cloned()
        })
}

pub(crate) fn task_checkpoint_from_result_json(result_json: &Value) -> Option<TaskCheckpoint> {
    extract_task_checkpoint_payload(result_json)
        .and_then(|payload| serde_json::from_value::<TaskCheckpoint>(payload).ok())
}

pub(crate) fn has_matching_nonterminal_checkpoint(
    lifecycle: Option<&Value>,
    checkpoint: Option<&Value>,
) -> bool {
    let Some(lifecycle) = lifecycle else {
        return false;
    };
    let state = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(state, "waiting" | "background" | "needs_user") {
        return false;
    }
    let lifecycle_checkpoint_id = lifecycle
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let checkpoint_id = checkpoint
        .and_then(|value| value.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    matches!(
        (lifecycle_checkpoint_id, checkpoint_id),
        (Some(lifecycle_id), Some(checkpoint_id)) if lifecycle_id == checkpoint_id
    )
}

fn fallback_task_lifecycle_payload(db_status: &str) -> Value {
    let state = lifecycle_state_from_db_status(db_status);
    let mut payload = json!({
        "schema_version": 1,
        "state": state,
        "source": "db_status_projection",
    });
    if db_status.trim() == "timeout" {
        payload["terminal_reason"] = json!("worker_task_timeout");
    }
    payload
}

fn lifecycle_state_from_db_status(db_status: &str) -> &'static str {
    match db_status.trim() {
        "queued" => "queued",
        "running" => "running",
        "succeeded" => "succeeded",
        "canceled" => "cancelled",
        "failed" | "timeout" => "failed",
        _ => "failed",
    }
}

fn lifecycle_state_token_is_active(state: &str) -> bool {
    matches!(
        state.trim(),
        "queued" | "running" | "waiting" | "background" | "needs_user"
    )
}

fn lifecycle_state_token_is_paused(state: &str) -> bool {
    matches!(state.trim(), "waiting" | "background")
}

fn append_pause_resume_due_fields(
    obj: &mut serde_json::Map<String, Value>,
    state: &str,
    now_ts: i64,
) {
    if !lifecycle_state_token_is_paused(state) {
        return;
    }
    let Some(next_check_after) = obj.get("next_check_after").and_then(Value::as_i64) else {
        return;
    };
    let wait_seconds = next_check_after.saturating_sub(now_ts).max(0);
    obj.entry("resume_due".to_string())
        .or_insert(json!(wait_seconds == 0));
    obj.entry("resume_wait_seconds".to_string())
        .or_insert(json!(wait_seconds));
}

fn append_lifecycle_product_contract_fields(
    obj: &mut serde_json::Map<String, Value>,
    result_json: &Value,
    state: &str,
) {
    if lifecycle_state_token_is_paused(state) || state.trim() == "needs_user" {
        if let Some(reason_code) = string_field(obj, "resume_reason")
            .or_else(|| string_field(obj, "terminal_reason"))
            .or_else(|| non_empty_state_token(state))
        {
            obj.entry("waiting_reason_code".to_string())
                .or_insert(json!(reason_code));
        }
    }
    if let Some(next_check_after) = obj.get("next_check_after").cloned() {
        obj.entry("next_poll_after".to_string())
            .or_insert(next_check_after);
    }
    if let Some(owner) = first_nested_string_field(
        obj,
        &[
            &["resume_claim", "owner"],
            &["resume_executor_claim", "owner"],
            &["resume_executor_handoff_claim", "owner"],
            &["resume_executor_dispatch_claim", "owner"],
            &["resume_executor_result_projection_claim", "owner"],
        ],
    ) {
        obj.entry("resume_owner".to_string())
            .or_insert(json!(owner));
    }
    let Some(checkpoint_payload) = extract_task_checkpoint_payload(result_json) else {
        return;
    };
    if let Some(checkpoint_id) = checkpoint_payload
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        obj.entry("checkpoint_id".to_string())
            .or_insert(json!(checkpoint_id));
    }
    let Ok(checkpoint) = serde_json::from_value::<TaskCheckpoint>(checkpoint_payload) else {
        return;
    };
    obj.entry("last_stable_checkpoint_id".to_string())
        .or_insert(json!(checkpoint.checkpoint_id.as_str()));
    obj.entry("resume_entrypoint".to_string())
        .or_insert(json!(checkpoint.resume_entrypoint.as_str()));
    obj.entry("last_stable_resume_entrypoint".to_string())
        .or_insert(json!(checkpoint.resume_entrypoint.as_str()));
    if let Some(round) = checkpoint.last_successful_round {
        obj.entry("last_successful_round".to_string())
            .or_insert(json!(round));
    }
    let completed_side_effect_count = checkpoint.completed_side_effect_refs.len();
    let visible_completed_side_effect_refs = checkpoint
        .completed_side_effect_refs
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .take(8)
        .collect::<Vec<_>>();
    let visible_completed_side_effect_count = visible_completed_side_effect_refs.len();
    let completed_side_effect_refs_truncated =
        completed_side_effect_count > visible_completed_side_effect_count;
    obj.entry("completed_side_effect_count".to_string())
        .or_insert(json!(completed_side_effect_count));
    obj.entry("requires_idempotency_guard".to_string())
        .or_insert(json!(completed_side_effect_count > 0));
    if !visible_completed_side_effect_refs.is_empty() {
        obj.entry("completed_side_effect_refs".to_string())
            .or_insert(json!(visible_completed_side_effect_refs));
        obj.entry("completed_side_effect_refs_truncated".to_string())
            .or_insert(json!(completed_side_effect_refs_truncated));
    }
    if let Some(last_safe_step_id) = checkpoint
        .last_successful_step
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        obj.entry("last_safe_step_id".to_string())
            .or_insert(json!(last_safe_step_id));
    }
    obj.entry("evidence_ref_count".to_string())
        .or_insert(json!(checkpoint.evidence_refs.len()));
    obj.entry("artifact_ref_count".to_string())
        .or_insert(json!(checkpoint.artifact_refs.len()));
    append_bounded_string_ref_projection(
        obj,
        "artifact_refs",
        "artifact_refs_truncated",
        &checkpoint.artifact_refs,
        8,
    );
    if let Some(last_evidence_ref) = checkpoint
        .evidence_refs
        .iter()
        .rev()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
    {
        obj.entry("last_successful_evidence_ref".to_string())
            .or_insert(json!(last_evidence_ref));
    }
    if let Some(job) = checkpoint.pending_async_job.as_ref() {
        obj.entry("poll_ref".to_string())
            .or_insert(json!(job.job_id.as_str()));
        obj.entry("cancel_ref".to_string())
            .or_insert(json!(job.cancel_ref.as_str()));
        obj.entry("poll_after_seconds".to_string())
            .or_insert(json!(job.poll_after_seconds));
        obj.entry("async_job_expires_at".to_string())
            .or_insert(json!(job.expires_at));
        obj.entry("async_job_message_key".to_string())
            .or_insert(json!(job.message_key.as_str()));
    }
    append_lifecycle_open_issue_fields(obj, &checkpoint);
    append_lifecycle_provider_blocker_fields(obj, &checkpoint);
}

fn append_bounded_string_ref_projection(
    obj: &mut serde_json::Map<String, Value>,
    refs_key: &str,
    truncated_key: &str,
    refs: &[String],
    limit: usize,
) {
    let visible_refs = refs
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .take(limit)
        .collect::<Vec<_>>();
    if visible_refs.is_empty() {
        return;
    }
    obj.entry(refs_key.to_string())
        .or_insert(json!(visible_refs));
    obj.entry(truncated_key.to_string())
        .or_insert(json!(refs.len() > limit));
}

fn append_lifecycle_open_issue_fields(
    obj: &mut serde_json::Map<String, Value>,
    checkpoint: &TaskCheckpoint,
) {
    let Some(fields) = checkpoint_open_issue_fields(checkpoint) else {
        return;
    };
    for (key, value) in fields {
        obj.entry(key).or_insert(value);
    }
}

fn checkpoint_open_issue_fields(
    checkpoint: &TaskCheckpoint,
) -> Option<serde_json::Map<String, Value>> {
    if let Some(signal) = checkpoint.repair_signal.as_ref() {
        if let Some(fields) = open_issue_fields_from_signal(signal, None) {
            return Some(fields);
        }
    }
    let Some(entries) = checkpoint.attempt_ledger.as_ref().and_then(Value::as_array) else {
        return None;
    };
    for entry in entries.iter().rev() {
        let Some(signal) = entry.get("repair_signal") else {
            continue;
        };
        if let Some(fields) = open_issue_fields_from_signal(signal, Some(entry)) {
            return Some(fields);
        }
    }
    None
}

fn open_issue_fields_from_signal(
    signal: &Value,
    attempt_entry: Option<&Value>,
) -> Option<serde_json::Map<String, Value>> {
    let issue_codes = string_array_values(signal.pointer("/repair_envelope/issue_codes"));
    let missing_fields = string_array_values(signal.get("missing_fields"))
        .into_iter()
        .chain(string_array_values(
            signal.pointer("/repair_envelope/missing_evidence"),
        ))
        .collect::<Vec<_>>();
    let next_recovery_kind = string_value(signal.get("next_recovery_kind"))
        .or_else(|| string_value(signal.pointer("/repair_envelope/next_recovery_kind")));
    let status_code = string_value(signal.get("status_code"));
    let reason_code = string_value(signal.get("reason_code"));
    let has_issue = !issue_codes.is_empty()
        || !missing_fields.is_empty()
        || status_code.is_some()
        || reason_code.is_some();
    if !has_issue {
        return None;
    }

    let mut deduped_missing_fields = missing_fields;
    deduped_missing_fields.sort();
    deduped_missing_fields.dedup();
    let open_issue_count = if issue_codes.is_empty() {
        1
    } else {
        issue_codes.len()
    };
    let mut fields = serde_json::Map::new();
    fields.insert("open_issue_count".to_string(), json!(open_issue_count));
    insert_string_projection(
        &mut fields,
        "open_issue_status_code",
        status_code.as_deref(),
    );
    insert_string_projection(
        &mut fields,
        "open_issue_reason_code",
        reason_code.as_deref(),
    );
    insert_string_projection(
        &mut fields,
        "open_issue_next_recovery_kind",
        next_recovery_kind.as_deref(),
    );
    insert_string_projection(
        &mut fields,
        "open_issue_source",
        string_value(signal.get("source")).as_deref(),
    );
    if let Some(retryable) = signal.get("retryable").and_then(Value::as_bool) {
        fields.insert("open_issue_retryable".to_string(), json!(retryable));
    }
    insert_string_array_projection(&mut fields, "open_issue_codes", &issue_codes, 8);
    insert_string_array_projection(
        &mut fields,
        "open_issue_missing_fields",
        &deduped_missing_fields,
        8,
    );
    if let Some(entry) = attempt_entry {
        insert_string_projection(
            &mut fields,
            "open_issue_action_ref",
            string_value(entry.get("action_ref")).as_deref(),
        );
        insert_string_projection(
            &mut fields,
            "open_issue_recovery_action",
            string_value(entry.get("recovery_action")).as_deref(),
        );
    }
    Some(fields)
}

fn append_lifecycle_provider_blocker_fields(
    obj: &mut serde_json::Map<String, Value>,
    checkpoint: &TaskCheckpoint,
) {
    let Some(fields) = checkpoint_provider_blocker_fields(checkpoint) else {
        return;
    };
    for (key, value) in fields {
        obj.entry(key).or_insert(value);
    }
}

fn checkpoint_provider_blocker_fields(
    checkpoint: &TaskCheckpoint,
) -> Option<serde_json::Map<String, Value>> {
    if let Some(signal) = checkpoint.repair_signal.as_ref() {
        if let Some(fields) = provider_blocker_fields_from_signal(signal, None) {
            return Some(fields);
        }
    }
    let Some(entries) = checkpoint.attempt_ledger.as_ref().and_then(Value::as_array) else {
        return None;
    };
    for entry in entries.iter().rev() {
        let Some(signal) = entry.get("repair_signal") else {
            continue;
        };
        if let Some(fields) = provider_blocker_fields_from_signal(signal, Some(entry)) {
            return Some(fields);
        }
    }
    None
}

fn provider_blocker_fields_from_signal(
    signal: &Value,
    attempt_entry: Option<&Value>,
) -> Option<serde_json::Map<String, Value>> {
    let provider_status = signal
        .get("provider_status")
        .or_else(|| signal.pointer("/repair_envelope/provider_status"))?;
    let next_recovery_kind = string_value(signal.get("next_recovery_kind"))
        .or_else(|| string_value(signal.pointer("/repair_envelope/next_recovery_kind")))
        .or_else(|| attempt_entry.and_then(|entry| string_value(entry.get("recovery_action"))));
    let status_code = string_value(provider_status.get("status_code"))
        .or_else(|| string_value(provider_status.get("provider_error_class")))
        .or_else(|| string_value(signal.get("status_code")));
    let external_provider_blocked = provider_status
        .get("external_provider_blocked")
        .and_then(Value::as_bool);
    let retry_after_seconds = provider_status
        .get("retry_after_seconds")
        .and_then(Value::as_i64);
    let is_provider_blocker = external_provider_blocked == Some(true)
        || retry_after_seconds.is_some()
        || next_recovery_kind.as_deref() == Some("wait_background")
        || status_code
            .as_deref()
            .is_some_and(provider_blocker_status_code);
    if !is_provider_blocker {
        return None;
    }

    let mut fields = serde_json::Map::new();
    fields.insert("provider_blocker_active".to_string(), json!(true));
    insert_string_projection(
        &mut fields,
        "provider_blocker_status_code",
        status_code.as_deref(),
    );
    insert_string_projection(
        &mut fields,
        "provider_blocker_next_recovery_kind",
        next_recovery_kind.as_deref(),
    );
    insert_string_projection(
        &mut fields,
        "provider_blocker_provider",
        string_value(provider_status.get("provider")).as_deref(),
    );
    insert_string_projection(
        &mut fields,
        "provider_blocker_message_key",
        string_value(provider_status.get("message_key")).as_deref(),
    );
    insert_string_projection(
        &mut fields,
        "provider_blocker_unsupported_reason",
        string_value(provider_status.get("unsupported_reason")).as_deref(),
    );
    insert_string_projection(
        &mut fields,
        "provider_blocker_signal_source",
        string_value(signal.get("source")).as_deref(),
    );
    insert_string_projection(
        &mut fields,
        "provider_blocker_reason_code",
        string_value(signal.get("reason_code")).as_deref(),
    );
    if let Some(value) = external_provider_blocked {
        fields.insert(
            "provider_blocker_external_blocked".to_string(),
            json!(value),
        );
    }
    if let Some(value) = provider_status
        .get("provider_supported")
        .and_then(Value::as_bool)
    {
        fields.insert(
            "provider_blocker_provider_supported".to_string(),
            json!(value),
        );
    }
    if let Some(value) = retry_after_seconds {
        fields.insert(
            "provider_blocker_retry_after_seconds".to_string(),
            json!(value),
        );
    }
    if let Some(entry) = attempt_entry {
        insert_string_projection(
            &mut fields,
            "provider_blocker_action_ref",
            string_value(entry.get("action_ref")).as_deref(),
        );
        insert_string_projection(
            &mut fields,
            "provider_blocker_tool_or_skill",
            string_value(entry.get("tool_or_skill")).as_deref(),
        );
        insert_string_projection(
            &mut fields,
            "provider_blocker_recovery_action",
            string_value(entry.get("recovery_action")).as_deref(),
        );
    }
    Some(fields)
}

fn provider_blocker_status_code(status_code: &str) -> bool {
    matches!(
        status_code.trim(),
        "rate_limited"
            | "quota_exhausted"
            | "quota_exceeded"
            | "provider_retryable_response"
            | "provider_error"
            | "timeout"
    )
}

fn insert_string_projection(
    obj: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<&str>,
) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    obj.insert(key.to_string(), json!(value));
}

fn insert_string_array_projection(
    obj: &mut serde_json::Map<String, Value>,
    key: &str,
    values: &[String],
    limit: usize,
) {
    let visible_values = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .take(limit)
        .collect::<Vec<_>>();
    if visible_values.is_empty() {
        return;
    }
    obj.insert(key.to_string(), json!(visible_values));
}

fn append_lifecycle_reason_code_field(obj: &mut serde_json::Map<String, Value>, state: &str) {
    let reason_code = string_field(obj, "reason_code")
        .or_else(|| string_field(obj, "resume_reason"))
        .or_else(|| string_field(obj, "terminal_reason"))
        .or_else(|| string_field(obj, "waiting_reason_code"))
        .or_else(|| non_empty_state_token(state));
    if let Some(reason_code) = reason_code {
        obj.entry("reason_code".to_string())
            .or_insert(json!(reason_code));
    }
}

fn append_lifecycle_next_action_fields(obj: &mut serde_json::Map<String, Value>, state: &str) {
    let state = state.trim();
    let next_action_kind =
        if matches!(state, "waiting" | "background") && obj.get("poll_ref").is_some() {
            Some("poll_async_job")
        } else if matches!(state, "waiting" | "background") && obj.get("checkpoint_id").is_some() {
            Some("resume_checkpoint")
        } else if state == "needs_user" {
            Some("await_user_input")
        } else if matches!(state, "queued" | "running") {
            Some("poll_task")
        } else if matches!(state, "succeeded" | "failed" | "cancelled") {
            Some("inspect_result")
        } else {
            None
        };
    if let Some(kind) = next_action_kind {
        obj.entry("next_action_kind".to_string())
            .or_insert(json!(kind));
    }
    let next_action_ref = obj
        .get("poll_ref")
        .or_else(|| obj.get("checkpoint_id"))
        .or_else(|| obj.get("db_status"))
        .cloned();
    if let Some(next_action_ref) = next_action_ref {
        obj.entry("next_action_ref".to_string())
            .or_insert(next_action_ref);
    }
}

fn append_lifecycle_recommended_user_action_fields(
    obj: &mut serde_json::Map<String, Value>,
    state: &str,
) {
    let state = state.trim();
    let recommended_user_action_kind = if state == "needs_user" {
        Some("provide_required_input")
    } else if matches!(state, "waiting" | "background") {
        if obj.get("poll_ref").is_some() {
            Some("wait_for_async_poll")
        } else if obj
            .get("resume_due")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            Some("wait_for_worker_resume")
        } else {
            Some("wait_until_next_check")
        }
    } else if matches!(state, "queued" | "running") {
        Some("poll_task_status")
    } else if matches!(state, "succeeded" | "failed" | "cancelled") {
        Some("inspect_result")
    } else {
        None
    };
    if let Some(kind) = recommended_user_action_kind {
        obj.entry("recommended_user_action_kind".to_string())
            .or_insert(json!(kind));
    }
}

fn non_empty_state_token(state: &str) -> Option<String> {
    let state = state.trim();
    (!state.is_empty()).then(|| state.to_string())
}

fn string_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn string_value(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn string_array_values(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn first_nested_string_field(
    obj: &serde_json::Map<String, Value>,
    paths: &[&[&str]],
) -> Option<String> {
    paths.iter().find_map(|path| {
        let (last, parents) = path.split_last()?;
        let mut current = obj.get(*parents.first()?)?;
        for key in &parents[1..] {
            current = current.get(*key)?;
        }
        current
            .get(*last)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct TaskCheckpoint {
    pub(crate) schema_version: u8,
    pub(crate) checkpoint_id: String,
    pub(crate) boundary_context: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_successful_round: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_successful_step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pending_action: Option<Value>,
    pub(crate) observations: Vec<Value>,
    pub(crate) evidence_refs: Vec<String>,
    pub(crate) artifact_refs: Vec<String>,
    pub(crate) completed_side_effect_refs: Vec<String>,
    pub(crate) budget: CheckpointBudgetCounters,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) attempt_ledger: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pending_async_job: Option<AsyncJobRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) repair_signal: Option<Value>,
    pub(crate) resume_entrypoint: ResumeEntrypoint,
}

impl TaskCheckpoint {
    pub(crate) fn to_machine_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({"schema_version": 1}))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AsyncJobStatus {
    Accepted,
    Running,
    Succeeded,
    Failed,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AsyncJobRef {
    pub(crate) job_id: String,
    pub(crate) status: AsyncJobStatus,
    pub(crate) poll_after_seconds: u64,
    pub(crate) expires_at: i64,
    pub(crate) cancel_ref: String,
    pub(crate) message_key: String,
}

impl AsyncJobRef {
    pub(crate) fn missing_required_fields(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.job_id.trim().is_empty() {
            missing.push("job_id");
        }
        if self.poll_after_seconds == 0 {
            missing.push("poll_after_seconds");
        }
        if self.expires_at <= 0 {
            missing.push("expires_at");
        }
        if self.cancel_ref.trim().is_empty() {
            missing.push("cancel_ref");
        }
        if self.message_key.trim().is_empty() {
            missing.push("message_key");
        }
        missing
    }
}

#[cfg(test)]
#[path = "task_lifecycle_tests.rs"]
mod tests;
