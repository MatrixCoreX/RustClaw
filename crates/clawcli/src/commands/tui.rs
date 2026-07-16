use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use crate::{client, events::EventFilters, output, task};

use super::{
    permission::permission_report_json,
    report::{coding_review_json, subagent_report_json, task_report_json},
    task_query::watch_progress_json,
};

const TUI_DEFAULT_PAUSE_SECONDS: u64 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TuiCommand {
    Refresh,
    Watch,
    Pause,
    Cancel,
    Resume,
    Continue,
    Export,
    Report,
    Review,
    Subagents,
    Permission,
    Quit,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_tui(
    base_url: &str,
    key: &str,
    user_id: i64,
    chat_id: i64,
    selected_task_id: Option<&str>,
    include_events: bool,
    once: bool,
    interval_ms: u64,
    json_output: bool,
    interactive: bool,
    export_path: Option<&Path>,
) -> Result<()> {
    let interval = Duration::from_millis(interval_ms.max(250));
    loop {
        let active = active_tasks(base_url, key, user_id, chat_id)?;
        let selected = selected_task_id
            .map(|task_id| task::get_task_status(base_url, key, task_id))
            .transpose()?;
        if json_output {
            output::print_json_pretty(&tui_snapshot_json(&active, selected.as_ref()));
        } else {
            print!("\x1b[2J\x1b[H");
            output::print_active_task_table(&active);
            if let Some(selected) = selected.as_ref() {
                println!();
                output::print_task_status(selected, include_events, &EventFilters::default());
                for line in tui_selected_task_lines(selected) {
                    println!("{line}");
                }
            }
            if interactive {
                print_tui_command_help();
            }
        }
        if once {
            return Ok(());
        }
        if interactive && !json_output {
            match read_tui_command()? {
                TuiCommand::Refresh => continue,
                TuiCommand::Quit => return Ok(()),
                TuiCommand::Watch => {
                    let task_id = selected_task_id.context("selected_task_required_for_watch")?;
                    watch_selected_task(base_url, key, task_id, include_events, interval)?;
                }
                TuiCommand::Pause => {
                    let task_id = selected_task_id.context("selected_task_required_for_pause")?;
                    output::print_json_pretty(&task::pause_task_by_id(
                        base_url,
                        key,
                        task_id,
                        TUI_DEFAULT_PAUSE_SECONDS,
                    )?);
                }
                TuiCommand::Cancel => {
                    let task_id = selected_task_id.context("selected_task_required_for_cancel")?;
                    output::print_json_pretty(&task::cancel_task_by_id(base_url, key, task_id)?);
                }
                TuiCommand::Resume => {
                    let task_id = selected_task_id.context("selected_task_required_for_resume")?;
                    output::print_json_pretty(&task::resume_task_by_id(
                        base_url,
                        key,
                        task_id,
                        task::TaskResumeRequest {
                            resume_reason: Some("operator_tui"),
                            ..Default::default()
                        },
                    )?);
                }
                TuiCommand::Continue => {
                    let task_id =
                        selected_task_id.context("selected_task_required_for_continue")?;
                    let message = read_tui_continue_message()?;
                    let message = message.trim();
                    output::print_json_pretty(&task::resume_task_by_id(
                        base_url,
                        key,
                        task_id,
                        task::TaskResumeRequest {
                            resume_reason: Some("operator_tui_continue"),
                            user_message: (!message.is_empty()).then_some(message),
                            ..Default::default()
                        },
                    )?);
                }
                TuiCommand::Export => {
                    let export = tui_export_json(&active, selected.as_ref());
                    if let Some(path) = export_path {
                        write_tui_export(path, &export)?;
                        println!("export_path: {}", path.display());
                    } else {
                        output::print_json_pretty(&export);
                    }
                }
                TuiCommand::Report => {
                    let task = selected
                        .as_ref()
                        .context("selected_task_required_for_report")?;
                    output::print_json_pretty(&task_report_json(task, include_events));
                }
                TuiCommand::Review => {
                    let task = selected
                        .as_ref()
                        .context("selected_task_required_for_review")?;
                    output::print_json_pretty(&coding_review_json(task, include_events));
                }
                TuiCommand::Subagents => {
                    let task = selected
                        .as_ref()
                        .context("selected_task_required_for_subagents")?;
                    output::print_json_pretty(&subagent_report_json(task));
                }
                TuiCommand::Permission => {
                    let task = selected
                        .as_ref()
                        .context("selected_task_required_for_permission")?;
                    output::print_json_pretty(&permission_report_json(task));
                }
            }
            continue;
        }
        std::thread::sleep(interval);
    }
}

pub(super) fn tui_snapshot_json(active: &Value, selected: Option<&task::TaskStatusView>) -> Value {
    json!({
        "snapshot_kind": "rustclaw_cli_tui",
        "active": active,
        "selected_task": selected.map(|task| task.raw_data.clone()).unwrap_or(Value::Null),
        "selected_progress": selected.map(watch_progress_json).unwrap_or(Value::Null),
        "selected_summary": selected.map(|task| task_report_json(task, false)).unwrap_or(Value::Null),
    })
}

pub(super) fn tui_export_json(active: &Value, selected: Option<&task::TaskStatusView>) -> Value {
    json!({
        "export_kind": "rustclaw_cli_tui_export",
        "snapshot": tui_snapshot_json(active, selected),
        "selected_task_id": selected.map(|task| task.task_id.clone()).unwrap_or_default(),
    })
}

pub(super) fn tui_command_from_input(input: &str) -> Option<TuiCommand> {
    match input.trim().to_ascii_lowercase().as_str() {
        "" | "r" => Some(TuiCommand::Refresh),
        "w" => Some(TuiCommand::Watch),
        "p" => Some(TuiCommand::Pause),
        "c" => Some(TuiCommand::Cancel),
        "u" => Some(TuiCommand::Resume),
        "n" => Some(TuiCommand::Continue),
        "e" => Some(TuiCommand::Export),
        "1" => Some(TuiCommand::Report),
        "2" => Some(TuiCommand::Review),
        "3" => Some(TuiCommand::Subagents),
        "4" => Some(TuiCommand::Permission),
        "q" => Some(TuiCommand::Quit),
        _ => None,
    }
}

pub(super) fn tui_selected_task_lines(task: &task::TaskStatusView) -> Vec<String> {
    let progress = watch_progress_json(task);
    let summary = task_report_json(task, false);
    let mut lines = Vec::new();
    push_tui_machine_line(
        &mut lines,
        "tui_selected_checkpoint_id",
        &progress,
        "/checkpoint_id",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_resume_due",
        &progress,
        "/resume_due",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_resume_wait_seconds",
        &progress,
        "/resume_wait_seconds",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_next_action_kind",
        &progress,
        "/next_action_kind",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_pending_async_job_id",
        &progress,
        "/pending_async_job_id",
    );
    push_tui_machine_line(&mut lines, "tui_selected_poll_ref", &progress, "/poll_ref");
    push_tui_machine_line(
        &mut lines,
        "tui_selected_lease_owner",
        &progress,
        "/lease_owner",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_heartbeat_at",
        &progress,
        "/heartbeat_at",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_last_heartbeat_ts",
        &progress,
        "/last_heartbeat_ts",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_llm_call_count",
        &summary,
        "/llm/llm_call_count",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_llm_budget_status",
        &summary,
        "/llm/budget_health/status",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_changed_file_count",
        &summary,
        "/coding/changed_file_count",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_verification_command_count",
        &summary,
        "/coding/verification_command_count",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_verification_status",
        &summary,
        "/coding/state/verification_status",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_completed_side_effect_count",
        &summary,
        "/coding/state/completed_side_effect_count",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_unverified_risk",
        &summary,
        "/coding/unverified_risk",
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_artifact_ref_count",
        &summary,
        "/artifacts/ref_count",
    );
    push_first_tui_machine_line(
        &mut lines,
        "tui_selected_goal_id",
        &task.raw_data,
        &[
            "/goal/goal_id",
            "/task_goal/goal_id",
            "/result_json/task_goal/goal_id",
        ],
    );
    push_first_tui_machine_line(
        &mut lines,
        "tui_selected_goal_status",
        &task.raw_data,
        &[
            "/goal/goal_status",
            "/task_goal/goal_status",
            "/result_json/task_goal/goal_status",
        ],
    );
    push_tui_machine_line(
        &mut lines,
        "tui_selected_outcome_state",
        &summary,
        "/outcome/state",
    );
    push_tui_count_line(
        &mut lines,
        "tui_selected_done_condition_count",
        &summary,
        "/outcome/done_conditions",
    );
    push_tui_count_line(
        &mut lines,
        "tui_selected_current_progress_count",
        &summary,
        "/outcome/current_progress",
    );
    push_tui_count_line(
        &mut lines,
        "tui_selected_remaining_work_count",
        &summary,
        "/outcome/remaining_work",
    );
    lines
}

fn push_tui_machine_line(lines: &mut Vec<String>, key: &str, source: &Value, pointer: &str) {
    let Some(value) = source.pointer(pointer) else {
        return;
    };
    let text = match value {
        Value::String(value) => value.trim().to_string(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
    };
    if text.is_empty() {
        return;
    }
    lines.push(format!("{key}: {text}"));
}

fn push_first_tui_machine_line(
    lines: &mut Vec<String>,
    key: &str,
    source: &Value,
    pointers: &[&str],
) {
    for pointer in pointers {
        let before = lines.len();
        push_tui_machine_line(lines, key, source, pointer);
        if lines.len() != before {
            return;
        }
    }
}

fn push_tui_count_line(lines: &mut Vec<String>, key: &str, source: &Value, pointer: &str) {
    let Some(count) = source
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::len)
        .filter(|count| *count > 0)
    else {
        return;
    };
    lines.push(format!("{key}: {count}"));
}

fn print_tui_command_help() {
    println!();
    println!("tui_keys: r,w,p,c,u,n,e,1,2,3,4,q");
    println!("tui_key.r=refresh");
    println!("tui_key.w=watch");
    println!("tui_key.p=pause");
    println!("tui_key.c=cancel");
    println!("tui_key.u=resume");
    println!("tui_key.n=continue");
    println!("tui_key.e=export");
    println!("tui_key.1=report");
    println!("tui_key.2=review");
    println!("tui_key.3=subagents");
    println!("tui_key.4=permission");
    println!("tui_key.q=quit");
}

fn read_tui_command() -> Result<TuiCommand> {
    loop {
        print!("clawcli-tui> ");
        io::stdout().flush().context("flush tui prompt")?;
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("read tui command")?;
        if let Some(command) = tui_command_from_input(&input) {
            return Ok(command);
        }
        println!("unknown_key");
    }
}

fn read_tui_continue_message() -> Result<String> {
    print!("clawcli-tui-continue> ");
    io::stdout().flush().context("flush_tui_continue_prompt")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("read_tui_continue_message_failed")?;
    Ok(input)
}

fn watch_selected_task(
    base_url: &str,
    key: &str,
    task_id: &str,
    include_events: bool,
    interval: Duration,
) -> Result<()> {
    loop {
        let task = task::get_task_status(base_url, key, task_id)?;
        print!("\x1b[2J\x1b[H");
        output::print_task_status(&task, include_events, &EventFilters::default());
        if task.is_terminal() {
            return Ok(());
        }
        std::thread::sleep(interval);
    }
}

fn write_tui_export(path: &Path, value: &Value) -> Result<()> {
    let body = serde_json::to_string_pretty(value).context("serialize tui export")?;
    fs::write(path, body).with_context(|| format!("write tui export {}", path.display()))
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
        .context("list active tasks failed")?;
    let status = resp.status();
    let body: Value = resp.json().context("parse active response")?;
    if !status.is_success() {
        anyhow::bail!("active returned {}: {:?}", status, body.get("error"));
    }
    Ok(body)
}
