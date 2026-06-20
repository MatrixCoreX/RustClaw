#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TimeoutLayer {
    FrontendRequest,
    WorkerLeaseHeartbeat,
    ToolCall,
    LlmProviderWindow,
    AgentLoopSoftBudget,
}

impl TimeoutLayer {
    pub(crate) fn effect(self) -> TimeoutEffect {
        match self {
            Self::FrontendRequest => TimeoutEffect::CallerPollsExistingTask,
            Self::WorkerLeaseHeartbeat => TimeoutEffect::RecoveryOwnsRunningTask,
            Self::ToolCall => TimeoutEffect::TerminalToolTimeoutUnlessAsync,
            Self::LlmProviderWindow => TimeoutEffect::ProviderGapWaitOrFail,
            Self::AgentLoopSoftBudget => TimeoutEffect::CheckpointAndWait,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TimeoutEffect {
    CallerPollsExistingTask,
    RecoveryOwnsRunningTask,
    TerminalToolTimeoutUnlessAsync,
    ProviderGapWaitOrFail,
    CheckpointAndWait,
}

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

impl TaskLifecycleState {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    pub(crate) fn db_compat_status(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running | Self::Waiting | Self::Background | Self::NeedsUser => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "canceled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LifecycleTransitionOwner {
    Worker,
    AgentLoop,
    ToolAdapter,
    ConfirmationGate,
    ChannelUserAction,
    RecoveryWorker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LifecyclePauseContext {
    pub(crate) resume_reason: String,
    pub(crate) next_check_after: i64,
    pub(crate) checkpoint_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pending_job_ref: Option<String>,
}

impl LifecyclePauseContext {
    pub(crate) fn new(
        resume_reason: impl Into<String>,
        next_check_after: i64,
        checkpoint_id: impl Into<String>,
        pending_job_ref: Option<String>,
    ) -> Self {
        Self {
            resume_reason: resume_reason.into(),
            next_check_after,
            checkpoint_id: checkpoint_id.into(),
            pending_job_ref,
        }
    }

    fn missing_required_fields(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.resume_reason.trim().is_empty() {
            missing.push("resume_reason");
        }
        if self.next_check_after <= 0 {
            missing.push("next_check_after");
        }
        if self.checkpoint_id.trim().is_empty() {
            missing.push("checkpoint_id");
        }
        missing
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LifecycleTransition {
    pub(crate) from: TaskLifecycleState,
    pub(crate) to: TaskLifecycleState,
    pub(crate) owner: LifecycleTransitionOwner,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pause: Option<LifecyclePauseContext>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LifecycleTransitionError {
    pub(crate) code: &'static str,
    pub(crate) missing_fields: Vec<&'static str>,
}

impl LifecycleTransition {
    pub(crate) fn validate(&self) -> Result<(), LifecycleTransitionError> {
        if self.from.is_terminal() {
            return Err(LifecycleTransitionError {
                code: "terminal_state_transition_rejected",
                missing_fields: Vec::new(),
            });
        }

        if matches!(
            self.to,
            TaskLifecycleState::Waiting | TaskLifecycleState::Background
        ) {
            let Some(pause) = self.pause.as_ref() else {
                return Err(LifecycleTransitionError {
                    code: "pause_context_required",
                    missing_fields: vec!["resume_reason", "next_check_after", "checkpoint_id"],
                });
            };
            let missing = pause.missing_required_fields();
            if !missing.is_empty() {
                return Err(LifecycleTransitionError {
                    code: "pause_context_incomplete",
                    missing_fields: missing,
                });
            }
        }

        if !transition_owner_allowed(self.from, self.to, self.owner) {
            return Err(LifecycleTransitionError {
                code: "transition_owner_not_allowed",
                missing_fields: Vec::new(),
            });
        }

        Ok(())
    }
}

fn transition_owner_allowed(
    from: TaskLifecycleState,
    to: TaskLifecycleState,
    owner: LifecycleTransitionOwner,
) -> bool {
    use LifecycleTransitionOwner as Owner;
    use TaskLifecycleState as State;

    match (from, to, owner) {
        (State::Queued, State::Running, Owner::Worker) => true,
        (State::Running, State::Waiting, Owner::AgentLoop | Owner::ToolAdapter) => true,
        (State::Running, State::Background, Owner::AgentLoop | Owner::ToolAdapter) => true,
        (State::Running, State::NeedsUser, Owner::AgentLoop | Owner::ConfirmationGate) => true,
        (State::Waiting, State::Running, Owner::Worker | Owner::RecoveryWorker) => true,
        (State::Background, State::Running, Owner::Worker | Owner::RecoveryWorker) => true,
        (State::NeedsUser, State::Running, Owner::ChannelUserAction) => true,
        (State::Running, State::Succeeded, Owner::AgentLoop | Owner::Worker) => true,
        (
            State::Running | State::Waiting | State::Background | State::NeedsUser,
            State::Failed,
            Owner::AgentLoop | Owner::ToolAdapter | Owner::RecoveryWorker | Owner::Worker,
        ) => true,
        (_, State::Cancelled, Owner::ChannelUserAction | Owner::Worker) => true,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CheckpointBudgetCounters {
    pub(crate) round: u32,
    pub(crate) step: u32,
    pub(crate) llm_calls: u32,
    pub(crate) tool_calls: u32,
    pub(crate) elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResumeEntrypoint {
    NextPlannerRound,
    PollAsyncJob,
    AwaitUserInput,
    VerifyAndFinalize,
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

    pub(crate) fn requires_checkpoint(self) -> bool {
        matches!(
            self,
            Self::UserFollowup | Self::ScheduledWakeup | Self::WorkerRecovery | Self::AsyncJobPoll
        )
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
    let mut lifecycle = result_json
        .and_then(extract_task_lifecycle_payload)
        .unwrap_or_else(|| fallback_task_lifecycle_payload(db_status));
    if let Some(obj) = lifecycle.as_object_mut() {
        obj.entry("schema_version".to_string()).or_insert(json!(1));
        obj.insert("db_status".to_string(), json!(db_status.trim()));
        obj.entry("can_poll".to_string()).or_insert(json!(true));
        let state = obj
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let active_state = lifecycle_state_token_is_active(&state);
        obj.entry("can_cancel".to_string())
            .or_insert(json!(active_state));
        append_pause_resume_due_fields(obj, &state, crate::now_ts_u64() as i64);
        if active_state {
            if let Some(updated_at_ts) = updated_at_ts.filter(|ts| *ts > 0) {
                obj.entry("last_heartbeat_ts".to_string())
                    .or_insert(json!(updated_at_ts));
            }
        }
    }
    lifecycle
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
                poll_after_seconds,
                expires_at,
                cancel_ref,
                message_key,
            } => json!({
                "status_code": self.status_code(),
                "checkpoint_id": checkpoint_id,
                "job_id": job_id,
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
    if !lifecycle_state_token_is_paused(&state) {
        return PausedCheckpointRecoveryStatus::NotPaused;
    }
    let Some(next_check_after) = lifecycle.get("next_check_after").and_then(Value::as_i64) else {
        return PausedCheckpointRecoveryStatus::InvalidPausedCheckpoint;
    };
    let Some(checkpoint_id) = lifecycle
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
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
                        if let Some(status_code) = pending_async_job_resume_blocker(&job, now_ts) {
                            CheckpointResumeDirective::NotReady { status_code }
                        } else {
                            CheckpointResumeDirective::PollAsyncJob {
                                checkpoint_id,
                                job_id: job.job_id.clone(),
                                poll_after_seconds: job.poll_after_seconds,
                                expires_at: job.expires_at,
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

fn pending_async_job_resume_blocker(job: &AsyncJobRef, now_ts: i64) -> Option<&'static str> {
    if !job.missing_required_fields().is_empty() {
        return Some("invalid_pending_async_job");
    }
    match job.status {
        AsyncJobStatus::Accepted | AsyncJobStatus::Running if job.expires_at <= now_ts => {
            Some("async_job_expired")
        }
        AsyncJobStatus::Accepted | AsyncJobStatus::Running => None,
        AsyncJobStatus::Succeeded => Some("async_job_observation_required"),
        AsyncJobStatus::Failed => Some("async_job_failed"),
        AsyncJobStatus::Expired => Some("async_job_expired"),
    }
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

fn extract_task_lifecycle_payload(result_json: &Value) -> Option<Value> {
    result_json
        .get("task_lifecycle")
        .filter(|value| value.is_object())
        .cloned()
        .or_else(|| {
            result_json
                .pointer("/task_journal/summary/task_lifecycle")
                .filter(|value| value.is_object())
                .cloned()
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

    pub(crate) fn should_skip_completed_side_effect(&self, fingerprint: &str) -> bool {
        let fingerprint = fingerprint.trim();
        !fingerprint.is_empty()
            && self
                .completed_side_effect_refs
                .iter()
                .any(|item| item == fingerprint)
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

    pub(crate) fn loop_directive(&self, now_ts: i64) -> AsyncJobLoopDirective {
        match self.status {
            AsyncJobStatus::Accepted | AsyncJobStatus::Running if self.expires_at <= now_ts => {
                AsyncJobLoopDirective::TerminalFailure
            }
            AsyncJobStatus::Accepted | AsyncJobStatus::Running => {
                AsyncJobLoopDirective::CheckpointAndPoll
            }
            AsyncJobStatus::Succeeded => AsyncJobLoopDirective::ResumeWithObservation,
            AsyncJobStatus::Failed | AsyncJobStatus::Expired => {
                AsyncJobLoopDirective::TerminalFailure
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AsyncJobLoopDirective {
    CheckpointAndPoll,
    ResumeWithObservation,
    TerminalFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProviderGapDecision {
    pub(crate) status_code: String,
    pub(crate) failure_attribution: String,
    pub(crate) retryable: bool,
    pub(crate) raw_error_present: bool,
    pub(crate) message_key: String,
    pub(crate) target_state: TaskLifecycleState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pause: Option<LifecyclePauseContext>,
}

pub(crate) fn provider_gap_decision(
    retryable: bool,
    next_check_after: Option<i64>,
    checkpoint_id: impl Into<String>,
) -> ProviderGapDecision {
    let checkpoint_id = checkpoint_id.into();
    let pause = retryable.then(|| {
        LifecyclePauseContext::new(
            "provider_gap_retry_window",
            next_check_after.unwrap_or(1).max(1),
            checkpoint_id,
            None,
        )
    });
    ProviderGapDecision {
        status_code: "provider_gap".to_string(),
        failure_attribution: "provider_error".to_string(),
        retryable,
        raw_error_present: true,
        message_key: "clawd.msg.ask_runtime_failure".to_string(),
        target_state: if retryable {
            TaskLifecycleState::Waiting
        } else {
            TaskLifecycleState::Failed
        },
        pause,
    }
}

#[cfg(test)]
#[path = "task_lifecycle_tests.rs"]
mod tests;
