use std::time::Duration;

use anyhow::anyhow;
use rusqlite::Connection;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::{now_ts, now_ts_u64, repo, schedule_service, AppState, ScheduledJobDue};

#[path = "runtime_support/dispatch_result.rs"]
mod dispatch_result;
#[cfg(test)]
pub(super) use dispatch_result::paused_checkpoint_resume_dispatch_result_payload;
pub(super) use dispatch_result::{
    dispatch_claimed_paused_checkpoint_resume_handoff,
    paused_checkpoint_resume_reschedule_projection_payload,
    paused_checkpoint_resume_terminal_projection_payload,
    planned_paused_checkpoint_resume_executor_handoff,
    record_concrete_paused_checkpoint_resume_dispatch_result,
    record_paused_checkpoint_resume_dispatch_result,
    seeded_agent_loop_terminal_dispatch_result_payload, PausedCheckpointDispatchResultRecord,
};

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
pub(super) struct PausedCheckpointResumeExecutionDecision {
    pub(super) executor_state: &'static str,
    pub(super) lifecycle_state: Option<&'static str>,
    pub(super) next_check_after: Option<i64>,
    pub(super) payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ClaimedPausedCheckpointResumeExecutionPlan {
    pub(super) task: crate::ClaimedTask,
    pub(super) executor_action: &'static str,
    pub(super) executor_state: String,
    pub(super) resume_directive: String,
    pub(super) checkpoint_id: String,
    pub(super) payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PlannedPausedCheckpointResumeExecutorHandoff {
    pub(super) executor_action: String,
    pub(super) executor_status: &'static str,
    pub(super) checkpoint_id: String,
    pub(super) executor_state: String,
    pub(super) payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ClaimedPausedCheckpointResumeHandoffDispatch {
    pub(super) task: crate::ClaimedTask,
    pub(super) executor_action: String,
    pub(super) executor_status: String,
    pub(super) dispatch_state: &'static str,
    pub(super) checkpoint_id: String,
    pub(super) executor_state: String,
    pub(super) payload: Value,
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

fn checkpoint_resume_entrypoint_token(
    entrypoint: &crate::task_lifecycle::ResumeEntrypoint,
) -> &'static str {
    match entrypoint {
        crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound => "next_planner_round",
        crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob => "poll_async_job",
        crate::task_lifecycle::ResumeEntrypoint::AwaitUserInput => "await_user_input",
        crate::task_lifecycle::ResumeEntrypoint::VerifyAndFinalize => "verify_and_finalize",
    }
}

pub(super) fn plan_claimed_paused_checkpoint_resume_execution(
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

pub(super) fn prepare_paused_checkpoint_resume_execution(
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

pub(super) fn build_paused_checkpoint_resume_work_item(
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

fn recovery_should_preserve_paused_checkpoint(result_json: Option<&str>, now: i64) -> bool {
    let Some(result_json) = result_json.and_then(|raw| serde_json::from_str::<Value>(raw).ok())
    else {
        return false;
    };
    crate::task_lifecycle::paused_checkpoint_recovery_status(&result_json, now)
        .preserve_running_status_for_recovery()
}

pub(crate) fn recover_stale_running_tasks_on_startup(
    db: &Connection,
    no_progress_timeout_seconds: u64,
) -> anyhow::Result<Vec<String>> {
    let now = now_ts_u64() as i64;
    let timeout = no_progress_timeout_seconds.max(1) as i64;
    let stale_before = now.saturating_sub(timeout);
    let mut task_ids = Vec::new();
    {
        let mut stmt = db.prepare(
            "SELECT task_id, result_json
             FROM tasks
             WHERE status = 'running'
               AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?1
             ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![stale_before.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (task_id, result_json) = row?;
            if recovery_should_preserve_paused_checkpoint(result_json.as_deref(), now) {
                continue;
            }
            task_ids.push(task_id);
        }
    }
    if task_ids.is_empty() {
        return Ok(task_ids);
    }

    let stale_note = format!(
        "auto timeout on startup: no progress heartbeat for {}s while status=running",
        no_progress_timeout_seconds.max(1)
    );

    let mut changed = 0;
    for task_id in &task_ids {
        changed += db.execute(
            "UPDATE tasks
             SET status = 'timeout',
                 error_text = CASE
                     WHEN error_text IS NULL OR TRIM(error_text) = '' THEN ?3
                     ELSE error_text
                 END,
                 updated_at = ?4
             WHERE task_id = ?1
               AND status = 'running'
               AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?2",
            rusqlite::params![task_id, stale_before.to_string(), stale_note, now_ts()],
        )?;
    }
    if changed != task_ids.len() {
        warn!(
            "startup stale-running recovery count mismatch: selected={} updated={}",
            task_ids.len(),
            changed
        );
    }

    Ok(task_ids)
}

fn recover_stale_running_tasks_by_no_progress(state: &AppState) -> anyhow::Result<Vec<String>> {
    let timeout_secs = state
        .worker
        .worker_running_no_progress_timeout_seconds
        .max(60);
    let now = now_ts_u64() as i64;
    let stale_before = now.saturating_sub(timeout_secs as i64);
    let stale_note = format!(
        "auto timeout: no progress heartbeat for {}s while status=running",
        timeout_secs
    );
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;

    let mut task_ids = Vec::new();
    {
        let mut stmt = db.prepare(
            "SELECT task_id, result_json
             FROM tasks
             WHERE status = 'running'
               AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?1
             ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![stale_before.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        for row in rows {
            let (task_id, result_json) = row?;
            if recovery_should_preserve_paused_checkpoint(result_json.as_deref(), now) {
                continue;
            }
            task_ids.push(task_id);
        }
    }

    if task_ids.is_empty() {
        return Ok(task_ids);
    }

    let mut changed = 0;
    for task_id in &task_ids {
        changed += db.execute(
            "UPDATE tasks
             SET status = 'timeout',
                 error_text = CASE
                     WHEN error_text IS NULL OR TRIM(error_text) = '' THEN ?3
                     ELSE error_text
                 END,
                 updated_at = ?4
             WHERE task_id = ?1
               AND status = 'running'
               AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?2",
            rusqlite::params![task_id, stale_before.to_string(), stale_note, now_ts()],
        )?;
    }
    if changed != task_ids.len() {
        warn!(
            "runtime stale-running recovery count mismatch: selected={} updated={}",
            task_ids.len(),
            changed
        );
    }
    Ok(task_ids)
}

pub(crate) async fn maybe_recover_stale_running_tasks_runtime(
    state: &AppState,
) -> anyhow::Result<()> {
    let now = now_ts_u64();
    let interval = state
        .worker
        .worker_running_recovery_check_interval_seconds
        .max(10);
    let should_run_stale_recovery = {
        let mut guard = state
            .worker
            .last_running_recovery_check_ts
            .lock()
            .map_err(|_| anyhow!("running recovery lock poisoned"))?;
        if now.saturating_sub(*guard) < interval {
            false
        } else {
            *guard = now;
            true
        }
    };
    if should_run_stale_recovery {
        let recovered = recover_stale_running_tasks_by_no_progress(state)?;
        if !recovered.is_empty() {
            warn!(
                "runtime stale-running recovery applied: converted {} running tasks to timeout (no_progress_timeout={}s)",
                recovered.len(),
                state.worker.worker_running_no_progress_timeout_seconds
            );
        }
    }
    let lease_seconds = interval.max(60) as i64;
    let due_paused = repo::list_due_paused_checkpoint_tasks_internal(state, now as i64, 50)?;
    if !due_paused.is_empty() {
        info!(
            "runtime paused-checkpoint resume candidates detected: count={} first_task_id={} first_checkpoint_id={} first_state={} first_resume_entrypoint={} first_resume_directive={} first_wait_seconds={} first_completed_side_effect_count={} first_requires_idempotency_guard={}",
            due_paused.len(),
            due_paused[0].task_id,
            due_paused[0].checkpoint_id,
            due_paused[0].lifecycle_state,
            due_paused[0].resume_entrypoint,
            due_paused[0].resume_directive,
            due_paused[0].resume_wait_seconds,
            due_paused[0].completed_side_effect_count,
            due_paused[0].requires_idempotency_guard
        );
        for candidate in due_paused.iter().take(10) {
            match repo::claim_due_paused_checkpoint_task_internal(
                state,
                &candidate.task_id,
                &candidate.checkpoint_id,
                now as i64,
                lease_seconds,
            ) {
                Ok(Some(claimed)) => {
                    let mut seeded_loop_state = crate::agent_engine::LoopState::new(1);
                    let Some(seed_report) = crate::agent_engine::seed_loop_state_for_agent_run(
                        &mut seeded_loop_state,
                        None,
                        Some(&claimed.task_checkpoint),
                    ) else {
                        continue;
                    };
                    let work_item = build_paused_checkpoint_resume_work_item(
                        &claimed,
                        lease_seconds,
                        crate::task_lifecycle::ResumeTrigger::WorkerRecovery,
                        seed_report,
                    );
                    let work_item_payload = work_item.to_machine_json();
                    match repo::record_paused_checkpoint_resume_work_item_internal(
                        state,
                        &work_item.task_id,
                        &work_item.checkpoint_id,
                        &work_item_payload,
                        now as i64,
                    ) {
                        Ok(true) => {}
                        Ok(false) => debug!(
                            "runtime paused-checkpoint resume work item persist skipped: task_id={} checkpoint_id={}",
                            work_item.task_id,
                            work_item.checkpoint_id
                        ),
                        Err(err) => warn!(
                            "runtime paused-checkpoint resume work item persist failed: task_id={} checkpoint_id={} err={}",
                            work_item.task_id,
                            work_item.checkpoint_id,
                            err
                        ),
                    }
                    let execution_decision = prepare_paused_checkpoint_resume_execution(
                        &work_item,
                        &claimed.checkpoint_resume_directive,
                        now as i64,
                    );
                    match repo::record_paused_checkpoint_resume_executor_state_internal(
                        state,
                        &work_item.task_id,
                        &work_item.checkpoint_id,
                        execution_decision.executor_state,
                        &execution_decision.payload,
                        execution_decision.lifecycle_state,
                        execution_decision.next_check_after,
                        now as i64,
                    ) {
                        Ok(true) => {}
                        Ok(false) => debug!(
                            "runtime paused-checkpoint resume executor state persist skipped: task_id={} checkpoint_id={} executor_state={}",
                            work_item.task_id,
                            work_item.checkpoint_id,
                            execution_decision.executor_state
                        ),
                        Err(err) => warn!(
                            "runtime paused-checkpoint resume executor state persist failed: task_id={} checkpoint_id={} executor_state={} err={}",
                            work_item.task_id,
                            work_item.checkpoint_id,
                            execution_decision.executor_state,
                            err
                        ),
                    }
                    info!(
                        "runtime paused-checkpoint resume candidate claimed: task_id={} checkpoint_id={} resume_entrypoint={} resume_directive={} resume_executor_state={} lease_seconds={} completed_side_effect_count={} requires_idempotency_guard={}",
                        claimed.task_id,
                        claimed.checkpoint_id,
                        claimed.resume_entrypoint,
                        claimed.resume_directive,
                        execution_decision.executor_state,
                        lease_seconds,
                        claimed.completed_side_effect_count,
                        claimed.requires_idempotency_guard
                    );
                    debug!(
                        "runtime paused-checkpoint resume seed prepared: task_id={} checkpoint_id={} resume_entrypoint={:?} restored_round={} restored_step={} restored_tool_calls={} completed_side_effect_count={} observation_count={}",
                        claimed.task_id,
                        work_item.seed_report.checkpoint_id,
                        work_item.seed_report.resume_entrypoint,
                        work_item.seed_report.restored_round,
                        work_item.seed_report.restored_step,
                        work_item.seed_report.restored_tool_calls,
                        work_item.seed_report.completed_side_effect_count,
                        work_item.seed_report.observation_count
                    );
                    debug!(
                        "runtime paused-checkpoint resume work item materialized: task_id={} checkpoint_id={} resume_directive={} payload_bytes={}",
                        work_item.task_id,
                        work_item.checkpoint_id,
                        work_item.resume_directive,
                        work_item_payload.to_string().len()
                    );
                }
                Ok(None) => debug!(
                    "runtime paused-checkpoint resume candidate claim skipped: task_id={} checkpoint_id={}",
                    candidate.task_id,
                    candidate.checkpoint_id
                ),
                Err(err) => warn!(
                    "runtime paused-checkpoint resume candidate claim failed: task_id={} checkpoint_id={} err={}",
                    candidate.task_id,
                    candidate.checkpoint_id,
                    err
                ),
            }
        }
    }
    let ready_executors =
        repo::list_ready_paused_checkpoint_resume_executors_internal(state, now as i64, 50)?;
    if let Some(first) = ready_executors.first() {
        info!(
            "runtime paused-checkpoint resume executor queue ready: count={} first_task_id={} first_checkpoint_id={} first_state={} first_executor_state={} first_resume_trigger={} first_resume_directive={} first_next_check_after={:?} first_completed_side_effect_count={} first_work_item_present={} first_executor_payload_bytes={}",
            ready_executors.len(),
            first.task_id,
            first.checkpoint_id,
            first.lifecycle_state,
            first.executor_state,
            first.resume_trigger,
            first.resume_directive,
            first.next_check_after,
            first.task_checkpoint.completed_side_effect_refs.len(),
            first.resume_work_item.is_some(),
            first.resume_executor.to_string().len()
        );
    }
    for executor in ready_executors.iter().take(10) {
        match repo::claim_ready_paused_checkpoint_resume_executor_internal(
            state,
            &executor.task_id,
            &executor.checkpoint_id,
            &executor.executor_state,
            now as i64,
            lease_seconds,
        ) {
            Ok(Some(claimed)) => {
                info!(
                    "runtime paused-checkpoint resume executor claimed: task_id={} checkpoint_id={} task_kind={} task_channel={} previous_executor_state={} executor_state={} resume_trigger={} resume_directive={} lease_expires_at={} work_item_present={} checkpoint_side_effect_count={} executor_payload_bytes={}",
                    claimed.task_id,
                    claimed.checkpoint_id,
                    claimed.task.kind,
                    claimed.task.channel,
                    claimed.previous_executor_state,
                    claimed.executor_state,
                    claimed.resume_trigger,
                    claimed.resume_directive,
                    claimed.lease_expires_at,
                    claimed.resume_work_item.is_some(),
                    claimed.task_checkpoint.completed_side_effect_refs.len(),
                    claimed.resume_executor.to_string().len()
                );
                match plan_claimed_paused_checkpoint_resume_execution(&claimed) {
                    Some(plan) => {
                        match repo::record_paused_checkpoint_resume_execution_plan_internal(
                            state,
                            &claimed.task_id,
                            &claimed.checkpoint_id,
                            &claimed.executor_state,
                            &plan.payload,
                            now as i64,
                        ) {
                            Ok(true) => {}
                            Ok(false) => debug!(
                                "runtime paused-checkpoint resume execution plan persist skipped: task_id={} checkpoint_id={} executor_state={}",
                                claimed.task_id,
                                claimed.checkpoint_id,
                                claimed.executor_state
                            ),
                            Err(err) => warn!(
                                "runtime paused-checkpoint resume execution plan persist failed: task_id={} checkpoint_id={} executor_state={} err={}",
                                claimed.task_id,
                                claimed.checkpoint_id,
                                claimed.executor_state,
                                err
                            ),
                        }
                        info!(
                            "runtime paused-checkpoint resume executor planned: task_id={} checkpoint_id={} executor_action={} executor_state={} resume_directive={} payload_bytes={}",
                            claimed.task_id,
                            claimed.checkpoint_id,
                            plan.executor_action,
                            plan.executor_state,
                            plan.resume_directive,
                            plan.payload.to_string().len()
                        );
                    }
                    None => warn!(
                        "runtime paused-checkpoint resume executor plan skipped: task_id={} checkpoint_id={} executor_state={} resume_directive={}",
                        claimed.task_id,
                        claimed.checkpoint_id,
                        claimed.executor_state,
                        claimed.resume_directive
                    ),
                }
            }
            Ok(None) => debug!(
                "runtime paused-checkpoint resume executor claim skipped: task_id={} checkpoint_id={} executor_state={}",
                executor.task_id, executor.checkpoint_id, executor.executor_state
            ),
            Err(err) => warn!(
                "runtime paused-checkpoint resume executor claim failed: task_id={} checkpoint_id={} executor_state={} err={}",
                executor.task_id, executor.checkpoint_id, executor.executor_state, err
            ),
        }
    }
    let planned_executions =
        repo::list_planned_paused_checkpoint_resume_executions_internal(state, now as i64, 50)?;
    if let Some(first) = planned_executions.first() {
        let first_handoff =
            planned_paused_checkpoint_resume_executor_handoff(&first.execution_plan);
        info!(
            "runtime paused-checkpoint resume execution plans ready: count={} first_task_id={} first_checkpoint_id={} first_executor_state={} first_executor_action={} first_executor_status={} first_resume_trigger={} first_resume_directive={} first_lease_expires_at={} first_checkpoint_side_effect_count={} first_plan_payload_bytes={}",
            planned_executions.len(),
            first.task_id,
            first.checkpoint_id,
            first.executor_state,
            first.executor_action,
            first_handoff
                .as_ref()
                .map(|handoff| handoff.executor_status)
                .unwrap_or("invalid_execution_plan"),
            first.resume_trigger,
            first.resume_directive,
            first.lease_expires_at,
            first.task_checkpoint.completed_side_effect_refs.len(),
            first.execution_plan.to_string().len()
        );
    }
    for planned in planned_executions.iter().take(10) {
        let Some(handoff) =
            planned_paused_checkpoint_resume_executor_handoff(&planned.execution_plan)
        else {
            warn!(
                "runtime paused-checkpoint resume execution handoff skipped: task_id={} checkpoint_id={} executor_state={} executor_action={}",
                planned.task_id,
                planned.checkpoint_id,
                planned.executor_state,
                planned.executor_action
            );
            continue;
        };
        match repo::record_planned_paused_checkpoint_resume_handoff_internal(
            state,
            &planned.task_id,
            &planned.checkpoint_id,
            &planned.executor_state,
            &planned.executor_action,
            &handoff.payload,
            now as i64,
        ) {
            Ok(true) => info!(
                "runtime paused-checkpoint resume execution handoff recorded: task_id={} checkpoint_id={} executor_action={} executor_status={}",
                planned.task_id,
                planned.checkpoint_id,
                handoff.executor_action,
                handoff.executor_status
            ),
            Ok(false) => debug!(
                "runtime paused-checkpoint resume execution handoff persist skipped: task_id={} checkpoint_id={} executor_action={} executor_status={}",
                planned.task_id,
                planned.checkpoint_id,
                handoff.executor_action,
                handoff.executor_status
            ),
            Err(err) => warn!(
                "runtime paused-checkpoint resume execution handoff persist failed: task_id={} checkpoint_id={} executor_action={} executor_status={} err={}",
                planned.task_id,
                planned.checkpoint_id,
                handoff.executor_action,
                handoff.executor_status,
                err
            ),
        }
    }
    let handoff_executions =
        repo::list_handoff_paused_checkpoint_resume_executions_internal(state, now as i64, 50)?;
    if let Some(first) = handoff_executions.first() {
        info!(
            "runtime paused-checkpoint resume executor handoff queue ready: count={} first_task_id={} first_checkpoint_id={} first_executor_state={} first_executor_action={} first_executor_status={} first_resume_trigger={} first_resume_directive={} first_lease_expires_at={} first_checkpoint_side_effect_count={} first_handoff_payload_bytes={}",
            handoff_executions.len(),
            first.task_id,
            first.checkpoint_id,
            first.executor_state,
            first.executor_action,
            first.executor_status,
            first.resume_trigger,
            first.resume_directive,
            first.lease_expires_at,
            first.task_checkpoint.completed_side_effect_refs.len(),
            first.handoff_payload.to_string().len()
        );
    }
    for handoff in handoff_executions.iter().take(10) {
        match repo::claim_handoff_paused_checkpoint_resume_execution_internal(
            state,
            &handoff.task_id,
            &handoff.checkpoint_id,
            &handoff.executor_state,
            &handoff.executor_action,
            &handoff.executor_status,
            now as i64,
            lease_seconds,
        ) {
            Ok(Some(claimed)) => {
                info!(
                    "runtime paused-checkpoint resume executor handoff claimed: task_id={} checkpoint_id={} executor_state={} executor_action={} executor_status={} resume_directive={} lease_expires_at={} handoff_claim_expires_at={} checkpoint_side_effect_count={} handoff_payload_bytes={}",
                    claimed.task_id,
                    claimed.checkpoint_id,
                    claimed.executor_state,
                    claimed.executor_action,
                    claimed.executor_status,
                    claimed.resume_directive,
                    claimed.lease_expires_at,
                    claimed.handoff_claim_expires_at,
                    claimed.task_checkpoint.completed_side_effect_refs.len(),
                    claimed.handoff_payload.to_string().len()
                );
                match dispatch_claimed_paused_checkpoint_resume_handoff(&claimed) {
                    Some(dispatch) => {
                        info!(
                            "runtime paused-checkpoint resume executor handoff dispatch ready: task_id={} checkpoint_id={} executor_state={} executor_action={} executor_status={} dispatch_state={} payload_bytes={}",
                            dispatch.task.task_id,
                            dispatch.checkpoint_id,
                            dispatch.executor_state,
                            dispatch.executor_action,
                            dispatch.executor_status,
                            dispatch.dispatch_state,
                            dispatch.payload.to_string().len()
                        );
                        match repo::record_claimed_handoff_paused_checkpoint_resume_dispatch_internal(
                            state,
                            &dispatch.task.task_id,
                            &dispatch.checkpoint_id,
                            &dispatch.executor_state,
                            &dispatch.executor_action,
                            &dispatch.executor_status,
                            &dispatch.payload,
                            now as i64,
                        ) {
                            Ok(true) => debug!(
                                "runtime paused-checkpoint resume executor handoff dispatch recorded: task_id={} checkpoint_id={} dispatch_state={}",
                                dispatch.task.task_id,
                                dispatch.checkpoint_id,
                                dispatch.dispatch_state
                            ),
                            Ok(false) => debug!(
                                "runtime paused-checkpoint resume executor handoff dispatch persist skipped: task_id={} checkpoint_id={} dispatch_state={}",
                                dispatch.task.task_id,
                                dispatch.checkpoint_id,
                                dispatch.dispatch_state
                            ),
                            Err(err) => warn!(
                                "runtime paused-checkpoint resume executor handoff dispatch persist failed: task_id={} checkpoint_id={} dispatch_state={} err={}",
                                dispatch.task.task_id,
                                dispatch.checkpoint_id,
                                dispatch.dispatch_state,
                                err
                            ),
                        }
                    }
                    None => warn!(
                        "runtime paused-checkpoint resume executor handoff dispatch skipped: task_id={} checkpoint_id={} executor_action={} executor_status={}",
                        claimed.task_id,
                        claimed.checkpoint_id,
                        claimed.executor_action,
                        claimed.executor_status
                    ),
                }
            }
            Ok(None) => debug!(
                "runtime paused-checkpoint resume executor handoff claim skipped: task_id={} checkpoint_id={} executor_action={} executor_status={}",
                handoff.task_id,
                handoff.checkpoint_id,
                handoff.executor_action,
                handoff.executor_status
            ),
            Err(err) => warn!(
                "runtime paused-checkpoint resume executor handoff claim failed: task_id={} checkpoint_id={} executor_action={} executor_status={} err={}",
                handoff.task_id,
                handoff.checkpoint_id,
                handoff.executor_action,
                handoff.executor_status,
                err
            ),
        }
    }
    let dispatched_executions =
        repo::list_dispatched_paused_checkpoint_resume_executions_internal(state, now as i64, 50)?;
    if let Some(first) = dispatched_executions.first() {
        info!(
            "runtime paused-checkpoint resume dispatch queue ready: count={} first_task_id={} first_checkpoint_id={} first_executor_state={} first_executor_action={} first_executor_status={} first_dispatch_state={} first_dispatch_execution_state={} first_resume_trigger={} first_resume_directive={} first_lease_expires_at={} first_handoff_claim_expires_at={} first_checkpoint_side_effect_count={} first_dispatch_payload_bytes={}",
            dispatched_executions.len(),
            first.task_id,
            first.checkpoint_id,
            first.executor_state,
            first.executor_action,
            first.executor_status,
            first.dispatch_state,
            first.dispatch_execution_state,
            first.resume_trigger,
            first.resume_directive,
            first.lease_expires_at,
            first.handoff_claim_expires_at,
            first.task_checkpoint.completed_side_effect_refs.len(),
            first.dispatch_payload.to_string().len()
        );
    }
    for dispatched in dispatched_executions.iter().take(10) {
        if !sync_recovery_can_claim_dispatch_executor(&dispatched.executor_action) {
            debug!(
                "runtime paused-checkpoint resume dispatch claim waiting for async executor: task_id={} checkpoint_id={} executor_action={} dispatch_state={}",
                dispatched.task_id,
                dispatched.checkpoint_id,
                dispatched.executor_action,
                dispatched.dispatch_state
            );
            continue;
        }
        match repo::claim_dispatched_paused_checkpoint_resume_execution_internal(
            state,
            &dispatched.task_id,
            &dispatched.checkpoint_id,
            &dispatched.executor_state,
            &dispatched.executor_action,
            &dispatched.executor_status,
            &dispatched.dispatch_state,
            now as i64,
            lease_seconds,
        ) {
            Ok(Some(claimed)) => {
                info!(
                    "runtime paused-checkpoint resume dispatch claimed: task_id={} checkpoint_id={} executor_state={} executor_action={} executor_status={} dispatch_state={} dispatch_execution_state={} resume_directive={} lease_expires_at={} handoff_claim_expires_at={} dispatch_claim_expires_at={} checkpoint_side_effect_count={} dispatch_payload_bytes={}",
                    claimed.task_id,
                    claimed.checkpoint_id,
                    claimed.executor_state,
                    claimed.executor_action,
                    claimed.executor_status,
                    claimed.dispatch_state,
                    claimed.dispatch_execution_state,
                    claimed.resume_directive,
                    claimed.lease_expires_at,
                    claimed.handoff_claim_expires_at,
                    claimed.dispatch_claim_expires_at,
                    claimed.task_checkpoint.completed_side_effect_refs.len(),
                    claimed.dispatch_payload.to_string().len()
                );
                let dispatch_result = if claimed.executor_action == "run_seeded_agent_loop" {
                    match super::resume_replay_executor::execute_seeded_agent_loop_dispatch_result(
                        state,
                        &claimed,
                    )
                    .await?
                    {
                        Some(result_payload) => record_concrete_paused_checkpoint_resume_dispatch_result(
                            state,
                            &claimed,
                            &result_payload,
                            now as i64,
                        )?,
                        None => record_paused_checkpoint_resume_dispatch_result(
                            state,
                            &claimed,
                            now as i64,
                            lease_seconds,
                        )?,
                    }
                } else if claimed.executor_action == "poll_async_job" {
                    match super::async_poll_executor::execute_async_poll_dispatch_result_with_state(
                        state,
                        &claimed,
                        now as i64,
                        lease_seconds,
                    )
                    .await
                    {
                        Some(result_payload) => record_concrete_paused_checkpoint_resume_dispatch_result(
                            state,
                            &claimed,
                            &result_payload,
                            now as i64,
                        )?,
                        None => record_paused_checkpoint_resume_dispatch_result(
                            state,
                            &claimed,
                            now as i64,
                            lease_seconds,
                        )?,
                    }
                } else {
                    record_paused_checkpoint_resume_dispatch_result(
                        state,
                        &claimed,
                        now as i64,
                        lease_seconds,
                    )?
                };
                match dispatch_result {
                    PausedCheckpointDispatchResultRecord::Recorded {
                        executor_result_status,
                    } => info!(
                            "runtime paused-checkpoint resume dispatch result recorded: task_id={} checkpoint_id={} executor_action={} executor_result_status={}",
                            claimed.task_id,
                            claimed.checkpoint_id,
                            claimed.executor_action,
                            executor_result_status
                        ),
                    PausedCheckpointDispatchResultRecord::NotRecorded {
                        executor_result_status,
                    } => debug!(
                            "runtime paused-checkpoint resume dispatch result record skipped: task_id={} checkpoint_id={} executor_action={} executor_result_status={}",
                            claimed.task_id,
                            claimed.checkpoint_id,
                            claimed.executor_action,
                            executor_result_status
                        ),
                    PausedCheckpointDispatchResultRecord::DeferredToConcreteExecutor => debug!(
                        "runtime paused-checkpoint resume dispatch result deferred to concrete executor: task_id={} checkpoint_id={} executor_action={} dispatch_state={}",
                        claimed.task_id,
                        claimed.checkpoint_id,
                        claimed.executor_action,
                        claimed.dispatch_state
                    ),
                }
            }
            Ok(None) => debug!(
                "runtime paused-checkpoint resume dispatch claim skipped: task_id={} checkpoint_id={} executor_action={} dispatch_state={}",
                dispatched.task_id,
                dispatched.checkpoint_id,
                dispatched.executor_action,
                dispatched.dispatch_state
            ),
            Err(err) => warn!(
                "runtime paused-checkpoint resume dispatch claim failed: task_id={} checkpoint_id={} executor_action={} dispatch_state={} err={}",
                dispatched.task_id,
                dispatched.checkpoint_id,
                dispatched.executor_action,
                dispatched.dispatch_state,
                err
            ),
        }
    }
    let dispatch_results = repo::list_recorded_paused_checkpoint_resume_dispatch_results_internal(
        state, now as i64, 50,
    )?;
    if let Some(first) = dispatch_results.first() {
        info!(
            "runtime paused-checkpoint resume dispatch result projection queue ready: count={} first_task_id={} first_checkpoint_id={} first_executor_state={} first_executor_action={} first_executor_status={} first_dispatch_state={} first_executor_result_status={} first_result_projection_state={} first_recorded_at={} first_checkpoint_side_effect_count={} first_result_payload_bytes={}",
            dispatch_results.len(),
            first.task_id,
            first.checkpoint_id,
            first.executor_state,
            first.executor_action,
            first.executor_status,
            first.dispatch_state,
            first.executor_result_status,
            first.result_projection_state,
            first.recorded_at,
            first.task_checkpoint.completed_side_effect_refs.len(),
            first.execution_result_payload.to_string().len()
        );
    }
    for result in dispatch_results.iter().take(10) {
        match repo::claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
            state,
            &result.task_id,
            &result.checkpoint_id,
            &result.executor_state,
            &result.executor_action,
            &result.executor_status,
            &result.dispatch_state,
            &result.executor_result_status,
            now as i64,
            lease_seconds,
        ) {
            Ok(Some(claimed)) => {
                info!(
                    "runtime paused-checkpoint resume dispatch result projection claimed: task_id={} checkpoint_id={} executor_state={} executor_action={} executor_status={} dispatch_state={} executor_result_status={} result_projection_state={} recorded_at={} result_projection_claim_expires_at={} checkpoint_side_effect_count={} result_payload_bytes={}",
                    claimed.task_id,
                    claimed.checkpoint_id,
                    claimed.executor_state,
                    claimed.executor_action,
                    claimed.executor_status,
                    claimed.dispatch_state,
                    claimed.executor_result_status,
                    claimed.result_projection_state,
                    claimed.recorded_at,
                    claimed.result_projection_claim_expires_at,
                    claimed.task_checkpoint.completed_side_effect_refs.len(),
                    claimed.execution_result_payload.to_string().len()
                );
                if let Some(projection_payload) =
                    paused_checkpoint_resume_reschedule_projection_payload(&claimed).or_else(|| {
                        paused_checkpoint_resume_terminal_projection_payload(&claimed)
                    })
                {
                    match repo::record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
                        state,
                        &claimed.task_id,
                        &claimed.checkpoint_id,
                        &claimed.executor_state,
                        &claimed.executor_action,
                        &claimed.executor_status,
                        &claimed.dispatch_state,
                        &claimed.executor_result_status,
                        &projection_payload,
                        now as i64,
                    ) {
                        Ok(true) => info!(
                            "runtime paused-checkpoint resume dispatch result projection recorded: task_id={} checkpoint_id={} executor_action={} executor_result_status={} result_projection_state={}",
                            claimed.task_id,
                            claimed.checkpoint_id,
                            claimed.executor_action,
                            claimed.executor_result_status,
                            claimed.result_projection_state
                        ),
                        Ok(false) => debug!(
                            "runtime paused-checkpoint resume dispatch result projection record skipped: task_id={} checkpoint_id={} executor_action={} executor_result_status={} result_projection_state={}",
                            claimed.task_id,
                            claimed.checkpoint_id,
                            claimed.executor_action,
                            claimed.executor_result_status,
                            claimed.result_projection_state
                        ),
                        Err(err) => warn!(
                            "runtime paused-checkpoint resume dispatch result projection record failed: task_id={} checkpoint_id={} executor_action={} executor_result_status={} err={}",
                            claimed.task_id,
                            claimed.checkpoint_id,
                            claimed.executor_action,
                            claimed.executor_result_status,
                            err
                        ),
                    }
                } else {
                    debug!(
                        "runtime paused-checkpoint resume dispatch result projection deferred: task_id={} checkpoint_id={} executor_action={} executor_result_status={} result_projection_state={}",
                        claimed.task_id,
                        claimed.checkpoint_id,
                        claimed.executor_action,
                        claimed.executor_result_status,
                        claimed.result_projection_state
                    );
                }
            }
            Ok(None) => debug!(
                "runtime paused-checkpoint resume dispatch result projection claim skipped: task_id={} checkpoint_id={} executor_action={} executor_result_status={}",
                result.task_id,
                result.checkpoint_id,
                result.executor_action,
                result.executor_result_status
            ),
            Err(err) => warn!(
                "runtime paused-checkpoint resume dispatch result projection claim failed: task_id={} checkpoint_id={} executor_action={} executor_result_status={} err={}",
                result.task_id,
                result.checkpoint_id,
                result.executor_action,
                result.executor_result_status,
                err
            ),
        }
    }
    Ok(())
}

pub(super) fn sync_recovery_can_claim_dispatch_executor(executor_action: &str) -> bool {
    matches!(
        executor_action,
        "run_seeded_agent_loop" | "poll_async_job" | "verify_and_finalize"
    )
}

#[cfg(test)]
#[path = "runtime_support/dispatch_result_tests.rs"]
mod dispatch_result_tests;

pub(crate) fn start_task_heartbeat(state: AppState, task_id: String) -> oneshot::Sender<()> {
    let interval_secs = state.worker.worker_task_heartbeat_seconds.max(5);
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {
                    if let Err(err) = repo::touch_running_task(&state, &task_id) {
                        warn!(
                            "task heartbeat update failed: task_id={} interval_secs={} err={}",
                            task_id, interval_secs, err
                        );
                    }
                }
                _ = &mut stop_rx => {
                    break;
                }
            }
        }
    });
    stop_tx
}

pub(crate) fn spawn_long_term_summary_refresh(
    state: AppState,
    task: crate::ClaimedTask,
    force_refresh: bool,
) {
    tokio::spawn(async move {
        if let Err(err) =
            crate::memory::service::maybe_refresh_long_term_summary(&state, &task, force_refresh)
                .await
        {
            warn!("refresh long-term memory summary failed: {err}");
        }
    });
}

pub(crate) fn spawn_worker(state: AppState, poll_interval_ms: u64, concurrency: usize) {
    let worker_count = concurrency.max(1);
    info!(
        "spawn_worker: starting {} worker loop(s), poll_interval_ms={}",
        worker_count,
        poll_interval_ms.max(10)
    );
    for worker_idx in 0..worker_count {
        let state_cloned = state.clone();
        tokio::spawn(async move {
            loop {
                if let Err(err) = super::worker_once(&state_cloned).await {
                    error!("Worker tick failed (worker_idx={}): {}", worker_idx, err);
                }
                tokio::time::sleep(Duration::from_millis(poll_interval_ms.max(10))).await;
            }
        });
    }
}

pub(crate) fn spawn_cleanup_worker(state: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(
                state.policy.maintenance.cleanup_interval_seconds.max(30),
            ))
            .await;

            if let Err(err) = cleanup_once(&state) {
                error!("Cleanup task failed: {}", err);
            }
        }
    });
}

