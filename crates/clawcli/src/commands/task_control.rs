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
) -> Result<()> {
    let new_constraints = constraints_json
        .map(|raw| serde_json::from_str::<serde_json::Value>(raw))
        .transpose()
        .context("parse resume constraints json")?;
    let body = task::resume_task_by_id(
        base_url,
        key,
        task_id,
        checkpoint_id,
        resume_reason,
        user_message,
        new_constraints,
    )?;
    output::print_json_pretty(&body);
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
        None,
        Some("user_continue"),
        user_message,
        None,
    )?;
    if json_output {
        output::print_json_pretty(&body);
    } else {
        output::print_json_pretty(&json!({
            "task_id": task_id,
            "operation": "continue",
            "response": body,
        }));
    }
    Ok(())
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
