use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

use super::*;

#[derive(Default)]
struct CodingReportSignals {
    changed_files: BTreeSet<String>,
    commands: BTreeSet<String>,
    verification_commands: BTreeSet<String>,
    tests: BTreeSet<String>,
    verification_failure_kinds: BTreeSet<String>,
    checkpoint_kinds: BTreeSet<String>,
    checkpoint_refs: BTreeSet<String>,
    completed_side_effect_refs: BTreeSet<String>,
    resume_entrypoints: BTreeSet<String>,
    diff_summaries: Vec<Value>,
    failures: Vec<Value>,
    retry_count: u64,
}

pub(super) fn coding_report_json(data: &Value) -> Value {
    let scanned = coding_report_json_from_scan(data);
    coding_report_json_from_workflow(data, &scanned).unwrap_or(scanned)
}

fn coding_report_json_from_scan(data: &Value) -> Value {
    let mut signals = CodingReportSignals::default();
    collect_coding_report_signals(data, &mut signals, 0);
    let state = coding_state_json(&signals);
    let verification_status = coding_verification_status_from_signals(&signals);
    let validation_gate = coding_validation_gate_json_from_signals(&signals, verification_status);
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
        "state": state,
        "validation_gate": validation_gate,
        "diff_summary_count": signals.diff_summaries.len(),
        "diff_summaries": signals.diff_summaries,
        "failure_count": signals.failures.len(),
        "failures": signals.failures,
        "retry_count": signals.retry_count,
        "unverified_risk": unverified_risk,
    })
}

fn coding_report_json_from_workflow(data: &Value, scanned: &Value) -> Option<Value> {
    let workflow = data
        .pointer("/result_json/task_journal/summary/coding_workflow")
        .or_else(|| data.pointer("/task_journal/summary/coding_workflow"))?
        .as_object()?;
    let workflow_value = Value::Object(workflow.clone());
    let changed_file_count = report_u64(&workflow_value, "/changed_file_count");
    let verification_command_count = report_u64(&workflow_value, "/verification_command_count");
    let failure_kind_count = report_u64(&workflow_value, "/failure_kind_count");
    let repair_attempt_count = report_u64(&workflow_value, "/repair_attempt_count");
    let checkpoint_ref_count = report_u64(&workflow_value, "/checkpoint_ref_count");
    let completed_side_effect_count = report_u64(&workflow_value, "/completed_side_effect_count");
    let verification_status = workflow_value
        .get("verification_status")
        .and_then(Value::as_str)
        .unwrap_or("not_applicable");
    let unverified_risk = if report_string_array(&workflow_value, "/remaining_risks")
        .iter()
        .any(|risk| risk == "unverified_changes")
    {
        Value::String("unverified_changes".to_string())
    } else {
        Value::Null
    };
    Some(json!({
        "schema_version": 1,
        "source": "task_journal_coding_workflow",
        "planned_change_count": report_u64(&workflow_value, "/planned_change_count"),
        "planned_changes": report_value_or_empty_array(&workflow_value, "/planned_changes"),
        "diff_ref_count": report_u64(&workflow_value, "/diff_ref_count"),
        "diff_refs": report_value_or_empty_array(&workflow_value, "/diff_refs"),
        "changed_file_count": changed_file_count,
        "changed_files": report_value_or_empty_array(&workflow_value, "/changed_files"),
        "command_count": report_u64(scanned, "/command_count"),
        "commands": report_value_or_empty_array(scanned, "/commands"),
        "verification_command_count": verification_command_count,
        "verification_commands": report_value_or_empty_array(&workflow_value, "/verification_commands"),
        "test_count": report_u64(scanned, "/test_count"),
        "tests": report_value_or_empty_array(scanned, "/tests"),
        "verification_failure_kind_count": failure_kind_count,
        "verification_failure_kinds": report_value_or_empty_array(&workflow_value, "/failure_kinds"),
        "state": {
            "schema_version": 1,
            "current_phase_hint": workflow_value.get("current_phase_hint").cloned().unwrap_or(Value::Null),
            "next_step": workflow_value.get("next_step").cloned().unwrap_or(Value::Null),
            "has_changes": changed_file_count > 0,
            "has_commands": report_u64(scanned, "/command_count") > 0,
            "has_verification": verification_command_count > 0,
            "has_tests": report_u64(scanned, "/test_count") > 0,
            "has_failed_step": failure_kind_count > 0 || verification_status == "failed",
            "has_failed_verification": failure_kind_count > 0 || verification_status == "failed",
            "repair_observed": repair_attempt_count > 0,
            "checkpointed": checkpoint_ref_count > 0,
            "resumable": report_u64(scanned, "/state/resume_entrypoint_count") > 0,
            "requires_idempotency_guard": completed_side_effect_count > 0,
            "checkpoint_kind_count": report_u64(scanned, "/state/checkpoint_kind_count"),
            "checkpoint_kinds": report_value_or_empty_array(scanned, "/state/checkpoint_kinds"),
            "checkpoint_ref_count": checkpoint_ref_count,
            "checkpoint_refs": report_value_or_empty_array(&workflow_value, "/checkpoint_refs"),
            "completed_side_effect_count": completed_side_effect_count,
            "completed_side_effect_refs": report_value_or_empty_array(&workflow_value, "/completed_side_effect_refs"),
            "resume_entrypoint_count": report_u64(scanned, "/state/resume_entrypoint_count"),
            "resume_entrypoints": report_value_or_empty_array(scanned, "/state/resume_entrypoints"),
            "verification_status": verification_status,
            "can_report_fully_verified": workflow_value
                .pointer("/validation_gate/can_report_fully_verified")
                .cloned()
                .unwrap_or(Value::Null),
        },
        "validation_gate": workflow_value
            .get("validation_gate")
            .cloned()
            .unwrap_or_else(|| coding_validation_gate_json_from_report(
                changed_file_count,
                verification_status,
                failure_kind_count,
                checkpoint_ref_count,
                completed_side_effect_count,
            )),
        "diff_summary_count": report_u64(scanned, "/diff_summary_count"),
        "diff_summaries": report_value_or_empty_array(scanned, "/diff_summaries"),
        "failure_count": if verification_status == "failed" && failure_kind_count == 0 { 1 } else { failure_kind_count },
        "failures": report_value_or_empty_array(scanned, "/failures"),
        "retry_count": repair_attempt_count,
        "repair_attempt_refs": report_value_or_empty_array(&workflow_value, "/repair_attempt_refs"),
        "remaining_risks": report_value_or_empty_array(&workflow_value, "/remaining_risks"),
        "done_condition_coverage": report_value_or_empty_array(&workflow_value, "/done_condition_coverage"),
        "unverified_risk": unverified_risk,
    }))
}

