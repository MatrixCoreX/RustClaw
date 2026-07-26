use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::chat_attachments::{
    attachment_payload, extract_path_references, inspect_attachment, merge_attachment,
    RequestedAttachmentKind,
};
use crate::chat_command::{command_specs, parse_chat_command, ChatCommand};
use crate::chat_editor::{normalize_multiline_input, ChatEditorHelper};
use crate::chat_session::{ChatSessionState, ChatSessionTransition, PermissionMode};
use crate::{commands, events, output, task};

const POLL_FALLBACK_INTERVAL_MS: u64 = 800;
const STREAM_READ_WINDOW_SECONDS: u64 = 2;

pub(crate) fn run_chat(
    base_url: &str,
    key: &str,
    requested_conversation_id: Option<&str>,
    force_new: bool,
    jsonl_output: bool,
    submission_options: task::TaskSubmissionOptions,
    startup_files: &[PathBuf],
    startup_images: &[PathBuf],
) -> Result<()> {
    crate::interrupt::install()?;
    let mut session = commands::load_or_create_chat_session(requested_conversation_id, force_new)?;
    if submission_options.yolo {
        session.apply(ChatSessionTransition::PermissionChanged(
            PermissionMode::Yolo,
        ))?;
    }
    add_startup_attachments(&mut session, startup_files, startup_images, jsonl_output)?;
    commands::persist_chat_session(&session)?;
    print_session_binding(&session, jsonl_output)?;
    let terminal = crate::terminal_capabilities::detect(jsonl_output);
    let mut editor = if terminal.stdin_tty && terminal.stdout_tty {
        let mut editor =
            rustyline::Editor::<ChatEditorHelper, rustyline::history::DefaultHistory>::new()
                .context("chat_readline_init_failed")?;
        editor.set_helper(Some(ChatEditorHelper));
        Some(editor)
    } else {
        None
    };
    loop {
        let line = if let Some(editor) = editor.as_mut() {
            match editor.readline("> ") {
                Ok(line) => line,
                Err(rustyline::error::ReadlineError::Eof) => break,
                Err(rustyline::error::ReadlineError::Interrupted) => break,
                Err(error) => return Err(error).context("chat_readline_failed"),
            }
        } else {
            let mut line = String::new();
            let count = std::io::stdin()
                .read_line(&mut line)
                .context("chat_stdin_read_failed")?;
            if count == 0 {
                break;
            }
            line
        };
        let normalized_line = normalize_multiline_input(&line);
        let text = normalized_line.trim();
        if text.is_empty() {
            continue;
        }
        if let Some(editor) = editor.as_mut() {
            editor
                .add_history_entry(text)
                .context("chat_history_add_failed")?;
        }
        if let Some(command) = parse_chat_command(text) {
            let command = match command {
                Ok(command) => command,
                Err(error) => {
                    println!("{}", serde_json::to_string(&error)?);
                    continue;
                }
            };
            match command {
                ChatCommand::Exit | ChatCommand::Detach => break,
                ChatCommand::Help => print_command_help(jsonl_output)?,
                ChatCommand::New => {
                    session = commands::load_or_create_chat_session(None, true)?;
                    print_session_binding(&session, jsonl_output)?;
                }
                ChatCommand::Resume(conversation_id) => {
                    session = commands::load_or_create_chat_session(Some(&conversation_id), false)?;
                    print_session_binding(&session, jsonl_output)?;
                }
                ChatCommand::ResumeTask(task_id) => {
                    session.apply(ChatSessionTransition::TaskSelected(task_id))?;
                    commands::persist_chat_session(&session)?;
                    follow_and_render_task(base_url, key, &mut session, jsonl_output)?;
                }
                ChatCommand::Cancel => {
                    if let Some(task_id) = session.active_task_id.as_deref() {
                        let body = task::cancel_task_by_id(base_url, key, task_id)?;
                        print_chat_response("task_cancel_response", &body, jsonl_output)?;
                    } else {
                        print_chat_error("chat_task_missing", jsonl_output)?;
                    }
                }
                ChatCommand::Status => {
                    if let Some(task_id) = session.active_task_id.as_deref() {
                        let status = task::get_task_status(base_url, key, task_id)?;
                        print_chat_task_status(&status, jsonl_output, false)?;
                    } else {
                        print_chat_error("chat_task_missing", jsonl_output)?;
                    }
                }
                ChatCommand::Continue => {
                    continue_current_task(base_url, key, &mut session, jsonl_output)?;
                }
                ChatCommand::Approve => {
                    decide_current_task_approval(
                        base_url,
                        key,
                        &mut session,
                        "approve_once",
                        jsonl_output,
                    )?;
                }
                ChatCommand::ApproveScope => {
                    decide_current_task_approval(
                        base_url,
                        key,
                        &mut session,
                        "always_for_scope",
                        jsonl_output,
                    )?;
                }
                ChatCommand::Deny => {
                    decide_current_task_approval(
                        base_url,
                        key,
                        &mut session,
                        "deny",
                        jsonl_output,
                    )?;
                }
                ChatCommand::Model(requested) => {
                    update_model_override(
                        base_url,
                        key,
                        &mut session,
                        requested.as_deref(),
                        jsonl_output,
                    )?;
                }
                ChatCommand::Permissions(requested) => {
                    if let Some(mode) = requested {
                        session.apply(ChatSessionTransition::PermissionChanged(mode))?;
                        commands::persist_chat_session(&session)?;
                    }
                    print_permission_mode(&session, jsonl_output)?;
                }
                ChatCommand::File(path) => add_attachment(
                    &mut session,
                    &path,
                    RequestedAttachmentKind::File,
                    jsonl_output,
                )?,
                ChatCommand::Image(path) => add_attachment(
                    &mut session,
                    &path,
                    RequestedAttachmentKind::Image,
                    jsonl_output,
                )?,
                ChatCommand::Files | ChatCommand::Attachments => {
                    print_attachments(&session, jsonl_output)?;
                }
                ChatCommand::Diff(paths) => {
                    let task_id = task::submit_capability(
                        base_url,
                        key,
                        "workspace.diff",
                        commands::workspace_diff_args(None, &paths),
                        session_submission_options(&session),
                    )?;
                    commands::record_chat_session_task(&mut session, &task_id)?;
                    follow_and_render_task(base_url, key, &mut session, jsonl_output)?;
                }
                ChatCommand::Compact => {
                    let task_id = task::submit_conversation_compaction(
                        base_url,
                        key,
                        &session.conversation_id,
                        &session.session_id,
                        session.active_task_id.as_deref(),
                        session_submission_options(&session),
                    )?;
                    commands::record_chat_session_task(&mut session, &task_id)?;
                    follow_and_render_task(base_url, key, &mut session, jsonl_output)?;
                    let status = task::get_task_status(base_url, key, &task_id)?;
                    let reference = status
                        .raw_data
                        .pointer("/result_json/compaction/compaction_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                        .ok_or_else(|| anyhow::anyhow!("chat_compaction_ref_missing"))?;
                    session.apply(ChatSessionTransition::ContextCompacted(reference))?;
                    commands::persist_chat_session(&session)?;
                }
                ChatCommand::Goal => print_goal(base_url, key, &session)?,
            }
            continue;
        }

        for path in extract_path_references(text)? {
            let attachment = inspect_attachment(
                &session.working_directory,
                &path,
                RequestedAttachmentKind::File,
            )?;
            merge_attachment(&mut session.attachments, attachment)?;
        }
        commands::persist_chat_session(&session)?;
        let attachments = attachment_payload(&session.attachments)?;
        let task_id = task::submit_thread_ask(
            base_url,
            key,
            text,
            task::ThreadAskContext {
                conversation_id: &session.conversation_id,
                session_id: &session.session_id,
                resume_task_id: session.active_task_id.as_deref(),
                model_override: session.model_override.as_ref(),
                compacted_context_ref: session.compacted_context_ref.as_deref(),
                goal_ref: session.goal_ref.as_deref(),
                attachments: &attachments,
            },
            session_submission_options(&session),
        )?;
        session.apply(ChatSessionTransition::AttachmentsCleared)?;
        commands::record_chat_session_task(&mut session, &task_id)?;
        commands::persist_chat_session(&session)?;
        print_task_submitted(&task_id, jsonl_output)?;
        follow_and_render_task(base_url, key, &mut session, jsonl_output)?;
    }
    Ok(())
}

fn follow_and_render_task(
    base_url: &str,
    key: &str,
    session: &mut ChatSessionState,
    jsonl_output: bool,
) -> Result<()> {
    let task_id = session
        .active_task_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("chat_task_missing"))?;
    let mut cursor = session.event_cursor;
    let mut presentation = crate::assistant_presentation::AssistantPresentationReducer::default();
    let mut presentation_wrote_content = false;
    let mut presentation_line_open = false;
    if cursor > 0 {
        match events::read_task_event_snapshot(base_url, key, &task_id, 0) {
            Ok(replayed) => {
                for raw_event in replayed {
                    if let Some(event) = crate::assistant_presentation::decode(&raw_event)? {
                        let _ = presentation.apply(event)?;
                    }
                }
                if !jsonl_output {
                    if let Some(content) = presentation
                        .latest_display_content()
                        .filter(|content| !content.is_empty())
                    {
                        print!("{content}");
                        io::stdout().flush().context("chat_output_flush_failed")?;
                        presentation_wrote_content = true;
                        presentation_line_open = !content.ends_with('\n');
                    }
                }
            }
            Err(error) if events::task_event_stream_is_unavailable(&error) => {}
            Err(error) => return Err(error).context("chat_presentation_replay_failed"),
        }
    }
    loop {
        if crate::interrupt::requested() {
            return finish_chat_detach(session, cursor, &task_id, jsonl_output);
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
                if let Some(event) = crate::assistant_presentation::decode(raw_event)? {
                    let update = presentation.apply(event)?;
                    if jsonl_output {
                        println!("{}", serde_json::to_string(raw_event)?);
                    } else {
                        match update {
                            crate::assistant_presentation::PresentationUpdate::Delta(content) => {
                                print!("{content}");
                                io::stdout().flush().context("chat_output_flush_failed")?;
                                presentation_wrote_content = true;
                                presentation_line_open = !content.ends_with('\n');
                            }
                            crate::assistant_presentation::PresentationUpdate::Completed
                            | crate::assistant_presentation::PresentationUpdate::Aborted => {
                                if presentation_line_open {
                                    println!();
                                    presentation_line_open = false;
                                }
                            }
                            crate::assistant_presentation::PresentationUpdate::Started
                            | crate::assistant_presentation::PresentationUpdate::Replaced
                            | crate::assistant_presentation::PresentationUpdate::Duplicate => {}
                        }
                    }
                    return Ok(!events::task_event_is_terminal(raw_event)
                        && !events::task_event_is_background(raw_event)
                        && !crate::interrupt::requested());
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
        commands::record_chat_session_cursor(session, cursor)?;
        if crate::interrupt::requested() {
            return finish_chat_detach(session, cursor, &task_id, jsonl_output);
        }
        match followed {
            Ok(()) => {
                let status = task::get_task_status(base_url, key, &task_id)?;
                if status.is_terminal() || status.is_background_waiting() {
                    print_chat_task_status(
                        &status,
                        jsonl_output,
                        presentation_wrote_content
                            && presentation.completed_matches(status.result_text.as_deref()),
                    )?;
                    return Ok(());
                }
                continue;
            }
            Err(error) if events::task_event_stream_timed_out(&error) => continue,
            Err(error) => {
                eprintln!("error_code=chat_event_stream_failed detail={error}");
                if wait_with_poll_fallback(base_url, key, &task_id)? {
                    return finish_chat_detach(session, cursor, &task_id, jsonl_output);
                }
                break;
            }
        }
    }
    let status = task::get_task_status(base_url, key, &task_id)?;
    print_chat_task_status(
        &status,
        jsonl_output,
        presentation_wrote_content && presentation.completed_matches(status.result_text.as_deref()),
    )?;
    Ok(())
}

fn print_chat_task_status(
    status: &task::TaskStatusView,
    jsonl_output: bool,
    presentation_matches_final: bool,
) -> Result<()> {
    if jsonl_output {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "schema_version": 1,
                "record_type": "task_status",
                "status": "ok",
                "task": status.raw_data,
            }))?
        );
    } else if presentation_matches_final {
        output::print_task_status_without_result(status, false, &events::EventFilters::default());
    } else {
        output::print_task_status(status, false, &events::EventFilters::default());
    }
    Ok(())
}

