use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use crate::{client, task};

pub(crate) fn run_health(base_url: &str, key: Option<&str>) -> Result<()> {
    let url = format!("{}/health", client::base_v1(base_url));
    let mut req = Client::new().get(&url);
    if let Some(k) = key {
        req = req.header("x-rustclaw-key", k);
    }
    let resp = req.send().context("request failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse health response")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    if !status.is_success() {
        anyhow::bail!("health returned {}", status);
    }
    Ok(())
}

pub(crate) fn run_submit(
    base_url: &str,
    key: &str,
    text: &str,
    wait: bool,
    detach: bool,
    json_output: bool,
    interval_ms: u64,
) -> Result<()> {
    if wait && detach {
        anyhow::bail!("submit_wait_detach_conflict");
    }
    let task_id = task::submit_ask(base_url, key, text)?;
    if wait {
        let task = wait_for_terminal_task(base_url, key, &task_id, interval_ms)?;
        if json_output {
            println!("{}", serde_json::to_string_pretty(&task.raw_data)?);
        } else {
            print_task_status(&task, false, &[]);
        }
    } else if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "task_id": task_id,
                "detached": true,
            }))?
        );
    } else {
        println!("task_id: {}", task_id);
    }
    Ok(())
}

pub(crate) fn run_skill(
    base_url: &str,
    key: &str,
    skill_name: &str,
    args_json: Option<&str>,
    args_file: Option<&PathBuf>,
    wait: bool,
    json_output: bool,
    interval_ms: u64,
) -> Result<()> {
    let args = parse_run_skill_args(args_json, args_file)?;
    let task_id = task::submit_run_skill(base_url, key, skill_name, args)?;
    if wait {
        let task = wait_for_terminal_task(base_url, key, &task_id, interval_ms)?;
        if json_output {
            println!("{}", serde_json::to_string_pretty(&task.raw_data)?);
        } else {
            print_task_status(&task, false, &[]);
        }
    } else if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "task_id": task_id,
                "kind": "run_skill",
                "skill_name": skill_name,
                "detached": true,
            }))?
        );
    } else {
        println!("task_id: {}", task_id);
    }
    Ok(())
}

fn parse_run_skill_args(
    args_json: Option<&str>,
    args_file: Option<&PathBuf>,
) -> Result<serde_json::Value> {
    if args_json.is_some() && args_file.is_some() {
        anyhow::bail!("run_skill_args_source_conflict");
    }
    let raw = if let Some(raw) = args_json {
        Some(raw.to_string())
    } else if let Some(path) = args_file {
        Some(
            std::fs::read_to_string(path)
                .with_context(|| format!("read run-skill args file failed: {}", path.display()))?,
        )
    } else {
        None
    };
    let Some(raw) = raw else {
        return Ok(json!({}));
    };
    let value = serde_json::from_str::<serde_json::Value>(&raw).context("parse run-skill args")?;
    if !value.is_object() {
        anyhow::bail!("run_skill_args_must_be_json_object");
    }
    Ok(value)
}

pub(crate) fn run_resume(
    base_url: &str,
    key: &str,
    resume_task_id: &str,
    text: &str,
) -> Result<()> {
    let task_id = task::submit_resume_ask(base_url, key, resume_task_id, text)?;
    println!("task_id: {}", task_id);
    println!("resume_task_id: {}", resume_task_id);
    Ok(())
}

pub(crate) fn run_get(
    base_url: &str,
    key: &str,
    task_id: &str,
    events: bool,
    event_types: &[String],
    events_output: Option<&PathBuf>,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let requested_event_types = event_types
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    print_task_status(
        &task,
        events || !requested_event_types.is_empty(),
        &requested_event_types,
    );
    let filtered_events = filtered_event_lines(&task, &requested_event_types);
    if let Some(path) = events_output {
        let mut content = filtered_events.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        std::fs::write(path, content)
            .with_context(|| format!("write events output failed: path={}", path.display()))?;
    }
    Ok(())
}

fn wait_for_terminal_task(
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

pub(crate) fn run_watch(
    base_url: &str,
    key: &str,
    task_id: &str,
    events: bool,
    event_types: &[String],
    until_terminal: bool,
    interval_ms: u64,
    json_output: bool,
    jsonl_output: bool,
) -> Result<()> {
    let requested_event_types = event_types
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let mut last_snapshot = String::new();
    let mut seen_events = HashSet::new();
    let interval = Duration::from_millis(interval_ms.max(100));

    loop {
        let task = task::get_task_status(base_url, key, task_id)?;
        if jsonl_output {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "task_id": &task.task_id,
                    "status": &task.status,
                    "lifecycle_state": task.lifecycle_state(),
                    "lifecycle": task.lifecycle().cloned().unwrap_or(serde_json::Value::Null),
                    "terminal": task.is_terminal(),
                    "event_count": task.events.len(),
                }))?
            );
        } else if json_output {
            println!("{}", serde_json::to_string_pretty(&task.raw_data)?);
        } else {
            let snapshot = format!(
                "{}|{}",
                task.status,
                task.lifecycle_summary_tokens().join(" ")
            );
            if snapshot != last_snapshot {
                print_task_status(&task, false, &requested_event_types);
                last_snapshot = snapshot;
            }
        }

        if events || !requested_event_types.is_empty() {
            for line in filtered_event_lines(&task, &requested_event_types) {
                if seen_events.insert(line.clone()) {
                    println!("{line}");
                }
            }
        }

        if until_terminal && task.is_terminal() {
            break;
        }
        std::thread::sleep(interval);
    }
    Ok(())
}

