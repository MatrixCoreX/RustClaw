use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::chat_attachments::attachment_payload;
use crate::chat_session::{
    current_working_directory_identity, ChatSessionState, ChatSessionTransition, ModelOverride,
    PermissionMode, SessionAttachmentRef, WorkingDirectoryIdentity,
};
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
    let mut summary = session_list_json(user_id, chat_id, &active);
    let store = upsert_session_summary(&summary)?;
    attach_store_projection(&mut summary, &store);
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
    let mut summary = session_show_json(&task);
    let store = upsert_session_summary(&summary)?;
    attach_store_projection(&mut summary, &store);
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
        task::TaskResumeRequest {
            resume_reason: Some("session_resume"),
            user_message: message,
            ..Default::default()
        },
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

pub(crate) fn run_session_continue_latest(
    base_url: &str,
    key: &str,
    message: &str,
    json_output: bool,
    submission_options: task::TaskSubmissionOptions,
) -> Result<()> {
    let mut store = load_session_store()?;
    let mut session = session_store_select_latest_chat_session(&store)?;
    let source_task_id = session.active_task_id.clone();
    let attachments = attachment_payload(&session.attachments)?;
    let task_id = task::submit_thread_ask(
        base_url,
        key,
        message,
        task::ThreadAskContext {
            conversation_id: &session.conversation_id,
            session_id: &session.session_id,
            resume_task_id: source_task_id.as_deref(),
            model_override: session.model_override.as_ref(),
            compacted_context_ref: session.compacted_context_ref.as_deref(),
            goal_ref: session.goal_ref.as_deref(),
            attachments: &attachments,
        },
        submission_options,
    )?;
    session.apply(ChatSessionTransition::AttachmentsCleared)?;
    session_store_record_chat_task(&mut store, &mut session, &task_id)?;
    session_store_persist_chat_session(&mut store, &session)?;
    save_session_store(&store)?;
    let summary = json!({
        "operation": "session_continue_latest",
        "session_id": session.session_id,
        "conversation_id": session.conversation_id,
        "source_task_id": source_task_id,
        "task_id": task_id,
        "event_cursor": session.event_cursor,
    });
    if json_output {
        output::print_json_pretty(&summary);
    } else {
        println!("session_id={}", session.session_id);
        println!("conversation_id={}", session.conversation_id);
        println!("task_id={task_id}");
    }
    Ok(())
}

pub(crate) fn run_session_archive(session_id: &str, json_output: bool) -> Result<()> {
    let mut store = load_session_store()?;
    let summary = session_store_archive_json(&mut store, session_id);
    save_session_store(&store)?;
    print_session_store_operation(&summary, json_output);
    Ok(())
}

pub(crate) fn run_session_delete(session_id: &str, json_output: bool) -> Result<()> {
    let mut store = load_session_store()?;
    let summary = session_store_delete_json(&mut store, session_id);
    save_session_store(&store)?;
    print_session_store_operation(&summary, json_output);
    Ok(())
}

