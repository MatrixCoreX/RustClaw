#![allow(dead_code)]

use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub(crate) const SCHEDULED_RUN_SCHEMA_VERSION: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduledRunTriage {
    NoFindings,
    Findings,
    NeedsUser,
    Failed,
    Cancelled,
}

impl ScheduledRunTriage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoFindings => "no_findings",
            Self::Findings => "findings",
            Self::NeedsUser => "needs_user",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ScheduledRunEnqueued<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) job_id: &'a str,
    pub(crate) task_id: &'a str,
    pub(crate) thread_ref: &'a str,
    pub(crate) started_at: &'a str,
}

pub(crate) fn scheduled_run_thread_ref(job_id: &str) -> String {
    format!("scheduled_job:{}", stable_machine_ref(job_id))
}

pub(crate) fn scheduled_run_payload_metadata(job_id: &str, run_id: &str) -> Vec<(String, Value)> {
    let thread_ref = scheduled_run_thread_ref(job_id);
    vec![
        (
            "automation_run_id".to_string(),
            Value::String(stable_machine_ref(run_id)),
        ),
        (
            "automation_thread_ref".to_string(),
            Value::String(thread_ref.clone()),
        ),
        ("thread_ref".to_string(), Value::String(thread_ref.clone())),
        (
            "scheduled_run_schema_version".to_string(),
            Value::Number(SCHEDULED_RUN_SCHEMA_VERSION.into()),
        ),
        (
            "scheduled_run_ref".to_string(),
            Value::String(format!(
                "{}:{}",
                stable_machine_ref(job_id),
                stable_machine_ref(run_id)
            )),
        ),
    ]
}

pub(crate) fn insert_scheduled_run_enqueued(
    db: &Connection,
    record: &ScheduledRunEnqueued<'_>,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT OR IGNORE INTO scheduled_job_runs (
            run_id, job_id, task_id, thread_ref, task_status, triage_status,
            result_json, started_at, finished_at, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, 'queued', NULL, '{}', ?5, NULL, ?5, ?5)",
        params![
            stable_machine_ref(record.run_id),
            stable_machine_ref(record.job_id),
            stable_machine_ref(record.task_id),
            stable_machine_ref(record.thread_ref),
            record.started_at,
        ],
    )?;
    Ok(())
}

pub(crate) fn update_scheduled_run_terminal(
    db: &Connection,
    job_id: &str,
    task_id: &str,
    task_status: &str,
    finished_at: &str,
    result: &Value,
) -> anyhow::Result<usize> {
    let triage = scheduled_run_triage_from_machine(task_status, Some(result));
    let affected = db.execute(
        "UPDATE scheduled_job_runs
         SET task_status = ?1, triage_status = ?2, result_json = ?3,
             finished_at = ?4, updated_at = ?4
         WHERE job_id = ?5 AND task_id = ?6",
        params![
            stable_task_status(task_status),
            triage.as_str(),
            result.to_string(),
            finished_at,
            stable_machine_ref(job_id),
            stable_machine_ref(task_id),
        ],
    )?;
    Ok(affected)
}

pub(crate) fn list_scheduled_run_history(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    job_id: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<Value>> {
    let limit = limit.clamp(1, 100) as i64;
    let job_id = job_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(stable_machine_ref);
    let mut rows = Vec::new();
    if let Some(job_id) = job_id {
        let mut stmt = db.prepare(
            "SELECT r.run_id, r.job_id, r.task_id, r.thread_ref, r.task_status,
                    r.triage_status, r.result_json, r.started_at, r.finished_at, r.updated_at
             FROM scheduled_job_runs r
             JOIN scheduled_jobs j ON j.job_id = r.job_id
             WHERE j.user_id = ?1 AND j.chat_id = ?2 AND r.job_id = ?3
             ORDER BY CAST(r.updated_at AS INTEGER) DESC, r.id DESC
             LIMIT ?4",
        )?;
        let mapped = stmt.query_map(params![user_id, chat_id, job_id, limit], scheduled_run_row)?;
        for row in mapped {
            rows.push(row?);
        }
        return Ok(rows);
    }

    let mut stmt = db.prepare(
        "SELECT r.run_id, r.job_id, r.task_id, r.thread_ref, r.task_status,
                r.triage_status, r.result_json, r.started_at, r.finished_at, r.updated_at
         FROM scheduled_job_runs r
         JOIN scheduled_jobs j ON j.job_id = r.job_id
         WHERE j.user_id = ?1 AND j.chat_id = ?2
         ORDER BY CAST(r.updated_at AS INTEGER) DESC, r.id DESC
         LIMIT ?3",
    )?;
    let mapped = stmt.query_map(params![user_id, chat_id, limit], scheduled_run_row)?;
    for row in mapped {
        rows.push(row?);
    }
    Ok(rows)
}

pub(crate) fn scheduled_run_terminal_result(
    task_success: bool,
    payload: &Value,
    notification: Option<&Value>,
) -> Value {
    json!({
        "schema_version": SCHEDULED_RUN_SCHEMA_VERSION,
        "task_success": task_success,
        "automation_ref": machine_string(payload.get("automation_ref")),
        "automation_kind": machine_string(payload.get("automation_kind")),
        "automation_run_id": machine_string(payload.get("automation_run_id")),
        "thread_ref": machine_string(payload.get("thread_ref")),
        "policy_decision": machine_object(
            payload.get("policy_decision"),
            &["decision", "reason_code", "policy_id", "risk_level", "message_key"],
        ),
        "provider_status": machine_object(
            payload.get("provider_status"),
            &["provider", "status", "status_code", "error_code", "message_key"],
        ),
        "error_code": first_machine_string(payload, &["error_code", "reason_code"]),
        "finding_refs": machine_array(payload.get("finding_refs")),
        "evidence_refs": machine_array(payload.get("evidence_refs")),
        "notification": notification.cloned().unwrap_or(Value::Null),
    })
}

fn scheduled_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let result_text: String = row.get(6)?;
    let result = serde_json::from_str::<Value>(&result_text).unwrap_or_else(|_| json!({}));
    Ok(json!({
        "run_id": row.get::<_, String>(0)?,
        "job_id": row.get::<_, String>(1)?,
        "task_id": row.get::<_, String>(2)?,
        "thread_ref": row.get::<_, String>(3)?,
        "task_status": row.get::<_, String>(4)?,
        "triage_status": row.get::<_, Option<String>>(5)?,
        "result_summary": scheduled_run_result_summary(&result),
        "finding_refs": machine_array(result.get("finding_refs")),
        "evidence_refs": machine_array(result.get("evidence_refs")),
        "started_at": row.get::<_, String>(7)?,
        "finished_at": row.get::<_, Option<String>>(8)?,
        "updated_at": row.get::<_, String>(9)?,
    }))
}

