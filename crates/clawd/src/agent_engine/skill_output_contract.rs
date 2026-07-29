use claw_core::skill_registry::OutputKind;
use regex::Regex;
use serde_json::{json, Map, Value};

use super::AppState;

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn matches_json_schema_type(value: &Value, expected_type: &str) -> Result<bool, String> {
    match expected_type {
        "null" => Ok(value.is_null()),
        "string" => Ok(value.is_string()),
        "object" => Ok(value.is_object()),
        "array" => Ok(value.is_array()),
        "boolean" => Ok(value.is_boolean()),
        "number" => Ok(value.is_number()),
        "integer" => Ok(value
            .as_number()
            .is_some_and(|number| number.is_i64() || number.is_u64())),
        unsupported => Err(format!("unsupported schema type `{unsupported}`")),
    }
}

fn schema_type_names(schema: &Value) -> Result<Vec<&str>, String> {
    match schema.get("type") {
        None => Ok(Vec::new()),
        Some(Value::String(value)) => Ok(vec![value.as_str()]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| "schema `type` array must contain only strings".to_string())
            })
            .collect(),
        Some(_) => Err("schema `type` must be a string or string array".to_string()),
    }
}

fn schema_allows_type(schema: &Value, expected: &str) -> bool {
    schema_type_names(schema)
        .map(|types| types.is_empty() || types.contains(&expected))
        .unwrap_or(false)
}

fn validate_composed_contract(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    if let Some(branches) = schema.get("allOf").and_then(Value::as_array) {
        for branch in branches {
            validate_json_contract_at(value, branch, path)?;
        }
    }

    if let Some(branches) = schema.get("anyOf").and_then(Value::as_array) {
        let errors = branches
            .iter()
            .filter_map(|branch| validate_json_contract_at(value, branch, path).err())
            .collect::<Vec<_>>();
        if errors.len() == branches.len() {
            return Err(format!(
                "{path}: did not match anyOf branches ({})",
                errors.join("; ")
            ));
        }
    }

    if let Some(branches) = schema.get("oneOf").and_then(Value::as_array) {
        let matches = branches
            .iter()
            .filter(|branch| validate_json_contract_at(value, branch, path).is_ok())
            .count();
        if matches != 1 {
            return Err(format!(
                "{path}: expected exactly one oneOf branch, matched {matches}"
            ));
        }
    }
    Ok(())
}

fn validate_string_contract(value: &str, schema: &Value, path: &str) -> Result<(), String> {
    let length = value.chars().count() as u64;
    if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64) {
        if length < minimum {
            return Err(format!("{path}: string length {length} is below {minimum}"));
        }
    }
    if let Some(maximum) = schema.get("maxLength").and_then(Value::as_u64) {
        if length > maximum {
            return Err(format!("{path}: string length {length} exceeds {maximum}"));
        }
    }
    if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
        let regex = Regex::new(pattern)
            .map_err(|error| format!("{path}: invalid schema pattern `{pattern}`: {error}"))?;
        if !regex.is_match(value) {
            return Err(format!("{path}: string does not match pattern `{pattern}`"));
        }
    }
    Ok(())
}

fn validate_number_contract(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    let Some(number) = value.as_f64() else {
        return Ok(());
    };
    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
        if number < minimum {
            return Err(format!("{path}: number {number} is below {minimum}"));
        }
    }
    if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64) {
        if number > maximum {
            return Err(format!("{path}: number {number} exceeds {maximum}"));
        }
    }
    if let Some(minimum) = schema.get("exclusiveMinimum").and_then(Value::as_f64) {
        if number <= minimum {
            return Err(format!(
                "{path}: number {number} must be greater than {minimum}"
            ));
        }
    }
    if let Some(maximum) = schema.get("exclusiveMaximum").and_then(Value::as_f64) {
        if number >= maximum {
            return Err(format!(
                "{path}: number {number} must be less than {maximum}"
            ));
        }
    }
    Ok(())
}

