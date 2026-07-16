use std::time::Duration;

use anyhow::{Context, Result};

use crate::{commands, events, output, task};

const POLL_FALLBACK_INTERVAL_MS: u64 = 800;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChatControl<'a> {
    Exit,
    New,
    Detach,
    Cancel,
    Status,
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
    let followed = events::follow_task_events(base_url, key, &task_id, cursor, |raw_event| {
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
            && !events::task_event_is_background(raw_event))
    });
    commands::record_chat_cursor(thread, cursor)?;

    if let Err(error) = followed {
        eprintln!("error_code=chat_event_stream_failed detail={error}");
        wait_with_poll_fallback(base_url, key, &task_id)?;
    }
    let status = task::get_task_status(base_url, key, &task_id)?;
    output::print_task_status(&status, false, &events::EventFilters::default());
    Ok(())
}

fn wait_with_poll_fallback(base_url: &str, key: &str, task_id: &str) -> Result<()> {
    loop {
        let status = task::get_task_status(base_url, key, task_id)?;
        if status.is_terminal() || status.is_background_waiting() {
            return Ok(());
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
