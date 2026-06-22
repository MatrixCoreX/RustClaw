pub(crate) struct TaskEventLine {
    pub(crate) event_type: String,
    pub(crate) line: String,
}

pub(crate) fn task_event_lines(data: &serde_json::Value) -> Vec<TaskEventLine> {
    data.pointer("/result_json/task_journal/trace/event_stream")
        .and_then(serde_json::Value::as_array)
        .map(|events| events.iter().filter_map(task_event_line).collect())
        .unwrap_or_default()
}

fn task_event_line(event: &serde_json::Value) -> Option<TaskEventLine> {
    let mut parts = Vec::new();
    push_scalar_token(&mut parts, "seq", event.get("seq"));
    push_scalar_token(&mut parts, "type", event.get("event_type"));
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
        "error_kind",
        "failure_attribution",
        "owner_layer",
        "stage",
        "decision",
        "reason_code",
        "role",
        "execution_mode",
        "write_enabled",
        "external_publish_enabled",
        "failure_isolated",
        "child_run_id",
        "objective_present",
        "objective_char_count",
        "context_ref_count",
        "allowed_capability_count",
        "skill",
        "tool_or_skill",
        "step_id",
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
        "checkpoint_id",
        "poll_ref",
        "final_status",
        "final_stop_signal",
    ] {
        push_scalar_token(&mut parts, key, payload.and_then(|value| value.get(key)));
    }
    push_scalar_token(
        &mut parts,
        "child_trace_merge_status",
        payload.and_then(|value| value.pointer("/child_run_summary/trace_merge_status")),
    );
    push_scalar_token(
        &mut parts,
        "child_result_status",
        payload.and_then(|value| value.pointer("/child_run_summary/result_status")),
    );
    push_scalar_token(
        &mut parts,
        "child_request_state",
        payload.and_then(|value| value.pointer("/child_request/state")),
    );
    push_scalar_token(
        &mut parts,
        "scheduler_status",
        payload.and_then(|value| value.pointer("/scheduler/status")),
    );
    push_scalar_token(
        &mut parts,
        "scheduler_reason_code",
        payload.and_then(|value| value.pointer("/scheduler/reason_code")),
    );
    push_scalar_token(
        &mut parts,
        "merge_strategy",
        payload.and_then(|value| value.pointer("/merge_contract/strategy")),
    );
    push_scalar_token(
        &mut parts,
        "merge_status",
        payload.and_then(|value| value.pointer("/merge_contract/child_trace_merge_status")),
    );
    (!parts.is_empty()).then(|| TaskEventLine {
        event_type,
        line: parts.join(" "),
    })
}

fn push_scalar_token(parts: &mut Vec<String>, key: &str, value: Option<&serde_json::Value>) {
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
    parts.push(format!("{key}={token}"));
}
