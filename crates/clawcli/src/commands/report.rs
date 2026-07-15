use crate::task;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

#[path = "report_coding.rs"]
mod report_coding;
use report_coding::coding_report_json;

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
    let context_compaction = context_compaction_report_json(&task.raw_data);
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
        "context_compaction": context_compaction,
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

fn context_compaction_report_json(data: &Value) -> Value {
    data.pointer("/result_json/task_journal/summary/transcript_compaction_records")
        .or_else(|| data.pointer("/task_journal/summary/transcript_compaction_records"))
        .and_then(Value::as_array)
        .map(|records| {
            json!({
                "source": "task_journal_transcript_compaction_records",
                "record_count": records.len(),
                "records": records,
            })
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
        "team_count": signals.teams.len(),
        "teams": signals.teams,
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

fn is_report_machine_token(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 120
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.' | '/'))
}

#[derive(Default)]
struct SubagentReportSignals {
    seen: BTreeSet<String>,
    items: Vec<Value>,
    seen_teams: BTreeSet<String>,
    teams: Vec<Value>,
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
            if let Some(Value::Object(team_spec)) = map.get("team_spec") {
                push_subagent_team(team_spec, signals);
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

fn push_subagent_team(map: &Map<String, Value>, signals: &mut SubagentReportSignals) {
    let team_id = machine_string_field(map, "team_id").unwrap_or_default();
    if team_id.is_empty()
        || !signals.seen_teams.insert(team_id.clone())
        || signals.teams.len() >= 64
    {
        return;
    }
    let child_count = map
        .get("children")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    signals.teams.push(json!({
        "team_id": team_id,
        "parent_task_id": machine_string_field(map, "parent_task_id"),
        "max_parallel": map.get("max_parallel").and_then(Value::as_u64),
        "write_permission": machine_string_field(map, "write_permission"),
        "conflict_policy": machine_string_field(map, "conflict_policy"),
        "child_count": child_count,
        "child_task_ids": machine_ref_array(map.get("child_task_ids")),
    }));
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
