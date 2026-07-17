use anyhow::anyhow;
use tracing::{debug, info, warn};

use crate::{now_ts_u64, repo, AppState};

use super::stale_recovery::recover_stale_running_tasks_by_no_progress;
use super::{
    build_paused_checkpoint_resume_work_item, dispatch_claimed_paused_checkpoint_resume_handoff,
    paused_checkpoint_resume_reschedule_projection_payload,
    paused_checkpoint_resume_terminal_projection_payload,
    plan_claimed_paused_checkpoint_resume_execution,
    planned_paused_checkpoint_resume_executor_handoff, prepare_paused_checkpoint_resume_execution,
    record_concrete_paused_checkpoint_resume_dispatch_result,
    record_paused_checkpoint_resume_dispatch_result, run_with_renewable_resume_execution_lease,
    PausedCheckpointDispatchResultRecord, RenewableResumeExecution,
};

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
    let lease_seconds = resume_execution_lease_seconds(state);
    prepare_due_paused_checkpoint_resume_work(state, now, lease_seconds)?;
    plan_ready_paused_checkpoint_resume_executors(state, now, lease_seconds)?;
    record_planned_paused_checkpoint_resume_handoffs(state, now)?;
    dispatch_handoff_paused_checkpoint_resume_executions(state, now, lease_seconds)?;
    execute_dispatched_paused_checkpoint_resume_executions(state, now, lease_seconds).await?;
    project_recorded_paused_checkpoint_resume_dispatch_results(state, now, lease_seconds)?;
    Ok(())
}

fn resume_execution_lease_seconds(state: &AppState) -> i64 {
    state
        .worker
        .worker_task_heartbeat_seconds
        .clamp(5, 10)
        .saturating_mul(3) as i64
}

fn prepare_due_paused_checkpoint_resume_work(
    state: &AppState,
    now: u64,
    lease_seconds: i64,
) -> anyhow::Result<()> {
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
    Ok(())
}

fn plan_ready_paused_checkpoint_resume_executors(
    state: &AppState,
    now: u64,
    lease_seconds: i64,
) -> anyhow::Result<()> {
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
    Ok(())
}

fn record_planned_paused_checkpoint_resume_handoffs(
    state: &AppState,
    now: u64,
) -> anyhow::Result<()> {
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
    Ok(())
}

fn dispatch_handoff_paused_checkpoint_resume_executions(
    state: &AppState,
    now: u64,
    lease_seconds: i64,
) -> anyhow::Result<()> {
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
    Ok(())
}

async fn execute_dispatched_paused_checkpoint_resume_executions(
    state: &AppState,
    now: u64,
    lease_seconds: i64,
) -> anyhow::Result<()> {
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
                    let execution = run_with_renewable_resume_execution_lease(
                        state,
                        &claimed,
                        lease_seconds,
                        super::super::resume_replay_executor::execute_seeded_agent_loop_dispatch_result(
                            state,
                            &claimed,
                        ),
                    )
                    .await?;
                    let RenewableResumeExecution::Completed(execution) = execution else {
                        warn!(
                            "runtime paused-checkpoint seeded loop lease lost: task_id={} checkpoint_id={} executor_action={}",
                            claimed.task_id,
                            claimed.checkpoint_id,
                            claimed.executor_action
                        );
                        continue;
                    };
                    let completed_at = now_ts_u64() as i64;
                    match execution? {
                        Some(result_payload) => record_concrete_paused_checkpoint_resume_dispatch_result(
                            state,
                            &claimed,
                            &result_payload,
                            completed_at,
                        )?,
                        None => record_paused_checkpoint_resume_dispatch_result(
                            state,
                            &claimed,
                            completed_at,
                            lease_seconds,
                        )?,
                    }
                } else if claimed.executor_action == "poll_async_job" {
                    let execution = run_with_renewable_resume_execution_lease(
                        state,
                        &claimed,
                        lease_seconds,
                        super::super::async_poll_executor::execute_async_poll_dispatch_result_with_state(
                            state,
                            &claimed,
                            now as i64,
                            lease_seconds,
                        ),
                    )
                    .await?;
                    let RenewableResumeExecution::Completed(execution) = execution else {
                        warn!(
                            "runtime paused-checkpoint async poll lease lost: task_id={} checkpoint_id={} executor_action={}",
                            claimed.task_id,
                            claimed.checkpoint_id,
                            claimed.executor_action
                        );
                        continue;
                    };
                    let completed_at = now_ts_u64() as i64;
                    match execution {
                        Some(result_payload) => record_concrete_paused_checkpoint_resume_dispatch_result(
                            state,
                            &claimed,
                            &result_payload,
                            completed_at,
                        )?,
                        None => record_paused_checkpoint_resume_dispatch_result(
                            state,
                            &claimed,
                            completed_at,
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
    Ok(())
}

fn project_recorded_paused_checkpoint_resume_dispatch_results(
    state: &AppState,
    now: u64,
    lease_seconds: i64,
) -> anyhow::Result<()> {
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

pub(crate) fn sync_recovery_can_claim_dispatch_executor(executor_action: &str) -> bool {
    matches!(
        executor_action,
        "run_seeded_agent_loop" | "poll_async_job" | "verify_and_finalize"
    )
}
