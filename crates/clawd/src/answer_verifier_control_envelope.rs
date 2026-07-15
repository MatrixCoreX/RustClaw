use serde_json::{Map, Value};

pub(super) fn control_machine_envelope_answer_can_skip_answer_verifier(
    candidate_answer: &str,
) -> bool {
    let candidate = candidate_answer.trim();
    if control_machine_envelope_json_is_structured_projection(candidate) {
        return true;
    }
    candidate
        .lines()
        .map(str::trim)
        .any(control_machine_envelope_json_is_structured_projection)
}

fn control_machine_envelope_json_is_structured_projection(candidate_answer: &str) -> bool {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(candidate_answer.trim()) else {
        return false;
    };
    if object.get("output_format").and_then(Value::as_str) != Some("machine_json") {
        return false;
    }
    if object.get("owner_layer").and_then(Value::as_str) != Some("agent_loop_control") {
        return false;
    }
    let Some(required_fields) = object
        .get("required_machine_fields")
        .and_then(Value::as_array)
    else {
        return false;
    };
    if required_fields.is_empty() {
        return false;
    }
    let Some(decision_envelope) = object.get("decision_envelope").and_then(Value::as_object) else {
        return false;
    };
    if decision_envelope.is_empty() {
        return false;
    }
    required_fields
        .iter()
        .all(|field| control_machine_envelope_required_field_is_satisfied(field, decision_envelope))
}

fn control_machine_envelope_required_field_is_satisfied(
    field: &Value,
    decision_envelope: &Map<String, Value>,
) -> bool {
    let Some(field) = field.as_str() else {
        return false;
    };
    if field == "decision_envelope" {
        return true;
    }
    let Some(key) = field.strip_prefix("decision_envelope.") else {
        return false;
    };
    decision_envelope
        .get(key)
        .is_some_and(machine_projection_value_has_payload)
}

fn machine_projection_value_has_payload(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => machine_projection_string_has_payload(text),
        Value::Array(items) => {
            !items.is_empty() && items.iter().all(machine_projection_value_has_payload)
        }
        Value::Object(object) => {
            !object.is_empty() && object.values().all(machine_projection_value_has_payload)
        }
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn machine_projection_string_has_payload(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && !trimmed.contains("{{")
        && !matches!(trimmed, "<missing>" | "not_observed" | "null")
}
