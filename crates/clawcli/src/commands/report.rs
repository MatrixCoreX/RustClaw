use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

use crate::task;

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

pub(super) fn task_report_json(task: &task::TaskStatusView, include_events: bool) -> Value {
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

pub(super) fn task_report_text_lines(task: &task::TaskStatusView, report: &Value) -> Vec<String> {
    let mut lines = vec![
        format!("task_id: {}", task.task_id),
        format!("status: {}", task.status),
    ];
    if let Some(state) = task.execution_state() {
        lines.push(format!("execution_state: {state}"));
    }
    if let Some(state) = task.lifecycle_state() {
        lines.push(format!("lifecycle_state: {state}"));
    }
    lines.push(format!("terminal: {}", task.is_terminal()));
    lines.push(format!("event_count: {}", task.events.len()));
    lines.push(format!(
        "artifact_ref_count: {}",
        report_u64(report, "/artifacts/ref_count")
    ));

    let verification_status = coding_verification_status(report);
    lines.push(format!(
        "coding_changed_file_count: {}",
        report_u64(report, "/coding/changed_file_count")
    ));
    for path in report_string_array(report, "/coding/changed_files")
        .into_iter()
        .take(32)
    {
        lines.push(format!("changed_file: {path}"));
    }
    lines.push(format!(
        "coding_verification_command_count: {}",
        report_u64(report, "/coding/verification_command_count")
    ));
    for command in report_string_array(report, "/coding/verification_commands")
        .into_iter()
        .take(32)
    {
        lines.push(format!("verification_command: {command}"));
    }
    lines.push(format!(
        "coding_test_count: {}",
        report_u64(report, "/coding/test_count")
    ));
    lines.push(format!(
        "coding_failure_count: {}",
        report_u64(report, "/coding/failure_count")
    ));
    lines.push(format!("coding_verification_status: {verification_status}"));
    lines.push(format!(
        "coding_verification_failure_kind_count: {}",
        report_u64(report, "/coding/verification_failure_kind_count")
    ));
    for kind in report_string_array(report, "/coding/verification_failure_kinds")
        .into_iter()
        .take(16)
    {
        lines.push(format!("verification_failure_kind: {kind}"));
    }
    if let Some(unverified_risk) = report
        .pointer("/coding/unverified_risk")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("coding_unverified_risk: {unverified_risk}"));
    }
    if let Some(text) = task.result_text.as_deref() {
        lines.push(text.to_string());
    }
    lines
}

pub(super) fn coding_review_json(task: &task::TaskStatusView, include_events: bool) -> Value {
    let report = task_report_json(task, include_events);
    json!({
        "report_kind": "rustclaw_coding_review",
        "task_id": task.task_id,
        "status": task.status,
        "execution_state": task.execution_state(),
        "lifecycle_state": task.lifecycle_state(),
        "terminal": task.is_terminal(),
        "event_count": task.events.len(),
        "events": report.get("events").cloned().unwrap_or(Value::Null),
        "coding": report.get("coding").cloned().unwrap_or(Value::Null),
        "artifacts": report.get("artifacts").cloned().unwrap_or(Value::Null),
    })
}

pub(super) fn coding_review_text_lines(task: &task::TaskStatusView, review: &Value) -> Vec<String> {
    let mut lines = vec![
        format!("task_id: {}", task.task_id),
        format!("status: {}", task.status),
    ];
    if let Some(state) = task.execution_state() {
        lines.push(format!("execution_state: {state}"));
    }
    if let Some(state) = task.lifecycle_state() {
        lines.push(format!("lifecycle_state: {state}"));
    }
    lines.push(format!("terminal: {}", task.is_terminal()));
    lines.push(format!(
        "coding_changed_file_count: {}",
        report_u64(review, "/coding/changed_file_count")
    ));
    lines.push(format!(
        "coding_verification_command_count: {}",
        report_u64(review, "/coding/verification_command_count")
    ));
    lines.push(format!(
        "coding_test_count: {}",
        report_u64(review, "/coding/test_count")
    ));
    lines.push(format!(
        "coding_failure_count: {}",
        report_u64(review, "/coding/failure_count")
    ));
    lines.push(format!(
        "coding_verification_status: {}",
        coding_verification_status(review)
    ));
    for path in report_string_array(review, "/coding/changed_files")
        .into_iter()
        .take(32)
    {
        lines.push(format!("changed_file: {path}"));
    }
    for command in report_string_array(review, "/coding/verification_commands")
        .into_iter()
        .take(32)
    {
        lines.push(format!("verification_command: {command}"));
    }
    for kind in report_string_array(review, "/coding/verification_failure_kinds")
        .into_iter()
        .take(16)
    {
        lines.push(format!("verification_failure_kind: {kind}"));
    }
    if let Some(unverified_risk) = review
        .pointer("/coding/unverified_risk")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("coding_unverified_risk: {unverified_risk}"));
    }
    lines
}

