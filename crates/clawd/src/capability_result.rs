use claw_core::capability_result::{
    is_machine_ref, ArtifactRef, CapabilityResultEnvelope, CapabilityResultStatus, Continuation,
    ContinuationKind, EvidenceRef, StructuredError,
};
use serde_json::{json, Map as JsonMap, Value};

pub(crate) fn successful_execution_envelope(
    capability: &str,
    step_id: &str,
    args: &Value,
    output: &str,
    extra: Option<&Value>,
) -> CapabilityResultEnvelope {
    let mut envelope =
        CapabilityResultEnvelope::ok(capability, machine_action(args), result_data(output, extra));
    envelope.artifacts = artifact_refs_from_sources(output, extra);
    envelope.evidence.push(EvidenceRef {
        id: machine_evidence_id(step_id),
        source: capability.to_string(),
        locator: evidence_locator(extra),
        digest: None,
        metadata: json!({
            "step_id": step_id,
            "content_trust": "untrusted_tool_output",
        }),
    });
    if extra_requests_user_input(extra) {
        envelope.status = CapabilityResultStatus::NeedsUser;
        envelope.continuation = Some(Continuation {
            kind: ContinuationKind::AwaitUser,
            reference: continuation_reference(extra),
            poll_after_ms: None,
            state: continuation_state(extra),
        });
    } else if extra_reports_waiting(extra) {
        envelope.status = CapabilityResultStatus::Waiting;
        envelope.continuation = Some(Continuation {
            kind: ContinuationKind::Poll,
            reference: continuation_reference(extra),
            poll_after_ms: continuation_poll_after_ms(extra),
            state: continuation_state(extra),
        });
    }
    debug_assert!(envelope.validate().is_ok());
    envelope
}

pub(crate) fn failed_execution_envelope(
    capability: &str,
    step_id: &str,
    args: &Value,
    error: &str,
) -> CapabilityResultEnvelope {
    let structured = structured_error(error);
    let mut envelope =
        CapabilityResultEnvelope::failed(capability, machine_action(args), structured);
    envelope.evidence.push(EvidenceRef {
        id: machine_evidence_id(step_id),
        source: capability.to_string(),
        locator: None,
        digest: None,
        metadata: json!({
            "step_id": step_id,
            "content_trust": "untrusted_tool_error",
        }),
    });
    debug_assert!(envelope.validate().is_ok());
    envelope
}

pub(crate) fn envelope_for_step_execution(
    capability: &str,
    args: &Value,
    step: &crate::executor::StepExecutionResult,
    extra: Option<&Value>,
) -> CapabilityResultEnvelope {
    if step.status == crate::executor::StepExecutionStatus::Ok {
        if let Some(output) = step.output.as_deref() {
            return successful_execution_envelope(capability, &step.step_id, args, output, extra);
        }
    }
    failed_execution_envelope(
        capability,
        &step.step_id,
        args,
        step.error
            .as_deref()
            .unwrap_or("capability_execution_failed"),
    )
}

pub(crate) fn selected_exact_machine_result(
    results: &[CapabilityResultEnvelope],
    selector: &str,
) -> Option<String> {
    results.iter().rev().find_map(|result| {
        if result.status != CapabilityResultStatus::Ok {
            return None;
        }
        selected_result_value(&result.data, selector).and_then(exact_value_text)
    })
}

fn selected_result_value<'a>(data: &'a Value, selector: &str) -> Option<&'a Value> {
    structured_value_at_path(data, selector)
        .or_else(|| {
            data.get("extra")
                .and_then(|extra| structured_value_at_path(extra, selector))
        })
        .or_else(|| {
            data.get("output")
                .and_then(|output| structured_value_at_path(output, selector))
        })
        .or_else(|| {
            (selector == "command_output")
                .then(|| data.get("output"))
                .flatten()
        })
}

fn structured_value_at_path<'a>(value: &'a Value, selector: &str) -> Option<&'a Value> {
    selector
        .split('.')
        .try_fold(value, |current, segment| current.as_object()?.get(segment))
}

fn exact_value_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => (!value.trim().is_empty()).then(|| value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).ok(),
    }
}

fn machine_action(args: &Value) -> Option<String> {
    args.get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| is_machine_ref(action))
        .map(str::to_string)
}

fn result_data(output: &str, extra: Option<&Value>) -> Value {
    let output = output.trim();
    let parsed_output =
        serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()));
    let mut data = JsonMap::new();
    data.insert("output".to_string(), redact_for_model(parsed_output));
    if let Some(extra) = extra.filter(|extra| !extra.is_null()) {
        data.insert("extra".to_string(), redact_for_model(extra.clone()));
    }
    Value::Object(data)
}

fn redact_for_model(value: Value) -> Value {
    let serialized = value.to_string();
    let redacted = crate::visible_text::sanitize_user_visible_text(&serialized);
    serde_json::from_str(&redacted).unwrap_or(Value::String(redacted))
}

