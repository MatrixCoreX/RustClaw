use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

use crate::{client, events::EventFilters, output, task};

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_tui(
    base_url: &str,
    key: &str,
    user_id: i64,
    chat_id: i64,
    selected_task_id: Option<&str>,
    include_events: bool,
    once: bool,
    interval_ms: u64,
    json_output: bool,
) -> Result<()> {
    let interval = Duration::from_millis(interval_ms.max(250));
    loop {
        let active = active_tasks(base_url, key, user_id, chat_id)?;
        let selected = selected_task_id
            .map(|task_id| task::get_task_status(base_url, key, task_id))
            .transpose()?;
        if json_output {
            output::print_json_pretty(&tui_snapshot_json(&active, selected.as_ref()));
        } else {
            print!("\x1b[2J\x1b[H");
            output::print_active_task_table(&active);
            if let Some(selected) = selected.as_ref() {
                println!();
                output::print_task_status(selected, include_events, &EventFilters::default());
            }
        }
        if once {
            return Ok(());
        }
        std::thread::sleep(interval);
    }
}

pub(super) fn tui_snapshot_json(active: &Value, selected: Option<&task::TaskStatusView>) -> Value {
    json!({
        "snapshot_kind": "rustclaw_cli_tui",
        "active": active,
        "selected_task": selected.map(|task| task.raw_data.clone()).unwrap_or(Value::Null),
    })
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
        .context("list active tasks failed")?;
    let status = resp.status();
    let body: Value = resp.json().context("parse active response")?;
    if !status.is_success() {
        anyhow::bail!("active returned {}: {:?}", status, body.get("error"));
    }
    Ok(body)
}
