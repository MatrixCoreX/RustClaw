use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecExitClass {
    Success,
    Failed,
    Cancelled,
    Timeout,
    NeedsUser,
    PolicyDenied,
    ProviderUnavailable,
    InvalidRequest,
    Background,
}

impl ExecExitClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::NeedsUser => "needs_user",
            Self::PolicyDenied => "policy_denied",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::InvalidRequest => "invalid_request",
            Self::Background => "background",
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failed => 1,
            Self::Cancelled => 130,
            Self::Timeout => 124,
            Self::NeedsUser => 78,
            Self::PolicyDenied => 77,
            Self::ProviderUnavailable => 69,
            Self::InvalidRequest => 64,
            Self::Background => 75,
        }
    }
}

fn exec_summary_json(
    task: &task::TaskStatusView,
    outcome: ExecWaitOutcome,
    exit_class: ExecExitClass,
    resume_task_id: Option<&str>,
) -> serde_json::Value {
    let artifact_refs = exec_artifact_refs(&task.raw_data);
    json!({
        "task_id": task.task_id,
        "status": task.status,
        "lifecycle_state": task.lifecycle_state(),
        "lifecycle": task.lifecycle().cloned().unwrap_or(serde_json::Value::Null),
        "terminal": task.is_terminal(),
        "outcome": outcome.as_str(),
        "exit_class": exit_class.as_str(),
        "exit_code": exit_class.code(),
        "resume": exec_resume_summary(resume_task_id),
        "result_text": task.result_text,
        "async_result": async_final_result_json(&task.raw_data).unwrap_or(Value::Null),
        "error_text": task.error_text,
        "events": exec_event_summary(task),
        "artifacts": {
            "ref_count": artifact_refs.len(),
            "refs": artifact_refs,
        },
    })
}

fn exec_resume_summary(resume_task_id: Option<&str>) -> Value {
    let Some(source_task_id) = resume_task_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return json!({
            "mode": "new_task",
        });
    };
    json!({
        "mode": "resume_task",
        "source_task_id": source_task_id,
        "resume_trigger": "user_followup",
    })
}

fn exec_event_summary(task: &task::TaskStatusView) -> Vec<Value> {
    task.events
        .iter()
        .map(|event| {
            json!({
                "event_type": &event.event_type,
                "line": &event.line,
                "fields": &event.fields,
            })
        })
        .collect()
}

pub(crate) fn task_report_json(task: &task::TaskStatusView, include_events: bool) -> Value {
    let artifact_refs = exec_artifact_refs(&task.raw_data);
    json!({
        "report_kind": "rustclaw_task_report",
        "task_id": task.task_id,
        "status": task.status,
        "execution_state": task.execution_state(),
        "lifecycle_state": task.lifecycle_state(),
        "lifecycle": task.lifecycle().cloned().unwrap_or(Value::Null),
        "terminal": task.is_terminal(),
        "result_text": task.result_text,
        "async_result": async_final_result_json(&task.raw_data).unwrap_or(Value::Null),
        "error_text": task.error_text,
        "event_count": task.events.len(),
        "events": if include_events {
            Value::Array(exec_event_summary(task))
        } else {
            Value::Null
        },
        "coding": coding_report_json(&task.raw_data),
        "artifacts": {
            "ref_count": artifact_refs.len(),
            "refs": artifact_refs,
        },
    })
}

fn async_final_result_json(data: &Value) -> Option<Value> {
    data.get("result_json")
        .and_then(task::async_final_result_value)
        .cloned()
}

fn exec_artifact_refs(data: &Value) -> Vec<Value> {
    let mut refs = Vec::new();
    collect_exec_artifact_refs(data, &mut refs, 0);
    refs
}

