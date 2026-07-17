use serde_json::json;

use crate::task_lifecycle::{
    CheckpointBudgetCounters, ResumeEntrypoint, TaskCheckpoint, TaskLifecycleState,
};
use crate::{AppState, ClaimedTask, TaskCostBlocker};

const COST_WAIT_RESUME_REASON: &str = "llm_cost_policy_wait_background";

pub(super) fn record_cost_wait_checkpoint(
    state: &AppState,
    task: &ClaimedTask,
    journal: &mut crate::task_journal::TaskJournal,
    blocker: &TaskCostBlocker,
) -> String {
    let now_ts = crate::now_ts_u64().min(i64::MAX as u64) as i64;
    let retry_after_seconds = blocker.retry_after_seconds.max(1);
    let next_check_after = now_ts.saturating_add(retry_after_seconds.min(i64::MAX as u64) as i64);
    let checkpoint_id = format!(
        "llm-cost:{}:{}:{}",
        task.task_id, now_ts, blocker.status_code
    );
    let policy_status = blocker.to_machine_json();
    let budget = CheckpointBudgetCounters {
        round: 0,
        step: 0,
        llm_calls: state
            .task_llm_call_count(&task.task_id)
            .min(u32::MAX as u64) as u32,
        tool_calls: 0,
        elapsed_ms: state.task_llm_elapsed_ms(&task.task_id),
        llm_elapsed_ms: state.task_llm_elapsed_ms(&task.task_id),
        tool_elapsed_ms: 0,
    };
    let checkpoint = TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.clone(),
        boundary_context: json!({
            "schema_version": 1,
            "source": "llm_cost_governance",
            "task_id": task.task_id,
            "resume_reason": COST_WAIT_RESUME_REASON,
            "blocker_kind": "cost_policy",
            "policy_status": policy_status,
        }),
        last_successful_round: None,
        last_successful_step: None,
        pending_action: None,
        observations: vec![json!({
            "kind": "policy_blocker",
            "blocker_kind": "cost_policy",
            "policy_status": policy_status,
        })],
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: Vec::new(),
        budget: budget.clone(),
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: Some(json!({
            "schema_version": 1,
            "source": "llm_cost_governance",
            "status_code": COST_WAIT_RESUME_REASON,
            "reason_code": COST_WAIT_RESUME_REASON,
            "next_recovery_kind": "wait_background",
            "policy_status": policy_status,
        })),
        resume_entrypoint: ResumeEntrypoint::NextPlannerRound,
    };
    journal.record_task_lifecycle(json!({
        "schema_version": 1,
        "state": TaskLifecycleState::Waiting,
        "source": "llm_cost_governance",
        "resume_reason": COST_WAIT_RESUME_REASON,
        "blocker_kind": "cost_policy",
        "next_check_after": next_check_after,
        "checkpoint_id": checkpoint_id,
        "can_poll": true,
        "can_cancel": true,
        "last_heartbeat_ts": now_ts,
        "message_key": blocker.message_key,
        "policy_status": policy_status,
        "budget": budget,
    }));
    journal.record_task_checkpoint(checkpoint.to_machine_json());
    checkpoint_id
}

#[cfg(test)]
#[path = "task_cost_wait_tests.rs"]
mod tests;
