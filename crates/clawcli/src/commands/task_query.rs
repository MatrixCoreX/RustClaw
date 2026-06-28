use anyhow::{Context, Result};
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{events::EventFilters, output, task};

use super::report::{task_report_json, task_report_text_lines};

pub(crate) fn run_get(
    base_url: &str,
    key: &str,
    task_id: &str,
    events: bool,
    event_types: &[String],
    checkpoint_id: Option<&str>,
    policy_decision: Option<&str>,
    subagent_id: Option<&str>,
    async_job_id: Option<&str>,
    events_output: Option<&PathBuf>,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let event_filters = EventFilters::from_parts(
        event_types,
        checkpoint_id,
        policy_decision,
        subagent_id,
        async_job_id,
    );
    output::print_task_status(&task, events || !event_filters.is_empty(), &event_filters);
    let filtered_events = output::filtered_event_lines(&task, &event_filters);
    if let Some(path) = events_output {
        let mut content = filtered_events.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        std::fs::write(path, content)
            .with_context(|| format!("write events output failed: path={}", path.display()))?;
    }
    Ok(())
}

pub(crate) fn run_watch(
    base_url: &str,
    key: &str,
    task_id: &str,
    events: bool,
    event_types: &[String],
    checkpoint_id: Option<&str>,
    policy_decision: Option<&str>,
    subagent_id: Option<&str>,
    async_job_id: Option<&str>,
    until_terminal: bool,
    interval_ms: u64,
    json_output: bool,
    jsonl_output: bool,
) -> Result<()> {
    let event_filters = EventFilters::from_parts(
        event_types,
        checkpoint_id,
        policy_decision,
        subagent_id,
        async_job_id,
    );
    let mut last_snapshot = String::new();
    let mut seen_events = HashSet::new();
    let interval = Duration::from_millis(interval_ms.max(100));

    loop {
        let task = task::get_task_status(base_url, key, task_id)?;
        if jsonl_output {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "task_id": &task.task_id,
                    "status": &task.status,
                    "lifecycle_state": task.lifecycle_state(),
                    "lifecycle": task.lifecycle().cloned().unwrap_or(serde_json::Value::Null),
                    "terminal": task.is_terminal(),
                    "event_count": task.events.len(),
                }))?
            );
        } else if json_output {
            output::print_json_pretty(&task.raw_data);
        } else {
            let snapshot = format!(
                "{}|{}",
                task.status,
                task.lifecycle_summary_tokens().join(" ")
            );
            if snapshot != last_snapshot {
                output::print_task_status(&task, false, &event_filters);
                last_snapshot = snapshot;
            }
        }

        if events || !event_filters.is_empty() {
            for line in output::filtered_event_lines(&task, &event_filters) {
                if seen_events.insert(line.clone()) {
                    println!("{line}");
                }
            }
        }

        if until_terminal && task.is_terminal() {
            break;
        }
        std::thread::sleep(interval);
    }
    Ok(())
}

pub(crate) fn run_wait(
    base_url: &str,
    key: &str,
    task_id: &str,
    until: &str,
    timeout_seconds: Option<u64>,
    interval_ms: u64,
    json_output: bool,
    jsonl_output: bool,
) -> Result<u8> {
    let interval = Duration::from_millis(interval_ms.max(100));
    let deadline = timeout_seconds.map(|seconds| Instant::now() + Duration::from_secs(seconds));
    loop {
        let task = task::get_task_status(base_url, key, task_id)?;
        if wait_until_matches(&task, until) {
            let summary = wait_summary_json(&task, until, true, 0);
            if json_output || jsonl_output {
                output::print_json_pretty(&summary);
            } else {
                output::print_task_status(&task, false, &EventFilters::default());
                println!("wait_until: {until}");
                println!("wait_matched: true");
                println!("wait_exit_code: 0");
            }
            return Ok(0);
        }
        if let Some(deadline) = deadline {
            if Instant::now() >= deadline {
                let exit_code = 124;
                let summary = wait_summary_json(&task, until, false, exit_code);
                if json_output || jsonl_output {
                    output::print_json_pretty(&summary);
                } else {
                    output::print_task_status(&task, false, &EventFilters::default());
                    eprintln!("error_code=wait_timeout");
                    println!("wait_until: {until}");
                    println!("wait_matched: false");
                    println!("wait_exit_code: {exit_code}");
                }
                return Ok(exit_code);
            }
        }
        if jsonl_output {
            println!(
                "{}",
                serde_json::to_string(&wait_summary_json(&task, until, false, 0))?
            );
        }
        std::thread::sleep(interval);
    }
}

