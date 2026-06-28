use anyhow::{Context, Result};
use std::time::Duration;

use crate::{client, task};

pub(super) fn wait_for_terminal_task(
    base_url: &str,
    key: &str,
    task_id: &str,
    interval_ms: u64,
) -> Result<task::TaskStatusView> {
    let interval = Duration::from_millis(interval_ms.max(100));
    loop {
        let task = task::get_task_status(base_url, key, task_id)?;
        if task.is_terminal() {
            return Ok(task);
        }
        std::thread::sleep(interval);
    }
}

pub(super) fn get_v1_json(
    base_url: &str,
    key: &str,
    path: &str,
    context_label: &str,
) -> Result<serde_json::Value> {
    let url = format!("{}{}", client::base_v1(base_url), path);
    let resp = client::make_client()?
        .get(&url)
        .header("x-rustclaw-key", key)
        .send()
        .with_context(|| format!("request {context_label} failed"))?;
    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .with_context(|| format!("parse {context_label} response"))?;
    if !status.is_success() {
        anyhow::bail!(
            "{} returned {}: {:?}",
            context_label,
            status,
            body.get("error")
        );
    }
    Ok(body)
}