fn collect_exec_artifact_refs(value: &Value, refs: &mut Vec<Value>, depth: usize) {
    if depth > 8 || refs.len() >= 128 {
        return;
    }
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("artifact_refs") {
                for item in items.iter().take(128usize.saturating_sub(refs.len())) {
                    refs.push(item.clone());
                }
            }
            for value in map.values() {
                collect_exec_artifact_refs(value, refs, depth + 1);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_exec_artifact_refs(item, refs, depth + 1);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[derive(Default)]
struct CodingReportSignals {
    changed_files: BTreeSet<String>,
    commands: BTreeSet<String>,
    tests: BTreeSet<String>,
    failures: Vec<Value>,
    retry_count: u64,
}

fn coding_report_json(data: &Value) -> Value {
    let mut signals = CodingReportSignals::default();
    collect_coding_report_signals(data, &mut signals, 0);
    let unverified_risk = if !signals.changed_files.is_empty() && signals.tests.is_empty() {
        Value::String("tests_not_observed".to_string())
    } else {
        Value::Null
    };
    json!({
        "schema_version": 1,
        "changed_file_count": signals.changed_files.len(),
        "changed_files": signals.changed_files.into_iter().collect::<Vec<_>>(),
        "command_count": signals.commands.len(),
        "commands": signals.commands.into_iter().collect::<Vec<_>>(),
        "test_count": signals.tests.len(),
        "tests": signals.tests.into_iter().collect::<Vec<_>>(),
        "failure_count": signals.failures.len(),
        "failures": signals.failures,
        "retry_count": signals.retry_count,
        "unverified_risk": unverified_risk,
    })
}

fn collect_coding_report_signals(value: &Value, signals: &mut CodingReportSignals, depth: usize) {
    if depth > 12 {
        return;
    }
    match value {
        Value::Object(map) => {
            collect_changed_file_fields(map, signals);
            collect_command_fields(map, signals);
            collect_failure_fields(map, signals);
            collect_retry_fields(map, signals);
            for value in map.values() {
                collect_coding_report_signals(value, signals, depth + 1);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_coding_report_signals(item, signals, depth + 1);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn collect_changed_file_fields(map: &Map<String, Value>, signals: &mut CodingReportSignals) {
    for key in [
        "changed_files",
        "files_changed",
        "modified_files",
        "created_files",
        "deleted_files",
        "touched_files",
    ] {
        collect_path_tokens(map.get(key), &mut signals.changed_files);
    }
}

fn collect_path_tokens(value: Option<&Value>, out: &mut BTreeSet<String>) {
    match value {
        Some(Value::String(path)) => {
            if is_report_path_token(path) {
                out.insert(path.trim().to_string());
            }
        }
        Some(Value::Object(map)) => {
            for key in ["path", "file", "file_path", "resolved_path"] {
                collect_path_tokens(map.get(key), out);
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                collect_path_tokens(Some(item), out);
            }
        }
        Some(Value::Null | Value::Bool(_) | Value::Number(_)) | None => {}
    }
}

fn is_report_path_token(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 300
        && !trimmed.chars().any(|ch| matches!(ch, '\n' | '\r'))
}

fn collect_command_fields(map: &Map<String, Value>, signals: &mut CodingReportSignals) {
    if let Some(command) = map.get("command").and_then(Value::as_str) {
        collect_command_token(command, signals);
    }
    if let Some(summary) = map.get("sanitized_args_summary").and_then(Value::as_str) {
        if let Some(command) = summary.trim().strip_prefix("command=") {
            collect_command_token(command, signals);
        }
    }
}

fn collect_command_token(command: &str, signals: &mut CodingReportSignals) {
    let command = command.trim();
    if command.is_empty()
        || command.len() > 500
        || command.chars().any(|ch| matches!(ch, '\n' | '\r'))
    {
        return;
    }
    signals.commands.insert(command.to_string());
    if is_test_command_token(command) {
        signals.tests.insert(command.to_string());
    }
}

fn is_test_command_token(command: &str) -> bool {
    let command = command.trim().to_ascii_lowercase();
    command.starts_with("cargo test")
        || command.starts_with("npm test")
        || command.starts_with("npm run test")
        || command.starts_with("pnpm test")
        || command.starts_with("yarn test")
        || command.starts_with("pytest")
        || command.starts_with("go test")
}

fn collect_failure_fields(map: &Map<String, Value>, signals: &mut CodingReportSignals) {
    let Some(status) = map.get("status").and_then(Value::as_str) else {
        return;
    };
    if !is_failure_status_token(status) || signals.failures.len() >= 32 {
        return;
    }
    let has_step_identity = map.get("step_id").is_some()
        || map.get("action_ref").is_some()
        || map.get("requested_action_ref").is_some();
    if !has_step_identity {
        return;
    }
    signals.failures.push(json!({
        "step_id": map.get("step_id").cloned().unwrap_or(Value::Null),
        "status": status,
        "skill": map.get("skill").cloned().unwrap_or(Value::Null),
        "action_ref": map
            .get("action_ref")
            .or_else(|| map.get("requested_action_ref"))
            .cloned()
            .unwrap_or(Value::Null),
        "error_code": map
            .get("error_code")
            .or_else(|| map.get("error_kind"))
            .cloned()
            .unwrap_or(Value::Null),
    }));
}

fn is_failure_status_token(status: &str) -> bool {
    matches!(
        status.trim(),
        "error" | "failed" | "failure" | "timeout" | "cancelled" | "canceled"
    )
}

fn collect_retry_fields(map: &Map<String, Value>, signals: &mut CodingReportSignals) {
    for key in [
        "repair_count",
        "retry_count",
        "retry_attempt",
        "repair_attempt",
    ] {
        if let Some(count) = map.get(key).and_then(Value::as_u64) {
            signals.retry_count = signals.retry_count.max(count);
        }
    }
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
    artifact_dir: Option<&PathBuf>,
) -> Result<u8> {
    if continue_on_background && fail_on_background {
        let exit_class = ExecExitClass::InvalidRequest;
        let summary = json!({
            "exit_class": exit_class.as_str(),
            "exit_code": exit_class.code(),
            "error_code": "exec_background_policy_conflict",
            "resume": exec_resume_summary(resume_task_id),
        });
        if json_output || jsonl_output {
            output::print_json_pretty(&summary);
        } else {
            eprintln!("error_code=exec_background_policy_conflict");
        }
        if let Some(artifact_dir) = artifact_dir {
            write_exec_detached_artifacts(artifact_dir, &summary)?;
        }
        return Ok(exit_class.code());
    }
    let task_id = if let Some(resume_task_id) = resume_task_id {
        task::submit_resume_ask(base_url, key, resume_task_id, prompt)?
    } else {
        task::submit_ask(base_url, key, prompt)?
    };
    if detach {
        let exit_class = ExecExitClass::Success;
        let summary = json!({
            "task_id": task_id,
            "detached": true,
            "exit_class": exit_class.as_str(),
            "exit_code": exit_class.code(),
            "resume": exec_resume_summary(resume_task_id),
        });
        if json_output || jsonl_output {
            output::print_json_pretty(&summary);
        } else {
            println!("task_id: {}", task_id);
        }
        if let Some(artifact_dir) = artifact_dir {
            write_exec_detached_artifacts(artifact_dir, &summary)?;
        }
        return Ok(exit_class.code());
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
    let exit_class = exec_exit_class(&task, outcome, fail_on_background);
    let summary = exec_summary_json(&task, outcome, exit_class, resume_task_id);
    if let Some(artifact_dir) = artifact_dir {
        write_exec_artifacts(artifact_dir, &task, &summary)?;
    }
    if json_output || jsonl_output {
        output::print_json_pretty(&summary);
    } else {
        output::print_task_status(&task, false, &EventFilters::default());
        println!("exec_outcome: {}", outcome.as_str());
        println!("exec_exit_class: {}", exit_class.as_str());
        println!("exec_exit_code: {}", exit_class.code());
    }
    Ok(exit_class.code())
}

fn exec_exit_class(
    task: &task::TaskStatusView,
    outcome: ExecWaitOutcome,
    fail_on_background: bool,
) -> ExecExitClass {
    match outcome {
        ExecWaitOutcome::Timeout => ExecExitClass::Timeout,
        ExecWaitOutcome::Background if !fail_on_background => ExecExitClass::Success,
        ExecWaitOutcome::Background => {
            if task.lifecycle_state() == Some("needs_user") {
                ExecExitClass::NeedsUser
            } else {
                ExecExitClass::Background
            }
        }
        ExecWaitOutcome::Terminal => exec_terminal_exit_class(task),
    }
}

fn exec_terminal_exit_class(task: &task::TaskStatusView) -> ExecExitClass {
    match task.status.trim() {
        "succeeded" => ExecExitClass::Success,
        "canceled" | "cancelled" => ExecExitClass::Cancelled,
        "timeout" => ExecExitClass::Timeout,
        "failed" => exec_failure_class_from_machine_tokens(task),
        _ if task.lifecycle_state() == Some("needs_user") => ExecExitClass::NeedsUser,
        _ => ExecExitClass::Failed,
    }
}

fn exec_failure_class_from_machine_tokens(task: &task::TaskStatusView) -> ExecExitClass {
    let mut tokens = Vec::new();
    collect_exec_machine_token(task.lifecycle(), &mut tokens);
    for pointer in [
        "/error_code",
        "/message_key",
        "/reason_code",
        "/failure_kind",
        "/failure_attribution",
        "/result_json/error_code",
        "/result_json/message_key",
        "/result_json/reason_code",
        "/result_json/failure_kind",
        "/result_json/failure_attribution",
        "/result_json/task_journal/trace/final_status",
        "/result_json/task_journal/trace/final_stop_signal",
    ] {
        collect_exec_machine_token(task.raw_data.pointer(pointer), &mut tokens);
    }
    if tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "policy_denied" | "permission_denied" | "denied_by_policy" | "skill_policy_denied"
        )
    }) {
        return ExecExitClass::PolicyDenied;
    }
    if tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "provider_unavailable"
                | "provider_rate_limited"
                | "rate_limited"
                | "quota_exceeded"
                | "provider_timeout"
        )
    }) {
        return ExecExitClass::ProviderUnavailable;
    }
    if tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "invalid_request" | "invalid_args" | "schema_validation_failed"
        )
    }) {
        return ExecExitClass::InvalidRequest;
    }
    ExecExitClass::Failed
}

