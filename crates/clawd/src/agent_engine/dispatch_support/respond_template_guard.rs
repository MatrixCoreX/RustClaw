use serde_json::{json, Value};
use tracing::info;

use super::{
    append_progress_hint, encode_progress_i18n, AppState, ClaimedTask, LoopState,
    RespondActionOutcome,
};

pub(super) fn unresolved_runtime_template_respond_outcome(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    content: &str,
    resolved_text: &str,
) -> Option<RespondActionOutcome> {
    if !unresolved_runtime_template_respond(content, resolved_text) {
        return None;
    }

    let error = "unresolved_runtime_template";
    loop_state.has_recoverable_failure_context = true;
    super::super::attempt_ledger::record_attempt_with_retry_instruction(
        loop_state,
        "respond",
        &format!("content={}", crate::truncate_for_agent_trace(content)),
        crate::executor::StepExecutionStatus::Error,
        resolved_text,
        Some("unresolved_runtime_template"),
        error,
        Some("respond_requires_observed_output_before_runtime_template_delivery"),
    );
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        "respond",
        false,
        error,
    );
    append_progress_hint(
        state,
        task,
        &mut loop_state.progress_messages,
        encode_progress_i18n("telegram.progress.retry_replan", &[]),
    );
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: format!("step_{global_step}"),
            skill: "respond".to_string(),
            status: crate::executor::StepExecutionStatus::Error,
            output: None,
            error: Some(error.to_string()),
            started_at: 0,
            finished_at: 0,
        });
    loop_state.history_compact.push(format!(
        "round={} step={} respond unresolved_runtime_template",
        loop_state.round_no, step_in_round
    ));
    loop_state.task_observations.push(json!({
        "schema_version": 1,
        "kind": "planner_quality_signal",
        "owner_layer": "agent_loop",
        "signal": "unresolved_runtime_template_respond",
        "action_ref": "respond",
        "round_no": loop_state.round_no,
        "step_in_round": step_in_round,
        "recoverable": true,
        "status_code": "recoverable_failure_continue_round"
    }));
    info!(
        "respond_unresolved_runtime_template_replan task_id={} round={} step={} content={}",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        crate::truncate_for_log(content)
    );

    Some(RespondActionOutcome {
        ended_with_user_visible_output: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        should_stop: true,
    })
}

pub(super) fn bare_last_output_placeholder(content: &str) -> bool {
    let trimmed = content.trim();
    if !trimmed.starts_with("{{") || !trimmed.ends_with("}}") {
        return false;
    }
    let inner = trimmed[2..trimmed.len().saturating_sub(2)].trim();
    let lower = inner.to_ascii_lowercase();
    lower == "last_output" || lower.starts_with("last_output.") || lower.starts_with("last_output[")
}

pub(super) fn should_publish_respond_message(loop_state: &LoopState, text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|last| last.trim() == trimmed)
    {
        return false;
    }
    if respond_machine_envelope_payload(trimmed) {
        return true;
    }
    if respond_lifecycle_result_payload(trimmed) {
        return true;
    }
    if !loop_state.has_tool_or_skill_output {
        return true;
    }
    if loop_state
        .last_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|last| last == trimmed)
    {
        return false;
    }
    true
}

pub(super) fn terminal_last_output_machine_projection(loop_state: &LoopState) -> Option<String> {
    let value = serde_json::from_str::<Value>(loop_state.last_output.as_deref()?.trim()).ok()?;
    child_model_result_projection(&value).map(|projection| projection.to_string())
}

fn child_model_result_projection(value: &Value) -> Option<Value> {
    let child = value.get("child_model_result")?;
    let object = child.as_object()?;
    let owner_ok = object
        .get("owner_layer")
        .and_then(Value::as_str)
        .is_some_and(|owner| owner == "subagent_model_child");
    let format_ok = object
        .get("output_format")
        .and_then(Value::as_str)
        .is_some_and(|format| format == "machine_json");
    let status_ok = object
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "completed" | "needs_more_evidence" | "failed"));
    (owner_ok && format_ok && status_ok).then(|| child.clone())
}

fn respond_machine_envelope_payload(text: &str) -> bool {
    let Ok(payload) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    payload.is_object()
        && payload
            .get("output_format")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "machine_json")
        && payload
            .get("owner_layer")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|owner| !owner.is_empty())
}

fn respond_lifecycle_result_payload(text: &str) -> bool {
    let Ok(payload) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    payload.is_object()
        && payload.get("final_answer_shape").and_then(Value::as_str) == Some("lifecycle_result")
        && payload.get("status").and_then(Value::as_str) == Some("ok")
        && payload
            .get("steps")
            .and_then(Value::as_array)
            .is_some_and(|steps| !steps.is_empty())
        && payload
            .pointer("/final_state/cleanup_observed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn unresolved_runtime_template_respond(content: &str, resolved_text: &str) -> bool {
    bare_last_output_placeholder(content)
        && (resolved_text.contains("{{")
            || resolved_text.contains("}}")
            || redacted_runtime_template_sentinel(resolved_text))
}

fn redacted_runtime_template_sentinel(text: &str) -> bool {
    text.trim() == crate::visible_text::sanitize_user_visible_text("{{last_output}}")
}