fn validate_array_contract(items: &[Value], schema: &Value, path: &str) -> Result<(), String> {
    let length = items.len() as u64;
    if let Some(minimum) = schema.get("minItems").and_then(Value::as_u64) {
        if length < minimum {
            return Err(format!("{path}: array length {length} is below {minimum}"));
        }
    }
    if let Some(maximum) = schema.get("maxItems").and_then(Value::as_u64) {
        if length > maximum {
            return Err(format!("{path}: array length {length} exceeds {maximum}"));
        }
    }
    if schema.get("uniqueItems").and_then(Value::as_bool) == Some(true) {
        for (index, item) in items.iter().enumerate() {
            if items[..index].contains(item) {
                return Err(format!("{path}[{index}]: duplicate array item"));
            }
        }
    }
    if let Some(item_schema) = schema.get("items") {
        for (index, item) in items.iter().enumerate() {
            validate_json_contract_at(item, item_schema, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn validate_object_contract(
    object: &Map<String, Value>,
    schema: &Value,
    path: &str,
) -> Result<(), String> {
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required {
            let key = key
                .as_str()
                .ok_or_else(|| format!("{path}: schema `required` must contain strings"))?;
            if !object.contains_key(key) {
                return Err(format!("{path}: missing required field `{key}`"));
            }
        }
    }

    let properties = schema.get("properties").and_then(Value::as_object);
    if let Some(properties) = properties {
        for (key, property_schema) in properties {
            if let Some(field_value) = object.get(key) {
                validate_json_contract_at(field_value, property_schema, &format!("{path}.{key}"))?;
            }
        }
    }

    if let Some(additional) = schema.get("additionalProperties") {
        for (key, field_value) in object {
            if properties.is_some_and(|known| known.contains_key(key)) {
                continue;
            }
            match additional {
                Value::Bool(true) => {}
                Value::Bool(false) => {
                    return Err(format!("{path}: unexpected field `{key}`"));
                }
                Value::Object(_) => {
                    validate_json_contract_at(field_value, additional, &format!("{path}.{key}"))?
                }
                _ => {
                    return Err(format!(
                        "{path}: schema `additionalProperties` must be boolean or object"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_json_contract_at(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    match schema {
        Value::Bool(true) => return Ok(()),
        Value::Bool(false) => return Err(format!("{path}: schema rejects every value")),
        Value::Object(_) => {}
        _ => return Err(format!("{path}: schema must be an object or boolean")),
    }

    validate_composed_contract(value, schema, path)?;

    if let Some(expected) = schema.get("const") {
        if value != expected {
            return Err(format!("{path}: value does not match schema const"));
        }
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            return Err(format!("{path}: value is not in schema enum"));
        }
    }

    let expected_types = schema_type_names(schema).map_err(|error| format!("{path}: {error}"))?;
    if !expected_types.is_empty() {
        let mut type_matches = false;
        for expected_type in &expected_types {
            type_matches |= matches_json_schema_type(value, expected_type)
                .map_err(|error| format!("{path}: {error}"))?;
        }
        if !type_matches {
            return Err(format!(
                "{path}: expected type {}, got `{}`",
                expected_types
                    .iter()
                    .map(|value| format!("`{value}`"))
                    .collect::<Vec<_>>()
                    .join(" or "),
                json_type_name(value)
            ));
        }
    }

    match value {
        Value::String(value) => validate_string_contract(value, schema, path)?,
        Value::Number(_) => validate_number_contract(value, schema, path)?,
        Value::Array(items) => validate_array_contract(items, schema, path)?,
        Value::Object(object) => validate_object_contract(object, schema, path)?,
        Value::Null | Value::Bool(_) => {}
    }
    Ok(())
}

pub(super) fn validate_json_contract(value: &Value, schema: &Value) -> Result<(), String> {
    validate_json_contract_at(value, schema, "$")
}

pub(super) fn normalized_output_candidate(
    output_kind: OutputKind,
    output: &str,
    structured_extra: Option<&Value>,
    schema: &Value,
) -> Value {
    let wraps_text_protocol = schema_allows_type(schema, "object")
        && schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| properties.contains_key("text"));
    if wraps_text_protocol {
        let mut candidate = Map::new();
        candidate.insert("text".to_string(), Value::String(output.to_string()));
        if let Some(extra) = structured_extra {
            candidate.insert("extra".to_string(), extra.clone());
        }
        return Value::Object(candidate);
    }

    let structured_schema = schema_allows_type(schema, "object")
        || schema_allows_type(schema, "array")
        || schema.get("properties").is_some()
        || schema.get("items").is_some();
    if output_kind != OutputKind::Text || structured_schema {
        if let Some(parsed) = crate::parse_llm_json_raw_or_any::<Value>(output) {
            return parsed;
        }
    }
    Value::String(output.to_string())
}

pub(super) fn validate_skill_output_contract(
    state: &AppState,
    normalized_skill: &str,
    output: &str,
    structured_extra: Option<&Value>,
) -> Result<(), String> {
    let Some((output_kind, schema)) = state.skill_output_contract(normalized_skill) else {
        return Ok(());
    };
    let candidate = normalized_output_candidate(output_kind, output, structured_extra, &schema);
    validate_json_contract(&candidate, &schema)
}

pub(super) fn enforce_skill_output_contract(
    state: &AppState,
    normalized_skill: &str,
    step: &mut crate::executor::StepExecutionResult,
    structured_extra: Option<&Value>,
) -> Option<String> {
    if step.status != crate::executor::StepExecutionStatus::Ok {
        return None;
    }
    let output = step.output.as_deref()?;
    let contract_error =
        validate_skill_output_contract(state, normalized_skill, output, structured_extra).err()?;
    let error = crate::skills::structured_skill_error_from_parts(
        normalized_skill,
        "output_contract_violation",
        "skill output does not match its declared contract",
        None,
        Some(json!({
            "schema_version": 1,
            "source_skill": normalized_skill,
            "status": "error",
            "error_code": "output_contract_violation",
            "message_key": "clawd.contract.output_contract_violation",
            "retryable": false,
            "contract_error": contract_error,
        })),
    );
    step.status = crate::executor::StepExecutionStatus::Error;
    step.output = None;
    step.error = Some(error);
    Some(contract_error)
}

pub(super) fn skill_input_contract_error(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> Option<String> {
    let schema = if let Some(tool) = state.mcp_tool(normalized_skill) {
        tool.input_schema
    } else {
        state.skill_manifest(normalized_skill)?.input_schema?
    };
    let error = validate_json_contract(args, &schema).err()?;
    Some(crate::skills::structured_skill_error_from_parts(
        normalized_skill,
        "contract_arg_rejected",
        "skill input does not match its declared contract",
        None,
        Some(json!({
            "schema_version": 1,
            "source_skill": normalized_skill,
            "status": "error",
            "error_code": "input_contract_violation",
            "message_key": "clawd.contract.input_contract_violation",
            "retryable": true,
            "contract_error": error,
        })),
    ))
}

#[cfg(test)]
#[path = "skill_output_contract_tests.rs"]
mod tests;