fn collect_exec_machine_token(value: Option<&Value>, tokens: &mut Vec<String>) {
    match value {
        Some(Value::String(value)) => {
            let token = value.trim();
            if is_exec_machine_token(token) {
                tokens.push(token.to_ascii_lowercase());
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                collect_exec_machine_token(Some(item), tokens);
            }
        }
        Some(Value::Object(map)) => {
            for key in [
                "state",
                "status",
                "db_status",
                "terminal_reason",
                "waiting_reason_code",
                "message_key",
                "reason_code",
                "error_code",
                "failure_kind",
                "failure_attribution",
                "policy_decision",
                "provider_error_kind",
                "final_status",
                "final_stop_signal",
            ] {
                collect_exec_machine_token(map.get(key), tokens);
            }
        }
        Some(Value::Null | Value::Bool(_) | Value::Number(_)) | None => {}
    }
}

fn is_exec_machine_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | '.')
        })
}

fn write_exec_detached_artifacts(artifact_dir: &Path, summary: &Value) -> Result<()> {
    fs::create_dir_all(artifact_dir)
        .with_context(|| format!("create artifact dir {}", artifact_dir.display()))?;
    write_json_file(&artifact_dir.join("summary.json"), summary)
}

fn write_exec_artifacts(
    artifact_dir: &Path,
    task: &task::TaskStatusView,
    summary: &Value,
) -> Result<()> {
    fs::create_dir_all(artifact_dir)
        .with_context(|| format!("create artifact dir {}", artifact_dir.display()))?;
    write_json_file(&artifact_dir.join("summary.json"), summary)?;
    write_json_file(&artifact_dir.join("task.json"), &task.raw_data)?;
    let mut events = String::new();
    for event in &task.events {
        events.push_str(&event.line);
        events.push('\n');
    }
    fs::write(artifact_dir.join("events.jsonl"), events)
        .with_context(|| format!("write artifact dir {}", artifact_dir.display()))?;
    Ok(())
}

