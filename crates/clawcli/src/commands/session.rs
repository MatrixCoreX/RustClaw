use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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
) -> Result<()> {
    let mut store = load_session_store()?;
    let mut thread = session_store_select_latest_chat_thread(&store)?;
    let source_task_id = thread.current_task_id.clone();
    let task_id = task::submit_thread_ask(
        base_url,
        key,
        message,
        &thread.thread_id,
        &thread.session_id,
        source_task_id.as_deref(),
    )?;
    session_store_record_chat_task(&mut store, &mut thread, &task_id)?;
    save_session_store(&store)?;
    let summary = json!({
        "operation": "session_continue_latest",
        "session_id": thread.session_id,
        "thread_id": thread.thread_id,
        "source_task_id": source_task_id,
        "task_id": task_id,
        "event_cursor": thread.last_event_seq,
    });
    if json_output {
        output::print_json_pretty(&summary);
    } else {
        println!("session_id={}", thread.session_id);
        println!("thread_id={}", thread.thread_id);
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
    #[serde(default)]
    archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    forked_from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatThreadState {
    pub(crate) thread_id: String,
    pub(crate) session_id: String,
    pub(crate) current_task_id: Option<String>,
    pub(crate) task_ids: Vec<String>,
    pub(crate) last_event_seq: u64,
}

pub(crate) fn load_or_create_chat_thread(
    requested_thread_id: Option<&str>,
    force_new: bool,
) -> Result<ChatThreadState> {
    let mut store = load_session_store()?;
    let generated_id = format!("cli_thread_{}", uuid::Uuid::new_v4().simple());
    let state = session_store_select_chat_thread(
        &mut store,
        requested_thread_id,
        force_new,
        &generated_id,
    )?;
    save_session_store(&store)?;
    Ok(state)
}

pub(crate) fn record_chat_task(state: &mut ChatThreadState, task_id: &str) -> Result<()> {
    let mut store = load_session_store()?;
    session_store_record_chat_task(&mut store, state, task_id)?;
    save_session_store(&store)
}

pub(crate) fn record_chat_cursor(state: &mut ChatThreadState, cursor: u64) -> Result<()> {
    let mut store = load_session_store()?;
    session_store_record_chat_cursor(&mut store, state, cursor)?;
    save_session_store(&store)
}

pub(super) fn session_store_select_chat_thread(
    store: &mut SessionStore,
    requested_thread_id: Option<&str>,
    force_new: bool,
    generated_id: &str,
) -> Result<ChatThreadState> {
    let requested = requested_thread_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if requested.is_some_and(|value| !valid_cli_thread_ref(value)) {
        anyhow::bail!("chat_thread_id_invalid");
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
    if !valid_cli_thread_ref(selected_id) {
        anyhow::bail!("chat_thread_id_invalid");
    }
    let entry = store
        .sessions
        .entry(selected_id.to_string())
        .or_insert_with(|| StoredSession {
            session_id: selected_id.to_string(),
            thread_id: Some(selected_id.to_string()),
            ..StoredSession::default()
        });
    if entry.archived || entry.thread_id.is_none() {
        entry.archived = false;
        entry.thread_id = Some(selected_id.to_string());
    }
    store.latest_session_id = Some(selected_id.to_string());
    Ok(chat_thread_state(entry))
}

pub(super) fn session_store_select_latest_chat_thread(
    store: &SessionStore,
) -> Result<ChatThreadState> {
    let session_id = store
        .latest_session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("chat_session_latest_missing"))?;
    let session = store
        .sessions
        .get(session_id)
        .filter(|session| !session.archived && session.thread_id.is_some())
        .ok_or_else(|| anyhow::anyhow!("chat_session_latest_missing"))?;
    Ok(chat_thread_state(session))
}

pub(super) fn session_store_record_chat_task(
    store: &mut SessionStore,
    state: &mut ChatThreadState,
    task_id: &str,
) -> Result<()> {
    let task_id = task_id.trim();
    if !valid_cli_task_ref(task_id) {
        anyhow::bail!("chat_task_id_invalid");
    }
    let entry = store
        .sessions
        .get_mut(&state.session_id)
        .ok_or_else(|| anyhow::anyhow!("chat_session_missing"))?;
    if entry.task_ids.last().map(String::as_str) != Some(task_id) {
        entry.task_ids.push(task_id.to_string());
    }
    entry.current_task_id = Some(task_id.to_string());
    entry.latest_event_seq = Some("0".to_string());
    store.latest_session_id = Some(state.session_id.clone());
    state.current_task_id = Some(task_id.to_string());
    state.task_ids = entry.task_ids.clone();
    state.last_event_seq = 0;
    Ok(())
}

pub(super) fn session_store_record_chat_cursor(
    store: &mut SessionStore,
    state: &mut ChatThreadState,
    cursor: u64,
) -> Result<()> {
    let entry = store
        .sessions
        .get_mut(&state.session_id)
        .ok_or_else(|| anyhow::anyhow!("chat_session_missing"))?;
    entry.latest_event_seq = Some(cursor.to_string());
    store.latest_session_id = Some(state.session_id.clone());
    state.last_event_seq = cursor;
    Ok(())
}

fn chat_thread_state(session: &StoredSession) -> ChatThreadState {
    ChatThreadState {
        thread_id: session
            .thread_id
            .clone()
            .unwrap_or_else(|| session.session_id.clone()),
        session_id: session.session_id.clone(),
        current_task_id: session.current_task_id.clone(),
        task_ids: session.task_ids.clone(),
        last_event_seq: session
            .latest_event_seq
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0),
    }
}

fn valid_cli_thread_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
}

fn valid_cli_task_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
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
    if let Some(path) = env::var_os("RUSTCLAW_CLAWCLI_SESSION_STORE") {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(path)
            .join("rustclaw")
            .join("clawcli_sessions.json");
    }
    if let Some(path) = env::var_os("HOME") {
        return PathBuf::from(path)
            .join(".local")
            .join("state")
            .join("rustclaw")
            .join("clawcli_sessions.json");
    }
    PathBuf::from(".rustclaw_clawcli_sessions.json")
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
