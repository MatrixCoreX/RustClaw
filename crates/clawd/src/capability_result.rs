use claw_core::capability_result::{
    is_machine_ref, ArtifactRef, CapabilityResultEnvelope, CapabilityResultStatus,
    CapabilityResultValidationError, Continuation, ContinuationKind, EvidenceRef,
    ResultCompleteness, RetryDirective, StructuredError,
};
use serde_json::{json, Map as JsonMap, Value};

#[cfg(test)]
pub(crate) fn successful_execution_envelope(
    capability: &str,
    step_id: &str,
    args: &Value,
    output: &str,
    extra: Option<&Value>,
) -> CapabilityResultEnvelope {
    build_successful_execution_envelope(capability, step_id, args, output, extra)
        .expect("test capability result fixture must be valid")
}

fn build_successful_execution_envelope(
    capability: &str,
    step_id: &str,
    args: &Value,
    output: &str,
    extra: Option<&Value>,
) -> Result<CapabilityResultEnvelope, CapabilityResultValidationError> {
    let mut envelope =
        CapabilityResultEnvelope::ok(capability, machine_action(args), result_data(output, extra));
    envelope.artifacts = artifact_refs_from_sources(output, extra);
    apply_result_metadata(
        &mut envelope,
        serde_json::from_str::<Value>(output.trim()).ok().as_ref(),
        extra,
        step_id,
        "untrusted_tool_output",
    );
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
    envelope.validate()?;
    Ok(envelope)
}

#[cfg(test)]
pub(crate) fn failed_execution_envelope(
    capability: &str,
    step_id: &str,
    args: &Value,
    error: &str,
) -> CapabilityResultEnvelope {
    build_failed_execution_envelope(capability, step_id, args, error)
        .expect("test capability error fixture must be valid")
}

fn build_failed_execution_envelope(
    capability: &str,
    step_id: &str,
    args: &Value,
    error: &str,
) -> Result<CapabilityResultEnvelope, CapabilityResultValidationError> {
    let structured = structured_error(error);
    let mut envelope =
        CapabilityResultEnvelope::failed(capability, machine_action(args), structured);
    let parsed_error = serde_json::from_str::<Value>(error.trim()).ok();
    envelope.artifacts = artifact_refs(parsed_error.as_ref());
    apply_result_metadata(
        &mut envelope,
        parsed_error.as_ref(),
        None,
        step_id,
        "untrusted_tool_error",
    );
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
    envelope.validate()?;
    Ok(envelope)
}

pub(crate) fn envelope_for_step_execution(
    capability: &str,
    args: &Value,
    step: &crate::executor::StepExecutionResult,
    extra: Option<&Value>,
) -> Result<CapabilityResultEnvelope, CapabilityResultValidationError> {
    if step.status == crate::executor::StepExecutionStatus::Ok {
        if let Some(output) = step.output.as_deref() {
            return build_successful_execution_envelope(
                capability,
                &step.step_id,
                args,
                output,
                extra,
            );
        }
    }
    build_failed_execution_envelope(
        capability,
        &step.step_id,
        args,
        step.error
            .as_deref()
            .unwrap_or("capability_execution_failed"),
    )
}

pub(crate) fn settle_waiting_async_result(
    result: &mut CapabilityResultEnvelope,
    job_id: &str,
    final_result_json: &Value,
) -> bool {
    let job_id = job_id.trim();
    if job_id.is_empty()
        || !final_result_json.is_object()
        || result.status != CapabilityResultStatus::Waiting
        || !result.continuation.as_ref().is_some_and(|continuation| {
            continuation.kind == ContinuationKind::Poll
                && continuation.reference.as_deref() == Some(job_id)
        })
    {
        return false;
    }

    let mut completed = result.clone();
    let extra = final_result_json.get("extra");
    completed.status = CapabilityResultStatus::Ok;
    completed.data = result_data(&final_result_json.to_string(), extra);
    completed.artifacts = artifact_refs_from_sources(&final_result_json.to_string(), extra);
    completed.completeness = None;
    completed.page = None;
    completed.truncated = false;
    completed.retry = None;
    completed.error = None;
    completed.continuation = None;
    let step_id = result
        .provenance
        .get("step_id")
        .and_then(Value::as_str)
        .unwrap_or("async_completion")
        .to_string();
    apply_result_metadata(
        &mut completed,
        Some(final_result_json),
        extra,
        &step_id,
        "trusted_async_job_completion",
    );
    completed.provenance = json!({
        "source": "async_job_completion_checkpoint",
        "job_id": sanitized_reference(job_id),
        "step_id": machine_evidence_id(&step_id),
        "content_trust": "trusted_async_job_completion",
    });
    if final_result_json
        .pointer("/extra/transcription_review/required")
        .and_then(Value::as_bool)
        == Some(true)
    {
        // Durable transcription starts are intentionally silent while the
        // local process is still running. Once the terminal result requests
        // host review, that stale start-time delivery intent must not suppress
        // the shared correction and artifact-delivery path.
        completed.delivery.intent =
            claw_core::capability_result::CapabilityDeliveryIntent::ModelSynthesis;
    } else if let Some(intent) = final_result_json
        .pointer("/extra/delivery/intent")
        .and_then(Value::as_str)
    {
        completed.delivery.intent = match intent {
            "artifact" => claw_core::capability_result::CapabilityDeliveryIntent::Artifact,
            "save_only" | "silent" => {
                claw_core::capability_result::CapabilityDeliveryIntent::Silent
            }
            _ => claw_core::capability_result::CapabilityDeliveryIntent::ModelSynthesis,
        };
    }
    if completed.validate().is_err() {
        return false;
    }
    *result = completed;
    true
}