pub(crate) fn run_session_fork(
    session_id: &str,
    new_session_id: &str,
    json_output: bool,
) -> Result<()> {
    let mut store = load_session_store()?;
    let summary = session_store_fork_json(&mut store, session_id, new_session_id)?;
    save_session_store(&store)?;
    print_session_store_operation(&summary, json_output);
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct SessionStore {
    #[serde(default)]
    sessions: BTreeMap<String, StoredSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest_session_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct StoredSession {
    session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
    #[serde(default)]
    task_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_goal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest_checkpoint_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest_event_seq: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model_override: Option<ModelOverride>,
    #[serde(default)]
    permission_mode: PermissionMode,
    #[serde(default)]
    attachments: Vec<SessionAttachmentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compacted_context_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    working_directory: Option<WorkingDirectoryIdentity>,
    #[serde(default)]
    archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    forked_from: Option<String>,
}

pub(crate) fn load_or_create_chat_session(
    requested_conversation_id: Option<&str>,
    force_new: bool,
) -> Result<ChatSessionState> {
    let mut store = load_session_store()?;
    let generated_id = format!("cli_conversation_{}", uuid::Uuid::new_v4().simple());
    let state = session_store_select_chat_session(
        &mut store,
        requested_conversation_id,
        force_new,
        &generated_id,
    )?;
    save_session_store(&store)?;
    Ok(state)
}

pub(crate) fn record_chat_session_task(state: &mut ChatSessionState, task_id: &str) -> Result<()> {
    let mut store = load_session_store()?;
    session_store_record_chat_task(&mut store, state, task_id)?;
    save_session_store(&store)
}

pub(crate) fn record_chat_session_cursor(state: &mut ChatSessionState, cursor: u64) -> Result<()> {
    let mut store = load_session_store()?;
    session_store_record_chat_cursor(&mut store, state, cursor)?;
    save_session_store(&store)
}

pub(crate) fn persist_chat_session(state: &ChatSessionState) -> Result<()> {
    let mut store = load_session_store()?;
    session_store_persist_chat_session(&mut store, state)?;
    save_session_store(&store)
}

pub(super) fn session_store_select_chat_session(
    store: &mut SessionStore,
    requested_conversation_id: Option<&str>,
    force_new: bool,
    generated_id: &str,
) -> Result<ChatSessionState> {
    let requested = requested_conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if requested.is_some_and(|value| !valid_cli_conversation_ref(value)) {
        anyhow::bail!("chat_conversation_id_invalid");
    }
    let selected_id = if force_new {
        generated_id
    } else if let Some(requested) = requested {
        requested
    } else {
        store
            .latest_session_id
            .as_deref()
            .filter(|session_id| {
                store
                    .sessions
                    .get(*session_id)
                    .is_some_and(|session| !session.archived && session.thread_id.is_some())
            })
            .unwrap_or(generated_id)
    };
    if !valid_cli_conversation_ref(selected_id) {
        anyhow::bail!("chat_conversation_id_invalid");
    }
    let working_directory = current_working_directory_identity()?;
    let entry = store
        .sessions
        .entry(selected_id.to_string())
        .or_insert_with(|| StoredSession {
            session_id: selected_id.to_string(),
            thread_id: Some(selected_id.to_string()),
            working_directory: Some(working_directory.clone()),
            ..StoredSession::default()
        });
    if entry.archived || entry.thread_id.is_none() {
        entry.archived = false;
        entry.thread_id = Some(selected_id.to_string());
    }
    match entry.working_directory.as_ref() {
        Some(existing) if existing != &working_directory => {
            anyhow::bail!("chat_working_directory_mismatch")
        }
        Some(_) => {}
        None => entry.working_directory = Some(working_directory),
    }
    store.latest_session_id = Some(selected_id.to_string());
    chat_session_state(entry)
}

pub(super) fn session_store_select_latest_chat_session(
    store: &SessionStore,
) -> Result<ChatSessionState> {
    let session_id = store
        .latest_session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("chat_session_latest_missing"))?;
    let session = store
        .sessions
        .get(session_id)
        .filter(|session| !session.archived && session.thread_id.is_some())
        .ok_or_else(|| anyhow::anyhow!("chat_session_latest_missing"))?;
    chat_session_state(session)
}

pub(super) fn session_store_record_chat_task(
    store: &mut SessionStore,
    state: &mut ChatSessionState,
    task_id: &str,
) -> Result<()> {
    let task_id = task_id.trim().to_string();
    state.apply(ChatSessionTransition::TaskSubmitted(task_id.clone()))?;
    let entry = store
        .sessions
        .get_mut(&state.session_id)
        .ok_or_else(|| anyhow::anyhow!("chat_session_missing"))?;
    entry.task_ids = state.task_ids.clone();
    entry.current_task_id = state.active_task_id.clone();
    entry.latest_event_seq = Some("0".to_string());
    store.latest_session_id = Some(state.session_id.clone());
    Ok(())
}

pub(super) fn session_store_record_chat_cursor(
    store: &mut SessionStore,
    state: &mut ChatSessionState,
    cursor: u64,
) -> Result<()> {
    state.apply(ChatSessionTransition::CursorAdvanced(cursor))?;
    let entry = store
        .sessions
        .get_mut(&state.session_id)
        .ok_or_else(|| anyhow::anyhow!("chat_session_missing"))?;
    entry.latest_event_seq = Some(cursor.to_string());
    store.latest_session_id = Some(state.session_id.clone());
    Ok(())
}

pub(super) fn session_store_persist_chat_session(
    store: &mut SessionStore,
    state: &ChatSessionState,
) -> Result<()> {
    let entry = store
        .sessions
        .get_mut(&state.session_id)
        .ok_or_else(|| anyhow::anyhow!("chat_session_missing"))?;
    entry.thread_id = Some(state.conversation_id.clone());
    entry.current_task_id = state.active_task_id.clone();
    entry.task_ids = state.task_ids.clone();
    entry.model_override = state.model_override.clone();
    entry.permission_mode = state.permission_mode;
    entry.attachments = state.attachments.clone();
    entry.compacted_context_ref = state.compacted_context_ref.clone();
    entry.active_goal_id = state.goal_ref.clone();
    entry.latest_event_seq = Some(state.event_cursor.to_string());
    entry.working_directory = Some(state.working_directory.clone());
    store.latest_session_id = Some(state.session_id.clone());
    Ok(())
}

fn chat_session_state(session: &StoredSession) -> Result<ChatSessionState> {
    Ok(ChatSessionState {
        conversation_id: session
            .thread_id
            .clone()
            .unwrap_or_else(|| session.session_id.clone()),
        session_id: session.session_id.clone(),
        active_task_id: session.current_task_id.clone(),
        task_ids: session.task_ids.clone(),
        model_override: session.model_override.clone(),
        permission_mode: session.permission_mode,
        attachments: session.attachments.clone(),
        compacted_context_ref: session.compacted_context_ref.clone(),
        goal_ref: session.active_goal_id.clone(),
        event_cursor: session
            .latest_event_seq
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0),
        working_directory: session
            .working_directory
            .clone()
            .unwrap_or(current_working_directory_identity()?),
    })
}