fn finish_chat_detach(
    session: &mut ChatSessionState,
    cursor: u64,
    task_id: &str,
    jsonl_output: bool,
) -> Result<()> {
    commands::record_chat_session_cursor(session, cursor)?;
    if jsonl_output {
        print_jsonl_record(&serde_json::json!({
            "schema_version": 1,
            "record_type": "chat_outcome",
            "status": "detached",
            "task_id": task_id,
            "event_cursor": cursor,
        }))?;
    } else {
        println!("task_id={task_id}");
        println!("chat_outcome=detached");
        println!("event_cursor={cursor}");
    }
    crate::interrupt::reset();
    Ok(())
}

fn decide_current_task_approval(
    base_url: &str,
    key: &str,
    session: &mut ChatSessionState,
    decision: &str,
    jsonl_output: bool,
) -> Result<()> {
    let Some(task_id) = session.active_task_id.clone() else {
        print_chat_error("chat_task_missing", jsonl_output)?;
        return Ok(());
    };
    let status = task::get_task_status(base_url, key, &task_id)?;
    let Some(request_id) = status.pending_approval_request_id() else {
        print_chat_error("chat_approval_request_missing", jsonl_output)?;
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
    print_chat_response("task_approval_response", &body, jsonl_output)?;
    if matches!(decision, "approve_once" | "always_for_scope") {
        crate::interrupt::reset();
        follow_and_render_task(base_url, key, session, jsonl_output)
    } else {
        let status = task::get_task_status(base_url, key, &task_id)?;
        output::print_task_status(&status, false, &events::EventFilters::default());
        Ok(())
    }
}

fn continue_current_task(
    base_url: &str,
    key: &str,
    session: &mut ChatSessionState,
    jsonl_output: bool,
) -> Result<()> {
    let Some(task_id) = session.active_task_id.clone() else {
        print_chat_error("chat_task_missing", jsonl_output)?;
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
    print_chat_response("task_resume_response", &body, jsonl_output)?;
    crate::interrupt::reset();
    follow_and_render_task(base_url, key, session, jsonl_output)
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

fn session_submission_options(session: &ChatSessionState) -> task::TaskSubmissionOptions {
    task::TaskSubmissionOptions {
        yolo: false,
        permission_mode: Some(session.permission_mode),
    }
}

fn print_session_binding(session: &ChatSessionState, jsonl_output: bool) -> Result<()> {
    if jsonl_output {
        print_jsonl_record(&serde_json::json!({
            "schema_version": 1,
            "record_type": "chat_session",
            "status": "ok",
            "conversation_id": session.conversation_id,
            "session_id": session.session_id,
            "current_task_id": session.active_task_id,
            "event_cursor": session.event_cursor,
            "permission_mode": session.permission_mode.as_token(),
        }))?;
        return Ok(());
    }
    println!("conversation_id={}", session.conversation_id);
    println!("session_id={}", session.session_id);
    if let Some(task_id) = session.active_task_id.as_deref() {
        println!("current_task_id={task_id}");
    }
    println!("event_cursor={}", session.event_cursor);
    println!("permission_mode={}", session.permission_mode.as_token());
    Ok(())
}

fn print_command_help(jsonl_output: bool) -> Result<()> {
    for spec in command_specs() {
        if jsonl_output {
            print_jsonl_record(&serde_json::json!({
                "schema_version": 1,
                "record_type": "chat_command",
                "status": "ok",
                "command": spec.token,
                "argument_shape": spec.argument_shape,
            }))?;
        } else {
            println!(
                "chat_command={} argument_shape={}",
                spec.token, spec.argument_shape
            );
        }
    }
    Ok(())
}

fn update_model_override(
    base_url: &str,
    key: &str,
    session: &mut ChatSessionState,
    requested: Option<&str>,
    jsonl_output: bool,
) -> Result<()> {
    let selection = match requested {
        None => {
            if let Some(selection) = session.model_override.as_ref() {
                Some(selection.clone())
            } else {
                None
            }
        }
        Some("default") => {
            session.apply(ChatSessionTransition::ModelChanged(None))?;
            commands::persist_chat_session(session)?;
            None
        }
        Some(requested) => {
            let selection = commands::resolve_chat_model_override(base_url, key, requested)?;
            session.apply(ChatSessionTransition::ModelChanged(Some(selection.clone())))?;
            commands::persist_chat_session(session)?;
            Some(selection)
        }
    };
    if jsonl_output {
        print_jsonl_record(&serde_json::json!({
            "schema_version": 1,
            "record_type": "chat_model",
            "status": "ok",
            "provider": selection.as_ref().map(|selection| selection.provider.as_str()),
            "model": selection.as_ref().map(|selection| selection.model.as_str()),
            "uses_default": selection.is_none(),
        }))?;
    } else if let Some(selection) = selection {
        println!("model_provider={}", selection.provider);
        println!("model_id={}", selection.model);
    } else {
        println!("model_id=default");
    }
    Ok(())
}

fn add_startup_attachments(
    session: &mut ChatSessionState,
    files: &[PathBuf],
    images: &[PathBuf],
    jsonl_output: bool,
) -> Result<()> {
    for path in files {
        add_attachment(session, path, RequestedAttachmentKind::File, jsonl_output)?;
    }
    for path in images {
        add_attachment(session, path, RequestedAttachmentKind::Image, jsonl_output)?;
    }
    Ok(())
}

fn add_attachment(
    session: &mut ChatSessionState,
    path: &Path,
    kind: RequestedAttachmentKind,
    jsonl_output: bool,
) -> Result<()> {
    let attachment = inspect_attachment(&session.working_directory, path, kind)?;
    merge_attachment(&mut session.attachments, attachment)?;
    commands::persist_chat_session(session)?;
    print_attachment_count(session.attachments.len(), jsonl_output)?;
    Ok(())
}

fn print_attachments(session: &ChatSessionState, jsonl_output: bool) -> Result<()> {
    for (index, attachment) in session.attachments.iter().enumerate() {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "schema_version": 1,
                "record_type": "chat_attachment",
                "index": index,
                "path": attachment.display_path,
                "kind": attachment.kind,
                "mime_type": attachment.mime_type,
                "size": attachment.size,
                "sha256": attachment.sha256,
                "materialization": attachment.materialization,
                "truncated": attachment.truncated,
            }))?
        );
    }
    print_attachment_count(session.attachments.len(), jsonl_output)?;
    Ok(())
}

