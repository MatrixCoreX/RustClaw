use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::{client, output, task};

use super::report::task_report_json;

pub(crate) fn run_session_list(
    base_url: &str,
    key: &str,
    user_id: i64,
    chat_id: i64,
    json_output: bool,
) -> Result<()> {
    let active = active_tasks(base_url, key, user_id, chat_id)?;
    let summary = session_list_json(user_id, chat_id, &active);
    if json_output {
        output::print_json_pretty(&summary);
    } else {
        for line in session_list_text_lines(&summary) {
            println!("{line}");
        }
    }
    Ok(())
}

pub(crate) fn run_session_show(
    base_url: &str,
    key: &str,
    session_id: &str,
    json_output: bool,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, session_id)?;
    let summary = session_show_json(&task);
    if json_output {
        output::print_json_pretty(&summary);
    } else {
        for line in session_show_text_lines(&summary) {
            println!("{line}");
        }
    }
    Ok(())
}

pub(crate) fn run_session_resume(
    base_url: &str,
    key: &str,
    session_id: &str,
    message: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let body = task::resume_task_by_id(
        base_url,
        key,
        session_id,
        None,
        Some("session_resume"),
        message,
        None,
    )?;
    let summary = session_resume_json(session_id, &body);
    if json_output {
        output::print_json_pretty(&summary);
    } else {
        for line in session_resume_text_lines(&summary) {
            println!("{line}");
        }
    }
    Ok(())
}