fn structured_error(error: &str) -> StructuredError {
    let parsed = serde_json::from_str::<Value>(error.trim()).ok();
    let code = parsed
        .as_ref()
        .and_then(|value| machine_string(value, &["error_code", "error_kind", "code"]))
        .filter(|value| is_machine_ref(value))
        .unwrap_or("capability_execution_failed")
        .to_string();
    let message_key = parsed
        .as_ref()
        .and_then(|value| machine_string(value, &["message_key"]))
        .filter(|value| is_machine_ref(value))
        .unwrap_or(code.as_str())
        .to_string();
    let retryable = parsed
        .as_ref()
        .and_then(|value| value.get("retryable"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    StructuredError {
        code,
        message_key,
        retryable,
        details: json!({
            "untrusted_error": redact_for_model(
                parsed.unwrap_or_else(|| Value::String(error.trim().to_string()))
            ),
        }),
    }
}

fn machine_string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn machine_evidence_id(step_id: &str) -> String {
    let normalized = step_id
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if is_machine_ref(&normalized) {
        normalized
    } else {
        "step_result".to_string()
    }
}

fn extra_requests_user_input(extra: Option<&Value>) -> bool {
    extra.and_then(Value::as_object).is_some_and(|extra| {
        extra
            .get("requires_user_input")
            .or_else(|| extra.get("needs_user_input"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
}

fn extra_reports_waiting(extra: Option<&Value>) -> bool {
    continuation_object(extra)
        .and_then(|extra| extra.get("status"))
        .and_then(Value::as_str)
        .is_some_and(|status| {
            matches!(
                status,
                "accepted" | "pending" | "running" | "waiting" | "background"
            )
        })
}

fn continuation_object(extra: Option<&Value>) -> Option<&serde_json::Map<String, Value>> {
    let extra = extra?.as_object()?;
    if let Some(job) = extra.get("pending_async_job").and_then(Value::as_object) {
        return Some(job);
    }
    [
        "job_id",
        "poll_ref",
        "checkpoint_ref",
        "poll_after_ms",
        "poll_after_seconds",
        "expires_at",
        "cancel_ref",
        "cancel_token",
        "result_ref",
    ]
    .into_iter()
    .any(|key| extra.contains_key(key))
    .then_some(extra)
}

fn continuation_reference(extra: Option<&Value>) -> Option<String> {
    let extra = continuation_object(extra)?;
    ["job_id", "poll_ref", "checkpoint_ref", "result_ref"]
        .into_iter()
        .find_map(|key| extra.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(sanitized_reference)
}

fn continuation_poll_after_ms(extra: Option<&Value>) -> Option<u64> {
    let extra = continuation_object(extra)?;
    extra
        .get("poll_after_ms")
        .and_then(Value::as_u64)
        .or_else(|| {
            extra
                .get("poll_after_seconds")
                .and_then(Value::as_u64)
                .map(|seconds| seconds.saturating_mul(1_000))
        })
}

fn continuation_state(extra: Option<&Value>) -> Value {
    let Some(extra) = continuation_object(extra) else {
        return json!({});
    };
    let mut state = JsonMap::new();
    for key in [
        "status",
        "expires_at",
        "cancel_ref",
        "result_ref",
        "message_key",
    ] {
        if let Some(value) = extra.get(key) {
            state.insert(key.to_string(), redact_for_model(value.clone()));
        }
    }
    Value::Object(state)
}

fn evidence_locator(extra: Option<&Value>) -> Option<String> {
    let extra = extra?.as_object()?;
    ["resolved_path", "path", "uri", "url"]
        .into_iter()
        .find_map(|key| extra.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(sanitized_reference)
}

fn artifact_refs(extra: Option<&Value>) -> Vec<ArtifactRef> {
    let Some(extra) = extra.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    for key in ["artifacts", "artifact_refs"] {
        let Some(items) = extra.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(artifact) = artifact_ref(item) else {
                continue;
            };
            if !refs.iter().any(|existing: &ArtifactRef| {
                existing.id == artifact.id
                    && existing.path == artifact.path
                    && existing.uri == artifact.uri
            }) {
                refs.push(artifact);
            }
        }
    }
    refs
}

fn artifact_refs_from_sources(output: &str, extra: Option<&Value>) -> Vec<ArtifactRef> {
    let parsed_output = serde_json::from_str::<Value>(output.trim()).ok();
    let mut refs = artifact_refs(parsed_output.as_ref());
    for artifact in artifact_refs(extra) {
        if !refs.iter().any(|existing| {
            existing.id == artifact.id
                && existing.path == artifact.path
                && existing.uri == artifact.uri
        }) {
            refs.push(artifact);
        }
    }
    refs
}

fn artifact_ref(value: &Value) -> Option<ArtifactRef> {
    let object = value.as_object()?;
    let string = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(sanitized_reference)
    };
    let artifact = ArtifactRef {
        id: string("id").or_else(|| string("artifact_id")),
        path: string("path").or_else(|| string("output_path")),
        uri: string("uri").or_else(|| string("url")),
        media_type: string("media_type").or_else(|| string("mime_type")),
        sha256: string("sha256"),
        metadata: object
            .get("metadata")
            .cloned()
            .map(redact_for_model)
            .unwrap_or_else(|| json!({})),
    };
    (artifact.id.is_some() || artifact.path.is_some() || artifact.uri.is_some()).then_some(artifact)
}

fn sanitized_reference(value: &str) -> String {
    crate::visible_text::redact_sensitive_text(value)
}

#[cfg(test)]
#[path = "capability_result_tests.rs"]
mod tests;
