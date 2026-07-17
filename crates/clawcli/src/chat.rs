use std::time::Duration;

use anyhow::{Context, Result};

use crate::{commands, events, output, task};

const POLL_FALLBACK_INTERVAL_MS: u64 = 800;
const STREAM_READ_WINDOW_SECONDS: u64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChatControl<'a> {
    Exit,
    New,
    Detach,
    Cancel,
    Status,
    Continue,
    Approve,
    ApproveScope,
    Deny,
    Attach(&'a str),
    Unknown(&'a str),
}

pub(crate) fn run_chat(
    base_url: &str,
    key: &str,
    requested_thread_id: Option<&str>,
    force_new: bool,
    jsonl_output: bool,
) -> Result<()> {
    crate::interrupt::install()?;
    let mut thread = commands::load_or_create_chat_thread(requested_thread_id, force_new)?;
    print_thread_binding(&thread);
    let mut editor = rustyline::DefaultEditor::new().context("chat_readline_init_failed")?;
    loop {
        let line = match editor.readline("> ") {
            Ok(line) => line,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(rustyline::error::ReadlineError::Interrupted) => break,
            Err(error) => return Err(error).context("chat_readline_failed"),
        };
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(control) = chat_control(text) {
            match control {
                ChatControl::Exit | ChatControl::Detach => break,
                ChatControl::New => {
                    thread = commands::load_or_create_chat_thread(None, true)?;
                    print_thread_binding(&thread);
                }
                ChatControl::Cancel => {
                    if let Some(task_id) = thread.current_task_id.as_deref() {
                        let body = task::cancel_task_by_id(base_url, key, task_id)?;
                        output::print_json_pretty(&body);
                    } else {
                        println!("error_code=chat_task_missing");
                    }
                }
                ChatControl::Status => {
                    if let Some(task_id) = thread.current_task_id.as_deref() {
                        let status = task::get_task_status(base_url, key, task_id)?;
                        output::print_task_status(&status, false, &events::EventFilters::default());
                    } else {
                        println!("error_code=chat_task_missing");
                    }
                }
                ChatControl::Continue => {
                    continue_current_task(base_url, key, &mut thread, jsonl_output)?;
                }
                ChatControl::Approve => {
                    decide_current_task_approval(
                        base_url,
                        key,
                        &mut thread,
                        "approve_once",
                        jsonl_output,
                    )?;
                }
                ChatControl::ApproveScope => {
                    decide_current_task_approval(
                        base_url,
                        key,
                        &mut thread,
                        "always_for_scope",
                        jsonl_output,
                    )?;
                }
                ChatControl::Deny => {
                    decide_current_task_approval(base_url, key, &mut thread, "deny", jsonl_output)?;
                }
                ChatControl::Attach(task_id) => {
                    commands::record_chat_task(&mut thread, task_id)?;
                    follow_and_render_task(base_url, key, &mut thread, jsonl_output)?;
                }
                ChatControl::Unknown(command) => {
                    println!("error_code=chat_command_unknown command={command}");
                }
            }
            continue;
        }

        let task_id = task::submit_thread_ask(
            base_url,
            key,
            text,
            &thread.thread_id,
            &thread.session_id,
            thread.current_task_id.as_deref(),
        )?;
        commands::record_chat_task(&mut thread, &task_id)?;
        println!("task_id={task_id}");
        follow_and_render_task(base_url, key, &mut thread, jsonl_output)?;
    }
    Ok(())
}

fn follow_and_render_task(
    base_url: &str,
    key: &str,
    thread: &mut commands::ChatThreadState,
    jsonl_output: bool,
) -> Result<()> {
    let task_id = thread
        .current_task_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("chat_task_missing"))?;
    let mut cursor = thread.last_event_seq;
    loop {
        if crate::interrupt::requested() {
            return finish_chat_detach(thread, cursor, &task_id);
        }
        let followed = events::follow_task_events_with_timeout(
            base_url,
            key,
            &task_id,
            cursor,
            Some(Duration::from_secs(STREAM_READ_WINDOW_SECONDS)),
            |raw_event| {
                if let Some(seq) = events::task_event_seq(raw_event) {
                    cursor = cursor.max(seq);
                }
                let output_mode = if jsonl_output {
                    events::LiveEventOutputMode::Jsonl
                } else {
                    events::LiveEventOutputMode::Compact
                };
                if let Some(line) = events::live_task_event_output_line(
                    raw_event,
                    output_mode,
                    &events::EventFilters::default(),
                )? {
                    println!("{line}");
                }
                Ok(!events::task_event_is_terminal(raw_event)
                    && !events::task_event_is_background(raw_event)
                    && !crate::interrupt::requested())
            },
        );
        commands::record_chat_cursor(thread, cursor)?;
        if crate::interrupt::requested() {
            return finish_chat_detach(thread, cursor, &task_id);
        }
        match followed {
            Ok(()) => {
                let status = task::get_task_status(base_url, key, &task_id)?;
                if status.is_terminal() || status.is_background_waiting() {
                    output::print_task_status(&status, false, &events::EventFilters::default());
                    return Ok(());
                }
                continue;
            }
            Err(error) if events::task_event_stream_timed_out(&error) => continue,
            Err(error) => {
                eprintln!("error_code=chat_event_stream_failed detail={error}");
                if wait_with_poll_fallback(base_url, key, &task_id)? {
                    return finish_chat_detach(thread, cursor, &task_id);
                }
                break;
            }
        }
    }
    let status = task::get_task_status(base_url, key, &task_id)?;
    output::print_task_status(&status, false, &events::EventFilters::default());
    Ok(())
}

