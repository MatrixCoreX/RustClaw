use serde_json::{json, Value};

use crate::AppState;

pub(super) fn compose_answer_verifier_failure_user_message(
    state: &AppState,
    task: &crate::ClaimedTask,
    user_request: &str,
    err_text: &str,
) -> String {
    let language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let default_payload = answer_verifier_failure_default_payload(err_text);
    let missing_fields = answer_verifier_failure_missing_fields_text(err_text);
    crate::i18n_t_for_language_hint_with_default_vars(
        state,
        &language_hint,
        "clawd.msg.answer_verifier_required_evidence_block",
        &default_payload,
        &[("missing_evidence_fields", &missing_fields)],
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn answer_verifier_failure_default_payload(err_text: &str) -> String {
    let err = err_text.trim();
    if answer_text_is_machine_json_payload(err) {
        return err.to_string();
    }
    json!({
        "schema_version": 1,
        "message_key": "answer_verifier_required_evidence_block",
        "reason_code": "answer_verifier_required_evidence_block",
        "status_code": "answer_verifier_required_evidence_block",
        "failure_attribution": "answer_verifier_gap",
        "retryable": false,
        "missing_evidence_fields": answer_verifier_failure_missing_fields(err),
    })
    .to_string()
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn answer_verifier_failure_missing_fields_text(err_text: &str) -> String {
    let fields = answer_verifier_failure_missing_fields(err_text);
    if fields.is_empty() {
        "unknown".to_string()
    } else {
        fields.join(",")
    }
}

fn answer_verifier_failure_missing_fields(err_text: &str) -> Vec<String> {
    let err = err_text.trim();
    if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(err) {
        if let Some(fields) = obj
            .get("missing_evidence_fields")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|field| !field.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|fields| !fields.is_empty())
        {
            return fields;
        }
    }
    err.split_whitespace()
        .filter_map(|token| {
            let (key, value) = token.split_once('=')?;
            (key.trim() == "missing_evidence_fields")
                .then(|| value.trim())
                .filter(|value| !value.is_empty())
        })
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(super) fn machine_payload_observed_facts(payload_text: &str) -> Vec<String> {
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(payload_text.trim()) else {
        return Vec::new();
    };
    let mut facts = Vec::new();
    push_json_scalar_fact(&mut facts, &obj, "message_key");
    push_json_scalar_fact(&mut facts, &obj, "reason_code");
    push_json_scalar_fact(&mut facts, &obj, "status_code");
    push_json_scalar_fact(&mut facts, &obj, "failure_attribution");
    push_json_scalar_fact(&mut facts, &obj, "retryable");
    push_json_scalar_fact(&mut facts, &obj, "provider_error_class");
    push_json_scalar_fact(&mut facts, &obj, "external_provider_blocked");
    push_json_scalar_fact(&mut facts, &obj, "provider_http_status");
    push_json_scalar_fact(&mut facts, &obj, "provider_error_code");
    push_json_scalar_fact(&mut facts, &obj, "provider_error_type");
    push_json_scalar_fact(&mut facts, &obj, "provider_error_kind");
    push_json_scalar_fact(&mut facts, &obj, "raw_error_present");
    push_json_array_fact(&mut facts, &obj, "missing_evidence_fields");
    push_json_scalar_fact(&mut facts, &obj, "answer_incomplete_reason");
    push_json_scalar_fact(&mut facts, &obj, "confidence");
    facts
}

fn push_json_scalar_fact(facts: &mut Vec<String>, obj: &serde_json::Map<String, Value>, key: &str) {
    let Some(value) = obj.get(key) else {
        return;
    };
    let value = match value {
        Value::String(value) => value.trim().to_string(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        _ => String::new(),
    };
    if !value.is_empty() {
        facts.push(format!(
            "{key}: {}",
            crate::truncate_for_agent_trace(&value)
        ));
    }
}

fn push_json_array_fact(facts: &mut Vec<String>, obj: &serde_json::Map<String, Value>, key: &str) {
    let Some(values) = obj.get(key).and_then(Value::as_array) else {
        return;
    };
    let joined = values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(",");
    if !joined.is_empty() {
        facts.push(format!(
            "{key}: {}",
            crate::truncate_for_agent_trace(&joined)
        ));
    }
}

#[cfg(test)]
pub(super) fn answer_verifier_failure_observed_facts(err_text: &str) -> Vec<String> {
    let err = err_text.trim();
    let mut facts = machine_payload_observed_facts(err);
    if facts.is_empty() {
        facts = answer_verifier_machine_line_observed_facts(err);
    }
    if !facts.iter().any(|fact| fact.starts_with("message_key: ")) {
        facts.push("message_key: answer_verifier_required_evidence_block".to_string());
    }
    if !facts.iter().any(|fact| fact.starts_with("reason_code: ")) {
        facts.push("reason_code: answer_verifier_required_evidence_block".to_string());
    }
    if !facts.iter().any(|fact| fact.starts_with("status_code: ")) {
        facts.push("status_code: answer_verifier_required_evidence_block".to_string());
    }
    if !facts
        .iter()
        .any(|fact| fact.starts_with("failure_attribution: "))
    {
        facts.push("failure_attribution: answer_verifier_gap".to_string());
    }
    if !facts.iter().any(|fact| fact.starts_with("retryable: ")) {
        facts.push("retryable: false".to_string());
    }
    facts
}

#[cfg(test)]
fn answer_verifier_machine_line_observed_facts(line: &str) -> Vec<String> {
    line.split_whitespace()
        .filter_map(|token| {
            let (key, value) = token.split_once('=')?;
            let key = key.trim();
            let value = value.trim();
            (!key.is_empty()
                && !value.is_empty()
                && matches!(
                    key,
                    "message_key"
                        | "reason_code"
                        | "status_code"
                        | "failure_attribution"
                        | "retryable"
                        | "missing_evidence_fields"
                ))
            .then(|| format!("{key}: {}", crate::truncate_for_agent_trace(value)))
        })
        .collect()
}

#[cfg(test)]
pub(super) fn answer_verifier_failure_machine_line(err: &str) -> String {
    let mut parts = vec![
        "message_key=answer_verifier_required_evidence_block".to_string(),
        "reason_code=answer_verifier_required_evidence_block".to_string(),
    ];
    if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(err.trim()) {
        if let Some(fields) = obj
            .get("missing_evidence_fields")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty())
        {
            parts.push(format!("missing_evidence_fields={}", fields.join(",")));
        }
    }
    parts.join(" ")
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn answer_text_is_machine_json_payload(answer_text: &str) -> bool {
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(answer_text.trim()) else {
        return false;
    };
    [
        "message_key",
        "reason_code",
        "error_code",
        "missing_evidence_fields",
        "answer_incomplete_reason",
    ]
    .iter()
    .any(|key| obj.contains_key(*key))
}

fn answer_text_is_answer_verifier_machine_line(answer_text: &str) -> bool {
    let text = answer_text.trim();
    !text.is_empty()
        && (text.contains("message_key=answer_verifier_required_evidence_block")
            || text.contains("reason_code=answer_verifier_required_evidence_block"))
}

pub(super) fn answer_verifier_failure_needs_user_message(
    answer_text: &str,
    err_text: &str,
) -> bool {
    answer_text_is_machine_json_payload(answer_text)
        || answer_text_is_machine_json_payload(err_text)
        || answer_text_is_answer_verifier_machine_line(answer_text)
        || answer_text_is_answer_verifier_machine_line(err_text)
}