pub(crate) fn run_events(
    base_url: &str,
    key: &str,
    task_id: &str,
    event_types: &[String],
    jsonl_output: bool,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let requested_event_types = event_types
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let events = filtered_events(&task, &requested_event_types);
    for event in events {
        if jsonl_output {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "task_id": &task.task_id,
                    "event_type": &event.event_type,
                    "line": &event.line,
                }))?
            );
        } else {
            println!("event: {}", event.line);
        }
    }
    Ok(())
}

fn print_task_status(
    task: &task::TaskStatusView,
    include_events: bool,
    requested_event_types: &[String],
) {
    println!("task_id: {}", task.task_id);
    println!("status: {}", task.status);
    if let Some(state) = task.lifecycle_state() {
        println!("lifecycle_state: {state}");
    }
    let lifecycle_tokens = task.lifecycle_summary_tokens();
    if !lifecycle_tokens.is_empty() {
        println!("lifecycle: {}", lifecycle_tokens.join(" "));
    }
    if let Some(text) = task.result_text.as_deref() {
        println!("{text}");
    }
    if let Some(error_text) = task.error_text.as_deref() {
        eprintln!("error: {error_text}");
    }
    if include_events {
        for line in filtered_event_lines(task, requested_event_types) {
            println!("{line}");
        }
    }
}

fn filtered_event_lines(
    task: &task::TaskStatusView,
    requested_event_types: &[String],
) -> Vec<String> {
    filtered_events(task, requested_event_types)
        .into_iter()
        .map(|event| format!("event: {}", event.line))
        .collect()
}

fn filtered_events<'a>(
    task: &'a task::TaskStatusView,
    requested_event_types: &[String],
) -> Vec<&'a crate::events::TaskEventLine> {
    task.events
        .iter()
        .filter(|event| {
            requested_event_types.is_empty()
                || requested_event_types
                    .iter()
                    .any(|requested| requested == &event.event_type.to_ascii_lowercase())
        })
        .collect()
}

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
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else {
        print_active_task_table(&body);
    }
    Ok(())
}

fn print_active_task_table(body: &serde_json::Value) {
    let tasks = body
        .pointer("/data/tasks")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    println!(
        "{:<5} {:<36} {:<10} {:<12} {:<8} summary",
        "idx", "task_id", "status", "lifecycle", "age_s"
    );
    for task in tasks {
        let index = value_token(task.get("index"));
        let task_id = value_token(task.get("task_id"));
        let status = value_token(task.get("status"));
        let lifecycle = task
            .get("lifecycle")
            .and_then(|value| value.get("state"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let age_seconds = value_token(task.get("age_seconds"));
        let summary = truncate_display_token(&value_token(task.get("summary")), 80);
        println!(
            "{:<5} {:<36} {:<10} {:<12} {:<8} {}",
            index, task_id, status, lifecycle, age_seconds, summary
        );
    }
}

fn value_token(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(value)) => value.trim().to_string(),
        Some(serde_json::Value::Number(value)) => value.to_string(),
        Some(serde_json::Value::Bool(value)) => value.to_string(),
        Some(
            serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_),
        )
        | None => String::new(),
    }
}

fn truncate_display_token(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
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
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    if !status.is_success() {
        anyhow::bail!("cancel returned {}: {:?}", status, body.get("error"));
    }
    Ok(())
}

pub(crate) fn run_cancel_task(base_url: &str, key: &str, task_id: &str) -> Result<()> {
    let body = task::cancel_task_by_id(base_url, key, task_id)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
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
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    if !status.is_success() {
        anyhow::bail!("cancel-index returned {}: {:?}", status, body.get("error"));
    }
    Ok(())
}

pub(crate) fn run_reload_skills(base_url: &str, key: &str) -> Result<()> {
    let url = format!("{}/admin/reload-skills", client::base_v1(base_url));
    let resp = client::make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .send()
        .context("reload-skills failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse reload-skills response")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    if !status.is_success() {
        anyhow::bail!("reload-skills returned {}: {:?}", status, body.get("error"));
    }
    Ok(())
}
