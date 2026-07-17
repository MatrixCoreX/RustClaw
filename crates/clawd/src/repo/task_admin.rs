use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::{now_ts, AppState};

const TASK_CANCELLED_SOURCE: &str = "task_admin_cancel";
const TASK_CANCELLED_MESSAGE_KEY: &str = "clawd.task.cancelled";
const CHILD_TASK_PARENT_CANCELLED_REASON: &str = "parent_cancelled";
const CHILD_TASK_PARENT_CANCELLED_MESSAGE_KEY: &str = "clawd.task.parent_cancelled";
const TASK_CONTROL_SOURCE: &str = "task_admin_control";
const TASK_CONTROL_KIND_PAUSE: &str = "pause";
const TASK_CONTROL_KIND_RESUME: &str = "resume";
const TASK_CONTROL_STATUS_PENDING: &str = "pending";
const TASK_PAUSED_MESSAGE_KEY: &str = "clawd.task.pause_requested";
const TASK_RESUMED_MESSAGE_KEY: &str = "clawd.task.resume_requested";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskAdminTarget {
    pub(crate) task_id: String,
    pub(crate) user_id: i64,
    pub(crate) chat_id: i64,
    pub(crate) user_key: Option<String>,
    pub(crate) channel: String,
    pub(crate) status: String,
}

