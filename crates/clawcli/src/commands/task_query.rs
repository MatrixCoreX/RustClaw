use anyhow::{Context, Result};
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{events::EventFilters, output, task};

use super::report::{
    coding_review_json, coding_review_text_lines, subagent_report_json, subagent_report_text_lines,
    task_report_json, task_report_text_lines,
};

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
                    "progress": watch_progress_json(&task),
                    "terminal": task.is_terminal(),
                    "event_count": task.events.len(),
                }))?
            );
        } else if json_output {
            output::print_json_pretty(&task.raw_data);
        } else {
            let snapshot = format!(
                "{}|{}|{}",
                task.status,
                task.lifecycle_summary_tokens().join(" "),
                task.task_plan_snapshot()
                    .map(|plan| plan.plan_revision.to_string())
                    .unwrap_or_default()
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

pub(super) fn watch_progress_json(task: &task::TaskStatusView) -> serde_json::Value {
    let lifecycle = task.lifecycle();
    let task_plan = task
        .task_plan_snapshot()
        .map(|plan| plan.raw)
        .unwrap_or(serde_json::Value::Null);
    json!({
        "execution_state": task.execution_state(),
        "lifecycle_state": task.lifecycle_state(),
        "db_status": lifecycle_field(lifecycle, "db_status"),
        "checkpoint_id": lifecycle_field(lifecycle, "checkpoint_id"),
        "can_poll": lifecycle_field(lifecycle, "can_poll"),
        "can_cancel": lifecycle_field(lifecycle, "can_cancel"),
        "resume_entrypoint": lifecycle_field(lifecycle, "resume_entrypoint"),
        "resume_directive": lifecycle_field(lifecycle, "resume_directive"),
        "resume_reason": lifecycle_field(lifecycle, "resume_reason"),
        "resume_due": lifecycle_field(lifecycle, "resume_due"),
        "resume_wait_seconds": lifecycle_field(lifecycle, "resume_wait_seconds"),
        "next_action_kind": lifecycle_field(lifecycle, "next_action_kind"),
        "reason_code": lifecycle_field(lifecycle, "reason_code"),
        "next_poll_after": lifecycle_field(lifecycle, "next_poll_after"),
        "poll_after_seconds": lifecycle_field(lifecycle, "poll_after_seconds"),
        "poll_ref": lifecycle_field(lifecycle, "poll_ref"),
        "cancel_ref": lifecycle_field(lifecycle, "cancel_ref"),
        "pending_async_job_id": lifecycle_field(lifecycle, "pending_async_job_id"),
        "async_job_message_key": lifecycle_field(lifecycle, "async_job_message_key"),
        "heartbeat_at": lifecycle_field(lifecycle, "heartbeat_at"),
        "last_heartbeat_ts": lifecycle_field(lifecycle, "last_heartbeat_ts"),
        "lease_owner": lifecycle_field(lifecycle, "lease_owner"),
        "lease_expires_at": lifecycle_field(lifecycle, "lease_expires_at"),
        "claim_attempt": lifecycle_field(lifecycle, "claim_attempt"),
        "attempt_id": lifecycle_field(lifecycle, "attempt_id"),
        "claimed_at": lifecycle_field(lifecycle, "claimed_at"),
        "task_plan": task_plan,
    })
}

fn lifecycle_field(lifecycle: Option<&serde_json::Value>, key: &str) -> serde_json::Value {
    lifecycle
        .and_then(|value| value.get(key))
        .cloned()
        .unwrap_or(serde_json::Value::Null)
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
    follow: bool,
    cursor: u64,
) -> Result<()> {
    let event_filters = EventFilters::from_parts(
        event_types,
        checkpoint_id,
        policy_decision,
        subagent_id,
        async_job_id,
    );
    if follow {
        return crate::events::follow_task_events(base_url, key, task_id, cursor, |raw_event| {
            let terminal = raw_event
                .get("event_kind")
                .or_else(|| raw_event.get("event_type"))
                .and_then(serde_json::Value::as_str)
                == Some("task_final");
            let output_mode = if jsonl_output {
                crate::events::LiveEventOutputMode::Jsonl
            } else {
                crate::events::LiveEventOutputMode::Compact
            };
            if let Some(line) =
                crate::events::live_task_event_output_line(raw_event, output_mode, &event_filters)?
            {
                println!("{line}");
            }
            Ok(!terminal)
        });
    }
    let task = task::get_task_status(base_url, key, task_id)?;
    let persisted = match crate::events::read_task_event_snapshot(base_url, key, task_id, cursor) {
        Ok(events) => Some(events),
        Err(error) if crate::events::task_event_stream_is_unavailable(&error) => None,
        Err(error) => return Err(error).context("task_event_snapshot_failed"),
    };
    let persisted_lines = persisted
        .as_deref()
        .map(crate::events::task_event_lines_from_raw);
    let events = persisted_lines
        .as_ref()
        .map(|events| {
            events
                .iter()
                .filter(|event| event_filters.is_empty() || event_filters.matches(event))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| output::filtered_events(&task, &event_filters));
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
            lines.push(crate::events::compact_task_event_line(event));
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
        false,
        0,
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

pub(crate) fn run_review(
    base_url: &str,
    key: &str,
    task_id: &str,
    json_output: bool,
    include_events: bool,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let review = coding_review_json(&task, include_events);
    if json_output {
        output::print_json_pretty(&review);
    } else {
        for line in coding_review_text_lines(&task, &review) {
            println!("{line}");
        }
    }
    Ok(())
}

pub(crate) fn run_review_once(
    base_url: &str,
    key: &str,
    task_id: &str,
    json_output: bool,
    include_events: bool,
    submission_options: task::TaskSubmissionOptions,
) -> Result<()> {
    let review_task_id = task::submit_auto_review(base_url, key, task_id, submission_options)?;
    let review_task = super::common::wait_for_terminal_task(base_url, key, &review_task_id, 250)?;
    let mut review = coding_review_json(&review_task, include_events);
    if let Some(object) = review.as_object_mut() {
        object.insert(
            "review_target_task_id".to_string(),
            serde_json::json!(task_id),
        );
        object.insert(
            "review_task_id".to_string(),
            serde_json::json!(review_task_id),
        );
        object.insert("triggered".to_string(), serde_json::json!(true));
    }
    if json_output {
        output::print_json_pretty(&review);
    } else {
        for line in coding_review_text_lines(&review_task, &review) {
            println!("{line}");
        }
        println!("review_target_task_id={task_id}");
        println!("review_task_id={review_task_id}");
    }
    Ok(())
}

pub(crate) fn run_subagents(
    base_url: &str,
    key: &str,
    task_id: &str,
    command: Option<&crate::SubagentCommand>,
    json_output: bool,
) -> Result<()> {
    if let Some(command) = command {
        return run_subagent_control(base_url, key, task_id, command, json_output);
    }
    let task = task::get_task_status(base_url, key, task_id)?;
    let report = subagent_report_json(&task);
    if json_output {
        output::print_json_pretty(&report);
    } else {
        for line in subagent_report_text_lines(&report) {
            println!("{line}");
        }
    }
    Ok(())
}

fn run_subagent_control(
    base_url: &str,
    key: &str,
    parent_task_id: &str,
    command: &crate::SubagentCommand,
    json_output: bool,
) -> Result<()> {
    match command {
        crate::SubagentCommand::Open {
            child_task_id,
            events,
        } => {
            let child = task::get_task_status(base_url, key, child_task_id)?;
            let report = task_report_json(&child, *events);
            if json_output {
                output::print_json_pretty(&report);
            } else {
                for line in task_report_text_lines(&child, &report) {
                    println!("{line}");
                }
            }
        }
        crate::SubagentCommand::Steer {
            child_task_id,
            message,
            constraints_json,
        } => {
            let new_constraints = constraints_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(|_| anyhow::anyhow!("subagent_constraints_json_invalid"))?;
            if message
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
                && new_constraints.is_none()
            {
                anyhow::bail!("subagent_steer_input_required");
            }
            let response = task::resume_task_by_id(
                base_url,
                key,
                child_task_id,
                task::TaskResumeRequest {
                    resume_reason: Some("subagent_steered"),
                    user_message: message.as_deref(),
                    new_constraints,
                    ..Default::default()
                },
            )?;
            output::print_json_pretty(&subagent_control_result(
                parent_task_id,
                child_task_id,
                "steer",
                response,
            ));
        }
        crate::SubagentCommand::Pause {
            child_task_id,
            pause_seconds,
        } => {
            let response = task::pause_task_by_id(base_url, key, child_task_id, *pause_seconds)?;
            output::print_json_pretty(&subagent_control_result(
                parent_task_id,
                child_task_id,
                "pause",
                response,
            ));
        }
        crate::SubagentCommand::Resume { child_task_id } => {
            let response = task::resume_task_by_id(
                base_url,
                key,
                child_task_id,
                task::TaskResumeRequest {
                    resume_reason: Some("subagent_resumed"),
                    ..Default::default()
                },
            )?;
            output::print_json_pretty(&subagent_control_result(
                parent_task_id,
                child_task_id,
                "resume",
                response,
            ));
        }
        crate::SubagentCommand::Stop { child_task_id } => {
            let response = task::cancel_task_by_id(base_url, key, child_task_id)?;
            output::print_json_pretty(&subagent_control_result(
                parent_task_id,
                child_task_id,
                "stop",
                response,
            ));
        }
        crate::SubagentCommand::StopAll => {
            let response = task::stop_child_tasks_by_parent(base_url, key, parent_task_id)?;
            output::print_json_pretty(&subagent_control_result(
                parent_task_id,
                "",
                "stop_all",
                response,
            ));
        }
        crate::SubagentCommand::Close { child_task_id } => {
            let response =
                task::close_child_task_by_id(base_url, key, parent_task_id, child_task_id)?;
            output::print_json_pretty(&subagent_control_result(
                parent_task_id,
                child_task_id,
                "close",
                response,
            ));
        }
    }
    Ok(())
}

fn subagent_control_result(
    parent_task_id: &str,
    child_task_id: &str,
    operation: &str,
    response: serde_json::Value,
) -> serde_json::Value {
    json!({
        "schema_version": 1,
        "operation": operation,
        "parent_task_id": parent_task_id,
        "child_task_id": (!child_task_id.is_empty()).then_some(child_task_id),
        "response": response,
    })
}
