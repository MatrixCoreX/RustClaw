use std::collections::BTreeMap;
use std::fmt;
use std::io::{BufRead, BufReader};
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::StatusCode;

use crate::client;

#[derive(Debug, Clone)]
pub(crate) struct TaskEventLine {
    pub(crate) event_type: String,
    pub(crate) line: String,
    pub(crate) fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveEventOutputMode {
    Compact,
    Jsonl,
    Quiet,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct EventFilters {
    event_types: Vec<String>,
    checkpoint_id: Option<String>,
    policy_decision: Option<String>,
    subagent_id: Option<String>,
    async_job_id: Option<String>,
}

impl EventFilters {
    pub(crate) fn from_parts(
        event_types: &[String],
        checkpoint_id: Option<&str>,
        policy_decision: Option<&str>,
        subagent_id: Option<&str>,
        async_job_id: Option<&str>,
    ) -> Self {
        Self {
            event_types: event_types
                .iter()
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty())
                .collect(),
            checkpoint_id: normalize_filter_token(checkpoint_id),
            policy_decision: normalize_filter_token(policy_decision).map(|value| {
                value
                    .chars()
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
            }),
            subagent_id: normalize_filter_token(subagent_id),
            async_job_id: normalize_filter_token(async_job_id),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.event_types.is_empty()
            && self.checkpoint_id.is_none()
            && self.policy_decision.is_none()
            && self.subagent_id.is_none()
            && self.async_job_id.is_none()
    }

    pub(crate) fn matches(&self, event: &TaskEventLine) -> bool {
        if !self.event_types.is_empty()
            && !self
                .event_types
                .iter()
                .any(|requested| requested == &event.event_type.to_ascii_lowercase())
        {
            return false;
        }
        if let Some(checkpoint_id) = self.checkpoint_id.as_deref() {
            if !field_matches(event, &["checkpoint_id"], checkpoint_id, false) {
                return false;
            }
        }
        if let Some(policy_decision) = self.policy_decision.as_deref() {
            if !field_matches(event, &["decision"], policy_decision, true) {
                return false;
            }
        }
        if let Some(subagent_id) = self.subagent_id.as_deref() {
            if !field_matches(event, &["subagent_id", "child_run_id"], subagent_id, false) {
                return false;
            }
        }
        if let Some(async_job_id) = self.async_job_id.as_deref() {
            if !field_matches(
                event,
                &[
                    "async_job_id",
                    "pending_async_job_id",
                    "job_id",
                    "provider_job_id",
                    "poll_ref",
                ],
                async_job_id,
                false,
            ) {
                return false;
            }
        }
        true
    }
}

fn normalize_filter_token(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn field_matches(
    event: &TaskEventLine,
    keys: &[&str],
    expected: &str,
    ascii_case_insensitive: bool,
) -> bool {
    keys.iter().any(|key| {
        event
            .fields
            .get(*key)
            .map(|value| {
                if ascii_case_insensitive {
                    value.eq_ignore_ascii_case(expected)
                } else {
                    value == expected
                }
            })
            .unwrap_or(false)
    })
}

pub(crate) fn task_event_lines(data: &serde_json::Value) -> Vec<TaskEventLine> {
    let mut events: Vec<TaskEventLine> = data
        .pointer("/result_json/task_journal/trace/event_stream")
        .and_then(serde_json::Value::as_array)
        .map(|events| events.iter().filter_map(task_event_line).collect())
        .unwrap_or_default();
    if let Some(worker_events) = data
        .pointer("/result_json/task_lifecycle/worker_events")
        .and_then(serde_json::Value::as_array)
    {
        events.extend(worker_events.iter().filter_map(lifecycle_worker_event_line));
    }
    events
}

pub(crate) fn task_event_lines_from_raw(events: &[serde_json::Value]) -> Vec<TaskEventLine> {
    events.iter().filter_map(task_event_line).collect()
}

pub(crate) fn live_task_event_output_line(
    raw_event: &serde_json::Value,
    mode: LiveEventOutputMode,
    filters: &EventFilters,
) -> Result<Option<String>> {
    if mode == LiveEventOutputMode::Quiet {
        return Ok(None);
    }
    let event = task_event_line(raw_event);
    if !filters.is_empty() && !event.as_ref().is_some_and(|event| filters.matches(event)) {
        return Ok(None);
    }
    match mode {
        LiveEventOutputMode::Compact => Ok(event.map(|event| compact_task_event_line(&event))),
        LiveEventOutputMode::Jsonl => Ok(Some(serde_json::to_string(raw_event)?)),
        LiveEventOutputMode::Quiet => Ok(None),
    }
}

pub(crate) fn compact_task_event_line(event: &TaskEventLine) -> String {
    format!("event: {}", event.line)
}

pub(crate) fn follow_task_events<F>(
    base_url: &str,
    key: &str,
    task_id: &str,
    cursor: u64,
    on_event: F,
) -> Result<()>
where
    F: FnMut(&serde_json::Value) -> Result<bool>,
{
    follow_task_events_with_timeout(base_url, key, task_id, cursor, None, on_event)
}

pub(crate) fn follow_task_events_with_timeout<F>(
    base_url: &str,
    key: &str,
    task_id: &str,
    cursor: u64,
    request_timeout: Option<Duration>,
    mut on_event: F,
) -> Result<()>
where
    F: FnMut(&serde_json::Value) -> Result<bool>,
{
    let url = format!(
        "{}/tasks/{}/events?cursor={}",
        client::base_v1(base_url),
        task_id,
        cursor
    );
    let response = client::make_stream_client_with_timeout(request_timeout)?
        .get(url)
        .header("x-rustclaw-key", key)
        .header("accept", "text/event-stream")
        .header("last-event-id", cursor.to_string())
        .send()
        .context("task_event_stream_open_failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(TaskEventHttpStatusError { status, body }.into());
    }
    consume_sse(BufReader::new(response), &mut on_event)
}

pub(crate) fn read_task_event_snapshot(
    base_url: &str,
    key: &str,
    task_id: &str,
    cursor: u64,
) -> Result<Vec<serde_json::Value>> {
    let url = format!(
        "{}/tasks/{}/events?cursor={}&follow=false",
        client::base_v1(base_url),
        task_id,
        cursor
    );
    let response = client::make_stream_client_with_timeout(None)?
        .get(url)
        .header("x-rustclaw-key", key)
        .header("accept", "text/event-stream")
        .header("last-event-id", cursor.to_string())
        .send()
        .context("task_event_snapshot_open_failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(TaskEventHttpStatusError { status, body }.into());
    }
    let mut events = Vec::new();
    consume_sse(BufReader::new(response), &mut |event| {
        events.push(event.clone());
        Ok(true)
    })?;
    Ok(events)
}

#[derive(Debug)]
struct TaskEventHttpStatusError {
    status: StatusCode,
    body: String,
}

impl fmt::Display for TaskEventHttpStatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "task_event_stream_http_error:status={}:body={}",
            self.status, self.body
        )
    }
}

