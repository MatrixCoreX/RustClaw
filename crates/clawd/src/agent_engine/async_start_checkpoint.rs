use serde_json::{json, Value};
use tracing::{debug, warn};

use super::support::checkpoint_budget_counters;
use super::LoopState;
use crate::task_lifecycle::{
    AsyncJobRef, CheckpointBudgetCounters, ResumeEntrypoint, TaskCheckpoint, TaskLifecycleState,
};
use crate::{repo, AppState, ClaimedTask};

const START_ADAPTER_SOURCE: &str = "async_job_start_adapter";
const WAITING_STOP_SIGNAL: &str = "async_job_checkpoint_waiting";
const START_ADAPTER_ERROR_PREFIX: &str = "async_job_start_adapter_invalid";

pub(super) fn publish_pending_async_job_start_checkpoint(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    normalized_skill: &str,
    global_step: usize,
    step_in_round: usize,
    structured_extra: Option<&Value>,
) -> Result<Option<String>, String> {
    let Some(job) = pending_async_job_ref_from_extra(structured_extra)? else {
        return Ok(None);
    };
    let poll_adapter = pending_async_job_poll_adapter_from_extra(structured_extra)?;
    if let Some(slice) = loop_state.task_budget_slice.as_mut() {
        slice.set_decision(crate::task_budget_contract::BudgetDecision::Waiting);
    }
    let now_ts = crate::now_ts_u64() as i64;
    let payload = build_pending_async_job_checkpoint_progress_payload(
        task,
        loop_state,
        normalized_skill,
        global_step,
        step_in_round,
        &job,
        poll_adapter.as_ref(),
        now_ts,
        checkpoint_budget_counters(
            loop_state,
            state.task_llm_call_count(&task.task_id),
            state.task_llm_elapsed_ms(&task.task_id),
        ),
    );
    loop_state.task_lifecycle = payload.get("task_lifecycle").cloned();
    loop_state.task_checkpoint = payload.get("task_checkpoint").cloned();
    loop_state.output_vars.insert(
        "agent_loop.resume_reason".to_string(),
        "pending_async_job".to_string(),
    );
    loop_state
        .output_vars
        .insert("agent_loop.async_job_id".to_string(), job.job_id.clone());
    if let Some(checkpoint_id) = payload
        .pointer("/task_lifecycle/checkpoint_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
    {
        loop_state.output_vars.insert(
            "agent_loop.checkpoint_id".to_string(),
            checkpoint_id.clone(),
        );
        loop_state.history_compact.push(format!(
            "round={} step={} skill={} async_job_checkpoint checkpoint_id={} job_id={}",
            loop_state.round_no, step_in_round, normalized_skill, checkpoint_id, job.job_id
        ));
    }
    repo::update_task_progress_result(
        state,
        &task.task_id,
        task.claim_attempt,
        &payload.to_string(),
    )
    .map_err(|err| {
        warn!(
            "async_start_checkpoint_publish_failed task_id={} skill={} err={}",
            task.task_id, normalized_skill, err
        );
        format!("async_job_start_checkpoint_publish_failed: {err}")
    })?;
    if let Some(visible_reply) = pending_async_job_visible_reply_from_progress_payload(&payload) {
        if let Some(step) = loop_state.executed_step_results.last_mut() {
            step.output = Some(visible_reply);
        }
    }
    debug!(
        "async_start_checkpoint_published task_id={} skill={} job_id={} poll_after_seconds={}",
        task.task_id, normalized_skill, job.job_id, job.poll_after_seconds
    );
    Ok(Some(WAITING_STOP_SIGNAL.to_string()))
}

fn pending_async_job_ref_from_extra(extra: Option<&Value>) -> Result<Option<AsyncJobRef>, String> {
    crate::async_job_contract::parse_pending_async_job_ref_from_extra(
        extra,
        START_ADAPTER_ERROR_PREFIX,
    )
}

fn pending_async_job_poll_adapter_from_extra(
    extra: Option<&Value>,
) -> Result<Option<Value>, String> {
    crate::async_job_contract::parse_pending_async_job_poll_adapter_from_extra(
        extra,
        START_ADAPTER_ERROR_PREFIX,
    )
}

fn build_pending_async_job_checkpoint_progress_payload(
    task: &ClaimedTask,
    loop_state: &LoopState,
    normalized_skill: &str,
    global_step: usize,
    step_in_round: usize,
    job: &AsyncJobRef,
    poll_adapter: Option<&Value>,
    now_ts: i64,
    budget: CheckpointBudgetCounters,
) -> Value {
    let timeout_policy =
        crate::async_job_contract::pending_async_job_timeout_policy(poll_adapter, job, now_ts);
    let mut checkpoint_budget = budget.clone();
    checkpoint_budget.step = saturating_u32(global_step);
    let budget_json = serde_json::to_value(&checkpoint_budget).unwrap_or_else(|_| json!({}));
    let checkpoint_id = format!(
        "agent-loop:{}:round-{}:step-{}:async-job:{}",
        task.task_id, loop_state.round_no, global_step, job.job_id
    );
    let last_successful_step = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.is_ok())
        .map(|step| step.step_id.clone());
    let evidence_refs = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok())
        .map(|step| step.step_id.clone())
        .collect::<Vec<_>>();
    let mut boundary_context = json!({
        "schema_version": 1,
        "source": START_ADAPTER_SOURCE,
        "task_id": task.task_id,
        "skill": normalized_skill,
        "global_step": global_step,
        "step_in_round": step_in_round,
        "agent_loop_resume_state":
            super::checkpoint_resume_state::build_checkpoint_resume_state(
                loop_state,
                super::checkpoint_resume_state::AgentCheckpointStage::ToolExecution,
            ),
        "task_budget_slice": loop_state
            .task_budget_slice
            .as_ref()
            .map(crate::task_budget_contract::TaskBudgetSlice::to_machine_json),
    });
    if let (Some(obj), Some(adapter)) = (
        boundary_context.as_object_mut(),
        poll_adapter.filter(|value| value.is_object()),
    ) {
        obj.insert("async_poll_adapter".to_string(), adapter.clone());
    }

    let checkpoint = TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.clone(),
        boundary_context,
        last_successful_round: (loop_state.round_no > 0)
            .then_some(saturating_u32(loop_state.round_no)),
        last_successful_step,
        pending_action: None,
        observations: checkpoint_step_observations(loop_state),
        capability_results: loop_state.capability_results.clone(),
        evidence_refs,
        artifact_refs: Vec::new(),
        completed_side_effect_refs: completed_side_effect_refs(loop_state),
        budget: checkpoint_budget,
        attempt_ledger: super::attempt_ledger::build_attempt_ledger_snapshot(loop_state),
        pending_async_job: Some(job.clone()),
        repair_signal: None,
        resume_entrypoint: ResumeEntrypoint::PollAsyncJob,
    };

    json!({
        "progress_messages": loop_state.progress_messages,
        "task_lifecycle": {
            "schema_version": 1,
            "state": TaskLifecycleState::Waiting,
            "source": START_ADAPTER_SOURCE,
            "resume_reason": "pending_async_job",
            "next_check_after": now_ts.saturating_add(job.poll_after_seconds as i64).max(now_ts + 1),
            "checkpoint_id": checkpoint_id,
            "poll_ref": job.job_id,
            "cancel_ref": job.cancel_ref,
            "poll_after_seconds": job.poll_after_seconds,
            "async_job_expires_at": job.expires_at,
            "async_job_message_key": job.message_key,
            "async_timeout_policy": timeout_policy,
            "budget": budget_json,
            "task_budget_slice": loop_state
                .task_budget_slice
                .as_ref()
                .map(crate::task_budget_contract::TaskBudgetSlice::to_machine_json),
            "can_poll": true,
            "can_cancel": true,
            "last_heartbeat_ts": now_ts,
        },
        "task_checkpoint": checkpoint.to_machine_json(),
    })
}

