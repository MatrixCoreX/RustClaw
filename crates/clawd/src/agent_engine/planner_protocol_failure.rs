use super::LoopState;
use serde_json::{json, Value};

fn terminal_observation<'a>(loop_state: &'a LoopState, error_code: &str) -> Option<&'a Value> {
    if !matches!(
        error_code,
        "native_plan_contract_repair_stagnated"
            | "native_plan_contract_repair_budget_exhausted"
            | "native_plan_contract_repair_unavailable"
            | crate::llm_gateway::NATIVE_OUTPUT_LIMIT_EXHAUSTED_ERR
    ) {
        return None;
    }
    loop_state.task_observations.iter().rev().find(|value| {
        value.get("owner_layer").and_then(Value::as_str) == Some("planner")
            && value.get("reason_code").and_then(Value::as_str) == Some(error_code)
            && matches!(
                value.get("state").and_then(Value::as_str),
                Some("terminal" | "checkpoint_required")
            )
    })
}

pub(super) fn retain_terminal_failure(loop_state: &mut LoopState, error_code: &str) -> bool {
    if error_code == crate::llm_gateway::NATIVE_OUTPUT_LIMIT_EXHAUSTED_ERR {
        loop_state.task_observations.push(json!({
            "owner_layer":"planner", "source_owner":"task_budget", "state":"terminal",
            "reason_code":error_code, "source_error_code":"model_output_truncated"
        }));
    }
    if terminal_observation(loop_state, error_code).is_none() {
        return false;
    }
    loop_state.last_stop_signal = Some(error_code.to_string());
    true
}

pub(crate) fn terminal_failure_payload(loop_state: &LoopState) -> Option<Value> {
    let error_code = loop_state.last_stop_signal.as_deref()?;
    let observation = terminal_observation(loop_state, error_code)?;
    Some(json!({
        "schema_version": 1,
        "status": "error",
        "error_code": error_code,
        "source_error_code": observation.get("source_error_code"),
        "failure_owner": "planner_protocol",
        "failure_phase": if error_code == crate::llm_gateway::NATIVE_OUTPUT_LIMIT_EXHAUSTED_ERR {
            "generation_budget"
        } else { "response_validation" },
        "execution_failure": false,
        "completed_step_ids": loop_state.executed_step_results.iter()
            .filter(|step| step.is_ok()).map(|step| step.step_id.as_str()).collect::<Vec<_>>()
    }))
}

#[cfg(test)]
#[path = "planner_protocol_failure_tests.rs"]
mod tests;