fn print_permission_mode(session: &ChatSessionState, jsonl_output: bool) -> Result<()> {
    if jsonl_output {
        print_jsonl_record(&serde_json::json!({
            "schema_version": 1,
            "record_type": "chat_permission_mode",
            "status": "ok",
            "permission_mode": session.permission_mode.as_token(),
        }))
    } else {
        println!("permission_mode={}", session.permission_mode.as_token());
        Ok(())
    }
}

fn print_attachment_count(count: usize, jsonl_output: bool) -> Result<()> {
    if jsonl_output {
        print_jsonl_record(&serde_json::json!({
            "schema_version": 1,
            "record_type": "chat_attachment_summary",
            "status": "ok",
            "attachment_count": count,
        }))
    } else {
        println!("attachment_count={count}");
        Ok(())
    }
}

fn print_task_submitted(task_id: &str, jsonl_output: bool) -> Result<()> {
    if jsonl_output {
        print_jsonl_record(&serde_json::json!({
            "schema_version": 1,
            "record_type": "task_submitted",
            "status": "ok",
            "task_id": task_id,
        }))
    } else {
        println!("task_id={task_id}");
        Ok(())
    }
}

fn print_chat_error(error_code: &str, jsonl_output: bool) -> Result<()> {
    if jsonl_output {
        print_jsonl_record(&serde_json::json!({
            "schema_version": 1,
            "record_type": "chat_error",
            "status": "error",
            "error_code": error_code,
        }))
    } else {
        println!("error_code={error_code}");
        Ok(())
    }
}

