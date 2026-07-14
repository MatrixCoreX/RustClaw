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
        "llm": llm_report_json(task),
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
    lines.push(format!(
        "llm_call_count: {}",
        report_u64(report, "/llm/llm_call_count")
    ));
    lines.push(format!(
        "llm_prompt_bucket_count: {}",
        report_u64(report, "/llm/prompt_bucket_count")
    ));
    lines.push(format!(
        "llm_prompt_truncation_count: {}",
        report_u64(report, "/llm/prompt_truncation_count")
    ));
    if let Some(bytes) = report
        .pointer("/llm/prompt_bytes_before_max")
        .and_then(Value::as_u64)
    {
        lines.push(format!("llm_prompt_bytes_before_max: {bytes}"));
    }
    for item in report
        .pointer("/llm/by_prompt")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(32)
    {
        let label = item
            .get("prompt_label")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let calls = item
            .get("llm_call_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let truncations = item
            .get("prompt_truncation_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let bytes_before = item
            .get("prompt_bytes_before_max")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        lines.push(format!(
            "llm_prompt: prompt_label={label} llm_call_count={calls} prompt_truncation_count={truncations} prompt_bytes_before_max={bytes_before}"
        ));
    }

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
    if let Some(phase) = report
        .pointer("/coding/state/current_phase_hint")
        .and_then(Value::as_str)
    {
        lines.push(format!("coding_current_phase_hint: {phase}"));
    }
    lines.push(format!(
        "coding_checkpoint_ref_count: {}",
        report_u64(report, "/coding/state/checkpoint_ref_count")
    ));
    lines.push(format!(
        "coding_completed_side_effect_count: {}",
        report_u64(report, "/coding/state/completed_side_effect_count")
    ));
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
    if let Some(phase) = review
        .pointer("/coding/state/current_phase_hint")
        .and_then(Value::as_str)
    {
        lines.push(format!("coding_current_phase_hint: {phase}"));
    }
    lines.push(format!(
        "coding_checkpoint_ref_count: {}",
        report_u64(review, "/coding/state/checkpoint_ref_count")
    ));
    lines.push(format!(
        "coding_completed_side_effect_count: {}",
        report_u64(review, "/coding/state/completed_side_effect_count")
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

pub(super) fn coding_exec_summary_json(task: &task::TaskStatusView) -> Value {
    coding_report_json(&task.raw_data)
}

pub(super) fn coding_exec_has_signals(coding: &Value) -> bool {
    [
        "/changed_file_count",
        "/command_count",
        "/verification_command_count",
        "/test_count",
        "/failure_count",
        "/diff_summary_count",
        "/retry_count",
    ]
    .iter()
    .any(|pointer| report_u64(coding, pointer) > 0)
}

pub(super) fn coding_exec_text_lines(coding: &Value) -> Vec<String> {
    let mut lines = vec![
        format!(
            "coding_changed_file_count: {}",
            report_u64(coding, "/changed_file_count")
        ),
        format!(
            "coding_verification_command_count: {}",
            report_u64(coding, "/verification_command_count")
        ),
        format!("coding_test_count: {}", report_u64(coding, "/test_count")),
        format!(
            "coding_failure_count: {}",
            report_u64(coding, "/failure_count")
        ),
        format!(
            "coding_verification_status: {}",
            coding_verification_status_for_coding(coding)
        ),
        format!(
            "coding_verification_failure_kind_count: {}",
            report_u64(coding, "/verification_failure_kind_count")
        ),
    ];
    if let Some(phase) = coding
        .pointer("/state/current_phase_hint")
        .and_then(Value::as_str)
    {
        lines.push(format!("coding_current_phase_hint: {phase}"));
    }
    lines.push(format!(
        "coding_checkpoint_ref_count: {}",
        report_u64(coding, "/state/checkpoint_ref_count")
    ));
    lines.push(format!(
        "coding_completed_side_effect_count: {}",
        report_u64(coding, "/state/completed_side_effect_count")
    ));
    for path in report_string_array(coding, "/changed_files")
        .into_iter()
        .take(32)
    {
        lines.push(format!("changed_file: {path}"));
    }
    for command in report_string_array(coding, "/verification_commands")
        .into_iter()
        .take(32)
    {
        lines.push(format!("verification_command: {command}"));
    }
    for kind in report_string_array(coding, "/verification_failure_kinds")
        .into_iter()
        .take(16)
    {
        lines.push(format!("verification_failure_kind: {kind}"));
    }
    if let Some(unverified_risk) = coding
        .pointer("/unverified_risk")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("coding_unverified_risk: {unverified_risk}"));
    }
    lines
}

pub(super) fn coding_verification_artifact_json(task: &task::TaskStatusView) -> Value {
    let report = task_report_json(task, false);
    json!({
        "schema_version": 1,
        "artifact_kind": "rustclaw_exec_verification",
        "task_id": task.task_id,
        "status": task.status,
        "execution_state": task.execution_state(),
        "lifecycle_state": task.lifecycle_state(),
        "verification_status": coding_verification_status(&report),
        "coding_state": report
            .pointer("/coding/state")
            .cloned()
            .unwrap_or(Value::Null),
        "verification_command_count": report_u64(&report, "/coding/verification_command_count"),
        "verification_commands": report_value_or_empty_array(&report, "/coding/verification_commands"),
        "test_count": report_u64(&report, "/coding/test_count"),
        "tests": report_value_or_empty_array(&report, "/coding/tests"),
        "failure_count": report_u64(&report, "/coding/failure_count"),
        "failures": report_value_or_empty_array(&report, "/coding/failures"),
        "verification_failure_kind_count": report_u64(
            &report,
            "/coding/verification_failure_kind_count"
        ),
        "verification_failure_kinds": report_value_or_empty_array(
            &report,
            "/coding/verification_failure_kinds"
        ),
        "unverified_risk": report
            .pointer("/coding/unverified_risk")
            .cloned()
            .unwrap_or(Value::Null),
    })
}

pub(super) fn coding_diff_summary_artifact_json(task: &task::TaskStatusView) -> Value {
    let report = task_report_json(task, false);
    json!({
        "schema_version": 1,
        "artifact_kind": "rustclaw_exec_diff_summary",
        "task_id": task.task_id,
        "status": task.status,
        "execution_state": task.execution_state(),
        "lifecycle_state": task.lifecycle_state(),
        "changed_file_count": report_u64(&report, "/coding/changed_file_count"),
        "changed_files": report_value_or_empty_array(&report, "/coding/changed_files"),
        "diff_summary_count": report_u64(&report, "/coding/diff_summary_count"),
        "diff_summaries": report_value_or_empty_array(&report, "/coding/diff_summaries"),
    })
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

fn report_value_or_empty_array(report: &Value, pointer: &str) -> Value {
    report
        .pointer(pointer)
        .filter(|value| value.is_array())
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

#[derive(Default)]
struct LlmReportSignals {
    by_prompt: std::collections::BTreeMap<String, LlmPromptSignals>,
}

#[derive(Default)]
struct LlmPromptSignals {
    llm_call_count: u64,
    elapsed_ms: u64,
    provider_attempt_count: u64,
    provider_retry_count: u64,
    provider_retryable_error_count: u64,
    provider_final_error_count: u64,
    prompt_truncation_count: u64,
    prompt_bytes_before_max: Option<u64>,
    prompt_bytes_budget_min: Option<u64>,
    prompt_bytes_after_max: Option<u64>,
    prompt_truncated_bytes_total: u64,
}

pub(super) fn llm_report_json(task: &task::TaskStatusView) -> Value {
    let mut signals = LlmReportSignals::default();
    for event in task
        .events
        .iter()
        .filter(|event| event.event_type == "provider_call")
    {
        let prompt_label = event
            .fields
            .get("prompt_label")
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| is_report_machine_token(value))
            .unwrap_or("unknown")
            .to_string();
        let bucket = signals.by_prompt.entry(prompt_label).or_default();
        bucket.llm_call_count = bucket
            .llm_call_count
            .saturating_add(event_field_u64(event, "llm_call_count"));
        bucket.elapsed_ms = bucket
            .elapsed_ms
            .saturating_add(event_field_u64(event, "elapsed_ms"));
        bucket.provider_attempt_count = bucket
            .provider_attempt_count
            .saturating_add(event_field_u64(event, "provider_attempt_count"));
        bucket.provider_retry_count = bucket
            .provider_retry_count
            .saturating_add(event_field_u64(event, "provider_retry_count"));
        bucket.provider_retryable_error_count = bucket
            .provider_retryable_error_count
            .saturating_add(event_field_u64(event, "provider_retryable_error_count"));
        bucket.provider_final_error_count = bucket
            .provider_final_error_count
            .saturating_add(event_field_u64(event, "provider_final_error_count"));
        bucket.prompt_truncation_count = bucket
            .prompt_truncation_count
            .saturating_add(event_field_u64(event, "prompt_truncation_count"));
        bucket.prompt_truncated_bytes_total = bucket
            .prompt_truncated_bytes_total
            .saturating_add(event_field_u64(event, "prompt_truncated_bytes_total"));
        merge_optional_max(
            &mut bucket.prompt_bytes_before_max,
            event_field_optional_u64(event, "prompt_bytes_before_max"),
        );
        merge_optional_min(
            &mut bucket.prompt_bytes_budget_min,
            event_field_optional_u64(event, "prompt_bytes_budget_min"),
        );
        merge_optional_max(
            &mut bucket.prompt_bytes_after_max,
            event_field_optional_u64(event, "prompt_bytes_after_max"),
        );
    }
    let mut total = LlmPromptSignals::default();
    let by_prompt = signals
        .by_prompt
        .iter()
        .map(|(prompt_label, bucket)| {
            accumulate_llm_prompt_signals(&mut total, bucket);
            json!({
                "prompt_label": prompt_label,
                "llm_call_count": bucket.llm_call_count,
                "elapsed_ms": bucket.elapsed_ms,
                "provider_attempt_count": bucket.provider_attempt_count,
                "provider_retry_count": bucket.provider_retry_count,
                "provider_retryable_error_count": bucket.provider_retryable_error_count,
                "provider_final_error_count": bucket.provider_final_error_count,
                "prompt_truncation_count": bucket.prompt_truncation_count,
                "prompt_bytes_before_max": bucket.prompt_bytes_before_max,
                "prompt_bytes_budget_min": bucket.prompt_bytes_budget_min,
                "prompt_bytes_after_max": bucket.prompt_bytes_after_max,
                "prompt_truncated_bytes_total": bucket.prompt_truncated_bytes_total,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "provider_call_event_count": task
            .events
            .iter()
            .filter(|event| event.event_type == "provider_call")
            .count(),
        "prompt_bucket_count": by_prompt.len(),
        "llm_call_count": total.llm_call_count,
        "elapsed_ms": total.elapsed_ms,
        "provider_attempt_count": total.provider_attempt_count,
        "provider_retry_count": total.provider_retry_count,
        "provider_retryable_error_count": total.provider_retryable_error_count,
        "provider_final_error_count": total.provider_final_error_count,
        "prompt_truncation_count": total.prompt_truncation_count,
        "prompt_bytes_before_max": total.prompt_bytes_before_max,
        "prompt_bytes_budget_min": total.prompt_bytes_budget_min,
        "prompt_bytes_after_max": total.prompt_bytes_after_max,
        "prompt_truncated_bytes_total": total.prompt_truncated_bytes_total,
        "by_prompt": by_prompt,
    })
}

fn event_field_u64(event: &crate::events::TaskEventLine, key: &str) -> u64 {
    event_field_optional_u64(event, key).unwrap_or(0)
}

fn event_field_optional_u64(event: &crate::events::TaskEventLine, key: &str) -> Option<u64> {
    event
        .fields
        .get(key)
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn merge_optional_max(slot: &mut Option<u64>, value: Option<u64>) {
    let Some(value) = value else {
        return;
    };
    *slot = Some(slot.map_or(value, |current| current.max(value)));
}

fn merge_optional_min(slot: &mut Option<u64>, value: Option<u64>) {
    let Some(value) = value else {
        return;
    };
    *slot = Some(slot.map_or(value, |current| current.min(value)));
}

fn accumulate_llm_prompt_signals(total: &mut LlmPromptSignals, bucket: &LlmPromptSignals) {
    total.llm_call_count = total.llm_call_count.saturating_add(bucket.llm_call_count);
    total.elapsed_ms = total.elapsed_ms.saturating_add(bucket.elapsed_ms);
    total.provider_attempt_count = total
        .provider_attempt_count
        .saturating_add(bucket.provider_attempt_count);
    total.provider_retry_count = total
        .provider_retry_count
        .saturating_add(bucket.provider_retry_count);
    total.provider_retryable_error_count = total
        .provider_retryable_error_count
        .saturating_add(bucket.provider_retryable_error_count);
    total.provider_final_error_count = total
        .provider_final_error_count
        .saturating_add(bucket.provider_final_error_count);
    total.prompt_truncation_count = total
        .prompt_truncation_count
        .saturating_add(bucket.prompt_truncation_count);
    total.prompt_truncated_bytes_total = total
        .prompt_truncated_bytes_total
        .saturating_add(bucket.prompt_truncated_bytes_total);
    merge_optional_max(
        &mut total.prompt_bytes_before_max,
        bucket.prompt_bytes_before_max,
    );
    merge_optional_min(
        &mut total.prompt_bytes_budget_min,
        bucket.prompt_bytes_budget_min,
    );
    merge_optional_max(
        &mut total.prompt_bytes_after_max,
        bucket.prompt_bytes_after_max,
    );
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

fn coding_verification_status_for_coding(coding: &Value) -> &'static str {
    let failure_count = report_u64(coding, "/failure_count");
    let verification_count = report_u64(coding, "/verification_command_count");
    let changed_file_count = report_u64(coding, "/changed_file_count");
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
    checkpoint_kinds: BTreeSet<String>,
    checkpoint_refs: BTreeSet<String>,
    completed_side_effect_refs: BTreeSet<String>,
    resume_entrypoints: BTreeSet<String>,
    diff_summaries: Vec<Value>,
    failures: Vec<Value>,
    retry_count: u64,
}

fn coding_report_json(data: &Value) -> Value {
    let mut signals = CodingReportSignals::default();
    collect_coding_report_signals(data, &mut signals, 0);
    let state = coding_state_json(&signals);
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
        "diff_summary_count": signals.diff_summaries.len(),
        "diff_summaries": signals.diff_summaries,
        "failure_count": signals.failures.len(),
        "failures": signals.failures,
        "retry_count": signals.retry_count,
        "unverified_risk": unverified_risk,
    })
}

fn coding_state_json(signals: &CodingReportSignals) -> Value {
    let observed_phases = coding_observed_phases(signals);
    json!({
        "schema_version": 1,
        "current_phase_hint": coding_current_phase_hint(signals),
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
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
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