pub(crate) fn selected_exact_machine_result(
    results: &[CapabilityResultEnvelope],
    selector: &str,
) -> Option<String> {
    results.iter().rev().find_map(|result| {
        if result.status != CapabilityResultStatus::Ok {
            return None;
        }
        selected_result_value(result, selector).and_then(exact_value_text)
    })
}

pub(crate) fn selected_result_value<'a>(
    result: &'a CapabilityResultEnvelope,
    selector: &str,
) -> Option<&'a Value> {
    let data = &result.data;
    let selector = selector.strip_prefix("data.").unwrap_or(selector);
    (selector == "data")
        .then_some(data)
        .or_else(|| structured_value_at_path(data, selector))
        .or_else(|| {
            data.get("extra")
                .and_then(|extra| structured_value_at_path(extra, selector))
        })
        .or_else(|| {
            data.get("output")
                .and_then(|output| structured_value_at_path(output, selector))
        })
        .or_else(|| {
            data.get("output")
                .and_then(|output| output.get("extra"))
                .and_then(|extra| structured_value_at_path(extra, selector))
        })
        .or_else(|| {
            (selector == "command_output")
                .then(|| data.get("output"))
                .flatten()
        })
}

pub(crate) fn selected_result_machine_value(
    result: &CapabilityResultEnvelope,
    selector: &str,
) -> Option<Value> {
    if selector == "status" {
        return Some(Value::String(result.status.as_token().to_string()));
    }
    if selector == "data" || selector.starts_with("data.") {
        return selected_result_value(result, selector).cloned();
    }
    let error_selector = selector.strip_prefix("error.")?;
    let error = serde_json::to_value(result.error.as_ref()?).ok()?;
    structured_value_at_path(&error, error_selector).cloned()
}

pub(crate) fn exact_object_projection_from_result(
    result: &CapabilityResultEnvelope,
    object: &serde_json::Map<String, Value>,
) -> Option<Value> {
    if object.is_empty() {
        return None;
    }
    let result = serde_json::to_value(result).ok()?;
    object
        .iter()
        .map(|(name, expected)| {
            observed_named_value(&result, name, expected, 0)
                .cloned()
                .map(|value| (name.clone(), value))
        })
        .collect::<Option<JsonMap<String, Value>>>()
        .map(Value::Object)
}

fn observed_named_value<'a>(
    current: &'a Value,
    field_name: &str,
    expected: &Value,
    depth: usize,
) -> Option<&'a Value> {
    if depth > 12 {
        return None;
    }
    let object = current.as_object()?;
    object.iter().find_map(|(name, value)| {
        if name == field_name && value == expected {
            Some(value)
        } else {
            observed_named_value(value, field_name, expected, depth + 1)
        }
    })
}

fn structured_value_at_path<'a>(value: &'a Value, selector: &str) -> Option<&'a Value> {
    selector.split('.').try_fold(value, |current, segment| {
        if let Some(object) = current.as_object() {
            return object.get(segment);
        }
        current.as_array()?.get(segment.parse::<usize>().ok()?)
    })
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

pub(crate) fn explicit_model_observation(value: &Value) -> Option<&Value> {
    value
        .get("model_observation")
        .or_else(|| {
            value
                .get("extra")
                .and_then(|extra| extra.get("model_observation"))
        })
        .or_else(|| {
            value
                .get("output")
                .and_then(|output| output.get("model_observation"))
        })
        .or_else(|| {
            value
                .get("output")
                .and_then(|output| output.get("extra"))
                .and_then(|extra| extra.get("model_observation"))
        })
        .filter(|observation| observation.is_object() || observation.is_array())
}

fn redact_for_model(value: Value) -> Value {
    let serialized = value.to_string();
    let redacted = crate::visible_text::sanitize_user_visible_text(&serialized);
    serde_json::from_str(&redacted).unwrap_or(Value::String(redacted))
}