fn write_json_file(path: &Path, value: &Value) -> Result<()> {
    let body = serde_json::to_vec_pretty(value)?;
    fs::write(path, body).with_context(|| format!("write artifact {}", path.display()))
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
    for line in task_event_output_lines(&task, events, jsonl_output)? {
        println!("{line}");
    }
    Ok(())
}

fn task_event_output_lines(
    task: &task::TaskStatusView,
    events: Vec<&crate::events::TaskEventLine>,
    jsonl_output: bool,
) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    for event in events {
        if jsonl_output {
            lines.push(serde_json::to_string(&json!({
                "task_id": &task.task_id,
                "event_type": &event.event_type,
                "line": &event.line,
                "fields": &event.fields,
            }))?);
        } else {
            lines.push(format!("event: {}", event.line));
        }
    }
    Ok(lines)
}

pub(crate) fn run_logs(
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
    run_events(
        base_url,
        key,
        task_id,
        event_types,
        checkpoint_id,
        policy_decision,
        subagent_id,
        async_job_id,
        jsonl_output,
    )
}

pub(crate) fn run_report(
    base_url: &str,
    key: &str,
    task_id: &str,
    json_output: bool,
    include_events: bool,
) -> Result<()> {
    let task = task::get_task_status(base_url, key, task_id)?;
    let report = task_report_json(&task, include_events);
    if json_output {
        output::print_json_pretty(&report);
    } else {
        println!("task_id: {}", task.task_id);
        println!("status: {}", task.status);
        if let Some(state) = task.execution_state() {
            println!("execution_state: {state}");
        }
        if let Some(state) = task.lifecycle_state() {
            println!("lifecycle_state: {state}");
        }
        println!("terminal: {}", task.is_terminal());
        println!("event_count: {}", task.events.len());
        println!(
            "artifact_ref_count: {}",
            report
                .pointer("/artifacts/ref_count")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        );
        if let Some(text) = task.result_text.as_deref() {
            println!("{text}");
        }
        if let Some(error_text) = task.error_text.as_deref() {
            eprintln!("error: {error_text}");
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

fn automation_runs_request_payload(
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
