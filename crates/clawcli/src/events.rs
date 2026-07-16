use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(crate) struct TaskEventLine {
    pub(crate) event_type: String,
    pub(crate) line: String,
    pub(crate) fields: BTreeMap<String, String>,
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

fn task_event_line(event: &serde_json::Value) -> Option<TaskEventLine> {
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
        "decision",
        "reason_code",
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
        "verification_status",
        "verification_failure_kind_count",
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