struct CancelTaskRecord {
    task_id: String,
    result_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TaskControlUpdate {
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) lifecycle: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TaskResumeControlInput {
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: Option<String>,
    pub(crate) resume_reason: Option<String>,
    pub(crate) user_message: Option<String>,
    pub(crate) new_constraints: Option<Value>,
}

pub(crate) fn cancel_tasks_for_user_chat(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<&str>,
) -> anyhow::Result<i64> {
    let now = now_ts();
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let exclude_task_id = normalized_optional_task_id(exclude_task_id);
    let mut stmt = db.prepare(
        "SELECT task_id, result_json
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND status IN ('queued', 'running')
           AND (?3 IS NULL OR task_id <> ?3)",
    )?;
    let records = stmt
        .query_map(
            params![user_id, chat_id, exclude_task_id.as_deref()],
            |row| {
                Ok(CancelTaskRecord {
                    task_id: row.get(0)?,
                    result_json: row.get(1)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    cancel_task_records(state, &db, records, &now)
}

pub(crate) fn cancel_one_task_for_user_chat(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    task_id: &str,
) -> anyhow::Result<i64> {
    let now = now_ts();
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, result_json
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND task_id = ?3
           AND status IN ('queued', 'running')",
    )?;
    let records = stmt
        .query_map(params![user_id, chat_id, task_id], |row| {
            Ok(CancelTaskRecord {
                task_id: row.get(0)?,
                result_json: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    cancel_task_records(state, &db, records, &now)
}

pub(crate) fn get_task_admin_target(
    state: &AppState,
    task_id: &str,
) -> anyhow::Result<Option<TaskAdminTarget>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, user_id, chat_id, user_key, channel, status
         FROM tasks
         WHERE task_id = ?1
         LIMIT 1",
    )?;
    let target = stmt
        .query_row(params![task_id], |row| {
            Ok(TaskAdminTarget {
                task_id: row.get(0)?,
                user_id: row.get(1)?,
                chat_id: row.get(2)?,
                user_key: row.get(3)?,
                channel: row.get(4)?,
                status: row.get(5)?,
            })
        })
        .optional()?;
    Ok(target)
}

pub(crate) fn cancel_task_by_id(state: &AppState, task_id: &str) -> anyhow::Result<i64> {
    let now = now_ts();
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, result_json
         FROM tasks
         WHERE task_id = ?1
           AND status IN ('queued', 'running')",
    )?;
    let records = stmt
        .query_map(params![task_id], |row| {
            Ok(CancelTaskRecord {
                task_id: row.get(0)?,
                result_json: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    cancel_task_records(state, &db, records, &now)
}

pub(crate) fn resume_task_with_input(
    state: &AppState,
    input: TaskResumeControlInput,
) -> anyhow::Result<Option<TaskControlUpdate>> {
    let now_ts = crate::now_ts_u64() as i64;
    update_paused_checkpoint_schedule(
        state,
        &input.task_id,
        now_ts,
        now_ts,
        TASK_RESUMED_MESSAGE_KEY,
        Some(&input),
    )
}

pub(crate) fn pause_task_by_id(
    state: &AppState,
    task_id: &str,
    pause_seconds: u64,
) -> anyhow::Result<Option<TaskControlUpdate>> {
    let now_ts = crate::now_ts_u64() as i64;
    let pause_seconds = pause_seconds.clamp(1, 604_800) as i64;
    update_paused_checkpoint_schedule(
        state,
        task_id,
        now_ts,
        now_ts.saturating_add(pause_seconds),
        TASK_PAUSED_MESSAGE_KEY,
        None,
    )
}

fn cancel_task_records(
    state: &AppState,
    db: &Connection,
    records: Vec<CancelTaskRecord>,
    now: &str,
) -> anyhow::Result<i64> {
    let mut visited = HashSet::new();
    let reason = crate::task_lifecycle::TerminalFailureReason::UserCancelled.status_code();
    cancel_task_records_with_reason(
        state,
        db,
        records,
        now,
        reason,
        TASK_CANCELLED_MESSAGE_KEY,
        &mut visited,
    )
}

fn cancel_task_records_with_reason(
    state: &AppState,
    db: &Connection,
    records: Vec<CancelTaskRecord>,
    now: &str,
    reason: &str,
    message_key: &str,
    visited: &mut HashSet<String>,
) -> anyhow::Result<i64> {
    let now_ts = now.parse::<i64>().unwrap_or_default();
    let mut affected = 0_i64;
    let mut child_task_ids = Vec::new();
    for record in records {
        if !visited.insert(record.task_id.clone()) {
            continue;
        }
        append_child_task_ids_from_result(record.result_json.as_deref(), &mut child_task_ids);
        let cancel_adapter_result =
            cancel_adapter_result_from_task_result(record.result_json.as_deref(), now_ts);
        let result_json = cancelled_task_result_json(
            record.result_json.as_deref(),
            reason,
            now_ts,
            cancel_adapter_result.as_ref(),
            message_key,
        );
        let count = db.execute(
            "UPDATE tasks
             SET status = 'canceled',
                 error_text = ?1,
                 result_json = ?2,
                 updated_at = ?3
             WHERE task_id = ?4
               AND status IN ('queued', 'running')",
            params![reason, result_json.to_string(), now, record.task_id],
        )?;
        affected += count as i64;
        if count > 0 {
            state.worker.cancel_active_task(&record.task_id);
        }
    }
    let child_records = cancellable_child_task_records(db, &child_task_ids, visited)?;
    if !child_records.is_empty() {
        affected += cancel_task_records_with_reason(
            state,
            db,
            child_records,
            now,
            CHILD_TASK_PARENT_CANCELLED_REASON,
            CHILD_TASK_PARENT_CANCELLED_MESSAGE_KEY,
            visited,
        )?;
    }
    Ok(affected)
}

fn cancellable_child_task_records(
    db: &Connection,
    child_task_ids: &[String],
    visited: &HashSet<String>,
) -> anyhow::Result<Vec<CancelTaskRecord>> {
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    let mut stmt = db.prepare(
        "SELECT task_id, result_json
         FROM tasks
         WHERE task_id = ?1
           AND status IN ('queued', 'running')",
    )?;
    for child_task_id in child_task_ids {
        if visited.contains(child_task_id) || !seen.insert(child_task_id.clone()) {
            continue;
        }
        let record = stmt
            .query_row(params![child_task_id], |row| {
                Ok(CancelTaskRecord {
                    task_id: row.get(0)?,
                    result_json: row.get(1)?,
                })
            })
            .optional()?;
        if let Some(record) = record {
            records.push(record);
        }
    }
    Ok(records)
}

fn append_child_task_ids_from_result(raw_result_json: Option<&str>, output: &mut Vec<String>) {
    let Some(value) = raw_result_json.and_then(|raw| serde_json::from_str::<Value>(raw).ok())
    else {
        return;
    };
    append_child_task_ids_from_value(&value, output, 0);
}

fn append_child_task_ids_from_value(value: &Value, output: &mut Vec<String>, depth: usize) {
    if depth > 8 || output.len() >= 128 {
        return;
    }
    match value {
        Value::Object(map) => {
            if let Some(child_task_id) = map
                .get("child_task_id")
                .and_then(Value::as_str)
                .and_then(machine_child_task_id)
            {
                output.push(child_task_id);
            }
            if let Some(items) = map.get("child_task_ids").and_then(Value::as_array) {
                for item in items.iter().take(128usize.saturating_sub(output.len())) {
                    if let Some(child_task_id) = item.as_str().and_then(machine_child_task_id) {
                        output.push(child_task_id);
                    }
                }
            }
            for value in map.values() {
                append_child_task_ids_from_value(value, output, depth + 1);
            }
        }
        Value::Array(items) => {
            for item in items {
                append_child_task_ids_from_value(item, output, depth + 1);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn machine_child_task_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 160
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.'))
    {
        return None;
    }
    Some(value.to_string())
}

fn cancelled_task_result_json(
    raw_result_json: Option<&str>,
    reason: &str,
    now_ts: i64,
    cancel_adapter_result: Option<&Value>,
    message_key: &str,
) -> Value {
    let mut result = raw_result_json
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = result.as_object_mut() {
        obj.insert("status_code".to_string(), json!(reason));
        obj.insert("error_code".to_string(), json!(reason));
        obj.insert("terminal_reason".to_string(), json!(reason));
        obj.insert("message_key".to_string(), json!(message_key));
        obj.insert(
            "task_lifecycle".to_string(),
            json!({
                "schema_version": 1,
                "state": "cancelled",
                "source": TASK_CANCELLED_SOURCE,
                "terminal_reason": reason,
                "message_key": message_key,
                "cancel_adapter_kind": cancel_adapter_result
                    .and_then(|value| value.get("adapter_kind"))
                    .and_then(Value::as_str),
                "can_cancel": false,
                "cancelled_at": now_ts,
            }),
        );
        if let Some(cancel_adapter_result) = cancel_adapter_result.cloned() {
            obj.insert(
                "cancel_adapter_result".to_string(),
                cancel_adapter_result.clone(),
            );
            if let Some(lifecycle) = obj.get_mut("task_lifecycle").and_then(Value::as_object_mut) {
                lifecycle.insert("cancel_adapter_result".to_string(), cancel_adapter_result);
            }
        }
    }
    result
}

fn cancel_adapter_result_from_task_result(
    raw_result_json: Option<&str>,
    now_ts: i64,
) -> Option<Value> {
    let result = raw_result_json
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)?;
    let cancel_ref = task_cancel_ref(&result)?;
    if cancel_ref.starts_with("local_process:") {
        return Some(cancel_local_process_job(cancel_ref, now_ts));
    }
    if provider_cancel_ref_kind(cancel_ref).is_some() {
        return Some(provider_cancel_adapter_required_result(
            &result, cancel_ref, now_ts,
        ));
    }
    None
}

fn task_cancel_ref(result: &Value) -> Option<&str> {
    [
        "/task_checkpoint/pending_async_job/cancel_ref",
        "/task_checkpoint/pending_async_job/cancel_token",
        "/task_lifecycle/cancel_ref",
        "/task_lifecycle/cancel_token",
        "/cancel_ref",
        "/cancel_token",
    ]
    .into_iter()
    .filter_map(|pointer| result.pointer(pointer))
    .filter_map(Value::as_str)
    .map(str::trim)
    .find(|value| !value.is_empty())
}

fn provider_cancel_adapter_required_result(result: &Value, cancel_ref: &str, now_ts: i64) -> Value {
    let adapter_kind = task_cancel_adapter_kind(result, cancel_ref);
    let contract = provider_cancel_contract(cancel_ref);
    json!({
        "schema_version": 1,
        "adapter_kind": adapter_kind,
        "status": "requires_provider_adapter",
        "cancel_ref": cancel_ref,
        "cancelled_at": now_ts,
        "message_key": TASK_CANCELLED_MESSAGE_KEY,
        "error_code": "provider_cancel_adapter_missing",
        "provider_cancel_contract": contract,
    })
}

fn task_cancel_adapter_kind(result: &Value, cancel_ref: &str) -> &'static str {
    [
        "/task_checkpoint/pending_async_job/poll_adapter/adapter_kind",
        "/task_checkpoint/pending_async_job/poll_adapter/kind",
        "/task_lifecycle/async_timeout_policy/adapter_kind",
        "/task_lifecycle/poll_adapter_kind",
    ]
    .into_iter()
    .filter_map(|pointer| result.pointer(pointer))
    .filter_map(Value::as_str)
    .map(str::trim)
    .find_map(known_async_cancel_adapter_kind)
    .or_else(|| inferred_cancel_adapter_kind(cancel_ref))
    .unwrap_or("remote_job_poll")
}

fn known_async_cancel_adapter_kind(value: &str) -> Option<&'static str> {
    match value {
        "http_job_poll" => Some("http_job_poll"),
        "mcp_job_poll" => Some("mcp_job_poll"),
        "media_job_poll" => Some("media_job_poll"),
        "browser_job_poll" => Some("browser_job_poll"),
        "remote_job_poll" => Some("remote_job_poll"),
        _ => None,
    }
}

fn inferred_cancel_adapter_kind(cancel_ref: &str) -> Option<&'static str> {
    match provider_cancel_ref_kind(cancel_ref)? {
        "http" => Some("http_job_poll"),
        "mcp" => Some("mcp_job_poll"),
        "media" | "provider" => Some("media_job_poll"),
        "browser" => Some("browser_job_poll"),
        "remote" => Some("remote_job_poll"),
        _ => None,
    }
}

fn provider_cancel_ref_kind(cancel_ref: &str) -> Option<&'static str> {
    let cancel_ref = cancel_ref.trim();
    for (prefix, kind) in [
        ("provider:", "provider"),
        ("http:", "http"),
        ("mcp:", "mcp"),
        ("media:", "media"),
        ("browser:", "browser"),
        ("remote:", "remote"),
    ] {
        if cancel_ref.starts_with(prefix) {
            return Some(kind);
        }
    }
    None
}

fn provider_cancel_contract(cancel_ref: &str) -> Value {
    let kind = provider_cancel_ref_kind(cancel_ref).unwrap_or("unknown");
    let parts = cancel_ref
        .split(':')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "cancel_ref_kind": kind,
        "action": parts.get(1).copied(),
        "provider": parts.get(2).copied(),
        "job_id": parts.get(3).copied(),
        "required_adapter_fields": ["adapter_kind", "cancel_ref", "job_id"],
    })
}

fn cancel_local_process_job(cancel_ref: &str, now_ts: i64) -> Value {
    let Some(job_dir_raw) = cancel_ref.strip_prefix("local_process:").map(str::trim) else {
        return local_process_cancel_result(
            cancel_ref,
            now_ts,
            "failed",
            Some("local_process_cancel_ref_invalid"),
            None,
        );
    };
    if job_dir_raw.is_empty() {
        return local_process_cancel_result(
            cancel_ref,
            now_ts,
            "failed",
            Some("local_process_cancel_ref_invalid"),
            None,
        );
    }
    let job_dir = Path::new(job_dir_raw);
    if !job_dir.is_dir() {
        return local_process_cancel_result(
            cancel_ref,
            now_ts,
            "failed",
            Some("local_process_cancel_job_dir_missing"),
            Some(job_dir),
        );
    }
    let _ = std::fs::write(job_dir.join("cancel_requested_at"), now_ts.to_string());
    let _ = std::fs::write(job_dir.join("cancel_signal"), "TERM");
    if job_dir.join("exit_code").exists() {
        return local_process_cancel_result(
            cancel_ref,
            now_ts,
            "already_terminal",
            None,
            Some(job_dir),
        );
    }
    let pid = match std::fs::read_to_string(job_dir.join("pid"))
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .filter(|pid| *pid > 0)
    {
        Some(pid) => pid,
        None => {
            return local_process_cancel_result(
                cancel_ref,
                now_ts,
                "failed",
                Some("local_process_cancel_pid_invalid"),
                Some(job_dir),
            );
        }
    };
    let status = terminate_local_process(pid);
    let mut result = local_process_cancel_result(
        cancel_ref,
        now_ts,
        if status { "accepted" } else { "failed" },
        (!status).then_some("local_process_cancel_signal_failed"),
        Some(job_dir),
    );
    if let Some(obj) = result.as_object_mut() {
        obj.insert("pid".to_string(), json!(pid));
        obj.insert("signal".to_string(), json!("TERM"));
        obj.insert("signal_scope".to_string(), json!("process_group_or_pid"));
    }
    result
}

fn local_process_cancel_result(
    cancel_ref: &str,
    now_ts: i64,
    status: &str,
    error_code: Option<&str>,
    job_dir: Option<&Path>,
) -> Value {
    let mut result = json!({
        "schema_version": 1,
        "adapter_kind": "local_process_poll",
        "status": status,
        "cancel_ref": cancel_ref,
        "cancelled_at": now_ts,
        "message_key": TASK_CANCELLED_MESSAGE_KEY,
    });
    if let Some(obj) = result.as_object_mut() {
        if let Some(error_code) = error_code {
            obj.insert("error_code".to_string(), json!(error_code));
        }
        if let Some(job_dir) = job_dir {
            obj.insert(
                "job_dir".to_string(),
                json!(job_dir.to_string_lossy().to_string()),
            );
        }
    }
    result
}

#[cfg(unix)]
fn terminate_local_process(pid: u32) -> bool {
    if terminate_local_process_group(pid) {
        return true;
    }
    terminate_local_process_pid(pid)
}

#[cfg(unix)]
fn terminate_local_process_group(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    Command::new("kill")
        .arg("-TERM")
        .arg(format!("-{pid}"))
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn terminate_local_process_pid(pid: u32) -> bool {
    Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn terminate_local_process(_pid: u32) -> bool {
    false
}

fn update_paused_checkpoint_schedule(
    state: &AppState,
    task_id: &str,
    now_ts: i64,
    next_check_after: i64,
    message_key: &str,
    resume_input: Option<&TaskResumeControlInput>,
) -> anyhow::Result<Option<TaskControlUpdate>> {
    let task_id = task_id.trim();
    if task_id.is_empty() {
        return Ok(None);
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let raw_result_json = db
        .query_row(
            "SELECT result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
             LIMIT 1",
            params![task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let Some(raw_result_json) = raw_result_json else {
        return Ok(None);
    };
    let mut result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) if value.is_object() => value,
        _ => return Ok(None),
    };
    let readiness = crate::task_lifecycle::paused_checkpoint_resume_readiness(&result_json, now_ts);
    if matches!(
        &readiness,
        crate::task_lifecycle::PausedCheckpointResumeReadiness::NotPaused
            | crate::task_lifecycle::PausedCheckpointResumeReadiness::InvalidPausedCheckpoint
            | crate::task_lifecycle::PausedCheckpointResumeReadiness::MissingTaskCheckpoint { .. }
            | crate::task_lifecycle::PausedCheckpointResumeReadiness::InvalidTaskCheckpoint { .. }
            | crate::task_lifecycle::PausedCheckpointResumeReadiness::CheckpointMismatch { .. }
            | crate::task_lifecycle::PausedCheckpointResumeReadiness::ActiveResumeLease { .. }
    ) {
        return Ok(None);
    }
    let checkpoint_id = match readiness {
        crate::task_lifecycle::PausedCheckpointResumeReadiness::WaitingNotDue {
            checkpoint_id,
            ..
        }
        | crate::task_lifecycle::PausedCheckpointResumeReadiness::Ready { checkpoint_id, .. } => {
            checkpoint_id
        }
        _ => return Ok(None),
    };
    if let Some(expected_checkpoint_id) = resume_input
        .and_then(|input| input.checkpoint_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if expected_checkpoint_id != checkpoint_id {
            return Ok(None);
        }
    }
    if resume_input.is_some()
        && pending_manual_resume_request_without_claim(&result_json, &checkpoint_id)
    {
        return Ok(None);
    }
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(obj) = lifecycle.as_object_mut() else {
        return Ok(None);
    };
    obj.insert("source".to_string(), json!(TASK_CONTROL_SOURCE));
    obj.insert("next_check_after".to_string(), json!(next_check_after));
    obj.insert(
        "resume_due".to_string(),
        json!(next_check_after.saturating_sub(now_ts) == 0),
    );
    obj.insert(
        "resume_wait_seconds".to_string(),
        json!(next_check_after.saturating_sub(now_ts).max(0)),
    );
    obj.insert("message_key".to_string(), json!(message_key));
    obj.insert("manual_control_requested_at".to_string(), json!(now_ts));
    obj.insert(
        "manual_control_kind".to_string(),
        json!(if resume_input.is_some() {
            TASK_CONTROL_KIND_RESUME
        } else {
            TASK_CONTROL_KIND_PAUSE
        }),
    );
    obj.insert(
        "manual_control_status".to_string(),
        json!(TASK_CONTROL_STATUS_PENDING),
    );
    if let Some(resume_input) = resume_input {
        obj.insert(
            "resume_input".to_string(),
            task_resume_control_input_json(resume_input, &checkpoint_id),
        );
    }
    result_json["task_lifecycle"] = lifecycle.clone();
    let updated_result_json = result_json.to_string();
    let changed = db.execute(
        "UPDATE tasks
         SET result_json = ?2,
             updated_at = ?3
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?4",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json
        ],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    Ok(Some(TaskControlUpdate {
        task_id: task_id.to_string(),
        checkpoint_id,
        lifecycle,
    }))
}

fn pending_manual_resume_request_without_claim(result: &Value, checkpoint_id: &str) -> bool {
    let Some(lifecycle) = result.get("task_lifecycle").and_then(Value::as_object) else {
        return false;
    };
    let same_checkpoint = lifecycle
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        == Some(checkpoint_id);
    let pending_resume = lifecycle
        .get("manual_control_kind")
        .and_then(Value::as_str)
        .map(str::trim)
        == Some(TASK_CONTROL_KIND_RESUME)
        && lifecycle
            .get("manual_control_status")
            .and_then(Value::as_str)
            .map(str::trim)
            == Some(TASK_CONTROL_STATUS_PENDING);
    let claim_exists = lifecycle
        .get("resume_claim")
        .and_then(Value::as_object)
        .is_some_and(|claim| {
            claim
                .get("checkpoint_id")
                .and_then(Value::as_str)
                .map(str::trim)
                == Some(checkpoint_id)
        });
    same_checkpoint && pending_resume && !claim_exists
}

fn task_resume_control_input_json(input: &TaskResumeControlInput, checkpoint_id: &str) -> Value {
    let mut payload = json!({
        "schema_version": 1,
        "task_id": input.task_id.trim(),
        "checkpoint_id": checkpoint_id,
        "resume_trigger": "user_followup",
        "user_message_present": input
            .user_message
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty()),
        "user_message_char_count": input
            .user_message
            .as_deref()
            .map(str::trim)
            .map(|value| value.chars().count())
            .unwrap_or(0),
    });
    if let Some(obj) = payload.as_object_mut() {
        if let Some(reason) = input
            .resume_reason
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            obj.insert("resume_reason".to_string(), json!(reason));
        }
        if let Some(constraints) = input.new_constraints.as_ref() {
            obj.insert("new_constraints".to_string(), constraints.clone());
        }
    }
    payload
}

fn normalized_optional_task_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
#[path = "task_admin_tests.rs"]
mod tests;