pub(super) fn subagent_report_json(task: &task::TaskStatusView) -> Value {
    let mut signals = SubagentReportSignals::default();
    collect_subagent_report_signals(&task.raw_data, &mut signals, 0);
    for event in &task.events {
        collect_subagent_event_fields(event, &mut signals);
    }
    json!({
        "report_kind": "rustclaw_subagent_report",
        "task_id": task.task_id,
        "status": task.status,
        "execution_state": task.execution_state(),
        "lifecycle_state": task.lifecycle_state(),
        "subagent_count": signals.items.len(),
        "subagents": signals.items,
    })
}

pub(super) fn subagent_report_text_lines(report: &Value) -> Vec<String> {
    let mut lines = vec![
        format!(
            "task_id: {}",
            report.get("task_id").and_then(Value::as_str).unwrap_or("")
        ),
        format!(
            "status: {}",
            report.get("status").and_then(Value::as_str).unwrap_or("")
        ),
        format!(
            "subagent_count: {}",
            report
                .get("subagent_count")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        ),
    ];
    if let Some(items) = report.get("subagents").and_then(Value::as_array) {
        for item in items.iter().take(64) {
            let child_run_id = item
                .get("child_run_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let subagent_id = item
                .get("subagent_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let status = item.get("status").and_then(Value::as_str).unwrap_or("");
            let finding_refs = report_string_array(item, "/finding_refs").join(",");
            let evidence_refs = report_string_array(item, "/evidence_refs").join(",");
            lines.push(format!(
                "subagent: child_run_id={child_run_id} subagent_id={subagent_id} status={status} finding_refs={finding_refs} evidence_refs={evidence_refs}"
            ));
        }
    }
    lines
}

fn report_u64(report: &Value, pointer: &str) -> u64 {
    report.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

fn report_string_array(report: &Value, pointer: &str) -> Vec<String> {
    report
        .pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn coding_verification_status(report: &Value) -> &'static str {
    let failure_count = report_u64(report, "/coding/failure_count");
    let verification_count = report_u64(report, "/coding/verification_command_count");
    let changed_file_count = report_u64(report, "/coding/changed_file_count");
    if failure_count > 0 {
        "failed"
    } else if verification_count > 0 {
        "verified"
    } else if changed_file_count > 0 {
        "unverified"
    } else {
        "not_applicable"
    }
}

pub(super) fn async_final_result_json(data: &Value) -> Option<Value> {
    data.get("result_json")
        .and_then(task::async_final_result_value)
        .cloned()
}

pub(super) fn exec_artifact_refs(data: &Value) -> Vec<Value> {
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
    verification_commands: BTreeSet<String>,
    tests: BTreeSet<String>,
    verification_failure_kinds: BTreeSet<String>,
    diff_summaries: Vec<Value>,
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
        "verification_command_count": signals.verification_commands.len(),
        "verification_commands": signals.verification_commands.into_iter().collect::<Vec<_>>(),
        "test_count": signals.tests.len(),
        "tests": signals.tests.into_iter().collect::<Vec<_>>(),
        "verification_failure_kind_count": signals.verification_failure_kinds.len(),
        "verification_failure_kinds": signals.verification_failure_kinds.into_iter().collect::<Vec<_>>(),
        "diff_summary_count": signals.diff_summaries.len(),
        "diff_summaries": signals.diff_summaries,
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
            collect_diff_summary_fields(map, signals);
            collect_verification_failure_kind_fields(map, signals);
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

fn collect_diff_summary_fields(map: &Map<String, Value>, signals: &mut CodingReportSignals) {
    if signals.diff_summaries.len() >= 16 {
        return;
    }
    for key in [
        "diff_summary",
        "final_diff_summary",
        "change_summary",
        "patch_summary",
        "git_diff_summary",
    ] {
        let Some(value) = map.get(key).and_then(bounded_diff_summary_value) else {
            continue;
        };
        signals.diff_summaries.push(json!({
            "field": key,
            "value": value,
        }));
        if signals.diff_summaries.len() >= 16 {
            return;
        }
    }
}

fn bounded_diff_summary_value(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.len() > 2_000 {
                None
            } else {
                Some(Value::String(trimmed.to_string()))
            }
        }
        Value::Object(_) | Value::Array(_) => serde_json::to_string(value)
            .ok()
            .filter(|serialized| serialized.len() <= 4_000)
            .map(|_| value.clone()),
        Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::Null => None,
    }
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
    if is_verification_command_token(command) {
        signals.verification_commands.insert(command.to_string());
    }
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

fn is_verification_command_token(command: &str) -> bool {
    let command = command.trim().to_ascii_lowercase();
    is_test_command_token(&command)
        || command.starts_with("cargo check")
        || command.starts_with("cargo clippy")
        || command.starts_with("cargo fmt")
        || command.starts_with("npm run lint")
        || command.starts_with("npm run build")
        || command.starts_with("pnpm lint")
        || command.starts_with("pnpm build")
        || command.starts_with("yarn lint")
        || command.starts_with("yarn build")
        || command.starts_with("pytest")
        || command.starts_with("ruff check")
        || command.starts_with("go vet")
        || command.starts_with("go test")
}

fn collect_verification_failure_kind_fields(
    map: &Map<String, Value>,
    signals: &mut CodingReportSignals,
) {
    collect_string_tokens(
        map.get("verification_failure_kinds"),
        &mut signals.verification_failure_kinds,
    );
    if let Some(command) = map.get("command").and_then(Value::as_str) {
        let Some(status) = map.get("status").and_then(Value::as_str) else {
            return;
        };
        if is_failure_status_token(status) {
            record_verification_failure_kind(command, signals);
        }
    }
}

fn collect_string_tokens(value: Option<&Value>, out: &mut BTreeSet<String>) {
    match value {
        Some(Value::String(token)) => {
            let token = token.trim();
            if is_report_machine_token(token) {
                out.insert(token.to_string());
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                collect_string_tokens(Some(item), out);
            }
        }
        Some(Value::Object(_) | Value::Null | Value::Bool(_) | Value::Number(_)) | None => {}
    }
}

fn record_verification_failure_kind(command: &str, signals: &mut CodingReportSignals) {
    let Some(kind) = verification_failure_kind_for_command(command) else {
        return;
    };
    signals.verification_failure_kinds.insert(kind.to_string());
}

fn verification_failure_kind_for_command(command: &str) -> Option<&'static str> {
    let command = command.trim().to_ascii_lowercase();
    if is_test_command_token(&command) {
        Some("test")
    } else if command.starts_with("cargo check") {
        Some("compile")
    } else if command.starts_with("cargo fmt") {
        Some("formatter")
    } else if command.starts_with("cargo clippy")
        || command.starts_with("npm run lint")
        || command.starts_with("pnpm lint")
        || command.starts_with("yarn lint")
        || command.starts_with("ruff check")
        || command.starts_with("go vet")
    {
        Some("lint")
    } else if command.starts_with("npm run build")
        || command.starts_with("pnpm build")
        || command.starts_with("yarn build")
    {
        Some("build")
    } else if is_verification_command_token(&command) {
        Some("other_verification")
    } else {
        None
    }
}

fn is_report_machine_token(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 120
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.' | '/'))
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

#[derive(Default)]
struct SubagentReportSignals {
    seen: BTreeSet<String>,
    items: Vec<Value>,
}

fn collect_subagent_report_signals(
    value: &Value,
    signals: &mut SubagentReportSignals,
    depth: usize,
) {
    if depth > 12 || signals.items.len() >= 128 {
        return;
    }
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(children)) = map.get("child_results") {
                for child in children {
                    collect_subagent_report_signals(child, signals, depth + 1);
                }
            }
            if is_subagent_object(map) {
                push_subagent_item(map, signals);
            }
            for value in map.values() {
                collect_subagent_report_signals(value, signals, depth + 1);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_subagent_report_signals(item, signals, depth + 1);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn collect_subagent_event_fields(
    event: &crate::events::TaskEventLine,
    signals: &mut SubagentReportSignals,
) {
    let mut map = Map::new();
    for key in [
        "child_run_id",
        "subagent_id",
        "status",
        "role",
        "request_ref",
        "error_code",
    ] {
        if let Some(value) = event.fields.get(key) {
            map.insert(key.to_string(), Value::String(value.clone()));
        }
    }
    if is_subagent_object(&map) {
        push_subagent_item(&map, signals);
    }
}

fn is_subagent_object(map: &Map<String, Value>) -> bool {
    map.get("child_run_id").is_some()
        || map.get("subagent_id").is_some()
        || map.get("subagent_results").is_some()
        || map.get("finding_refs").is_some()
}

fn push_subagent_item(map: &Map<String, Value>, signals: &mut SubagentReportSignals) {
    let child_run_id = machine_string_field(map, "child_run_id");
    let subagent_id = machine_string_field(map, "subagent_id");
    let request_ref = machine_string_field(map, "request_ref");
    let identity = child_run_id
        .as_deref()
        .or(subagent_id.as_deref())
        .or(request_ref.as_deref())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            serde_json::to_string(map)
                .unwrap_or_default()
                .chars()
                .take(160)
                .collect()
        });
    if !signals.seen.insert(identity) || signals.items.len() >= 128 {
        return;
    }
    signals.items.push(json!({
        "child_run_id": child_run_id,
        "subagent_id": subagent_id,
        "request_ref": request_ref,
        "role": machine_string_field(map, "role"),
        "status": machine_string_field(map, "status"),
        "error_code": machine_string_field(map, "error_code"),
        "failure_isolation": machine_string_field(map, "failure_isolation"),
        "required": map.get("required").and_then(Value::as_bool),
        "optional": map.get("optional").and_then(Value::as_bool),
        "finding_refs": machine_ref_array(map.get("finding_refs")),
        "evidence_refs": machine_ref_array(map.get("evidence_refs")),
    }));
}

fn machine_string_field(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| is_report_machine_token(value))
        .map(ToString::to_string)
}

fn machine_ref_array(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(value)) if is_report_machine_token(value) => vec![value.to_string()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| is_report_machine_token(value))
            .map(ToString::to_string)
            .collect(),
        Some(Value::Object(_) | Value::Null | Value::Bool(_) | Value::Number(_)) | None => {
            Vec::new()
        }
        Some(Value::String(_)) => Vec::new(),
    }
}