pub(super) fn session_list_json(user_id: i64, chat_id: i64, active: &Value) -> Value {
    let tasks = active
        .pointer("/data/tasks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let task_ids = tasks
        .iter()
        .filter_map(|task| string_at(task, "/task_id"))
        .collect::<Vec<_>>();
    let summaries = tasks
        .iter()
        .map(session_task_summary_json)
        .collect::<Vec<_>>();
    json!({
        "session_kind": "user_chat_active_tasks",
        "session_id": format!("user_chat:{user_id}:{chat_id}"),
        "user_id": user_id,
        "chat_id": chat_id,
        "task_count": task_ids.len(),
        "task_ids": task_ids,
        "active_goal_id": first_string(&tasks, &["/goal/goal_id", "/task_goal/goal_id"]),
        "latest_checkpoint_id": first_string(&tasks, &["/task_lifecycle/checkpoint_id", "/lifecycle/checkpoint_id", "/checkpoint_id"]),
        "latest_event_seq": first_string(&tasks, &["/latest_event_seq", "/event_seq"]),
        "archived": false,
        "tasks": summaries,
    })
}

pub(super) fn session_show_json(task: &task::TaskStatusView) -> Value {
    let lifecycle = task.lifecycle().cloned().unwrap_or(Value::Null);
    let goal = task
        .raw_data
        .get("goal")
        .or_else(|| task.raw_data.get("task_goal"))
        .cloned()
        .unwrap_or(Value::Null);
    json!({
        "session_kind": "task_session",
        "session_id": task.task_id.clone(),
        "task_ids": [task.task_id.clone()],
        "active_goal_id": string_at(&goal, "/goal_id"),
        "workspace_root": string_at(&task.raw_data, "/workspace_root")
            .or_else(|| string_at(&task.raw_data, "/result_json/workspace_root")),
        "latest_checkpoint_id": string_at(&lifecycle, "/checkpoint_id")
            .or_else(|| string_at(&task.raw_data, "/checkpoint_id")),
        "latest_event_seq": task.events.last().and_then(|event| {
            event.fields
                .get("event_seq")
                .or_else(|| event.fields.get("seq"))
                .cloned()
        }),
        "archived": false,
        "status": task.status.clone(),
        "execution_state": task.execution_state(),
        "lifecycle_state": task.lifecycle_state(),
        "lifecycle": lifecycle,
        "goal": goal,
        "summary": task_report_json(task, false),
    })
}

pub(super) fn session_resume_json(session_id: &str, body: &Value) -> Value {
    let data = body.get("data").unwrap_or(body);
    let lifecycle = data
        .get("task_lifecycle")
        .or_else(|| data.get("lifecycle"))
        .unwrap_or(&Value::Null);
    json!({
        "operation": "session_resume",
        "session_id": session_id,
        "task_id": string_at(data, "/task_id").unwrap_or_else(|| session_id.to_string()),
        "status": string_at(data, "/status"),
        "execution_state": string_at(lifecycle, "/execution_state"),
        "lifecycle_state": string_at(lifecycle, "/state"),
        "checkpoint_id": string_at(lifecycle, "/checkpoint_id").or_else(|| string_at(data, "/checkpoint_id")),
        "resume_due": lifecycle.get("resume_due").cloned().unwrap_or(Value::Null),
        "resume_reason": string_at(lifecycle, "/resume_reason"),
        "next_action_kind": string_at(lifecycle, "/next_action_kind"),
        "response": body,
    })
}

fn session_task_summary_json(task: &Value) -> Value {
    json!({
        "task_id": string_at(task, "/task_id"),
        "status": string_at(task, "/status"),
        "execution_state": string_at(task, "/execution_state")
            .or_else(|| string_at(task, "/task_lifecycle/execution_state"))
            .or_else(|| string_at(task, "/lifecycle/execution_state")),
        "lifecycle_state": string_at(task, "/task_lifecycle/state")
            .or_else(|| string_at(task, "/lifecycle/state")),
        "checkpoint_id": string_at(task, "/task_lifecycle/checkpoint_id")
            .or_else(|| string_at(task, "/lifecycle/checkpoint_id"))
            .or_else(|| string_at(task, "/checkpoint_id")),
        "goal_id": string_at(task, "/goal/goal_id")
            .or_else(|| string_at(task, "/task_goal/goal_id")),
        "latest_event_seq": string_at(task, "/latest_event_seq").or_else(|| string_at(task, "/event_seq")),
    })
}

fn session_list_text_lines(summary: &Value) -> Vec<String> {
    let mut lines = vec![
        format!(
            "session_id: {}",
            summary
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        format!(
            "session_task_count: {}",
            summary
                .get("task_count")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        ),
    ];
    push_optional_line(
        &mut lines,
        "session_active_goal_id",
        summary,
        "/active_goal_id",
    );
    push_optional_line(
        &mut lines,
        "session_latest_checkpoint_id",
        summary,
        "/latest_checkpoint_id",
    );
    if let Some(tasks) = summary.get("tasks").and_then(Value::as_array) {
        for task in tasks {
            let task_id = string_at(task, "/task_id").unwrap_or_default();
            if task_id.is_empty() {
                continue;
            }
            let status = string_at(task, "/status").unwrap_or_default();
            let lifecycle_state = string_at(task, "/lifecycle_state").unwrap_or_default();
            lines.push(format!(
                "session_task: task_id={task_id} status={status} lifecycle_state={lifecycle_state}"
            ));
        }
    }
    lines
}

fn session_show_text_lines(summary: &Value) -> Vec<String> {
    let mut lines = vec![format!(
        "session_id: {}",
        summary
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("")
    )];
    push_optional_line(&mut lines, "session_status", summary, "/status");
    push_optional_line(
        &mut lines,
        "session_execution_state",
        summary,
        "/execution_state",
    );
    push_optional_line(
        &mut lines,
        "session_lifecycle_state",
        summary,
        "/lifecycle_state",
    );
    push_optional_line(
        &mut lines,
        "session_active_goal_id",
        summary,
        "/active_goal_id",
    );
    push_optional_line(
        &mut lines,
        "session_latest_checkpoint_id",
        summary,
        "/latest_checkpoint_id",
    );
    push_optional_line(
        &mut lines,
        "session_workspace_root",
        summary,
        "/workspace_root",
    );
    lines
}

fn session_resume_text_lines(summary: &Value) -> Vec<String> {
    let task_id = summary.get("task_id").and_then(Value::as_str).unwrap_or("");
    let mut lines = vec![format!("session_resume_task_id={task_id}")];
    push_optional_line(&mut lines, "session_resume_status", summary, "/status");
    push_optional_line(
        &mut lines,
        "session_resume_lifecycle_state",
        summary,
        "/lifecycle_state",
    );
    push_optional_line(
        &mut lines,
        "session_resume_checkpoint_id",
        summary,
        "/checkpoint_id",
    );
    lines
}

fn push_optional_line(lines: &mut Vec<String>, key: &str, value: &Value, pointer: &str) {
    let Some(text) = string_at(value, pointer) else {
        return;
    };
    if !text.is_empty() {
        lines.push(format!("{key}: {text}"));
    }
}

fn first_string(tasks: &[Value], pointers: &[&str]) -> Option<String> {
    tasks.iter().find_map(|task| {
        pointers
            .iter()
            .find_map(|pointer| string_at(task, pointer))
            .filter(|value| !value.is_empty())
    })
}

fn string_at(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn active_tasks(base_url: &str, key: &str, user_id: i64, chat_id: i64) -> Result<Value> {
    let url = format!("{}/tasks/active", client::base_v1(base_url));
    let payload = json!({
        "user_id": user_id,
        "chat_id": chat_id,
        "exclude_task_id": Value::Null,
    });
    let resp = client::make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .context("session_active_list_failed")?;
    let status = resp.status();
    let body: Value = resp.json().context("session_active_parse_failed")?;
    if !status.is_success() {
        anyhow::bail!(
            "session active returned {}: {:?}",
            status,
            body.get("error")
        );
    }
    Ok(body)
}
