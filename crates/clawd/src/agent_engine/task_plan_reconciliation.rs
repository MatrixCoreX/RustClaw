use super::LoopState;
use crate::{AppState, ClaimedTask};
use serde_json::{json, Value};

const REASON: &str = "task_plan_reconciliation_required";
const INCOMPLETE: &str = "task_plan_reconciliation_incomplete";
const ATTEMPT_LIMIT: usize = 2;

fn unfinished_steps(snapshot: &Value) -> usize {
    snapshot
        .get("steps")
        .and_then(Value::as_array)
        .map_or(0, |steps| {
            steps
                .iter()
                .filter(|step| {
                    matches!(
                        step.get("status").and_then(Value::as_str),
                        Some("pending" | "in_progress")
                    )
                })
                .count()
        })
}

pub(super) fn prepare_task_plan_reconciliation(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
) -> Result<bool, String> {
    if loop_state.last_user_visible_respond.is_none() {
        return Ok(false);
    }
    let snapshot = crate::repo::read_task_plan(state, &task.task_id, "read_plan")
        .map_err(|error| error.machine_extra().to_string())?;
    let remaining = unfinished_steps(&snapshot);
    if remaining == 0 {
        return Ok(false);
    }
    let attempts = loop_state
        .task_observations
        .iter()
        .filter(|item| {
            item.get("owner_layer").and_then(Value::as_str) == Some("agent_loop")
                && item.get("reason_code").and_then(Value::as_str) == Some(REASON)
        })
        .collect::<Vec<_>>();
    let attempt_count = attempts.len();
    let progressed = attempts.last().is_none_or(|previous| {
        snapshot["plan_revision"].as_u64() > previous["snapshot"]["plan_revision"].as_u64()
            && remaining < unfinished_steps(&previous["snapshot"])
    });
    if attempt_count >= ATTEMPT_LIMIT || !progressed {
        if !loop_state.task_observations.iter().any(|item| {
            item.get("owner_layer").and_then(Value::as_str) == Some("agent_loop")
                && item.get("reason_code").and_then(Value::as_str) == Some(INCOMPLETE)
        }) {
            loop_state.task_observations.push(json!({
                "schema_version": 1,
                "owner_layer": "agent_loop",
                "observation_kind": "task_plan_snapshot",
                "reason_code": INCOMPLETE,
                "snapshot": snapshot,
                "reconciliation_attempt_count": attempt_count,
                "remaining_step_count": remaining,
                "next_action": "report_observed_outcome",
            }));
        }
        return Ok(false);
    }
    // The planner owns completion semantics. Never mark steps complete in Rust.
    let observation = json!({
        "schema_version": 1,
        "owner_layer": "agent_loop",
        "observation_kind": "task_plan_snapshot",
        "reason_code": REASON,
        "snapshot": snapshot,
        "next_action": "reconcile_plan_from_execution_evidence",
        "completed_actions_must_not_replay": true,
        "reconciliation_attempt": attempt_count + 1,
        "reconciliation_attempt_limit": ATTEMPT_LIMIT,
        "candidate_response_prepared": true,
        "response_delivery_owner": "runtime",
    });
    loop_state.last_output = Some(observation.to_string());
    loop_state.task_observations.push(observation);
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = None;
    loop_state.last_publishable_synthesis_output = None;
    loop_state.last_capability_synthesis_output = None;
    loop_state.last_stop_signal = Some(REASON.to_string());
    loop_state.has_recoverable_failure_context = true;
    Ok(true)
}

#[cfg(test)]
#[path = "task_plan_reconciliation_tests.rs"]
mod tests;