impl std::error::Error for TaskEventHttpStatusError {}

pub(crate) fn task_event_stream_is_unavailable(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<TaskEventHttpStatusError>()
        .is_some_and(|error| {
            matches!(
                error.status,
                StatusCode::NOT_FOUND
                    | StatusCode::METHOD_NOT_ALLOWED
                    | StatusCode::NOT_ACCEPTABLE
                    | StatusCode::NOT_IMPLEMENTED
            )
        })
}

pub(crate) fn task_event_stream_has_http_status(error: &anyhow::Error) -> bool {
    error.downcast_ref::<TaskEventHttpStatusError>().is_some()
}

pub(crate) fn task_event_stream_timed_out(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        source
            .downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_timeout)
            || source
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| {
                    error.kind() == std::io::ErrorKind::TimedOut
                        || error
                            .get_ref()
                            .and_then(|source| source.downcast_ref::<reqwest::Error>())
                            .is_some_and(reqwest::Error::is_timeout)
                })
    })
}

pub(crate) fn task_event_seq(event: &serde_json::Value) -> Option<u64> {
    event.get("seq").and_then(serde_json::Value::as_u64)
}

pub(crate) fn task_event_is_terminal(event: &serde_json::Value) -> bool {
    event_kind(event) == Some("task_final")
        || matches!(
            event_execution_state(event),
            Some("completed" | "failed" | "cancelled")
        )
}

pub(crate) fn task_event_is_background(event: &serde_json::Value) -> bool {
    matches!(
        event_execution_state(event),
        Some("background" | "waiting" | "needs_user" | "needs_confirmation")
    )
}