fn valid_cli_conversation_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
}

pub(super) fn session_store_upsert_summary(store: &mut SessionStore, summary: &Value) -> Value {
    let session_id = string_at(summary, "/session_id").unwrap_or_default();
    if session_id.is_empty() {
        return json!({
            "operation": "session_store_upsert",
            "status": "skipped",
            "reason_code": "missing_session_id",
        });
    }
    let previous_archived = store
        .sessions
        .get(&session_id)
        .map(|session| session.archived)
        .unwrap_or(false);
    let previous_forked_from = store
        .sessions
        .get(&session_id)
        .and_then(|session| session.forked_from.clone());
    let previous = store.sessions.get(&session_id).cloned();
    let session = StoredSession {
        session_id: session_id.clone(),
        thread_id: string_at(summary, "/thread_id"),
        task_ids: string_array_at(summary, "/task_ids"),
        current_task_id: string_at(summary, "/current_task_id")
            .or_else(|| string_array_at(summary, "/task_ids").last().cloned()),
        active_goal_id: string_at(summary, "/active_goal_id"),
        workspace_root: string_at(summary, "/workspace_root"),
        latest_checkpoint_id: string_at(summary, "/latest_checkpoint_id"),
        latest_event_seq: string_at(summary, "/latest_event_seq"),
        model_override: previous
            .as_ref()
            .and_then(|session| session.model_override.clone()),
        permission_mode: previous
            .as_ref()
            .map(|session| session.permission_mode)
            .unwrap_or_default(),
        attachments: previous
            .as_ref()
            .map(|session| session.attachments.clone())
            .unwrap_or_default(),
        compacted_context_ref: previous
            .as_ref()
            .and_then(|session| session.compacted_context_ref.clone()),
        working_directory: previous.and_then(|session| session.working_directory),
        archived: summary
            .get("archived")
            .and_then(Value::as_bool)
            .unwrap_or(previous_archived),
        forked_from: previous_forked_from,
    };
    store.sessions.insert(session_id.clone(), session);
    store.latest_session_id = Some(session_id.clone());
    json!({
        "operation": "session_store_upsert",
        "status": "ok",
        "session_id": session_id,
    })
}

pub(super) fn session_store_archive_json(store: &mut SessionStore, session_id: &str) -> Value {
    let entry = store
        .sessions
        .entry(session_id.to_string())
        .or_insert_with(|| StoredSession {
            session_id: session_id.to_string(),
            task_ids: vec![session_id.to_string()],
            ..StoredSession::default()
        });
    entry.archived = true;
    json!({
        "operation": "session_archive",
        "session_id": session_id,
        "archived": true,
        "store_session_count": store.sessions.len(),
    })
}

pub(super) fn session_store_delete_json(store: &mut SessionStore, session_id: &str) -> Value {
    let existed = store.sessions.remove(session_id).is_some();
    json!({
        "operation": "session_delete",
        "session_id": session_id,
        "deleted": existed,
        "store_session_count": store.sessions.len(),
    })
}

