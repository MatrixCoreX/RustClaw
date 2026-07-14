use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

use crate::task;

use super::report_budget_health::{
    llm_budget_health_json, llm_budget_text_lines, LlmBudgetMetrics,
};

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
    let coding = coding_report_json(&task.raw_data);
    let outcome = super::report_outcome::task_outcome_report_json(&task.raw_data, &coding);
    let session = task_session_projection_json(task);
    let context_budget = context_budget_report_json(&task.raw_data);
    json!({
        "report_kind": "rustclaw_task_report",
        "task_id": task.task_id,
        "goal_id": task_goal_id(&task.raw_data),
        "session_id": session.get("session_id").cloned().unwrap_or(Value::Null),
        "session": session,
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
        "context_budget": context_budget,
        "coding": coding,
        "outcome": outcome,
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
    if let Some(goal_id) = report.get("goal_id").and_then(Value::as_str) {
        lines.push(format!("goal_id={goal_id}"));
    }
    if let Some(session_id) = report.get("session_id").and_then(Value::as_str) {
        lines.push(format!("session_id={session_id}"));
    }
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
    for line in report
        .get("llm")
        .map(llm_budget_text_lines)
        .unwrap_or_default()
    {
        lines.push(line);
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
    if let Some(next_step) = report
        .pointer("/coding/state/next_step")
        .and_then(Value::as_str)
    {
        lines.push(format!("coding_next_step: {next_step}"));
    }
    if let Some(state) = report.pointer("/outcome/state").and_then(Value::as_str) {
        lines.push(format!("outcome_state: {state}"));
    }
    for item in report_string_array(report, "/outcome/done_conditions")
        .into_iter()
        .take(16)
    {
        lines.push(format!("done_condition: {item}"));
    }
    for item in report_string_array(report, "/outcome/current_progress")
        .into_iter()
        .take(16)
    {
        lines.push(format!("current_progress: {item}"));
    }
    for item in report_string_array(report, "/outcome/remaining_work")
        .into_iter()
        .take(16)
    {
        lines.push(format!("remaining_work: {item}"));
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

fn task_goal_id(raw_data: &Value) -> Value {
    first_string_at(
        raw_data,
        &[
            "/goal/goal_id",
            "/task_goal/goal_id",
            "/result_json/task_goal/goal_id",
            "/result_json/task_journal/summary/task_goal/goal_id",
            "/task_journal/summary/task_goal/goal_id",
        ],
    )
    .map(Value::String)
    .unwrap_or(Value::Null)
}

fn task_session_projection_json(task: &task::TaskStatusView) -> Value {
    let session_id = first_string_at(
        &task.raw_data,
        &[
            "/session_id",
            "/conversation_state/session_id",
            "/result_json/session_id",
            "/task_journal/summary/session_id",
            "/result_json/task_journal/summary/session_id",
        ],
    )
    .or_else(|| user_chat_session_id(&task.raw_data));
    json!({
        "session_id": session_id,
        "user_id": scalar_string_at(&task.raw_data, "/user_id"),
        "chat_id": scalar_string_at(&task.raw_data, "/chat_id"),
        "task_ids": [task.task_id.clone()],
        "active_goal_id": task_goal_id(&task.raw_data),
    })
}

fn context_budget_report_json(data: &Value) -> Value {
    data.pointer("/result_json/task_journal/summary/context_budget_report")
        .or_else(|| data.pointer("/task_journal/summary/context_budget_report"))
        .and_then(Value::as_object)
        .map(|report| {
            let mut projected = report.clone();
            projected.insert(
                "source".to_string(),
                Value::String("task_journal_context_budget_report".to_string()),
            );
            Value::Object(projected)
        })
        .unwrap_or(Value::Null)
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
    if let Some(next_step) = review
        .pointer("/coding/state/next_step")
        .and_then(Value::as_str)
    {
        lines.push(format!("coding_next_step: {next_step}"));
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
    if let Some(next_step) = coding.pointer("/state/next_step").and_then(Value::as_str) {
        lines.push(format!("coding_next_step: {next_step}"));
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
            let conflict_count = item
                .get("conflict_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let decision_status = item
                .get("decision_status")
                .and_then(Value::as_str)
                .unwrap_or("");
            let tool_permission_profile = item
                .get("tool_permission_profile")
                .and_then(Value::as_str)
                .unwrap_or("");
            let read_only_enforced = item
                .get("read_only_enforced")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let write_isolation_status = item
                .get("write_isolation_status")
                .and_then(Value::as_str)
                .unwrap_or("");
            let timeout_ms = item.get("timeout_ms").and_then(Value::as_u64).unwrap_or(0);
            lines.push(format!(
                "subagent: child_run_id={child_run_id} subagent_id={subagent_id} status={status} tool_permission_profile={tool_permission_profile} read_only_enforced={read_only_enforced} write_isolation_status={write_isolation_status} timeout_ms={timeout_ms} conflict_count={conflict_count} decision_status={decision_status} finding_refs={finding_refs} evidence_refs={evidence_refs}"
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

fn first_string_at(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers
        .iter()
        .find_map(|pointer| scalar_string_at(value, pointer))
}

fn scalar_string_at(value: &Value, pointer: &str) -> Option<String> {
    value.pointer(pointer).and_then(|item| match item {
        Value::String(text) => {
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    })
}

fn user_chat_session_id(value: &Value) -> Option<String> {
    let user_id = scalar_string_at(value, "/user_id")?;
    let chat_id = scalar_string_at(value, "/chat_id")?;
    Some(format!("user_chat:{user_id}:{chat_id}"))
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
    let budget_health = llm_budget_health_json(&LlmBudgetMetrics {
        prompt_bucket_count: by_prompt.len() as u64,
        llm_call_count: total.llm_call_count,
        elapsed_ms: total.elapsed_ms,
        provider_retry_count: total.provider_retry_count,
        provider_retryable_error_count: total.provider_retryable_error_count,
        provider_final_error_count: total.provider_final_error_count,
        prompt_truncation_count: total.prompt_truncation_count,
        prompt_bytes_before_max: total.prompt_bytes_before_max,
        prompt_truncated_bytes_total: total.prompt_truncated_bytes_total,
    });
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
        "budget_health": budget_health,
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
        "tool_permission_profile",
        "write_isolation_status",
        "execution_mode",
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
        "tool_permission_profile": subagent_tool_permission_profile(map),
        "read_only_enforced": subagent_read_only_enforced(map),
        "write_isolation_status": subagent_write_isolation_status(map),
        "timeout_ms": subagent_timeout_ms(map),
        "timeout_source": subagent_timeout_source(map),
        "status": machine_string_field(map, "status"),
        "result_status": machine_string_field(map, "result_status"),
        "outcome_code": machine_string_field(map, "outcome_code"),
        "error_code": machine_string_field(map, "error_code"),
        "failure_isolation": machine_string_field(map, "failure_isolation"),
        "failure_isolated": map.get("failure_isolated").and_then(Value::as_bool),
        "required": map.get("required").and_then(Value::as_bool),
        "optional": map.get("optional").and_then(Value::as_bool),
        "conflict_count": subagent_conflict_count(map),
        "decision_status": subagent_decision_status(map),
        "confidence_min": subagent_confidence_value(map, "min"),
        "confidence_max": subagent_confidence_value(map, "max"),
        "finding_refs": machine_ref_array(map.get("finding_refs")),
        "evidence_refs": machine_ref_array(map.get("evidence_refs")),
    }));
}

fn subagent_timeout_ms(map: &Map<String, Value>) -> Option<u64> {
    map.get("timeout_ms").and_then(Value::as_u64).or_else(|| {
        map.get("timeout_policy")
            .and_then(|value| value.get("timeout_ms"))
            .and_then(Value::as_u64)
    })
}

fn subagent_timeout_source(map: &Map<String, Value>) -> Option<String> {
    machine_string_field(map, "timeout_source").or_else(|| {
        map.get("timeout_policy")
            .and_then(Value::as_object)
            .and_then(|policy| machine_string_field(policy, "source"))
    })
}

fn subagent_tool_permission_profile(map: &Map<String, Value>) -> Option<String> {
    machine_string_field(map, "tool_permission_profile").or_else(|| {
        map.get("role_metadata")
            .and_then(Value::as_object)
            .and_then(|role| machine_string_field(role, "tool_permission_profile"))
    })
}

fn subagent_read_only_enforced(map: &Map<String, Value>) -> Option<bool> {
    if let Some(read_only) = map.get("read_only").and_then(Value::as_bool) {
        return Some(read_only);
    }
    if subagent_tool_permission_profile(map).as_deref() == Some("read_only") {
        return Some(true);
    }
    let execution_mode = machine_string_field(map, "execution_mode");
    Some(matches!(
        execution_mode.as_deref(),
        Some("inline_readonly_child_run" | "bounded_parallel_readonly_child_runs")
    ))
}

fn subagent_write_isolation_status(map: &Map<String, Value>) -> &'static str {
    if matches!(
        machine_string_field(map, "write_isolation_status").as_deref(),
        Some("enabled")
    ) {
        return "enabled";
    }
    if map
        .get("requested_write_isolation")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return "requested_denied";
    }
    "not_supported"
}

fn subagent_conflict_count(map: &Map<String, Value>) -> Option<u64> {
    map.get("conflict_count")
        .and_then(Value::as_u64)
        .or_else(|| {
            map.get("conflict_summary")
                .and_then(|value| value.get("conflict_count"))
                .and_then(Value::as_u64)
        })
        .or_else(|| {
            map.get("aggregation")
                .and_then(|value| value.get("conflict_count"))
                .and_then(Value::as_u64)
        })
        .or_else(|| {
            map.get("aggregation")
                .and_then(|value| value.get("conflict_summary"))
                .and_then(|value| value.get("conflict_count"))
                .and_then(Value::as_u64)
        })
}

fn subagent_decision_status(map: &Map<String, Value>) -> Option<String> {
    map.get("main_thread_decision")
        .and_then(|value| value.get("decision_status"))
        .and_then(Value::as_str)
        .or_else(|| {
            map.get("aggregation")
                .and_then(|value| value.get("main_thread_decision"))
                .and_then(|value| value.get("decision_status"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| is_report_machine_token(value))
        .map(ToString::to_string)
}

fn subagent_confidence_value(map: &Map<String, Value>, key: &str) -> Option<f64> {
    map.get("confidence_summary")
        .and_then(|value| value.get(key))
        .and_then(Value::as_f64)
        .or_else(|| {
            map.get("aggregation")
                .and_then(|value| value.get("confidence_summary"))
                .and_then(|value| value.get(key))
                .and_then(Value::as_f64)
        })
        .filter(|value| value.is_finite())
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