fn event_kind(event: &serde_json::Value) -> Option<&str> {
    event
        .get("event_kind")
        .or_else(|| event.get("event_type"))
        .and_then(serde_json::Value::as_str)
}

fn event_execution_state(event: &serde_json::Value) -> Option<&str> {
    event
        .pointer("/payload/execution_state")
        .or_else(|| event.pointer("/payload/state"))
        .and_then(serde_json::Value::as_str)
}

fn consume_sse<R, F>(mut reader: R, on_event: &mut F) -> Result<()>
where
    R: BufRead,
    F: FnMut(&serde_json::Value) -> Result<bool>,
{
    let mut line = String::new();
    let mut data_lines = Vec::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            emit_sse_data(&mut data_lines, on_event)?;
            return Ok(());
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if !emit_sse_data(&mut data_lines, on_event)? {
                return Ok(());
            }
            continue;
        }
        if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.strip_prefix(' ').unwrap_or(data).to_string());
        }
    }
}

fn emit_sse_data<F>(data_lines: &mut Vec<String>, on_event: &mut F) -> Result<bool>
where
    F: FnMut(&serde_json::Value) -> Result<bool>,
{
    if data_lines.is_empty() {
        return Ok(true);
    }
    let data = data_lines.join("\n");
    data_lines.clear();
    let value: serde_json::Value =
        serde_json::from_str(&data).context("task_event_sse_json_parse_failed")?;
    on_event(&value)
}

fn lifecycle_worker_event_line(event: &serde_json::Value) -> Option<TaskEventLine> {
    let event_type = event
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let normalized = serde_json::json!({
        "event_type": event_type,
        "owner_layer": "task_lifecycle",
        "payload": event,
    });
    task_event_line(&normalized)
}