fn pending_async_job_visible_reply_from_progress_payload(payload: &Value) -> Option<String> {
    let lifecycle = payload.get("task_lifecycle")?.as_object()?;
    let checkpoint_id = lifecycle
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let poll_ref = lifecycle
        .get("poll_ref")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let next_check_after = lifecycle.get("next_check_after")?.clone();
    let mut reply = json!({
        "schema_version": 1,
        "output_format": "machine_json",
        "status": "accepted",
        "checkpoint_id": checkpoint_id,
        "poll_ref": poll_ref,
        "next_check_after": next_check_after,
    });
    if let Some(obj) = reply.as_object_mut() {
        for key in [
            "poll_after_seconds",
            "async_job_expires_at",
            "async_job_message_key",
            "async_timeout_policy",
            "can_poll",
            "can_cancel",
            "cancel_ref",
        ] {
            if let Some(value) = lifecycle.get(key) {
                obj.insert(key.to_string(), value.clone());
            }
        }
    }
    Some(reply.to_string())
}

fn checkpoint_step_observations(loop_state: &LoopState) -> Vec<Value> {
    let mut observations = loop_state
        .executed_step_results
        .iter()
        .rev()
        .take(8)
        .map(|step| {
            json!({
                "step_id": step.step_id,
                "skill": step.skill,
                "status": step.status.as_str(),
                "has_output": step.output.as_deref().is_some_and(|value| !value.trim().is_empty()),
                "has_error": step.error.as_deref().is_some_and(|value| !value.trim().is_empty()),
            })
        })
        .collect::<Vec<_>>();
    observations.reverse();
    observations
}

fn completed_side_effect_refs(loop_state: &LoopState) -> Vec<String> {
    let mut refs = loop_state
        .successful_action_fingerprints
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    refs.sort();
    refs
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[path = "async_start_checkpoint_tests.rs"]
mod tests;
