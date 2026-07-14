use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::{events::EventFilters, output, task};

use super::report::{
    async_final_result_json, coding_diff_summary_artifact_json, coding_exec_has_signals,
    coding_exec_summary_json, coding_exec_text_lines, coding_verification_artifact_json,
    exec_artifact_refs, llm_report_json,
};
use super::report_budget_health::llm_budget_text_lines;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExecWaitOutcome {
    Terminal,
    Background,
    Timeout,
}

impl ExecWaitOutcome {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Background => "background",
            Self::Timeout => "timeout",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExecExitClass {
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

    pub(super) fn code(self) -> u8 {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExecEffectiveOptions {
    pub(super) profile: Option<String>,
    pub(super) detach: bool,
    pub(super) json_output: bool,
    pub(super) jsonl_output: bool,
    pub(super) timeout_seconds: Option<u64>,
    pub(super) interval_ms: u64,
    pub(super) continue_on_background: bool,
    pub(super) fail_on_background: bool,
    pub(super) artifact_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct ExecProfileDefaults {
    timeout_seconds: Option<u64>,
    interval_ms: u64,
    continue_on_background: bool,
    fail_on_background: bool,
    artifact_dir: Option<PathBuf>,
}

pub(super) fn exec_effective_options(
    profile_name: Option<&str>,
    detach: bool,
    json_output: bool,
    jsonl_output: bool,
    timeout_seconds: Option<u64>,
    interval_ms: u64,
    continue_on_background: bool,
    fail_on_background: bool,
    artifact_dir: Option<&PathBuf>,
) -> Result<ExecEffectiveOptions> {
    let profile = profile_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let defaults = match profile.as_deref() {
        None => ExecProfileDefaults::default(),
        Some("quick") => ExecProfileDefaults {
            timeout_seconds: Some(120),
            ..ExecProfileDefaults::default()
        },
        Some("coding") => ExecProfileDefaults {
            timeout_seconds: Some(900),
            artifact_dir: Some(PathBuf::from("artifacts/rustclaw-exec/coding")),
            ..ExecProfileDefaults::default()
        },
        Some("release-gate") => ExecProfileDefaults {
            timeout_seconds: Some(600),
            fail_on_background: true,
            artifact_dir: Some(PathBuf::from("artifacts/rustclaw-exec/release-gate")),
            ..ExecProfileDefaults::default()
        },
        Some("long-tail") => ExecProfileDefaults {
            timeout_seconds: Some(3600),
            continue_on_background: true,
            artifact_dir: Some(PathBuf::from("artifacts/rustclaw-exec/long-tail")),
            ..ExecProfileDefaults::default()
        },
        Some(other) => anyhow::bail!("exec_profile_unknown:{other}"),
    };

    Ok(ExecEffectiveOptions {
        profile,
        detach,
        json_output,
        jsonl_output,
        timeout_seconds: timeout_seconds.or(defaults.timeout_seconds),
        interval_ms: interval_ms.max(defaults.interval_ms).max(100),
        continue_on_background: continue_on_background || defaults.continue_on_background,
        fail_on_background: fail_on_background || defaults.fail_on_background,
        artifact_dir: artifact_dir.cloned().or(defaults.artifact_dir),
    })
}

impl Default for ExecProfileDefaults {
    fn default() -> Self {
        Self {
            timeout_seconds: None,
            interval_ms: 1000,
            continue_on_background: false,
            fail_on_background: false,
            artifact_dir: None,
        }
    }
}

fn exec_effective_config_json(options: &ExecEffectiveOptions) -> Value {
    json!({
        "schema_version": 1,
        "profile": options.profile,
        "detach": options.detach,
        "json": options.json_output,
        "jsonl": options.jsonl_output,
        "timeout_seconds": options.timeout_seconds,
        "poll_interval_ms": options.interval_ms,
        "continue_on_background": options.continue_on_background,
        "fail_on_background": options.fail_on_background,
        "artifact_dir": options
            .artifact_dir
            .as_ref()
            .map(|path| path.display().to_string()),
    })
}

pub(super) fn exec_summary_json(
    task: &task::TaskStatusView,
    outcome: ExecWaitOutcome,
    exit_class: ExecExitClass,
    resume_task_id: Option<&str>,
) -> serde_json::Value {
    let artifact_refs = exec_artifact_refs(&task.raw_data);
    let coding = coding_exec_summary_json(task);
    let llm = llm_report_json(task);
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
        "llm": llm,
        "coding": coding,
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
    profile_name: Option<&str>,
    resume_task_id: Option<&str>,
    detach: bool,
    json_output: bool,
    jsonl_output: bool,
    timeout_seconds: Option<u64>,
    interval_ms: u64,
    continue_on_background: bool,
    fail_on_background: bool,
    artifact_dir: Option<&PathBuf>,
    print_effective_config: bool,
) -> Result<u8> {
    let effective = match exec_effective_options(
        profile_name,
        detach,
        json_output,
        jsonl_output,
        timeout_seconds,
        interval_ms,
        continue_on_background,
        fail_on_background,
        artifact_dir,
    ) {
        Ok(options) => options,
        Err(error) => {
            let exit_class = ExecExitClass::InvalidRequest;
            let summary = json!({
                "exit_class": exit_class.as_str(),
                "exit_code": exit_class.code(),
                "error_code": "exec_profile_invalid",
                "error_detail": error.to_string(),
                "resume": exec_resume_summary(resume_task_id),
            });
            if json_output || jsonl_output {
                output::print_json_pretty(&summary);
            } else {
                eprintln!("error_code=exec_profile_invalid");
            }
            return Ok(exit_class.code());
        }
    };

    if print_effective_config {
        output::print_json_pretty(&exec_effective_config_json(&effective));
        return Ok(ExecExitClass::Success.code());
    }

    if effective.continue_on_background && effective.fail_on_background {
        let exit_class = ExecExitClass::InvalidRequest;
        let summary = json!({
            "exit_class": exit_class.as_str(),
            "exit_code": exit_class.code(),
            "error_code": "exec_background_policy_conflict",
            "resume": exec_resume_summary(resume_task_id),
            "effective_config": exec_effective_config_json(&effective),
        });
        if effective.json_output || effective.jsonl_output {
            output::print_json_pretty(&summary);
        } else {
            eprintln!("error_code=exec_background_policy_conflict");
        }
        if let Some(artifact_dir) = effective.artifact_dir.as_deref() {
            write_exec_detached_artifacts(artifact_dir, &summary)?;
        }
        return Ok(exit_class.code());
    }
    let task_id = if let Some(resume_task_id) = resume_task_id {
        task::submit_resume_ask(base_url, key, resume_task_id, prompt)?
    } else {
        task::submit_ask(base_url, key, prompt)?
    };
    if effective.detach {
        let exit_class = ExecExitClass::Success;
        let summary = json!({
            "task_id": task_id,
            "detached": true,
            "exit_class": exit_class.as_str(),
            "exit_code": exit_class.code(),
            "resume": exec_resume_summary(resume_task_id),
            "effective_config": exec_effective_config_json(&effective),
        });
        if effective.json_output || effective.jsonl_output {
            output::print_json_pretty(&summary);
        } else {
            println!("task_id: {}", task_id);
        }
        if let Some(artifact_dir) = effective.artifact_dir.as_deref() {
            write_exec_detached_artifacts(artifact_dir, &summary)?;
        }
        return Ok(exit_class.code());
    }

    let (task, outcome) = wait_for_exec_task(
        base_url,
        key,
        &task_id,
        ExecWaitOptions {
            interval_ms: effective.interval_ms,
            timeout_seconds: effective.timeout_seconds,
            continue_on_background: effective.continue_on_background,
            fail_on_background: effective.fail_on_background,
            jsonl_output: effective.jsonl_output,
        },
    )?;
    let exit_class = exec_exit_class(&task, outcome, effective.fail_on_background);
    let mut summary = exec_summary_json(&task, outcome, exit_class, resume_task_id);
    let artifact_index = effective.artifact_dir.as_deref().map(|artifact_dir| {
        exec_artifact_index_json(&summary, artifact_dir, exec_artifact_index_file_set())
    });
    if let Some(map) = summary.as_object_mut() {
        map.insert(
            "effective_config".to_string(),
            exec_effective_config_json(&effective),
        );
        map.insert("resume_hint".to_string(), exec_resume_artifact_json(&task));
        if let Some(artifact_index) = artifact_index {
            map.insert("artifact_index".to_string(), artifact_index);
        }
    }
    if let Some(artifact_dir) = effective.artifact_dir.as_deref() {
        write_exec_artifacts(artifact_dir, &task, &summary)?;
    }
    if effective.json_output || effective.jsonl_output {
        output::print_json_pretty(&summary);
    } else {
        output::print_task_status(&task, false, &EventFilters::default());
        for line in exec_compact_text_lines(&summary) {
            println!("{line}");
        }
        let llm = llm_report_json(&task);
        for line in llm_budget_text_lines(&llm) {
            println!("{line}");
        }
        let coding = coding_exec_summary_json(&task);
        if coding_exec_has_signals(&coding) {
            for line in coding_exec_text_lines(&coding) {
                println!("{line}");
            }
        }
        println!("exec_outcome: {}", outcome.as_str());
        println!("exec_exit_class: {}", exit_class.as_str());
        println!("exec_exit_code: {}", exit_class.code());
    }
    Ok(exit_class.code())
}

pub(super) fn exec_compact_text_lines(summary: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    push_summary_machine_line(
        &mut lines,
        "exec_compact_profile",
        summary,
        "/effective_config/profile",
    );
    push_summary_machine_line(&mut lines, "exec_compact_task_id", summary, "/task_id");
    push_summary_machine_line(&mut lines, "exec_compact_status", summary, "/status");
    push_summary_machine_line(
        &mut lines,
        "exec_compact_lifecycle_state",
        summary,
        "/lifecycle_state",
    );
    push_summary_machine_line(&mut lines, "exec_compact_outcome", summary, "/outcome");
    push_summary_machine_line(
        &mut lines,
        "exec_compact_exit_class",
        summary,
        "/exit_class",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_budget_status",
        summary,
        "/llm/budget_health/status",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_resume_mode",
        summary,
        "/resume/mode",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_checkpoint_id",
        summary,
        "/resume_hint/checkpoint_id",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_resume_due",
        summary,
        "/resume_hint/resume_due",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_changed_file_count",
        summary,
        "/coding/changed_file_count",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_verification_command_count",
        summary,
        "/coding/verification_command_count",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_verification_status",
        summary,
        "/coding/state/verification_status",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_next_step",
        summary,
        "/coding/state/next_step",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_checkpoint_ref_count",
        summary,
        "/coding/state/checkpoint_ref_count",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_completed_side_effect_count",
        summary,
        "/coding/state/completed_side_effect_count",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_unverified_risk",
        summary,
        "/coding/unverified_risk",
    );
    push_summary_machine_line(
        &mut lines,
        "exec_compact_artifact_index",
        summary,
        "/artifact_index/path",
    );
    push_summary_array_lines(
        &mut lines,
        "exec_compact_changed_file",
        summary,
        "/coding/changed_files",
        8,
    );
    push_summary_array_lines(
        &mut lines,
        "exec_compact_verification_command",
        summary,
        "/coding/verification_commands",
        4,
    );
    lines
}

fn push_summary_machine_line(lines: &mut Vec<String>, key: &str, source: &Value, pointer: &str) {
    let Some(value) = source.pointer(pointer) else {
        return;
    };
    let text = match value {
        Value::String(value) => value.trim().to_string(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
    };
    if text.is_empty() {
        return;
    }
    push_exec_machine_line(lines, key, &text);
}

fn push_summary_array_lines(
    lines: &mut Vec<String>,
    key: &str,
    source: &Value,
    pointer: &str,
    limit: usize,
) {
    let Some(items) = source.pointer(pointer).and_then(Value::as_array) else {
        return;
    };
    for value in items.iter().filter_map(Value::as_str).take(limit) {
        let text = value.trim();
        if !text.is_empty() {
            push_exec_machine_line(lines, key, text);
        }
    }
}

fn push_exec_machine_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let mut line = String::with_capacity(key.len() + value.len() + 2);
    line.push_str(key);
    line.push(':');
    line.push(' ');
    line.push_str(value);
    lines.push(line);
}

pub(super) fn exec_exit_class(
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

pub(super) fn exec_failure_class_from_machine_tokens(task: &task::TaskStatusView) -> ExecExitClass {
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
    write_json_file(&artifact_dir.join("summary.json"), summary)?;
    write_json_file(
        &artifact_dir.join("index.json"),
        &exec_artifact_index_json(
            summary,
            artifact_dir,
            exec_detached_artifact_index_file_set(),
        ),
    )
}

pub(super) fn write_exec_artifacts(
    artifact_dir: &Path,
    task: &task::TaskStatusView,
    summary: &Value,
) -> Result<()> {
    fs::create_dir_all(artifact_dir)
        .with_context(|| format!("create artifact dir {}", artifact_dir.display()))?;
    write_json_file(&artifact_dir.join("summary.json"), summary)?;
    write_json_file(&artifact_dir.join("task.json"), &task.raw_data)?;
    write_json_file(
        &artifact_dir.join("verification.json"),
        &coding_verification_artifact_json(task),
    )?;
    write_json_file(
        &artifact_dir.join("diff_summary.json"),
        &coding_diff_summary_artifact_json(task),
    )?;
    write_json_file(
        &artifact_dir.join("llm_summary.json"),
        &llm_report_json(task),
    )?;
    write_json_file(
        &artifact_dir.join("resume.json"),
        &exec_resume_artifact_json(task),
    )?;
    write_json_file(
        &artifact_dir.join("index.json"),
        &exec_artifact_index_json(summary, artifact_dir, exec_artifact_index_file_set()),
    )?;
    let mut events = String::new();
    for event in &task.events {
        events.push_str(&event.line);
        events.push('\n');
    }
    fs::write(artifact_dir.join("events.jsonl"), events)
        .with_context(|| format!("write artifact dir {}", artifact_dir.display()))?;
    Ok(())
}

fn exec_artifact_index_file_set() -> &'static [(&'static str, &'static str)] {
    &[
        ("summary", "summary.json"),
        ("task", "task.json"),
        ("events", "events.jsonl"),
        ("verification", "verification.json"),
        ("diff_summary", "diff_summary.json"),
        ("llm_summary", "llm_summary.json"),
        ("resume", "resume.json"),
        ("index", "index.json"),
    ]
}

fn exec_detached_artifact_index_file_set() -> &'static [(&'static str, &'static str)] {
    &[("summary", "summary.json"), ("index", "index.json")]
}

pub(super) fn exec_artifact_index_json(
    summary: &Value,
    artifact_dir: &Path,
    files: &[(&str, &str)],
) -> Value {
    let entries = files
        .iter()
        .map(|(kind, path)| {
            json!({
                "kind": kind,
                "path": path,
                "absolute_path": artifact_dir.join(path).display().to_string(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "artifact_kind": "rustclaw_exec_artifact_index",
        "schema_version": 1,
        "task_id": summary.get("task_id").cloned().unwrap_or(Value::Null),
        "status": summary.get("status").cloned().unwrap_or(Value::Null),
        "outcome": summary.get("outcome").cloned().unwrap_or(Value::Null),
        "exit_class": summary.get("exit_class").cloned().unwrap_or(Value::Null),
        "path": "index.json",
        "absolute_path": artifact_dir.join("index.json").display().to_string(),
        "file_count": entries.len(),
        "files": entries,
    })
}

fn exec_resume_artifact_json(task: &task::TaskStatusView) -> Value {
    let lifecycle = task.lifecycle();
    let checkpoint_id = lifecycle_string(lifecycle, "checkpoint_id");
    let completed_side_effect_refs = exec_completed_side_effect_refs(&task.raw_data);
    let completed_side_effect_count = lifecycle_value(lifecycle, "completed_side_effect_count")
        .as_u64()
        .unwrap_or(
            completed_side_effect_refs
                .as_array()
                .map(|items| items.len() as u64)
                .unwrap_or(0),
        );
    let requires_idempotency_guard = lifecycle_bool(lifecycle, "requires_idempotency_guard")
        .unwrap_or(completed_side_effect_count > 0);
    let mut recommended_command_tokens = vec![
        "clawcli".to_string(),
        "watch".to_string(),
        task.task_id.clone(),
        "--until-terminal".to_string(),
    ];
    if let Some(checkpoint_id) = checkpoint_id.as_deref() {
        recommended_command_tokens.extend([
            "clawcli".to_string(),
            "resume-task".to_string(),
            task.task_id.clone(),
            "--checkpoint-id".to_string(),
            checkpoint_id.to_string(),
        ]);
    }
    json!({
        "schema_version": 1,
        "task_id": task.task_id,
        "status": task.status,
        "execution_state": task.execution_state(),
        "lifecycle_state": task.lifecycle_state(),
        "checkpoint_id": checkpoint_id,
        "resume_due": lifecycle_bool(lifecycle, "resume_due"),
        "next_poll_after": lifecycle_value(lifecycle, "next_poll_after"),
        "poll_after_seconds": lifecycle_value(lifecycle, "poll_after_seconds"),
        "poll_ref": lifecycle_string(lifecycle, "poll_ref"),
        "cancel_ref": lifecycle_string(lifecycle, "cancel_ref"),
        "recommended_command_tokens": recommended_command_tokens,
        "completed_side_effect_count": completed_side_effect_count,
        "completed_side_effect_refs": completed_side_effect_refs,
        "requires_idempotency_guard": requires_idempotency_guard,
        "coding": coding_exec_summary_json(task),
    })
}

fn exec_completed_side_effect_refs(data: &Value) -> Value {
    let refs = exec_task_checkpoint_value(data)
        .and_then(|checkpoint| checkpoint.get("completed_side_effect_refs"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| is_resume_ref_token(value))
        .take(128)
        .map(|value| Value::String(value.to_string()))
        .collect::<Vec<_>>();
    Value::Array(refs)
}

fn exec_task_checkpoint_value(data: &Value) -> Option<&Value> {
    data.pointer("/result_json/task_checkpoint")
        .or_else(|| data.pointer("/result_json/task_journal/summary/task_checkpoint"))
        .or_else(|| data.get("task_checkpoint"))
        .or_else(|| data.pointer("/task_journal/summary/task_checkpoint"))
        .filter(|value| value.is_object())
}

fn is_resume_ref_token(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 300
        && !trimmed.chars().any(|ch| matches!(ch, '\n' | '\r'))
}

fn lifecycle_value(lifecycle: Option<&Value>, key: &str) -> Value {
    lifecycle
        .and_then(|value| value.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}

fn lifecycle_string(lifecycle: Option<&Value>, key: &str) -> Option<String> {
    lifecycle
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn lifecycle_bool(lifecycle: Option<&Value>, key: &str) -> Option<bool> {
    lifecycle
        .and_then(|value| value.get(key))
        .and_then(Value::as_bool)
}

fn write_json_file(path: &Path, value: &Value) -> Result<()> {
    let body = serde_json::to_vec_pretty(value)?;
    fs::write(path, body).with_context(|| format!("write artifact {}", path.display()))
}
