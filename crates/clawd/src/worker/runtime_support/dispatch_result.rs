use serde_json::{json, Value};

use crate::{repo, AppState};

use super::{
    checkpoint_resume_entrypoint_token, ClaimedPausedCheckpointResumeHandoffDispatch,
    PlannedPausedCheckpointResumeExecutorHandoff,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PausedCheckpointDispatchResultRecord {
    Recorded { executor_result_status: String },
    NotRecorded { executor_result_status: String },
    DeferredToConcreteExecutor,
}

pub(crate) fn planned_paused_checkpoint_resume_executor_handoff(
    execution_plan: &Value,
) -> Option<PlannedPausedCheckpointResumeExecutorHandoff> {
    if !execution_plan.is_object()
        || execution_plan.get("text").is_some()
        || execution_plan.get("error_text").is_some()
    {
        return None;
    }
    let executor_action = execution_plan
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let checkpoint_id = execution_plan
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let executor_state = execution_plan
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let executor_status = match executor_action {
        "run_seeded_agent_loop" => "seeded_loop_requires_provider_window",
        "poll_async_job" => {
            let job_id = execution_plan
                .get("job_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            if job_id.is_empty() {
                return None;
            }
            "async_poll_adapter_pending"
        }
        "verify_and_finalize" => "checkpoint_finalize_executor_pending",
        _ => return None,
    };
    Some(PlannedPausedCheckpointResumeExecutorHandoff {
        executor_action: executor_action.to_string(),
        executor_status,
        checkpoint_id: checkpoint_id.to_string(),
        executor_state: executor_state.to_string(),
        payload: json!({
            "schema_version": 1,
            "executor_action": executor_action,
            "executor_status": executor_status,
            "checkpoint_id": checkpoint_id,
            "executor_state": executor_state,
        }),
    })
}

pub(crate) fn dispatch_claimed_paused_checkpoint_resume_handoff(
    claimed: &repo::ClaimedHandoffPausedCheckpointResumeExecution,
) -> Option<ClaimedPausedCheckpointResumeHandoffDispatch> {
    if claimed.task_checkpoint.checkpoint_id != claimed.checkpoint_id
        || claimed.execution_plan.get("text").is_some()
        || claimed.execution_plan.get("error_text").is_some()
        || claimed.handoff_payload.get("text").is_some()
        || claimed.handoff_payload.get("error_text").is_some()
        || claimed.handoff_claim.get("text").is_some()
        || claimed.handoff_claim.get("error_text").is_some()
    {
        return None;
    }
    let dispatch_state = match (
        claimed.executor_action.as_str(),
        claimed.executor_status.as_str(),
        claimed.resume_directive.as_str(),
        &claimed.task_checkpoint.resume_entrypoint,
    ) {
        (
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            "run_next_planner_round",
            crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
        ) => "ready_to_run_seeded_agent_loop",
        (
            "poll_async_job",
            "async_poll_adapter_pending",
            "poll_async_job",
            crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob,
        ) => "ready_to_poll_async_job",
        (
            "verify_and_finalize",
            "checkpoint_finalize_executor_pending",
            "verify_and_finalize",
            crate::task_lifecycle::ResumeEntrypoint::VerifyAndFinalize,
        ) => "ready_to_verify_and_finalize",
        _ => return None,
    };
    let completed_side_effect_count = claimed.task_checkpoint.completed_side_effect_refs.len();
    let requires_idempotency_guard = claimed
        .execution_plan
        .get("requires_idempotency_guard")
        .and_then(Value::as_bool)
        .unwrap_or(completed_side_effect_count > 0);
    let mut payload = json!({
        "schema_version": 1,
        "task_id": claimed.task_id,
        "checkpoint_id": claimed.checkpoint_id,
        "executor_action": claimed.executor_action,
        "executor_state": claimed.executor_state,
        "executor_status": claimed.executor_status,
        "dispatch_state": dispatch_state,
        "resume_directive": claimed.resume_directive,
        "resume_trigger": claimed.resume_trigger,
        "lease_expires_at": claimed.lease_expires_at,
        "handoff_claim_expires_at": claimed.handoff_claim_expires_at,
        "task_kind": claimed.task.kind,
        "task_channel": claimed.task.channel,
        "task_payload_bytes": claimed.task.payload_json.len(),
        "resume_entrypoint": checkpoint_resume_entrypoint_token(&claimed.task_checkpoint.resume_entrypoint),
        "completed_side_effect_count": completed_side_effect_count,
        "requires_idempotency_guard": requires_idempotency_guard,
    });
    if claimed.executor_action == "poll_async_job" {
        let job_id = claimed
            .execution_plan
            .get("job_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("job_id".to_string(), json!(job_id));
            for key in ["cancel_ref", "message_key"] {
                if let Some(value) = claimed.execution_plan.get(key).and_then(Value::as_str) {
                    obj.insert(key.to_string(), json!(value));
                }
            }
            for key in ["poll_after_seconds", "expires_at"] {
                if let Some(value) = claimed.execution_plan.get(key).and_then(Value::as_i64) {
                    obj.insert(key.to_string(), json!(value));
                }
            }
        }
    }

    Some(ClaimedPausedCheckpointResumeHandoffDispatch {
        task: claimed.task.clone(),
        executor_action: claimed.executor_action.clone(),
        executor_status: claimed.executor_status.clone(),
        dispatch_state,
        checkpoint_id: claimed.checkpoint_id.clone(),
        executor_state: claimed.executor_state.clone(),
        payload,
    })
}

pub(crate) fn paused_checkpoint_resume_reschedule_projection_payload(
    claimed: &repo::ClaimedPausedCheckpointResumeDispatchResult,
) -> Option<Value> {
    match (
        claimed.executor_action.as_str(),
        claimed.executor_result_status.as_str(),
    ) {
        ("run_seeded_agent_loop", "seeded_loop_deferred")
        | ("poll_async_job", "async_poll_rescheduled") => {}
        _ => return None,
    }

    if claimed.execution_result_payload.get("text").is_some()
        || claimed.execution_result_payload.get("error_text").is_some()
    {
        return None;
    }
    if claimed
        .execution_result_payload
        .get("next_check_after")
        .and_then(Value::as_i64)
        .is_none()
        && claimed
            .execution_result_payload
            .get("retry_after_seconds")
            .and_then(Value::as_i64)
            .filter(|seconds| *seconds > 0)
            .is_none()
    {
        return None;
    }

    let mut payload = claimed.execution_result_payload.clone();
    let Some(obj) = payload.as_object_mut() else {
        return None;
    };
    obj.insert("schema_version".to_string(), json!(1));
    obj.insert("task_id".to_string(), json!(claimed.task_id));
    obj.insert("checkpoint_id".to_string(), json!(claimed.checkpoint_id));
    obj.insert("executor_state".to_string(), json!(claimed.executor_state));
    obj.insert(
        "executor_action".to_string(),
        json!(claimed.executor_action),
    );
    obj.insert(
        "executor_status".to_string(),
        json!(claimed.executor_status),
    );
    obj.insert("dispatch_state".to_string(), json!(claimed.dispatch_state));
    obj.insert(
        "executor_result_status".to_string(),
        json!(claimed.executor_result_status),
    );
    obj.insert(
        "result_projection_state".to_string(),
        json!(claimed.result_projection_state),
    );
    Some(payload)
}

pub(crate) fn paused_checkpoint_resume_terminal_projection_payload(
    claimed: &repo::ClaimedPausedCheckpointResumeDispatchResult,
) -> Option<Value> {
    match (
        claimed.executor_action.as_str(),
        claimed.executor_result_status.as_str(),
    ) {
        ("run_seeded_agent_loop", "seeded_loop_completed")
        | ("run_seeded_agent_loop", "seeded_loop_failed")
        | ("poll_async_job", "async_poll_completed")
        | ("poll_async_job", "async_poll_failed")
        | ("verify_and_finalize", "finalize_completed")
        | ("verify_and_finalize", "finalize_failed") => {}
        _ => return None,
    }

    if claimed.execution_result_payload.get("text").is_some()
        || claimed.execution_result_payload.get("error_text").is_some()
    {
        return None;
    }
    if matches!(
        claimed.executor_result_status.as_str(),
        "seeded_loop_completed" | "async_poll_completed" | "finalize_completed"
    ) && !claimed
        .execution_result_payload
        .get("final_result_json")
        .is_some_and(Value::is_object)
    {
        return None;
    }
    if matches!(
        claimed.executor_result_status.as_str(),
        "seeded_loop_failed" | "async_poll_failed" | "finalize_failed"
    ) && claimed
        .execution_result_payload
        .get("error_code")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            claimed
                .execution_result_payload
                .get("message_key")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .is_none()
    {
        return None;
    }

    let mut payload = claimed.execution_result_payload.clone();
    let Some(obj) = payload.as_object_mut() else {
        return None;
    };
    obj.insert("schema_version".to_string(), json!(1));
    obj.insert("task_id".to_string(), json!(claimed.task_id));
    obj.insert("checkpoint_id".to_string(), json!(claimed.checkpoint_id));
    obj.insert("executor_state".to_string(), json!(claimed.executor_state));
    obj.insert(
        "executor_action".to_string(),
        json!(claimed.executor_action),
    );
    obj.insert(
        "executor_status".to_string(),
        json!(claimed.executor_status),
    );
    obj.insert("dispatch_state".to_string(), json!(claimed.dispatch_state));
    obj.insert(
        "executor_result_status".to_string(),
        json!(claimed.executor_result_status),
    );
    obj.insert(
        "result_projection_state".to_string(),
        json!(claimed.result_projection_state),
    );
    Some(payload)
}

pub(crate) fn record_paused_checkpoint_resume_dispatch_result(
    state: &AppState,
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    now_ts: i64,
    retry_after_seconds: i64,
) -> anyhow::Result<PausedCheckpointDispatchResultRecord> {
    let Some(result_payload) =
        paused_checkpoint_resume_dispatch_result_payload(claimed, now_ts, retry_after_seconds)
    else {
        return Ok(PausedCheckpointDispatchResultRecord::DeferredToConcreteExecutor);
    };
    let executor_result_status = result_payload
        .get("executor_result_status")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string();
    let recorded =
        repo::record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal(
            state,
            &claimed.task_id,
            &claimed.checkpoint_id,
            &claimed.executor_state,
            &claimed.executor_action,
            &claimed.executor_status,
            &claimed.dispatch_state,
            &result_payload,
            now_ts,
        )?;
    if recorded {
        Ok(PausedCheckpointDispatchResultRecord::Recorded {
            executor_result_status,
        })
    } else {
        Ok(PausedCheckpointDispatchResultRecord::NotRecorded {
            executor_result_status,
        })
    }
}

pub(crate) fn record_concrete_paused_checkpoint_resume_dispatch_result(
    state: &AppState,
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    result_payload: &Value,
    now_ts: i64,
) -> anyhow::Result<PausedCheckpointDispatchResultRecord> {
    let executor_result_status = result_payload
        .get("executor_result_status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let recorded =
        repo::record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal(
            state,
            &claimed.task_id,
            &claimed.checkpoint_id,
            &claimed.executor_state,
            &claimed.executor_action,
            &claimed.executor_status,
            &claimed.dispatch_state,
            result_payload,
            now_ts,
        )?;
    if recorded {
        Ok(PausedCheckpointDispatchResultRecord::Recorded {
            executor_result_status,
        })
    } else {
        Ok(PausedCheckpointDispatchResultRecord::NotRecorded {
            executor_result_status,
        })
    }
}

pub(crate) fn paused_checkpoint_resume_dispatch_result_payload(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    now_ts: i64,
    retry_after_seconds: i64,
) -> Option<Value> {
    if claimed.execution_plan.get("text").is_some()
        || claimed.execution_plan.get("error_text").is_some()
        || claimed.dispatch_payload.get("text").is_some()
        || claimed.dispatch_payload.get("error_text").is_some()
        || claimed.dispatch_claim.get("text").is_some()
        || claimed.dispatch_claim.get("error_text").is_some()
        || claimed.task_checkpoint.checkpoint_id != claimed.checkpoint_id
    {
        return None;
    }

    let (mut executor_result_status, mut reason_code) = match (
        claimed.executor_action.as_str(),
        claimed.executor_status.as_str(),
        claimed.dispatch_state.as_str(),
        claimed.dispatch_execution_state.as_str(),
        claimed.resume_directive.as_str(),
    ) {
        (
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            "ready_to_run_seeded_agent_loop",
            "claimed_to_run_seeded_agent_loop",
            "run_next_planner_round",
        ) => (
            "seeded_loop_deferred",
            "seeded_loop_executor_pending_integration",
        ),
        (
            "poll_async_job",
            "async_poll_adapter_pending",
            "ready_to_poll_async_job",
            "claimed_to_poll_async_job",
            "poll_async_job",
        ) => ("async_poll_rescheduled", "async_poll_adapter_pending"),
        (
            "verify_and_finalize",
            "checkpoint_finalize_executor_pending",
            "ready_to_verify_and_finalize",
            "claimed_to_verify_and_finalize",
            "verify_and_finalize",
        ) => (
            "finalize_failed",
            "checkpoint_finalize_missing_final_result",
        ),
        _ => return None,
    };
    let mut terminal_error: Option<(&'static str, &'static str)> = None;
    let mut terminal_result_json: Option<Value> = None;
    let poll_expires_at = (claimed.executor_action == "poll_async_job")
        .then(|| {
            claimed
                .execution_plan
                .get("expires_at")
                .or_else(|| claimed.dispatch_payload.get("expires_at"))
                .and_then(Value::as_i64)
        })
        .flatten();
    if claimed.executor_action == "poll_async_job" {
        match poll_expires_at {
            Some(expires_at) if expires_at <= now_ts => {
                executor_result_status = "async_poll_failed";
                reason_code = "async_poll_expired";
                terminal_error = Some(("async_poll_expired", "clawd.task.async_poll_expired"));
            }
            Some(_) => {}
            None => {
                executor_result_status = "async_poll_failed";
                reason_code = "async_poll_invalid_contract";
                terminal_error = Some((
                    "async_poll_invalid_contract",
                    "clawd.task.async_poll_invalid_contract",
                ));
            }
        }
    }
    if claimed.executor_action == "verify_and_finalize" {
        if let Some(final_result_json) = checkpoint_finalize_final_result_json(claimed) {
            executor_result_status = "finalize_completed";
            reason_code = "checkpoint_finalize_completed";
            terminal_result_json = Some(final_result_json);
        } else {
            terminal_error = Some((
                "checkpoint_finalize_missing_final_result",
                "clawd.task.checkpoint_finalize_missing_final_result",
            ));
        }
    }

    let retry_after_seconds = retry_after_seconds.max(1);
    let mut next_check_after = now_ts.saturating_add(retry_after_seconds);
    if terminal_error.is_none() && terminal_result_json.is_none() {
        if let Some(expires_at) = poll_expires_at.filter(|expires_at| *expires_at > now_ts) {
            next_check_after = next_check_after.min(expires_at);
        }
    }
    let mut payload = json!({
        "schema_version": 1,
        "task_id": claimed.task_id,
        "checkpoint_id": claimed.checkpoint_id,
        "executor_state": claimed.executor_state,
        "executor_action": claimed.executor_action,
        "executor_status": claimed.executor_status,
        "dispatch_state": claimed.dispatch_state,
        "dispatch_execution_state": claimed.dispatch_execution_state,
        "executor_result_status": executor_result_status,
        "reason_code": reason_code,
        "resume_trigger": claimed.resume_trigger,
        "resume_directive": claimed.resume_directive,
        "lease_expires_at": claimed.lease_expires_at,
        "handoff_claim_expires_at": claimed.handoff_claim_expires_at,
        "dispatch_claim_expires_at": claimed.dispatch_claim_expires_at,
        "completed_side_effect_count": claimed.task_checkpoint.completed_side_effect_refs.len(),
    });
    if terminal_error.is_none() && terminal_result_json.is_none() {
        let obj = payload.as_object_mut()?;
        obj.insert("defer_reason_code".to_string(), json!(reason_code));
        obj.insert(
            "retry_after_seconds".to_string(),
            json!(retry_after_seconds),
        );
        obj.insert("next_check_after".to_string(), json!(next_check_after));
    }
    if let Some((error_code, message_key)) = terminal_error {
        let obj = payload.as_object_mut()?;
        obj.insert("error_code".to_string(), json!(error_code));
        obj.insert("message_key".to_string(), json!(message_key));
    }
    if let Some(final_result_json) = terminal_result_json {
        let obj = payload.as_object_mut()?;
        obj.insert("final_result_json".to_string(), final_result_json);
    }

    if claimed.executor_action == "poll_async_job" {
        let job_id = claimed
            .execution_plan
            .get("job_id")
            .or_else(|| claimed.dispatch_payload.get("job_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let obj = payload.as_object_mut()?;
        obj.insert("job_id".to_string(), json!(job_id));
        for key in ["cancel_ref", "message_key"] {
            if key == "message_key" && terminal_error.is_some() {
                continue;
            }
            if let Some(value) = claimed
                .execution_plan
                .get(key)
                .or_else(|| claimed.dispatch_payload.get(key))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                obj.insert(key.to_string(), json!(value));
            }
        }
        for key in ["poll_after_seconds", "expires_at"] {
            if let Some(value) = claimed
                .execution_plan
                .get(key)
                .or_else(|| claimed.dispatch_payload.get(key))
                .and_then(Value::as_i64)
            {
                obj.insert(key.to_string(), json!(value));
            }
        }
    }

    Some(payload)
}

fn checkpoint_finalize_final_result_json(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
) -> Option<Value> {
    [
        &claimed.execution_plan,
        &claimed.dispatch_payload,
        &claimed.dispatch_claim,
    ]
    .into_iter()
    .find_map(final_result_json_from_machine_value)
    .or_else(|| {
        claimed
            .task_checkpoint
            .pending_action
            .as_ref()
            .and_then(final_result_json_from_machine_value)
    })
    .or_else(|| {
        claimed
            .task_checkpoint
            .observations
            .iter()
            .find_map(final_result_json_from_machine_value)
    })
}

fn final_result_json_from_machine_value(value: &Value) -> Option<Value> {
    final_result_json_object(value)
        .or_else(|| task_journal_final_answer_result_json(value))
        .or_else(|| final_answer_field_result_json(value))
        .or_else(|| answer_object_result_json(value))
}

fn final_result_json_object(value: &Value) -> Option<Value> {
    value
        .get("final_result_json")
        .filter(|candidate| candidate.is_object())
        .cloned()
}

fn task_journal_final_answer_result_json(value: &Value) -> Option<Value> {
    let task_journal = value
        .get("task_journal")
        .filter(|candidate| candidate.is_object())?;
    let final_answer = task_journal
        .get("summary")
        .and_then(|summary| summary.get("final_answer"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|answer| !answer.is_empty())?;
    let mut result = final_result_json_from_text(final_answer, value);
    if let Some(obj) = result.as_object_mut() {
        obj.insert("task_journal".to_string(), task_journal.clone());
    }
    Some(result)
}

fn final_answer_field_result_json(value: &Value) -> Option<Value> {
    let final_answer = value
        .get("final_answer")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|answer| !answer.is_empty())?;
    Some(final_result_json_from_text(final_answer, value))
}

fn answer_object_result_json(value: &Value) -> Option<Value> {
    let answer = value
        .get("answer")
        .filter(|candidate| candidate.is_object())?;
    let text = answer
        .get("text")
        .or_else(|| answer.get("final_answer"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|answer| !answer.is_empty())?;
    Some(final_result_json_from_text(text, answer))
}

fn final_result_json_from_text(text: &str, source: &Value) -> Value {
    let mut result = json!({ "text": text });
    if let (Some(obj), Some(messages)) = (
        result.as_object_mut(),
        source
            .get("messages")
            .filter(|messages| string_array_is_non_empty(messages)),
    ) {
        obj.insert("messages".to_string(), messages.clone());
    }
    result
}

fn string_array_is_non_empty(value: &Value) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    !items.is_empty()
        && items.iter().all(|item| {
            item.as_str()
                .map(str::trim)
                .is_some_and(|message| !message.is_empty())
        })
}

pub(crate) fn seeded_agent_loop_terminal_dispatch_result_payload(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    result: Result<crate::AskReply, String>,
) -> Option<Value> {
    if !claimed_seeded_agent_loop_dispatch_matches(claimed) {
        return None;
    }
    match result {
        Ok(answer) if answer.should_fail_task => Some(seeded_agent_loop_failure_payload(
            claimed,
            "seeded_loop_answer_marked_failed",
            "clawd.task.seeded_loop_answer_marked_failed",
            answer
                .error_text
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
        )),
        Ok(answer) => {
            let mut payload = seeded_agent_loop_base_payload(claimed, "seeded_loop_completed");
            let obj = payload.as_object_mut()?;
            obj.insert(
                "final_result_json".to_string(),
                ask_reply_final_result_json(answer),
            );
            Some(payload)
        }
        Err(_) => Some(seeded_agent_loop_failure_payload(
            claimed,
            "seeded_loop_runtime_error",
            "clawd.task.seeded_loop_runtime_error",
            true,
        )),
    }
}

fn claimed_seeded_agent_loop_dispatch_matches(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
) -> bool {
    claimed.task_checkpoint.checkpoint_id == claimed.checkpoint_id
        && claimed.executor_action == "run_seeded_agent_loop"
        && claimed.executor_status == "seeded_loop_requires_provider_window"
        && claimed.dispatch_state == "ready_to_run_seeded_agent_loop"
        && claimed.dispatch_execution_state == "claimed_to_run_seeded_agent_loop"
        && claimed.resume_directive == "run_next_planner_round"
        && matches!(
            claimed.task_checkpoint.resume_entrypoint,
            crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound
        )
        && claimed.execution_plan.get("text").is_none()
        && claimed.execution_plan.get("error_text").is_none()
        && claimed.dispatch_payload.get("text").is_none()
        && claimed.dispatch_payload.get("error_text").is_none()
        && claimed.dispatch_claim.get("text").is_none()
        && claimed.dispatch_claim.get("error_text").is_none()
}

fn seeded_agent_loop_base_payload(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    executor_result_status: &'static str,
) -> Value {
    json!({
        "schema_version": 1,
        "task_id": claimed.task_id,
        "checkpoint_id": claimed.checkpoint_id,
        "executor_state": claimed.executor_state,
        "executor_action": claimed.executor_action,
        "executor_status": claimed.executor_status,
        "dispatch_state": claimed.dispatch_state,
        "dispatch_execution_state": claimed.dispatch_execution_state,
        "executor_result_status": executor_result_status,
        "resume_trigger": claimed.resume_trigger,
        "resume_directive": claimed.resume_directive,
        "lease_expires_at": claimed.lease_expires_at,
        "handoff_claim_expires_at": claimed.handoff_claim_expires_at,
        "dispatch_claim_expires_at": claimed.dispatch_claim_expires_at,
        "completed_side_effect_count": claimed.task_checkpoint.completed_side_effect_refs.len(),
    })
}

fn seeded_agent_loop_failure_payload(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    error_code: &'static str,
    message_key: &'static str,
    source_error_present: bool,
) -> Value {
    let mut payload = seeded_agent_loop_base_payload(claimed, "seeded_loop_failed");
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("error_code".to_string(), json!(error_code));
        obj.insert("message_key".to_string(), json!(message_key));
        obj.insert(
            "source_error_present".to_string(),
            json!(source_error_present),
        );
    }
    payload
}

fn ask_reply_final_result_json(mut answer: crate::AskReply) -> Value {
    let mut result = if answer.messages.is_empty() {
        json!({ "text": answer.text })
    } else {
        json!({ "text": answer.text, "messages": answer.messages })
    };
    if let (Some(obj), Some(resume_context)) =
        (result.as_object_mut(), answer.resume_context.take())
    {
        obj.insert("resume_context".to_string(), resume_context);
    }
    match answer.task_journal.as_ref() {
        Some(journal) => journal.attach_to_result(result),
        None => result,
    }
}
