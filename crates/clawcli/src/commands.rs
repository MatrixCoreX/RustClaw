use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{client, events::EventFilters, output, task};

pub(crate) fn run_health(base_url: &str, key: Option<&str>) -> Result<()> {
    let url = format!("{}/health", client::base_v1(base_url));
    let mut req = Client::new().get(&url);
    if let Some(k) = key {
        req = req.header("x-rustclaw-key", k);
    }
    let resp = req.send().context("request failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse health response")?;
    output::print_json_pretty(&body);
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
            output::print_json_pretty(&task.raw_data);
        } else {
            output::print_task_status(&task, false, &EventFilters::default());
        }
    } else if json_output {
        output::print_json_pretty(&json!({
            "task_id": task_id,
            "detached": true,
        }));
    } else {
        println!("task_id: {}", task_id);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecWaitOutcome {
    Terminal,
    Background,
    Timeout,
}

impl ExecWaitOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Background => "background",
            Self::Timeout => "timeout",
        }
    }
}

fn exec_summary_json(task: &task::TaskStatusView, outcome: ExecWaitOutcome) -> serde_json::Value {
    json!({
        "task_id": task.task_id,
        "status": task.status,
        "lifecycle_state": task.lifecycle_state(),
        "lifecycle": task.lifecycle().cloned().unwrap_or(serde_json::Value::Null),
        "terminal": task.is_terminal(),
        "outcome": outcome.as_str(),
        "result_text": task.result_text,
        "error_text": task.error_text,
    })
}

struct ExecWaitOptions {
    interval_ms: u64,
    timeout_seconds: Option<u64>,
    continue_on_background: bool,
    fail_on_background: bool,
    jsonl_output: bool,
}