pub(super) fn session_store_fork_json(
    store: &mut SessionStore,
    session_id: &str,
    new_session_id: &str,
) -> Result<Value> {
    let Some(source) = store.sessions.get(session_id).cloned() else {
        anyhow::bail!("session_store_source_missing");
    };
    let mut forked = source;
    forked.session_id = new_session_id.to_string();
    forked.task_ids = forked.task_ids.clone();
    forked.archived = false;
    forked.forked_from = Some(session_id.to_string());
    store.sessions.insert(new_session_id.to_string(), forked);
    Ok(json!({
        "operation": "session_fork",
        "session_id": new_session_id,
        "forked_from": session_id,
        "archived": false,
        "store_session_count": store.sessions.len(),
    }))
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
    push_optional_line(
        &mut lines,
        "session_store_session_count",
        summary,
        "/store/session_count",
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
    push_optional_line(
        &mut lines,
        "session_store_session_count",
        summary,
        "/store/session_count",
    );
    lines
}

fn print_session_store_operation(summary: &Value, json_output: bool) {
    if json_output {
        output::print_json_pretty(summary);
    } else {
        let operation = summary
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or("");
        let session_id = summary
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        println!("session_operation={operation}");
        println!("session_id={session_id}");
    }
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

fn string_array_at(value: &Value, pointer: &str) -> Vec<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn upsert_session_summary(summary: &Value) -> Result<SessionStore> {
    let mut store = load_session_store()?;
    session_store_upsert_summary(&mut store, summary);
    save_session_store(&store)?;
    Ok(store)
}

fn attach_store_projection(summary: &mut Value, store: &SessionStore) {
    let Some(map) = summary.as_object_mut() else {
        return;
    };
    map.insert("store".to_string(), session_store_projection(store));
}

fn session_store_projection(store: &SessionStore) -> Value {
    let sessions = store
        .sessions
        .values()
        .map(stored_session_json)
        .collect::<Vec<_>>();
    json!({
        "session_count": sessions.len(),
        "latest_session_id": store.latest_session_id,
        "sessions": sessions,
    })
}

fn stored_session_json(session: &StoredSession) -> Value {
    json!({
        "session_id": session.session_id.clone(),
        "thread_id": session.thread_id.clone(),
        "task_ids": session.task_ids.clone(),
        "current_task_id": session.current_task_id.clone(),
        "active_goal_id": session.active_goal_id.clone(),
        "workspace_root": session.workspace_root.clone(),
        "latest_checkpoint_id": session.latest_checkpoint_id.clone(),
        "latest_event_seq": session.latest_event_seq.clone(),
        "model_override": session.model_override.clone(),
        "permission_mode": session.permission_mode.as_token(),
        "attachments": session.attachments.clone(),
        "compacted_context_ref": session.compacted_context_ref.clone(),
        "working_directory": session.working_directory.clone(),
        "archived": session.archived,
        "forked_from": session.forked_from.clone(),
    })
}

fn load_session_store() -> Result<SessionStore> {
    load_session_store_from_path(&session_store_path())
}

fn save_session_store(store: &SessionStore) -> Result<()> {
    save_session_store_to_path(&session_store_path(), store)
}

fn load_session_store_from_path(path: &Path) -> Result<SessionStore> {
    if !path.exists() {
        return Ok(SessionStore::default());
    }
    let body = fs::read_to_string(path).context("session_store_read_failed")?;
    serde_json::from_str(&body).context("session_store_parse_failed")
}

fn save_session_store_to_path(path: &Path, store: &SessionStore) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("session_store_dir_create_failed")?;
    }
    let body = serde_json::to_string_pretty(store).context("session_store_serialize_failed")?;
    fs::write(path, body).context("session_store_write_failed")
}

fn session_store_path() -> PathBuf {
    const SESSION_NAMESPACE: &str = "agent-runtime";
    if let Some(path) = claw_core::product_identity::env_os("CLAWCLI_SESSION_STORE") {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("XDG_STATE_HOME") {
        let root = PathBuf::from(path);
        return root.join(SESSION_NAMESPACE).join("clawcli_sessions.json");
    }
    if let Some(path) = env::var_os("HOME") {
        let root = PathBuf::from(path).join(".local").join("state");
        return root.join(SESSION_NAMESPACE).join("clawcli_sessions.json");
    }
    PathBuf::from(".agent_clawcli_sessions.json")
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
        .header("x-agent-key", key)
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