fn coding_state_json(signals: &CodingReportSignals) -> Value {
    let observed_phases = coding_observed_phases(signals);
    json!({
        "schema_version": 1,
        "current_phase_hint": coding_current_phase_hint(signals),
        "next_step": coding_next_step(signals),
        "observed_phases": observed_phases,
        "has_changes": !signals.changed_files.is_empty(),
        "has_commands": !signals.commands.is_empty(),
        "has_verification": !signals.verification_commands.is_empty(),
        "has_tests": !signals.tests.is_empty(),
        "has_failed_step": !signals.failures.is_empty(),
        "has_failed_verification": !signals.verification_failure_kinds.is_empty(),
        "repair_observed": signals.retry_count > 0
            || (!signals.failures.is_empty() && !signals.verification_commands.is_empty()),
        "checkpointed": !signals.checkpoint_refs.is_empty() || !signals.checkpoint_kinds.is_empty(),
        "resumable": !signals.resume_entrypoints.is_empty() || !signals.completed_side_effect_refs.is_empty(),
        "requires_idempotency_guard": !signals.completed_side_effect_refs.is_empty(),
        "checkpoint_kind_count": signals.checkpoint_kinds.len(),
        "checkpoint_kinds": signals.checkpoint_kinds.iter().cloned().collect::<Vec<_>>(),
        "checkpoint_ref_count": signals.checkpoint_refs.len(),
        "checkpoint_refs": signals.checkpoint_refs.iter().cloned().collect::<Vec<_>>(),
        "completed_side_effect_count": signals.completed_side_effect_refs.len(),
        "completed_side_effect_refs": signals
            .completed_side_effect_refs
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        "resume_entrypoint_count": signals.resume_entrypoints.len(),
        "resume_entrypoints": signals.resume_entrypoints.iter().cloned().collect::<Vec<_>>(),
        "verification_status": coding_verification_status_from_signals(signals),
    })
}

fn coding_validation_gate_json_from_signals(
    signals: &CodingReportSignals,
    verification_status: &str,
) -> Value {
    coding_validation_gate_json_from_report(
        signals.changed_files.len() as u64,
        verification_status,
        signals.verification_failure_kinds.len() as u64,
        signals.checkpoint_refs.len() as u64,
        signals.completed_side_effect_refs.len() as u64,
    )
}

