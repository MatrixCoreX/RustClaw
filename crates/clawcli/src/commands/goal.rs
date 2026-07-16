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

pub(crate) fn run_goal_pause(
    base_url: &str,
    key: &str,
    task_id: &str,
    pause_seconds: u64,
) -> Result<()> {
    let body = task::pause_task_by_id(base_url, key, task_id, pause_seconds)?;
    output::print_json_pretty(&goal_control_summary_json("goal_pause", task_id, &body));
    Ok(())
}

pub(crate) fn run_goal_resume(
    base_url: &str,
    key: &str,
    task_id: &str,
    checkpoint_id: Option<&str>,
    user_message: Option<&str>,
    constraints_json: Option<&str>,
) -> Result<()> {
    let new_constraints = constraints_json
        .map(|raw| serde_json::from_str::<Value>(raw))
        .transpose()
        .map_err(|err| {
            anyhow::anyhow!("{}={}", "goal_resume_constraints_json_parse_failed", err)
        })?;
    let body = task::resume_task_by_id(
        base_url,
        key,
        task_id,
        task::TaskResumeRequest {
            checkpoint_id,
            resume_reason: Some("goal_resume"),
            user_message,
            new_constraints,
            ..Default::default()
        },
    )?;
    output::print_json_pretty(&goal_control_summary_json("goal_resume", task_id, &body));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_goal_edit(
    base_url: &str,
    key: &str,
    task_id: &str,
    goal_json: Option<&str>,
    objective: Option<&str>,
    done_conditions: &[String],
    verification_commands: &[String],
    constraints: &[String],
    allowed_scopes: &[String],
    forbidden_actions: &[String],
    goal_status: Option<&str>,
) -> Result<()> {
    let goal = goal_edit_patch_json(
        goal_json,
        objective,
        done_conditions,
        verification_commands,
        constraints,
        allowed_scopes,
        forbidden_actions,
        goal_status,
    )?;
    let body = task::update_goal_by_task_id(base_url, key, task_id, "edit", Some(goal))?;
    output::print_json_pretty(&goal_control_summary_json("goal_edit", task_id, &body));
    Ok(())
}

pub(crate) fn run_goal_clear(base_url: &str, key: &str, task_id: &str) -> Result<()> {
    let body = task::update_goal_by_task_id(base_url, key, task_id, "clear", None)?;
    output::print_json_pretty(&goal_control_summary_json("goal_clear", task_id, &body));
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

#[allow(clippy::too_many_arguments)]
pub(super) fn goal_edit_patch_json(
    goal_json: Option<&str>,
    objective: Option<&str>,
    done_conditions: &[String],
    verification_commands: &[String],
    constraints: &[String],
    allowed_scopes: &[String],
    forbidden_actions: &[String],
    goal_status: Option<&str>,
) -> Result<Value> {
    let mut patch = goal_json
        .map(|raw| serde_json::from_str::<Value>(raw))
        .transpose()
        .map_err(|err| anyhow::anyhow!("{}={}", "goal_json_parse_failed", err))?
        .unwrap_or_else(|| json!({}));
    if !patch.is_object() {
        anyhow::bail!("goal_json_must_be_object");
    }
    let Some(map) = patch.as_object_mut() else {
        anyhow::bail!("goal_json_must_be_object");
    };
    if let Some(objective) = objective.map(str::trim).filter(|value| !value.is_empty()) {
        map.insert("objective".to_string(), json!(objective));
    }
    insert_string_array(map, "done_conditions", done_conditions);
    insert_string_array(map, "verification_commands", verification_commands);
    insert_string_array(map, "constraints", constraints);
    insert_string_array(map, "allowed_files_or_scopes", allowed_scopes);
    insert_string_array(map, "forbidden_actions", forbidden_actions);
    if let Some(goal_status) = goal_status.map(str::trim).filter(|value| !value.is_empty()) {
        map.insert("goal_status".to_string(), json!(goal_status));
    }
    if map.is_empty() {
        anyhow::bail!("goal_patch_empty");
    }
    Ok(patch)
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

pub(super) fn goal_control_summary_json(
    operation: &str,
    requested_task_id: &str,
    body: &Value,
) -> Value {
    let safe_body = goal_public_json(body);
    let data = safe_body.get("data").unwrap_or(&safe_body);
    let lifecycle = data
        .get("task_lifecycle")
        .or_else(|| data.get("lifecycle"))
        .unwrap_or(&Value::Null);
    json!({
        "schema_version": 1,
        "operation": operation,
        "task_id": scalar_string(data, "task_id").unwrap_or(requested_task_id),
        "status": scalar_string(data, "status"),
        "checkpoint_id": scalar_string(data, "checkpoint_id")
            .or_else(|| scalar_string(lifecycle, "checkpoint_id")),
        "lifecycle_state": scalar_string(lifecycle, "state"),
        "execution_state": scalar_string(lifecycle, "execution_state"),
        "resume_due": lifecycle.get("resume_due").and_then(Value::as_bool),
        "resume_wait_seconds": lifecycle.get("resume_wait_seconds").and_then(Value::as_i64),
        "resume_entrypoint": scalar_string(lifecycle, "resume_entrypoint"),
        "resume_directive": scalar_string(lifecycle, "resume_directive"),
        "resume_reason": scalar_string(lifecycle, "resume_reason"),
        "next_action_kind": scalar_string(lifecycle, "next_action_kind"),
        "goal": data.get("goal").cloned().unwrap_or(Value::Null),
        "payload_json": data.get("payload_json").cloned().unwrap_or(Value::Null),
        "response": safe_body,
    })
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

fn scalar_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn goal_sensitive_field_name(field: &str) -> bool {
    let normalized = field.trim().to_ascii_lowercase().replace(['-', '.'], "_");
    normalized == "key"
        || normalized == "auth"
        || normalized.ends_with("_key")
        || normalized.contains("token")
        || normalized.contains("secret")
        || normalized.contains("password")
        || normalized.contains("passwd")
        || normalized.contains("cookie")
        || normalized.contains("credential")
        || normalized.contains("ticket")
        || normalized.contains("signature")
        || normalized.contains("authorization")
}

fn goal_public_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                let value = if goal_sensitive_field_name(key) {
                    json!("[REDACTED]")
                } else {
                    goal_public_json(child)
                };
                out.insert(key.clone(), value);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(goal_public_json).collect()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}