pub(crate) fn task_event_line(event: &serde_json::Value) -> Option<TaskEventLine> {
    let mut parts = Vec::new();
    let mut fields = BTreeMap::new();
    push_scalar_token(&mut parts, &mut fields, "seq", event.get("seq"));
    push_scalar_token(&mut parts, &mut fields, "type", event.get("event_type"));
    let event_type = event
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string();
    let payload = event.get("payload");
    push_scalar_token(
        &mut parts,
        &mut fields,
        "model_phase",
        payload.and_then(|value| value.get("type")),
    );
    for key in [
        "status",
        "state",
        "task_id",
        "transition_index",
        "transition_ref",
        "evidence_ref",
        "state_from",
        "state_to",
        "error_kind",
        "failure_attribution",
        "owner_layer",
        "stage",
        "phase",
        "model_event_index",
        "provider",
        "tool_index",
        "tool_call_id",
        "tool_name",
        "arguments_delta_bytes",
        "text_delta_bytes",
        "finish_reason",
        "retryable",
        "code",
        "decision",
        "reason_code",
        "status_code",
        "error_code",
        "handler_id",
        "handler_kind",
        "blocking",
        "failure_policy",
        "trust_status",
        "content_sha256",
        "duration_ms",
        "attempts",
        "output_truncated",
        "role",
        "execution_mode",
        "write_enabled",
        "external_publish_enabled",
        "failure_isolated",
        "subagent_id",
        "child_run_id",
        "objective_present",
        "objective_char_count",
        "context_ref_count",
        "allowed_capability_count",
        "skill",
        "tool_or_skill",
        "step_id",
        "step_ref",
        "action_kind",
        "action_ref",
        "requested_capability",
        "requested_action_ref",
        "resolved_tool_or_skill",
        "resolved_capability",
        "resolution_source",
        "output_evidence_count",
        "artifact_ref_count",
        "prompt_label",
        "llm_call_count",
        "elapsed_ms",
        "provider_attempt_count",
        "provider_retry_count",
        "provider_retryable_error_count",
        "provider_final_error_count",
        "prompt_truncation_count",
        "prompt_bytes_before_max",
        "prompt_bytes_budget_min",
        "prompt_bytes_after_max",
        "prompt_truncated_bytes_total",
        "profile",
        "soft_slice_ms",
        "continuation_index",
        "cumulative_model_turns",
        "cumulative_tool_calls",
        "cumulative_input_tokens",
        "cumulative_output_tokens",
        "cumulative_cost_usd_nanos",
        "cumulative_elapsed_ms",
        "stagnation_tolerance",
        "provider_timeout_class",
        "tool_timeout_class",
        "observed_progress",
        "soft_slice_exhausted",
        "resumable",
        "planned_action_count",
        "independently_batchable_count",
        "executed_action_count",
        "round_stop_signal",
        "replan_cause",
        "next_resumable_action",
        "hard_model_turns",
        "hard_tool_calls",
        "hard_total_tokens",
        "hard_cost_usd_nanos",
        "hard_elapsed_ms",
        "hard_continuations",
        "hard_non_resumable_tool_runtime_ms",
        "at_ms",
        "started_at",
        "finished_at",
        "round_no",
        "contract_ref",
        "checkpoint_id",
        "checkpoint_kind",
        "checkpoint_ref",
        "patch_id",
        "mutation_id",
        "compensates_checkpoint_id",
        "compensates_patch_id",
        "compensates_mutation_id",
        "target_path",
        "isolation_root",
        "reversible",
        "reversibility_status",
        "reversibility_reason_code",
        "additions",
        "deletions",
        "changed_hunks",
        "completed_side_effect_count",
        "requires_idempotency_guard",
        "poll_ref",
        "cancel_ref",
        "message_key",
        "async_job_id",
        "pending_async_job_id",
        "job_id",
        "provider_job_id",
        "files_read_count",
        "files_changed_count",
        "commands_run_count",
        "tests_run_count",
        "changed_file_count",
        "command_count",
        "command_index",
        "verification_command_count",
        "verification_command",
        "test_count",
        "diff_summary_count",
        "failure_count",
        "projection_revision",
        "latest_verification_step_ref",
        "verification_status",
        "verification_failure_kind_count",
        "historical_verification_failure_kind_count",
        "retry_count",
        "unverified_risk",
        "final_status",
        "final_stop_signal",
        "recovered_at",
        "worker_id",
        "lease_owner",
        "lease_expires_at",
    ] {
        push_scalar_token(
            &mut parts,
            &mut fields,
            key,
            payload.and_then(|value| value.get(key)),
        );
    }
    push_scalar_token(
        &mut parts,
        &mut fields,
        "child_trace_merge_status",
        payload.and_then(|value| value.pointer("/child_run_summary/trace_merge_status")),
    );
    push_scalar_token(
        &mut parts,
        &mut fields,
        "child_result_status",
        payload.and_then(|value| value.pointer("/child_run_summary/result_status")),
    );
    push_scalar_token(
        &mut parts,
        &mut fields,
        "child_run_id",
        payload.and_then(|value| value.pointer("/child_run_summary/child_run_id")),
    );
    push_scalar_token(
        &mut parts,
        &mut fields,
        "child_request_state",
        payload.and_then(|value| value.pointer("/child_request/state")),
    );
    push_scalar_token(
        &mut parts,
        &mut fields,
        "scheduler_status",
        payload.and_then(|value| value.pointer("/scheduler/status")),
    );
    push_scalar_token(
        &mut parts,
        &mut fields,
        "scheduler_reason_code",
        payload.and_then(|value| value.pointer("/scheduler/reason_code")),
    );
    push_scalar_token(
        &mut parts,
        &mut fields,
        "merge_strategy",
        payload.and_then(|value| value.pointer("/merge_contract/strategy")),
    );
    push_scalar_token(
        &mut parts,
        &mut fields,
        "merge_status",
        payload.and_then(|value| value.pointer("/merge_contract/child_trace_merge_status")),
    );
    push_scalar_token(
        &mut parts,
        &mut fields,
        "async_job_id",
        payload.and_then(|value| value.pointer("/async_job/job_id")),
    );
    push_scalar_token(
        &mut parts,
        &mut fields,
        "provider_job_id",
        payload.and_then(|value| value.pointer("/async_job/provider_job_id")),
    );
    (!parts.is_empty()).then(|| TaskEventLine {
        event_type,
        line: parts.join(" "),
        fields,
    })
}

fn push_scalar_token(
    parts: &mut Vec<String>,
    fields: &mut BTreeMap<String, String>,
    key: &str,
    value: Option<&serde_json::Value>,
) {
    let Some(value) = value else {
        return;
    };
    let token = match value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            String::new()
        }
    };
    if token.is_empty() {
        return;
    }
    fields.insert(key.to_string(), token.clone());
    parts.push(format!("{key}={token}"));
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