fn coding_validation_gate_json_from_report(
    changed_file_count: u64,
    verification_status: &str,
    failure_kind_count: u64,
    checkpoint_ref_count: u64,
    completed_side_effect_count: u64,
) -> Value {
    let can_report_fully_verified = verification_status != "failed"
        && (changed_file_count == 0 || verification_status == "verified");
    let gate_status = if verification_status == "failed" {
        "repair_required"
    } else if changed_file_count > 0 && verification_status != "verified" {
        "verification_required"
    } else {
        "satisfied"
    };
    let repair_signal = if verification_status == "failed" {
        json!({
            "signal_kind": "verification_failed",
            "next_step": "repair_failed_verification",
            "failure_kind_count": failure_kind_count,
        })
    } else {
        Value::Null
    };
    json!({
        "schema_version": 1,
        "gate_status": gate_status,
        "can_report_fully_verified": can_report_fully_verified,
        "requires_verification": changed_file_count > 0 && verification_status != "verified",
        "requires_repair": verification_status == "failed",
        "checkpoint_recommended": !can_report_fully_verified
            && (checkpoint_ref_count > 0 || completed_side_effect_count > 0),
        "repair_signal": repair_signal,
    })
}

fn coding_next_step(signals: &CodingReportSignals) -> &'static str {
    if !signals.verification_failure_kinds.is_empty() || !signals.failures.is_empty() {
        "repair_failed_verification"
    } else if !signals.changed_files.is_empty() && signals.verification_commands.is_empty() {
        "run_verification"
    } else if !signals.resume_entrypoints.is_empty() {
        "resume_from_checkpoint"
    } else if !signals.verification_commands.is_empty() {
        "summarize"
    } else if !signals.commands.is_empty() {
        "inspect_results"
    } else {
        "inspect"
    }
}

fn coding_observed_phases(signals: &CodingReportSignals) -> Vec<&'static str> {
    let mut phases = Vec::new();
    if !signals.commands.is_empty() || !signals.changed_files.is_empty() {
        phases.push("inspect_or_plan");
    }
    if !signals.changed_files.is_empty() {
        phases.push("edit");
    }
    if !signals.verification_commands.is_empty() {
        phases.push("verify");
    }
    if signals.retry_count > 0 || !signals.verification_failure_kinds.is_empty() {
        phases.push("repair");
    }
    if !signals.checkpoint_refs.is_empty() || !signals.checkpoint_kinds.is_empty() {
        phases.push("checkpoint");
    }
    if !signals.changed_files.is_empty() || !signals.verification_commands.is_empty() {
        phases.push("summarize");
    }
    phases
}

fn coding_current_phase_hint(signals: &CodingReportSignals) -> &'static str {
    if signals.retry_count > 0 || !signals.verification_failure_kinds.is_empty() {
        "repair"
    } else if !signals.resume_entrypoints.is_empty() {
        "background"
    } else if !signals.verification_commands.is_empty() {
        "summarize"
    } else if !signals.changed_files.is_empty() {
        "verify"
    } else if !signals.commands.is_empty() {
        "inspect"
    } else {
        "idle"
    }
}

fn coding_verification_status_from_signals(signals: &CodingReportSignals) -> &'static str {
    if !signals.failures.is_empty() {
        "failed"
    } else if !signals.verification_commands.is_empty() {
        "verified"
    } else if !signals.changed_files.is_empty() {
        "unverified"
    } else {
        "not_applicable"
    }
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
            collect_coding_checkpoint_fields(map, signals);
            for value in map.values() {
                collect_coding_report_signals(value, signals, depth + 1);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_coding_report_signals(item, signals, depth + 1);
            }
        }
        Value::String(text) => {
            if let Ok(value) = serde_json::from_str::<Value>(text.trim()) {
                collect_coding_report_signals(&value, signals, depth + 1);
            } else {
                collect_command_from_machine_excerpt(text, signals);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn collect_coding_checkpoint_fields(map: &Map<String, Value>, signals: &mut CodingReportSignals) {
    collect_string_tokens(map.get("checkpoint_kind"), &mut signals.checkpoint_kinds);
    collect_string_tokens(map.get("checkpoint_ref"), &mut signals.checkpoint_refs);
    collect_string_tokens(map.get("evidence_ref"), &mut signals.checkpoint_refs);
    collect_string_tokens(
        map.get("completed_side_effect_refs"),
        &mut signals.completed_side_effect_refs,
    );
    collect_string_tokens(
        map.get("resume_entrypoint"),
        &mut signals.resume_entrypoints,
    );
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
            "normalized": normalized_diff_summary_json(&value, map),
        }));
        if signals.diff_summaries.len() >= 16 {
            return;
        }
    }
}