fn print_chat_response(
    record_type: &str,
    body: &serde_json::Value,
    jsonl_output: bool,
) -> Result<()> {
    if jsonl_output {
        print_jsonl_record(&serde_json::json!({
            "schema_version": 1,
            "record_type": record_type,
            "status": "ok",
            "response": body,
        }))
    } else {
        output::print_json_pretty(body);
        Ok(())
    }
}

fn print_jsonl_record(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn print_goal(base_url: &str, key: &str, session: &ChatSessionState) -> Result<()> {
    let task = session
        .active_task_id
        .as_deref()
        .map(|task_id| task::get_task_status(base_url, key, task_id))
        .transpose()?;
    println!(
        "{}",
        serde_json::to_string(&goal_projection(session, task.as_ref()))?
    );
    Ok(())
}

fn goal_projection(
    session: &ChatSessionState,
    task: Option<&task::TaskStatusView>,
) -> serde_json::Value {
    let authoritative_goal = task.and_then(|task| {
        task.raw_data
            .get("goal")
            .or_else(|| task.raw_data.get("task_goal"))
            .cloned()
    });
    let authoritative_goal_ref = authoritative_goal
        .as_ref()
        .and_then(|goal| goal.get("goal_id"))
        .and_then(serde_json::Value::as_str);
    serde_json::json!({
        "schema_version": 1,
        "record_type": "chat_goal",
        "status": "ok",
        "source": if task.is_some() { "server_task" } else { "session_reference" },
        "conversation_id": session.conversation_id,
        "active_task_id": session.active_task_id,
        "goal_ref": authoritative_goal_ref.or(session.goal_ref.as_deref()),
        "goal": authoritative_goal,
        "compacted_context_ref": session.compacted_context_ref,
    })
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