fn finish_chat_detach(
    thread: &mut commands::ChatThreadState,
    cursor: u64,
    task_id: &str,
) -> Result<()> {
    commands::record_chat_cursor(thread, cursor)?;
    println!("task_id={task_id}");
    println!("chat_outcome=detached");
    println!("event_cursor={cursor}");
    crate::interrupt::reset();
    Ok(())
}

fn decide_current_task_approval(
    base_url: &str,
    key: &str,
    thread: &mut commands::ChatThreadState,
    decision: &str,
    jsonl_output: bool,
) -> Result<()> {
    let Some(task_id) = thread.current_task_id.clone() else {
        println!("error_code=chat_task_missing");
        return Ok(());
    };
    let status = task::get_task_status(base_url, key, &task_id)?;
    let Some(request_id) = status.pending_approval_request_id() else {
        println!("error_code=chat_approval_request_missing");
        return Ok(());
    };
    let body = task::resume_task_by_id(
        base_url,
        key,
        &task_id,
        task::TaskResumeRequest {
            approval_request_id: Some(request_id),
            approval_decision: Some(decision),
            ..Default::default()
        },
    )?;
    output::print_json_pretty(&body);
    if matches!(decision, "approve_once" | "always_for_scope") {
        crate::interrupt::reset();
        follow_and_render_task(base_url, key, thread, jsonl_output)
    } else {
        let status = task::get_task_status(base_url, key, &task_id)?;
        output::print_task_status(&status, false, &events::EventFilters::default());
        Ok(())
    }
}

fn continue_current_task(
    base_url: &str,
    key: &str,
    thread: &mut commands::ChatThreadState,
    jsonl_output: bool,
) -> Result<()> {
    let Some(task_id) = thread.current_task_id.clone() else {
        println!("error_code=chat_task_missing");
        return Ok(());
    };
    let body = task::resume_task_by_id(
        base_url,
        key,
        &task_id,
        task::TaskResumeRequest {
            resume_reason: Some("user_continue"),
            ..Default::default()
        },
    )?;
    output::print_json_pretty(&body);
    crate::interrupt::reset();
    follow_and_render_task(base_url, key, thread, jsonl_output)
}

fn wait_with_poll_fallback(base_url: &str, key: &str, task_id: &str) -> Result<bool> {
    loop {
        if crate::interrupt::requested() {
            return Ok(true);
        }
        let status = task::get_task_status(base_url, key, task_id)?;
        if status.is_terminal() || status.is_background_waiting() {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(POLL_FALLBACK_INTERVAL_MS));
    }
}

pub(super) fn chat_control(input: &str) -> Option<ChatControl<'_>> {
    let mut parts = input.split_whitespace();
    let command = parts.next()?;
    if !command.starts_with('/') {
        return None;
    }
    let argument = parts.next();
    if parts.next().is_some() {
        return Some(ChatControl::Unknown(command));
    }
    Some(match (command, argument) {
        ("/exit", None) => ChatControl::Exit,
        ("/new", None) => ChatControl::New,
        ("/detach", None) => ChatControl::Detach,
        ("/cancel", None) => ChatControl::Cancel,
        ("/status", None) => ChatControl::Status,
        ("/continue", None) => ChatControl::Continue,
        ("/approve", None) => ChatControl::Approve,
        ("/approve-scope", None) => ChatControl::ApproveScope,
        ("/deny", None) => ChatControl::Deny,
        ("/attach", Some(task_id)) => ChatControl::Attach(task_id),
        _ => ChatControl::Unknown(command),
    })
}

fn print_thread_binding(thread: &commands::ChatThreadState) {
    println!("thread_id={}", thread.thread_id);
    println!("session_id={}", thread.session_id);
    if let Some(task_id) = thread.current_task_id.as_deref() {
        println!("current_task_id={task_id}");
    }
    println!("event_cursor={}", thread.last_event_seq);
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