fn structured_error(error: &str) -> StructuredError {
    if let Some(structured) = crate::skills::parse_structured_skill_error(error) {
        let extra = structured.extra.unwrap_or(Value::Null);
        let code = machine_string(&extra, &["error_code"])
            .filter(|value| is_machine_ref(value))
            .unwrap_or(structured.error_code.as_str())
            .to_string();
        let code = if is_machine_ref(&code) {
            code
        } else {
            "capability_execution_failed".to_string()
        };
        let message_key = machine_string(&extra, &["message_key"])
            .filter(|value| is_machine_ref(value))
            .unwrap_or(code.as_str())
            .to_string();
        let retryable = extra
            .get("retryable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        return StructuredError {
            code: code.clone(),
            message_key,
            retryable,
            details: json!({
                "structured_error": {
                    "skill": structured.skill,
                    "error_code": code,
                    "error_text": redact_for_model(Value::String(structured.error_text)),
                    "platform": structured.platform,
                    "manager_type": structured.manager_type,
                    "service_name": structured.service_name,
                    "extra": redact_for_model(extra),
                }
            }),
        };
    }
    let parsed = serde_json::from_str::<Value>(error.trim()).ok();
    let code = parsed
        .as_ref()
        .and_then(|value| machine_string(value, &["error_code"]))
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
    let Some(root) = extra.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    for object in [Some(root), root.get("extra").and_then(Value::as_object)]
        .into_iter()
        .flatten()
    {
        for key in ["artifacts", "artifact_refs", "output_artifact_refs"] {
            let Some(items) = object.get(key).and_then(Value::as_array) else {
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
    }
    refs
}

fn apply_result_metadata(
    envelope: &mut CapabilityResultEnvelope,
    output: Option<&Value>,
    extra: Option<&Value>,
    step_id: &str,
    content_trust: &str,
) {
    envelope.page = result_metadata_value(output, extra, "page")
        .filter(|value| value.is_object())
        .cloned()
        .map(redact_for_model);
    envelope.truncated = result_metadata_value(output, extra, "truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if envelope.truncated {
        if let Some(page) = envelope.page.as_ref() {
            if let Some(next_cursor) = page.get("next_cursor").filter(|value| !value.is_null()) {
                envelope.continuation = Some(Continuation {
                    kind: ContinuationKind::Opaque,
                    reference: Some(sanitized_reference(&format!("cursor:{next_cursor}"))),
                    poll_after_ms: None,
                    state: json!({
                        "cursor": page.get("cursor"),
                        "next_cursor": next_cursor,
                        "snapshot_sha256": page.get("snapshot_sha256"),
                    }),
                });
            }
        }
        let returned_count = envelope
            .page
            .as_ref()
            .and_then(|page| page.get("returned_count"))
            .and_then(Value::as_u64);
        let known_total = envelope
            .page
            .as_ref()
            .and_then(|page| page.get("total_count").or_else(|| page.get("known_total")))
            .and_then(Value::as_u64);
        envelope.completeness = Some(ResultCompleteness::partial(
            "bounded_result",
            returned_count,
            known_total,
            envelope.continuation.is_none() && envelope.artifacts.is_empty(),
        ));
    }
    envelope.effect = result_metadata_value(output, extra, "effect")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| is_machine_ref(value))
        .map(str::to_string);
    envelope.verification = result_metadata_value(output, extra, "verification")
        .filter(|value| value.is_object())
        .cloned()
        .map(redact_for_model)
        .unwrap_or_else(|| json!({}));
    envelope.provenance = json!({
        "source": "runtime_step",
        "step_id": machine_evidence_id(step_id),
        "content_trust": content_trust,
    });

    let retryable = result_metadata_value(output, extra, "retryable")
        .and_then(Value::as_bool)
        .or_else(|| envelope.error.as_ref().map(|error| error.retryable))
        .unwrap_or(false);
    let retry_class = result_metadata_value(output, extra, "retry_class")
        .or_else(|| result_metadata_value(output, extra, "error_code"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| is_machine_ref(value))
        .map(str::to_string);
    let retry_after_ms = result_metadata_value(output, extra, "retry_after_ms")
        .or_else(|| result_metadata_value(output, extra, "poll_after_ms"))
        .and_then(Value::as_u64)
        .or_else(|| {
            result_metadata_value(output, extra, "poll_after_seconds")
                .and_then(Value::as_u64)
                .map(|seconds| seconds.saturating_mul(1_000))
        });
    if retryable || retry_class.is_some() || retry_after_ms.is_some() {
        envelope.retry = Some(RetryDirective {
            retryable,
            class: retry_class,
            after_ms: retry_after_ms,
        });
    }
}

fn result_metadata_value<'a>(
    output: Option<&'a Value>,
    extra: Option<&'a Value>,
    key: &str,
) -> Option<&'a Value> {
    extra
        .and_then(|value| value.get(key))
        .or_else(|| output.and_then(|value| value.get(key)))
        .or_else(|| {
            output
                .and_then(|value| value.get("extra"))
                .and_then(|value| value.get(key))
        })
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