pub(crate) fn spawn_schedule_worker(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(err) = schedule_once(&state) {
                error!("Schedule worker tick failed: {}", err);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

fn schedule_once(state: &AppState) -> anyhow::Result<()> {
    let now = now_ts_u64() as i64;
    let mut due_jobs: Vec<ScheduledJobDue> = Vec::new();

    {
        let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
        let mut stmt = db.prepare(
            "SELECT job_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, task_kind, task_payload_json, next_run_at,
                    schedule_type, time_of_day, weekday, every_minutes, timezone
             FROM scheduled_jobs
             WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1
             ORDER BY next_run_at ASC
             LIMIT 16",
        )?;
        let rows = stmt.query_map(rusqlite::params![now], |row| {
            Ok(ScheduledJobDue {
                job_id: row.get(0)?,
                user_id: row.get(1)?,
                chat_id: row.get(2)?,
                user_key: row.get(3)?,
                channel: row.get(4)?,
                external_user_id: row.get(5)?,
                external_chat_id: row.get(6)?,
                task_kind: row.get(7)?,
                task_payload_json: row.get(8)?,
                next_run_at: row.get(9)?,
                schedule_type: row.get(10)?,
                time_of_day: row.get(11)?,
                weekday: row.get(12)?,
                every_minutes: row.get(13)?,
                timezone: row.get(14)?,
            })
        })?;
        for row in rows {
            due_jobs.push(row?);
        }
    }

    if due_jobs.is_empty() {
        return Ok(());
    }

    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;

    for job in due_jobs {
        let next_run = schedule_service::compute_next_run_for_schedule(
            &job.schedule_type,
            job.time_of_day.as_deref(),
            job.weekday,
            job.every_minutes,
            &job.timezone,
            now,
        );

        let mut payload =
            serde_json::from_str::<Value>(&job.task_payload_json).unwrap_or_else(|_| json!({}));
        if let Value::Object(map) = &mut payload {
            for (k, v) in schedule_service::schedule_invocation_metadata(&job.job_id) {
                map.insert(k, v);
            }
            map.insert("channel".to_string(), Value::String(job.channel.clone()));
            if let Some(v) = job.external_user_id.as_ref() {
                map.insert("external_user_id".to_string(), Value::String(v.clone()));
            }
            if let Some(v) = job.external_chat_id.as_ref() {
                map.insert("external_chat_id".to_string(), Value::String(v.clone()));
            }
        }

        let task_id = Uuid::new_v4().to_string();
        let now_text = now_ts();
        db.execute(
            "INSERT INTO tasks (task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, message_id, kind, payload_json, status, result_json, error_text, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, 'queued', NULL, NULL, ?10, ?10)",
            rusqlite::params![
                task_id,
                job.user_id,
                job.chat_id,
                job.user_key,
                job.channel,
                job.external_user_id,
                job.external_chat_id,
                job.task_kind,
                payload.to_string(),
                now_text
            ],
        )?;

        match next_run {
            Some(ts) => {
                db.execute(
                    "UPDATE scheduled_jobs
                     SET last_run_at = ?2, next_run_at = ?3, updated_at = ?2
                     WHERE job_id = ?1 AND next_run_at = ?4",
                    rusqlite::params![job.job_id, now.to_string(), ts, job.next_run_at],
                )?;
            }
            None => {
                db.execute(
                    "UPDATE scheduled_jobs
                     SET enabled = 0, last_run_at = ?2, next_run_at = NULL, updated_at = ?2
                     WHERE job_id = ?1 AND next_run_at = ?3",
                    rusqlite::params![job.job_id, now.to_string(), job.next_run_at],
                )?;
            }
        }
    }

    Ok(())
}

fn cleanup_once(state: &AppState) -> anyhow::Result<()> {
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;

    let now = now_ts_u64() as i64;

    let task_cutoff = now - (state.policy.maintenance.tasks_retention_days as i64 * 86400);
    db.execute(
        "DELETE FROM tasks WHERE CAST(created_at AS INTEGER) < ?1",
        rusqlite::params![task_cutoff],
    )?;

    db.execute(
        "DELETE FROM tasks WHERE task_id IN (
             SELECT task_id FROM tasks
             ORDER BY CAST(created_at AS INTEGER) DESC
             LIMIT -1 OFFSET ?1
         )",
        rusqlite::params![state.policy.maintenance.tasks_max_rows as i64],
    )?;

    // Phase 2.2 Stage 2: audit_logs 已经搬到独立 audit pool（见 db_init::init_audit_db）。
    // 这里清理也走 audit_db，避免在主库 writer 锁上和任务回收争抢。
    {
        let audit_db = state
            .core
            .audit_db
            .get()
            .map_err(|e| anyhow!("audit db pool: {e}"))?;
        let audit_cutoff = now - (state.policy.maintenance.audit_retention_days as i64 * 86400);
        audit_db.execute(
            "DELETE FROM audit_logs WHERE CAST(ts AS INTEGER) < ?1",
            rusqlite::params![audit_cutoff],
        )?;

        audit_db.execute(
            "DELETE FROM audit_logs WHERE id IN (
                 SELECT id FROM audit_logs
                 ORDER BY id DESC
                 LIMIT -1 OFFSET ?1
             )",
            rusqlite::params![state.policy.maintenance.audit_max_rows as i64],
        )?;
    }

    let memory_cutoff = now - (state.policy.memory.retention_days as i64 * 86400);
    db.execute(
        "DELETE FROM memories
         WHERE COALESCE(created_at_ts, CAST(created_at AS INTEGER)) < ?1",
        rusqlite::params![memory_cutoff],
    )?;

    db.execute(
        "DELETE FROM memories WHERE id IN (
             SELECT id FROM memories
             ORDER BY id DESC
             LIMIT -1 OFFSET ?1
         )",
        rusqlite::params![state.policy.memory.max_rows as i64],
    )?;
    if state.policy.memory.hybrid_recall_enabled {
        let index_max_rows = state.policy.memory.max_rows.saturating_mul(3).max(2000);
        crate::memory::indexing::cleanup_retrieval_index(&db, memory_cutoff, index_max_rows)?;
    }

    let long_term_cutoff = now - (state.policy.memory.long_term_retention_days as i64 * 86400);
    db.execute(
        "DELETE FROM long_term_memories
         WHERE COALESCE(updated_at_ts, CAST(updated_at AS INTEGER)) < ?1",
        rusqlite::params![long_term_cutoff],
    )?;

    db.execute(
        "DELETE FROM long_term_memories WHERE id IN (
             SELECT id FROM long_term_memories
             ORDER BY id DESC
             LIMIT -1 OFFSET ?1
         )",
        rusqlite::params![state.policy.memory.long_term_max_rows as i64],
    )?;
    drop(db);

    // model_io.log：不再每次 append 后做全量 prune（会 O(N²) 磁盘）。
    // 改由这里按 cleanup 节拍把跨天的行迁到 `model_io.log.YYYY-MM-DD` 归档，
    // 主文件只保留当天；同时清理超过 keep_days 的旧归档。
    let model_io_path = state
        .skill_rt
        .workspace_root
        .join("logs")
        .join("model_io.log");
    if let Err(err) = crate::providers::rotate_model_io_log_daily(
        &model_io_path,
        crate::providers::MODEL_IO_LOG_KEEP_DAYS,
    ) {
        tracing::warn!("rotate model io log failed: {err}");
    }

    Ok(())
}
