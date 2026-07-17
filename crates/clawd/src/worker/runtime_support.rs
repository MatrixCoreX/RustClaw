#[path = "runtime_support/background_workers.rs"]
mod background_workers;
pub(crate) use background_workers::{
    spawn_cleanup_worker, spawn_long_term_summary_refresh, spawn_schedule_worker, spawn_worker,
    start_task_heartbeat,
};

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

#[path = "runtime_support/resume_plan.rs"]
mod resume_plan;
pub(super) use resume_plan::{
    build_paused_checkpoint_resume_work_item, checkpoint_resume_entrypoint_token,
    plan_claimed_paused_checkpoint_resume_execution, prepare_paused_checkpoint_resume_execution,
    ClaimedPausedCheckpointResumeHandoffDispatch, PlannedPausedCheckpointResumeExecutorHandoff,
};

#[path = "runtime_support/resume_execution_lease.rs"]
mod resume_execution_lease;
pub(super) use resume_execution_lease::{
    run_with_renewable_resume_execution_lease, RenewableResumeExecution,
};

#[path = "runtime_support/stale_recovery.rs"]
mod stale_recovery;
pub(crate) use stale_recovery::recover_stale_running_tasks_on_startup;

#[path = "runtime_support/recovery_flow.rs"]
mod recovery_flow;
pub(crate) use recovery_flow::maybe_recover_stale_running_tasks_runtime;
#[cfg(test)]
pub(super) use recovery_flow::sync_recovery_can_claim_dispatch_executor;

#[cfg(test)]
#[path = "runtime_support/dispatch_result_tests.rs"]
mod dispatch_result_tests;
