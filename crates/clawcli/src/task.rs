use anyhow::{Context, Result};
use serde_json::json;

use crate::client;
use crate::events::{task_event_lines, TaskEventLine};

pub(crate) struct TaskStatusView {
    pub(crate) task_id: String,
    pub(crate) status: String,
    pub(crate) raw_data: serde_json::Value,
    pub(crate) result_text: Option<String>,
    pub(crate) error_text: Option<String>,
    pub(crate) events: Vec<TaskEventLine>,
}

impl TaskStatusView {
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            "succeeded" | "failed" | "canceled" | "cancelled" | "timeout"
        )
    }
}

pub(crate) fn submit_ask(base_url: &str, key: &str, text: &str) -> Result<String> {
    submit_ask_with_payload(
        base_url,
        key,
        json!({
            "text": text
        }),
    )
}

pub(crate) fn submit_resume_ask(
    base_url: &str,
    key: &str,
    task_id: &str,
    text: &str,
) -> Result<String> {
    submit_ask_with_payload(
        base_url,
        key,
        json!({
            "text": text,
            "resume_task_id": task_id,
            "resume_trigger": "user_followup"
        }),
    )
}

fn submit_ask_with_payload(
    base_url: &str,
    key: &str,
    payload: serde_json::Value,
) -> Result<String> {
    let url = format!("{}/tasks", client::base_v1(base_url));
    let body = json!({
        "user_key": key,
        "channel": "ui",
        "kind": "ask",
        "payload": payload
    });
    let resp = client::make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .context("submit task failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse submit response")?;
    if !status.is_success() {
        anyhow::bail!("submit returned {}: {:?}", status, body.get("error"));
    }
    let task_id = body
        .get("data")
        .and_then(|d| d.get("task_id"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("response missing data.task_id"))?;
    Ok(task_id.to_string())
}

pub(crate) fn get_task_status(base_url: &str, key: &str, task_id: &str) -> Result<TaskStatusView> {
    let url = format!("{}/tasks/{}", client::base_v1(base_url), task_id);
    let resp = client::make_client()?
        .get(&url)
        .header("x-rustclaw-key", key)
        .send()
        .context("get task failed")?;
    let status_code = resp.status();
    let body: serde_json::Value = resp.json().context("parse get task response")?;
    if !status_code.is_success() {
        anyhow::bail!("get task returned {}: {:?}", status_code, body.get("error"));
    }
    let data = body
        .get("data")
        .ok_or_else(|| anyhow::anyhow!("response missing data"))?;
    let status = data
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let result_json = data.get("result_json");
    let result_text = result_json
        .and_then(|v| v.get("messages").and_then(|m| m.as_array()))
        .and_then(|arr| {
            let lines: Vec<String> = arr
                .iter()
                .filter_map(|m| {
                    m.get("text")
                        .and_then(|t| t.as_str())
                        .map(String::from)
                        .or_else(|| m.as_str().map(String::from))
                })
                .collect();
            if lines.is_empty() {
                None
            } else {
                Some(lines.join("\n\n"))
            }
        })
        .or_else(|| {
            result_json.and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
        });
    let error_text = data
        .get("error_text")
        .and_then(|e| e.as_str())
        .map(String::from);
    let events = task_event_lines(data);
    Ok(TaskStatusView {
        task_id: task_id.to_string(),
        status,
        raw_data: data.clone(),
        result_text,
        error_text,
        events,
    })
}