fn normalized_diff_summary_json(value: &Value, parent: &Map<String, Value>) -> Value {
    let file_path = first_string_from_value(value, &["file_path", "path", "file", "resolved_path"])
        .or_else(|| first_path_from_parent(parent));
    let change_kind = first_string_from_value(value, &["change_kind", "kind", "status"])
        .or_else(|| file_path.as_ref().map(|_| "modified".to_string()));
    let bounded_hunk_summary = first_string_from_value(
        value,
        &[
            "bounded_hunk_summary",
            "hunk_summary",
            "summary",
            "summary_code",
            "description",
        ],
    )
    .or_else(|| match value {
        Value::String(text) => bounded_text(text, 500),
        _ => None,
    });
    json!({
        "schema_version": 1,
        "file_path": file_path,
        "change_kind": change_kind,
        "bounded_hunk_summary": bounded_hunk_summary,
        "verification_evidence_refs": first_string_array_from_value(value, &[
            "verification_evidence_refs",
            "verification_refs",
            "evidence_refs",
        ]),
        "rollback_refs": first_string_array_from_value(value, &[
            "rollback_refs",
            "side_effect_refs",
            "completed_side_effect_refs",
        ]),
    })
}

fn first_path_from_parent(parent: &Map<String, Value>) -> Option<String> {
    for key in [
        "changed_files",
        "files_changed",
        "modified_files",
        "created_files",
        "deleted_files",
        "touched_files",
    ] {
        let mut paths = BTreeSet::new();
        collect_path_tokens(parent.get(key), &mut paths);
        if let Some(path) = paths.into_iter().next() {
            return Some(path);
        }
    }
    None
}

fn first_string_from_value(value: &Value, keys: &[&str]) -> Option<String> {
    let map = value.as_object()?;
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(Value::as_str)
            .and_then(|text| bounded_text(text, 500))
    })
}

fn first_string_array_from_value(value: &Value, keys: &[&str]) -> Vec<String> {
    let Some(map) = value.as_object() else {
        return Vec::new();
    };
    for key in keys {
        let mut out = BTreeSet::new();
        collect_string_tokens(map.get(*key), &mut out);
        if !out.is_empty() {
            return out.into_iter().take(16).collect();
        }
    }
    Vec::new()
}

fn bounded_text(text: &str, max_chars: usize) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(max_chars).collect())
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
    if let Some(command) = map
        .get("args")
        .and_then(Value::as_object)
        .and_then(|args| args.get("command"))
        .and_then(Value::as_str)
    {
        collect_command_token(command, signals);
    }
    if let Some(command) = map.get("verification_command").and_then(Value::as_str) {
        collect_command_token(command, signals);
    }
    if let Some(command) = map.get("test_command").and_then(Value::as_str) {
        collect_command_token(command, signals);
    }
    if let Some(summary) = map.get("sanitized_args_summary").and_then(Value::as_str) {
        if let Some(command) = summary.trim().strip_prefix("command=") {
            collect_command_token(command, signals);
        }
    }
    if let Some(output_excerpt) = map.get("output_excerpt").and_then(Value::as_str) {
        collect_command_from_machine_excerpt(output_excerpt, signals);
    }
}

fn collect_command_from_machine_excerpt(excerpt: &str, signals: &mut CodingReportSignals) {
    let excerpt = excerpt.trim();
    if excerpt.chars().any(|ch| matches!(ch, '\n' | '\r')) {
        return;
    }
    let Some(index) = excerpt.find("command=") else {
        return;
    };
    collect_command_token(&excerpt[index + "command=".len()..], signals);
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
        || is_python_test_command_token(&command)
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
        || is_python_test_command_token(&command)
        || command.starts_with("ruff check")
        || command.starts_with("go vet")
        || command.starts_with("go test")
}

fn is_python_test_command_token(command: &str) -> bool {
    let mut parts = command.split_whitespace();
    let Some(program) = parts.next() else {
        return false;
    };
    if !matches!(program, "python" | "python3") {
        return false;
    }
    let args = parts.collect::<Vec<_>>();
    if args.starts_with(&["-m", "pytest"]) || args.starts_with(&["-m", "unittest"]) {
        return true;
    }
    args.iter().any(|arg| {
        let arg = arg.trim_matches('"').trim_matches('\'');
        arg.starts_with("test_") || arg.ends_with("_test.py") || arg.contains("/test_")
    })
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
