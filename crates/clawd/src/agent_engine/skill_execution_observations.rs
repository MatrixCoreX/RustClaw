use serde_json::{json, Value};
use tracing::info;

use super::{ClaimedTask, LoopState};

pub(super) fn log_step_journal_summary(
    task: &ClaimedTask,
    round_no: usize,
    step_in_round: usize,
    action_trace_kind: &str,
    execution_recipe_summary: Option<&str>,
    step_execution: &crate::executor::StepExecutionResult,
) {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        &task.task_id,
        "ask",
        format!("step:{}", step_execution.skill),
    );
    let mut summary = format!(
        "round={} step={} action_type={}",
        round_no, step_in_round, action_trace_kind
    );
    if let Some(recipe_summary) = execution_recipe_summary.filter(|v| !v.trim().is_empty()) {
        summary.push(' ');
        summary.push_str(recipe_summary);
    }
    journal.record_context_bundle_summary(summary);
    journal.push_step_result(step_execution);
    info!(
        "task_journal_summary task_id={} kind=ask phase=step_execute round={} step={} {}",
        task.task_id,
        round_no,
        step_in_round,
        journal.to_log_json()
    );
}

pub(super) fn record_hook_outcome_observation(
    loop_state: &mut LoopState,
    normalized_skill: &str,
    global_step: usize,
    step_in_round: usize,
    outcome: &crate::agent_hooks::HookOutcome,
) {
    let mut payload = outcome.to_machine_json(normalized_skill);
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("global_step".to_string(), json!(global_step));
        obj.insert("step_in_round".to_string(), json!(step_in_round));
        obj.insert("round_no".to_string(), json!(loop_state.round_no));
    }
    loop_state.task_observations.push(payload);
}

pub(super) fn record_post_tool_use_observation(
    loop_state: &mut LoopState,
    normalized_skill: &str,
    action_args: &Value,
    global_step: usize,
    step_in_round: usize,
    step_status: crate::executor::StepExecutionStatus,
) {
    let outcome = crate::agent_hooks::post_tool_use_outcome(
        normalized_skill,
        action_args,
        step_status.as_str(),
    );
    let mut payload = outcome.to_machine_json(normalized_skill);
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("global_step".to_string(), json!(global_step));
        obj.insert("step_in_round".to_string(), json!(step_in_round));
        obj.insert("round_no".to_string(), json!(loop_state.round_no));
        obj.insert("step_status".to_string(), json!(step_status.as_str()));
        obj.insert("status".to_string(), json!(step_status.as_str()));
        if let Some(args) = safe_post_tool_observation_args(normalized_skill, action_args) {
            obj.insert("args".to_string(), args);
        }
    }
    loop_state.task_observations.push(payload);
}

fn safe_post_tool_observation_args(normalized_skill: &str, action_args: &Value) -> Option<Value> {
    if normalized_skill != "run_cmd" {
        return None;
    }
    let obj = action_args.as_object()?;
    let mut safe = serde_json::Map::new();
    if let Some(command) = obj
        .get("command")
        .and_then(Value::as_str)
        .and_then(safe_single_line_machine_text)
    {
        safe.insert("command".to_string(), json!(command));
    }
    if let Some(cwd) = obj
        .get("cwd")
        .and_then(Value::as_str)
        .and_then(safe_single_line_machine_text)
    {
        safe.insert("cwd".to_string(), json!(cwd));
    }
    if let Some(async_start) = obj
        .get("async_start")
        .or_else(|| obj.get(crate::agent_engine::CLAWD_RUNTIME_ASYNC_JOB_START_ARG))
        .and_then(Value::as_bool)
    {
        safe.insert("async_start".to_string(), json!(async_start));
    }
    if let Some(timeout_seconds) = obj.get("timeout_seconds").and_then(Value::as_u64) {
        safe.insert("timeout_seconds".to_string(), json!(timeout_seconds));
    }
    (!safe.is_empty()).then_some(Value::Object(safe))
}

fn safe_single_line_machine_text(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 500 || value.chars().any(|ch| matches!(ch, '\n' | '\r')) {
        return None;
    }
    let sanitized = crate::visible_text::sanitize_user_visible_text(value)
        .trim()
        .to_string();
    (!sanitized.is_empty()).then_some(sanitized)
}
