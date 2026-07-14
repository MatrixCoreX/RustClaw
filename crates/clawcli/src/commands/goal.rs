use anyhow::Result;
use serde_json::{json, Value};

use crate::{events::EventFilters, output, task};

use super::common::wait_for_terminal_task;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_goal_start(
    base_url: &str,
    key: &str,
    prompt: &str,
    objective: Option<&str>,
    done_conditions: &[String],
    verification_commands: &[String],
    constraints: &[String],
    wait: bool,
    detach: bool,
    json_output: bool,
    interval_ms: u64,
) -> Result<()> {
    if wait && detach {
        anyhow::bail!("goal_start_wait_detach_conflict");
    }
    let payload = goal_request_payload(
        prompt,
        objective,
        done_conditions,
        verification_commands,
        constraints,
    );
    let task_id = task::submit_goal_ask(base_url, key, payload)?;
    if wait {
        let task = wait_for_terminal_task(base_url, key, &task_id, interval_ms)?;
        if json_output {
            output::print_json_pretty(&task.raw_data);
        } else {
            output::print_task_status(&task, false, &EventFilters::default());
        }
    } else if json_output {
        output::print_json_pretty(&json!({
            "task_id": task_id,
            "detached": true,
        }));
    } else {
        println!("{}: {}", "task_id", task_id);
    }
    Ok(())
}

pub(crate) fn run_goal_status(
    base_url: &str,
    key: &str,
    task_id: &str,
    json_output: bool,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let summary = goal_status_summary_json(&task);
    if json_output {
        output::print_json_pretty(&summary);
    } else {
        for line in goal_status_text_lines(&summary) {
            println!("{line}");
        }
    }
    Ok(())
}

pub(super) fn goal_request_payload(
    prompt: &str,
    objective: Option<&str>,
    done_conditions: &[String],
    verification_commands: &[String],
    constraints: &[String],
) -> Value {
    let objective = objective
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| prompt.trim());
    let mut goal = serde_json::Map::new();
    goal.insert("schema_version".to_string(), json!(1));
    if !objective.is_empty() {
        goal.insert("objective".to_string(), json!(objective));
    }
    insert_string_array(&mut goal, "done_conditions", done_conditions);
    insert_string_array(&mut goal, "verification_commands", verification_commands);
    insert_string_array(&mut goal, "constraints", constraints);
    goal.insert("goal_status".to_string(), json!("created"));

    json!({
        "text": prompt,
        "goal": Value::Object(goal),
    })
}

pub(super) fn goal_status_summary_json(task: &task::TaskStatusView) -> Value {
    let goal = task.raw_data.get("goal").cloned().unwrap_or(Value::Null);
    json!({
        "report_kind": "rustclaw_goal_status",
        "task_id": task.task_id,
        "status": task.status,
        "execution_state": task.execution_state(),
        "lifecycle_state": task.lifecycle_state(),
        "terminal": task.is_terminal(),
        "goal": goal,
    })
}

pub(super) fn goal_status_text_lines(summary: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    push_scalar_line(&mut lines, "goal_task_id", summary.get("task_id"));
    push_scalar_line(&mut lines, "goal_task_status", summary.get("status"));
    push_scalar_line(
        &mut lines,
        "goal_execution_state",
        summary.get("execution_state"),
    );
    push_scalar_line(
        &mut lines,
        "goal_lifecycle_state",
        summary.get("lifecycle_state"),
    );
    push_scalar_line(&mut lines, "goal_terminal", summary.get("terminal"));
    let goal = summary.get("goal").unwrap_or(&Value::Null);
    push_scalar_line(&mut lines, "goal_id", goal.get("goal_id"));
    push_scalar_line(&mut lines, "goal_status", goal.get("goal_status"));
    push_scalar_line(
        &mut lines,
        "goal_status_source",
        goal.get("goal_status_source"),
    );
    push_scalar_line(&mut lines, "goal_objective", goal.get("objective"));
    push_array_count_line(
        &mut lines,
        "goal_done_condition_count",
        goal.get("done_conditions"),
    );
    push_array_count_line(
        &mut lines,
        "goal_verification_command_count",
        goal.get("verification_commands"),
    );
    push_array_count_line(&mut lines, "goal_constraint_count", goal.get("constraints"));
    push_array_count_line(
        &mut lines,
        "goal_current_progress_count",
        goal.get("current_progress"),
    );
    push_array_count_line(
        &mut lines,
        "goal_remaining_work_count",
        goal.get("remaining_work"),
    );
    lines
}

fn insert_string_array(map: &mut serde_json::Map<String, Value>, key: &str, values: &[String]) {
    let values = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(Value::from)
        .collect::<Vec<_>>();
    if !values.is_empty() {
        map.insert(key.to_string(), Value::Array(values));
    }
}

fn push_scalar_line(lines: &mut Vec<String>, key: &str, value: Option<&Value>) {
    let Some(value) = value else {
        return;
    };
    let token = match value {
        Value::String(value) => value.trim().to_string(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
    };
    if !token.is_empty() {
        lines.push(format!("{key}: {token}"));
    }
}

fn push_array_count_line(lines: &mut Vec<String>, key: &str, value: Option<&Value>) {
    let count = value.and_then(Value::as_array).map_or(0, Vec::len);
    lines.push(format!("{key}: {count}"));
}
