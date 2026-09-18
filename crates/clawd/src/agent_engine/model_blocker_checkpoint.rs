use serde_json::json;

use super::{ensure_task_running, support, LoopState};
use crate::{AppState, ClaimedTask};

// Handle the failure while the owning loop still has its execution evidence.
pub(super) fn checkpoint_blocked_model_error(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
) -> Result<bool, String> {
    let (reason, source, status_key, status, message_key, retry_after, kind) =
        if let Some(blocker) = state.task_cost_blocker(&task.task_id) {
            (
                "llm_cost_policy_wait_background",
                "llm_cost_governance",
                "policy_status",
                blocker.to_machine_json(),
                blocker.message_key,
                blocker.retry_after_seconds,
                "cost_policy",
            )
        } else if let Some(blocker) = state.task_provider_blocker(&task.task_id) {
            (
                claw_core::provider_failure_policy::PROVIDER_WAIT_RESUME_REASON,
                "llm_gateway_provider_wait",
                "provider_status",
                blocker.to_machine_json(),
                blocker.message_key,
                blocker.retry_after_seconds,
                "provider",
            )
        } else {
            return Ok(false);
        };
    ensure_task_running(state, task)?;
    let now = crate::now_ts_u64().min(i64::MAX as u64) as i64;
    let next_check = now.saturating_add(retry_after.max(1).min(i64::MAX as u64) as i64);
    loop_state.last_stop_signal = Some(reason.to_string());
    let budget = support::checkpoint_budget_counters(
        loop_state,
        state.task_llm_call_count(&task.task_id),
        state.task_llm_elapsed_ms(&task.task_id),
    );
    let mut payload = support::build_agent_loop_checkpoint_progress_payload_with_budget(
        task, loop_state, reason, now, next_check, budget,
    );
    for pointer in ["/task_lifecycle", "/task_checkpoint/boundary_context"] {
        let fields = payload
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap();
        fields.insert("source".to_string(), json!(source));
        fields.insert("blocker_kind".to_string(), json!(kind));
        fields.insert("message_key".to_string(), json!(message_key));
        fields.insert(status_key.to_string(), status.clone());
    }
    payload["task_checkpoint"]["repair_signal"] = json!({
        "schema_version": 1,
        "source": source,
        "status_code": reason,
        "reason_code": reason,
        "next_recovery_kind": "wait_background",
        (status_key): status,
    });
    support::attach_task_llm_metrics_checkpoint(state, &task.task_id, &mut payload);
    if let Err(error) = crate::repo::update_task_progress_result(
        state,
        &task.task_id,
        task.claim_attempt,
        &payload.to_string(),
    ) {
        // Finalization retries the durable write with this complete journal;
        // propagating here would instead lose the in-memory execution state.
        tracing::warn!(task_id = %task.task_id, %error, "model_blocker_checkpoint_progress_write_failed");
    }
    loop_state.task_lifecycle = Some(payload["task_lifecycle"].clone());
    loop_state.task_checkpoint = Some(payload["task_checkpoint"].clone());
    loop_state
        .output_vars
        .insert("agent_loop.resume_reason".to_string(), reason.to_string());
    Ok(true)
}

#[cfg(test)]
#[path = "model_blocker_checkpoint_tests.rs"]
mod tests;