fn wait_for_exec_task(
    base_url: &str,
    key: &str,
    task_id: &str,
    options: ExecWaitOptions,
) -> Result<(task::TaskStatusView, ExecWaitOutcome)> {
    let interval = Duration::from_millis(options.interval_ms.max(100));
    let deadline = options
        .timeout_seconds
        .map(|seconds| Instant::now() + Duration::from_secs(seconds.max(1)));
    loop {
        let task = task::get_task_status(base_url, key, task_id)?;
        if task.is_terminal() {
            return Ok((task, ExecWaitOutcome::Terminal));
        }
        if task.is_background_waiting()
            && (options.continue_on_background || options.fail_on_background)
        {
            return Ok((task, ExecWaitOutcome::Background));
        }
        if let Some(deadline) = deadline {
            if Instant::now() >= deadline {
                return Ok((task, ExecWaitOutcome::Timeout));
            }
        }
        if options.jsonl_output {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "task_id": task.task_id,
                    "status": task.status,
                    "lifecycle_state": task.lifecycle_state(),
                    "terminal": false,
                    "outcome": "poll",
                }))?
            );
        }
        std::thread::sleep(interval);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_exec(
    base_url: &str,
    key: &str,
    prompt: &str,
    resume_task_id: Option<&str>,
    detach: bool,
    json_output: bool,
    jsonl_output: bool,
    timeout_seconds: Option<u64>,
    interval_ms: u64,
    continue_on_background: bool,
    fail_on_background: bool,
) -> Result<()> {
    if continue_on_background && fail_on_background {
        anyhow::bail!("exec_background_policy_conflict");
    }
    let task_id = if let Some(resume_task_id) = resume_task_id {
        task::submit_resume_ask(base_url, key, resume_task_id, prompt)?
    } else {
        task::submit_ask(base_url, key, prompt)?
    };
    if detach {
        if json_output || jsonl_output {
            output::print_json_pretty(&json!({
                "task_id": task_id,
                "detached": true,
            }));
        } else {
            println!("task_id: {}", task_id);
        }
        return Ok(());
    }

    let (task, outcome) = wait_for_exec_task(
        base_url,
        key,
        &task_id,
        ExecWaitOptions {
            interval_ms,
            timeout_seconds,
            continue_on_background,
            fail_on_background,
            jsonl_output,
        },
    )?;
    if json_output || jsonl_output {
        output::print_json_pretty(&exec_summary_json(&task, outcome));
    } else {
        output::print_task_status(&task, false, &EventFilters::default());
        println!("exec_outcome: {}", outcome.as_str());
    }
    match outcome {
        ExecWaitOutcome::Terminal => {
            if task.status == "failed" || task.status == "timeout" || task.status == "canceled" {
                anyhow::bail!("exec_terminal_failure status={}", task.status);
            }
        }
        ExecWaitOutcome::Background if fail_on_background => {
            anyhow::bail!("exec_background status={}", task.status);
        }
        ExecWaitOutcome::Timeout => {
            anyhow::bail!("exec_timeout task_id={}", task.task_id);
        }
        ExecWaitOutcome::Background => {}
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
            output::print_json_pretty(&task.raw_data);
        } else {
            output::print_task_status(&task, false, &EventFilters::default());
        }
    } else if json_output {
        output::print_json_pretty(&json!({
            "task_id": task_id,
            "kind": "run_skill",
            "skill_name": skill_name,
            "detached": true,
        }));
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
    checkpoint_id: Option<&str>,
    policy_decision: Option<&str>,
    subagent_id: Option<&str>,
    async_job_id: Option<&str>,
    events_output: Option<&PathBuf>,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let event_filters = EventFilters::from_parts(
        event_types,
        checkpoint_id,
        policy_decision,
        subagent_id,
        async_job_id,
    );
    output::print_task_status(&task, events || !event_filters.is_empty(), &event_filters);
    let filtered_events = output::filtered_event_lines(&task, &event_filters);
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
    checkpoint_id: Option<&str>,
    policy_decision: Option<&str>,
    subagent_id: Option<&str>,
    async_job_id: Option<&str>,
    until_terminal: bool,
    interval_ms: u64,
    json_output: bool,
    jsonl_output: bool,
) -> Result<()> {
    let event_filters = EventFilters::from_parts(
        event_types,
        checkpoint_id,
        policy_decision,
        subagent_id,
        async_job_id,
    );
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
            output::print_json_pretty(&task.raw_data);
        } else {
            let snapshot = format!(
                "{}|{}",
                task.status,
                task.lifecycle_summary_tokens().join(" ")
            );
            if snapshot != last_snapshot {
                output::print_task_status(&task, false, &event_filters);
                last_snapshot = snapshot;
            }
        }

        if events || !event_filters.is_empty() {
            for line in output::filtered_event_lines(&task, &event_filters) {
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
    checkpoint_id: Option<&str>,
    policy_decision: Option<&str>,
    subagent_id: Option<&str>,
    async_job_id: Option<&str>,
    jsonl_output: bool,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let event_filters = EventFilters::from_parts(
        event_types,
        checkpoint_id,
        policy_decision,
        subagent_id,
        async_job_id,
    );
    let events = output::filtered_events(&task, &event_filters);
    for event in events {
        if jsonl_output {
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "task_id": &task.task_id,
                    "event_type": &event.event_type,
                    "line": &event.line,
                    "fields": &event.fields,
                }))?
            );
        } else {
            println!("event: {}", event.line);
        }
    }
    Ok(())
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
        output::print_json_pretty(&body);
    } else {
        output::print_active_task_table(&body);
    }
    Ok(())
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

pub(crate) fn run_resume_task(base_url: &str, key: &str, task_id: &str) -> Result<()> {
    let body = task::resume_task_by_id(base_url, key, task_id)?;
    output::print_json_pretty(&body);
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

pub(crate) fn run_skills(base_url: &str, key: &str, config: bool, json_output: bool) -> Result<()> {
    let path = if config { "/skills/config" } else { "/skills" };
    let body = get_v1_json(base_url, key, path, "skills")?;
    if json_output {
        output::print_json_pretty(&body);
    } else {
        output::print_skill_table(&body);
    }
    Ok(())
}

pub(crate) fn run_capabilities(base_url: &str, key: &str, json_output: bool) -> Result<()> {
    let body = get_v1_json(base_url, key, "/capabilities", "capabilities")?;
    if json_output {
        output::print_json_pretty(&body);
    } else {
        output::print_capability_table(&body);
    }
    Ok(())
}

fn get_v1_json(
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

pub(crate) fn run_reload_skills(base_url: &str, key: &str) -> Result<()> {
    let url = format!("{}/admin/reload-skills", client::base_v1(base_url));
    let resp = client::make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .send()
        .context("reload-skills failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse reload-skills response")?;
    output::print_json_pretty(&body);
    if !status.is_success() {
        anyhow::bail!("reload-skills returned {}: {:?}", status, body.get("error"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