fn scheduled_run_result_summary(result: &Value) -> Value {
    json!({
        "schema_version": result.get("schema_version").cloned().unwrap_or(Value::Null),
        "task_success": result.get("task_success").and_then(Value::as_bool),
        "error_code": first_machine_string(result, &["error_code", "reason_code"]),
        "policy_decision": machine_object(
            result.get("policy_decision"),
            &["decision", "reason_code", "policy_id", "risk_level", "message_key"],
        ),
        "provider_status": machine_object(
            result.get("provider_status"),
            &["provider", "status", "status_code", "error_code", "message_key"],
        ),
        "notification": machine_object(
            result.get("notification"),
            &["delivered", "runtime_channel", "error_code", "message_key"],
        ),
    })
}

pub(crate) fn scheduled_run_triage_from_machine(
    task_status: &str,
    machine_result: Option<&Value>,
) -> ScheduledRunTriage {
    match stable_task_status(task_status).as_str() {
        "cancelled" | "canceled" => return ScheduledRunTriage::Cancelled,
        "needs_user" => return ScheduledRunTriage::NeedsUser,
        "failed" | "timeout" => return ScheduledRunTriage::Failed,
        _ => {}
    }

    let Some(result) = machine_result else {
        return ScheduledRunTriage::NoFindings;
    };
    if result
        .get("task_lifecycle")
        .and_then(|v| v.get("state"))
        .and_then(Value::as_str)
        .map(stable_task_status)
        .as_deref()
        == Some("needs_user")
    {
        return ScheduledRunTriage::NeedsUser;
    }
    if has_machine_items(result.get("finding_refs")) || has_machine_items(result.get("findings")) {
        return ScheduledRunTriage::Findings;
    }
    ScheduledRunTriage::NoFindings
}

fn machine_string(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .map(stable_machine_ref)
        .filter(|value| !value.is_empty())
        .map(Value::String)
        .unwrap_or(Value::Null)
}

fn first_machine_string(value: &Value, keys: &[&str]) -> Value {
    for key in keys {
        if let Some(text) = value
            .get(*key)
            .and_then(Value::as_str)
            .map(stable_machine_ref)
            .filter(|value| !value.is_empty())
        {
            return Value::String(text);
        }
    }
    Value::Null
}

fn machine_array(value: Option<&Value>) -> Value {
    let Some(items) = value.and_then(Value::as_array) else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        items
            .iter()
            .filter_map(Value::as_str)
            .map(stable_machine_ref)
            .filter(|value| !value.is_empty())
            .map(Value::String)
            .collect(),
    )
}

fn machine_object(value: Option<&Value>, allowed_keys: &[&str]) -> Value {
    let Some(object) = value.and_then(Value::as_object) else {
        return Value::Null;
    };
    let mut output = serde_json::Map::new();
    for key in allowed_keys {
        if let Some(value) = object.get(*key) {
            match value {
                Value::String(text) => {
                    let token = stable_machine_ref(text);
                    if !token.is_empty() {
                        output.insert((*key).to_string(), Value::String(token));
                    }
                }
                Value::Number(_) | Value::Bool(_) | Value::Null => {
                    output.insert((*key).to_string(), value.clone());
                }
                Value::Array(items) => {
                    output.insert(
                        (*key).to_string(),
                        Value::Array(
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(stable_machine_ref)
                                .filter(|token| !token.is_empty())
                                .map(Value::String)
                                .collect(),
                        ),
                    );
                }
                Value::Object(_) => {}
            }
        }
    }
    Value::Object(output)
}

fn has_machine_items(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_array)
        .map(|items| !items.is_empty())
        .unwrap_or(false)
}

fn stable_task_status(value: &str) -> String {
    let status = stable_machine_ref(value);
    match status.as_str() {
        "cancelled" => "cancelled".to_string(),
        "canceled" => "canceled".to_string(),
        "needs_user" => "needs_user".to_string(),
        "failed" => "failed".to_string(),
        "timeout" => "timeout".to_string(),
        "succeeded" | "success" | "completed" => "succeeded".to_string(),
        "queued" | "running" | "waiting" | "background" => status,
        _ => "failed".to_string(),
    }
}

fn stable_machine_ref(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.' | '/'))
        .take(180)
        .collect()
}

#[cfg(test)]
#[path = "scheduled_run_contract_tests.rs"]
mod tests;