pub(super) fn wait_until_matches(task: &task::TaskStatusView, until: &str) -> bool {
    match until {
        "completed" => {
            task.execution_state() == Some("completed") || task.status.as_str() == "succeeded"
        }
        "terminal" => task.is_terminal(),
        "background" => matches!(
            task.execution_state().or_else(|| task.lifecycle_state()),
            Some("background" | "waiting")
        ),
        "needs_user" => matches!(
            task.execution_state().or_else(|| task.lifecycle_state()),
            Some("needs_user" | "needs_confirmation")
        ),
        _ => false,
    }
}

fn wait_summary_json(
    task: &task::TaskStatusView,
    until: &str,
    matched: bool,
    exit_code: u8,
) -> serde_json::Value {
    json!({
        "task_id": task.task_id,
        "status": task.status,
        "execution_state": task.execution_state(),
        "lifecycle_state": task.lifecycle_state(),
        "lifecycle": task.lifecycle().cloned().unwrap_or(serde_json::Value::Null),
        "terminal": task.is_terminal(),
        "wait_until": until,
        "matched": matched,
        "exit_code": exit_code,
    })
}

pub(crate) fn run_events(
    base_url: &str,
    key: &str,
    task_id: &str,
    event_types: &[String],
    checkpoint_id: Option<&str>,
    policy_decision: Option<&str>,
    subagent_id: Option<&str>,
    async_job_id: Option<&str>,
    jsonl_output: bool,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let event_filters = EventFilters::from_parts(
        event_types,
        checkpoint_id,
        policy_decision,
        subagent_id,
        async_job_id,
    );
    let events = output::filtered_events(&task, &event_filters);
    for line in task_event_output_lines(&task, events, jsonl_output)? {
        println!("{line}");
    }
    Ok(())
}

pub(super) fn task_event_output_lines(
    task: &task::TaskStatusView,
    events: Vec<&crate::events::TaskEventLine>,
    jsonl_output: bool,
) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    for event in events {
        if jsonl_output {
            lines.push(serde_json::to_string(&json!({
                "task_id": &task.task_id,
                "event_type": &event.event_type,
                "line": &event.line,
                "fields": &event.fields,
            }))?);
        } else {
            lines.push(format!("event: {}", event.line));
        }
    }
    Ok(lines)
}

pub(crate) fn run_logs(
    base_url: &str,
    key: &str,
    task_id: &str,
    event_types: &[String],
    checkpoint_id: Option<&str>,
    policy_decision: Option<&str>,
    subagent_id: Option<&str>,
    async_job_id: Option<&str>,
    jsonl_output: bool,
) -> Result<()> {
    run_events(
        base_url,
        key,
        task_id,
        event_types,
        checkpoint_id,
        policy_decision,
        subagent_id,
        async_job_id,
        jsonl_output,
    )
}

pub(crate) fn run_report(
    base_url: &str,
    key: &str,
    task_id: &str,
    json_output: bool,
    include_events: bool,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let report = task_report_json(&task, include_events);
    if json_output {
        output::print_json_pretty(&report);
    } else {
        for line in task_report_text_lines(&task, &report) {
            println!("{line}");
        }
        if let Some(error_text) = task.error_text.as_deref() {
            eprintln!("error: {error_text}");
        }
    }
    Ok(())
}
