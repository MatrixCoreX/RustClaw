use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::{client, output, task};

pub(crate) fn run_active(
    base_url: &str,
    key: &str,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<String>,
    json_output: bool,
) -> Result<()> {
    let url = format!("{}/tasks/active", client::base_v1(base_url));
    let payload = json!({
        "user_id": user_id,
        "chat_id": chat_id,
        "exclude_task_id": exclude_task_id,
    });
    let resp = client::make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .context("list active tasks failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse active response")?;
    if !status.is_success() {
        anyhow::bail!("active returned {}: {:?}", status, body.get("error"));
    }
    if json_output {
        output::print_json_pretty(&body);
    } else {
        output::print_active_task_table(&body);
    }
    Ok(())
}

pub(crate) fn run_automation_runs(
    base_url: &str,
    key: &str,
    user_id: i64,
    chat_id: i64,
    job_id: Option<String>,
    limit: usize,
    json_output: bool,
) -> Result<()> {
    let url = format!("{}/tasks/automation-runs", client::base_v1(base_url));
    let payload = automation_runs_request_payload(user_id, chat_id, job_id, limit);
    let resp = client::make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .context("list automation runs failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse automation runs response")?;
    if !status.is_success() {
        anyhow::bail!(
            "automation-runs returned {}: {:?}",
            status,
            body.get("error")
        );
    }
    if json_output {
        output::print_json_pretty(&body);
    } else {
        output::print_automation_run_table(&body);
    }
    Ok(())
}

pub(super) fn automation_runs_request_payload(
    user_id: i64,
    chat_id: i64,
    job_id: Option<String>,
    limit: usize,
) -> Value {
    json!({
        "user_id": user_id,
        "chat_id": chat_id,
        "job_id": job_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        "limit": limit.clamp(1, 100),
    })
}

pub(crate) fn run_cancel(
    base_url: &str,
    key: &str,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<String>,
) -> Result<()> {
    let url = format!("{}/tasks/cancel", client::base_v1(base_url));
    let payload = json!({
        "user_id": user_id,
        "chat_id": chat_id,
        "exclude_task_id": exclude_task_id,
    });
    let resp = client::make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .context("cancel tasks failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse cancel response")?;
    output::print_json_pretty(&body);
    if !status.is_success() {
        anyhow::bail!("cancel returned {}: {:?}", status, body.get("error"));
    }
    Ok(())
}

pub(crate) fn run_cancel_task(base_url: &str, key: &str, task_id: &str) -> Result<()> {
    let body = task::cancel_task_by_id(base_url, key, task_id)?;
    output::print_json_pretty(&body);
    Ok(())
}

pub(crate) fn run_resume_task(
    base_url: &str,
    key: &str,
    task_id: &str,
    checkpoint_id: Option<&str>,
    resume_reason: Option<&str>,
    user_message: Option<&str>,
    constraints_json: Option<&str>,
    approval_request_id: Option<&str>,
    approve: bool,
) -> Result<()> {
    let new_constraints = constraints_json
        .map(|raw| serde_json::from_str::<serde_json::Value>(raw))
        .transpose()
        .context("parse resume constraints json")?;
    let body = task::resume_task_by_id(
        base_url,
        key,
        task_id,
        task::TaskResumeRequest {
            checkpoint_id,
            resume_reason,
            user_message,
            new_constraints,
            approval_request_id,
            approve,
        },
    )?;
    output::print_json_pretty(&body_with_resume_summary(body, task_id, "resume_task"));
    Ok(())
}

pub(crate) fn run_continue_task(
    base_url: &str,
    key: &str,
    task_id: &str,
    user_message: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let body = task::resume_task_by_id(
        base_url,
        key,
        task_id,
        task::TaskResumeRequest {
            resume_reason: Some("user_continue"),
            user_message,
            ..Default::default()
        },
    )?;
    if json_output {
        output::print_json_pretty(&body);
    } else {
        let resume_summary = task_resume_control_summary_json(task_id, "continue", &body);
        output::print_json_pretty(&json!({
            "task_id": task_id,
            "operation": "continue",
            "resume_summary": resume_summary,
            "response": body,
        }));
    }
    Ok(())
}

fn body_with_resume_summary(mut body: Value, task_id: &str, operation: &str) -> Value {
    let summary = task_resume_control_summary_json(task_id, operation, &body);
    if let Some(map) = body.as_object_mut() {
        map.insert("resume_summary".to_string(), summary);
        body
    } else {
        json!({
            "response": body,
            "resume_summary": summary,
        })
    }
}

pub(super) fn task_resume_control_summary_json(
    requested_task_id: &str,
    operation: &str,
    body: &Value,
) -> Value {
    let data = body.get("data").unwrap_or(body);
    let lifecycle = data
        .get("task_lifecycle")
        .or_else(|| data.get("lifecycle"))
        .unwrap_or(&Value::Null);
    json!({
        "schema_version": 1,
        "operation": operation,
        "task_id": string_field(data, "task_id").unwrap_or(requested_task_id),
        "status": string_field(data, "status"),
        "checkpoint_id": string_field(data, "checkpoint_id")
            .or_else(|| string_field(lifecycle, "checkpoint_id")),
        "lifecycle_state": string_field(lifecycle, "state"),
        "execution_state": string_field(lifecycle, "execution_state"),
        "resume_due": lifecycle.get("resume_due").and_then(Value::as_bool),
        "resume_wait_seconds": lifecycle.get("resume_wait_seconds").and_then(Value::as_i64),
        "resume_entrypoint": string_field(lifecycle, "resume_entrypoint"),
        "resume_directive": string_field(lifecycle, "resume_directive"),
        "resume_reason": string_field(lifecycle, "resume_reason"),
        "resume_owner": string_field(lifecycle, "resume_owner")
            .or_else(|| nested_string_field(lifecycle, &["resume_claim", "owner"]))
            .or_else(|| nested_string_field(lifecycle, &["resume_executor_claim", "owner"])),
        "next_action_kind": string_field(lifecycle, "next_action_kind"),
        "last_successful_evidence_ref": string_field(lifecycle, "last_successful_evidence_ref"),
        "evidence_ref_count": lifecycle.get("evidence_ref_count").and_then(Value::as_u64),
        "budget": lifecycle.get("budget").cloned().unwrap_or(Value::Null),
    })
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn nested_string_field<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn run_pause_task(
    base_url: &str,
    key: &str,
    task_id: &str,
    pause_seconds: u64,
) -> Result<()> {
    let body = task::pause_task_by_id(base_url, key, task_id, pause_seconds)?;
    output::print_json_pretty(&body);
    Ok(())
}

pub(crate) fn run_cancel_index(
    base_url: &str,
    key: &str,
    user_id: i64,
    chat_id: i64,
    index: usize,
    exclude_task_id: Option<String>,
) -> Result<()> {
    let url = format!("{}/tasks/cancel-one", client::base_v1(base_url));
    let payload = json!({
        "user_id": user_id,
        "chat_id": chat_id,
        "index": index,
        "exclude_task_id": exclude_task_id,
    });
    let resp = client::make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .context("cancel task by index failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse cancel-index response")?;
    output::print_json_pretty(&body);
    if !status.is_success() {
        anyhow::bail!("cancel-index returned {}: {:?}", status, body.get("error"));
    }
    Ok(())
}
