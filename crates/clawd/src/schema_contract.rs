use serde_json::Value;

use crate::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnumConstraintViolation {
    pub(crate) field: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnknownArgumentViolation {
    pub(crate) field: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypeConstraintViolation {
    pub(crate) field: String,
    pub(crate) expected: String,
}

pub(crate) fn executable_enum_violations(
    state: &AppState,
    executable: &str,
    args: &Value,
) -> Vec<EnumConstraintViolation> {
    let input_schema = state
        .mcp_tool(executable)
        .map(|tool| tool.input_schema)
        .or_else(|| {
            state
                .skill_manifest(executable)
                .and_then(|manifest| manifest.input_schema)
        });
    input_schema
        .as_ref()
        .map(|schema| enum_constraint_violations(schema, args))
        .unwrap_or_default()
}

pub(crate) fn enum_constraint_violations(
    input_schema: &Value,
    args: &Value,
) -> Vec<EnumConstraintViolation> {
    let Some(properties) = input_schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let Some(args) = args.as_object() else {
        return Vec::new();
    };

    let mut violations = Vec::new();
    for (field, value) in args {
        let Some(allowed) = properties
            .get(field)
            .and_then(|property| property.get("enum"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        if !allowed.iter().any(|candidate| candidate == value) {
            violations.push(EnumConstraintViolation {
                field: field.clone(),
            });
        }
    }
    violations.sort_by(|left, right| left.field.cmp(&right.field));
    violations
}

pub(crate) fn executable_unknown_argument_violations(
    state: &AppState,
    executable: &str,
    args: &Value,
) -> Vec<UnknownArgumentViolation> {
    if let Some(tool) = state.mcp_tool(executable) {
        return unknown_argument_violations(&tool.input_schema, args);
    }
    let Some(manifest) = state.skill_manifest(executable) else {
        return Vec::new();
    };
    let Some(input_schema) = manifest.input_schema.as_ref() else {
        return Vec::new();
    };
    let mut violations = unknown_argument_violations(input_schema, args);
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let declared_by_action = claw_core::skill_registry::select_planner_capability_mapping(
        &manifest.planner_capabilities,
        action,
    )
    .map(|mapping| {
        mapping
            .required
            .iter()
            .chain(&mapping.optional)
            .flat_map(|field| field.split('|'))
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .collect::<std::collections::HashSet<_>>()
    })
    .unwrap_or_default();
    violations.retain(|violation| !declared_by_action.contains(violation.field.as_str()));
    violations
}

pub(crate) fn executable_type_constraint_violations(
    state: &AppState,
    executable: &str,
    args: &Value,
) -> Vec<TypeConstraintViolation> {
    let input_schema = state
        .mcp_tool(executable)
        .map(|tool| tool.input_schema)
        .or_else(|| {
            state
                .skill_manifest(executable)
                .and_then(|manifest| manifest.input_schema)
        });
    input_schema
        .as_ref()
        .map(|schema| type_constraint_violations(schema, args))
        .unwrap_or_default()
}

pub(crate) fn type_constraint_violations(
    input_schema: &Value,
    args: &Value,
) -> Vec<TypeConstraintViolation> {
    let Some(properties) = input_schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let Some(args) = args.as_object() else {
        return Vec::new();
    };

    let mut violations = Vec::new();
    for (field, value) in args {
        let Some(field_schema) = properties.get(field) else {
            continue;
        };
        let Some(expected) = expected_type_description(field_schema) else {
            continue;
        };
        if !schema_accepts_value_type(field_schema, value) {
            violations.push(TypeConstraintViolation {
                field: field.clone(),
                expected,
            });
        }
    }
    violations.sort_by(|left, right| left.field.cmp(&right.field));
    violations
}

fn schema_accepts_value_type(schema: &Value, value: &Value) -> bool {
    for branch_key in ["anyOf", "oneOf"] {
        if let Some(branches) = schema.get(branch_key).and_then(Value::as_array) {
            return branches
                .iter()
                .any(|branch| schema_accepts_value_type(branch, value));
        }
    }
    match schema.get("type") {
        Some(Value::String(expected)) => value_matches_type(value, expected),
        Some(Value::Array(expected)) => expected
            .iter()
            .filter_map(Value::as_str)
            .any(|kind| value_matches_type(value, kind)),
        Some(_) => false,
        None => true,
    }
}

fn value_matches_type(value: &Value, expected: &str) -> bool {
    match expected {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => value
            .as_f64()
            .map(|number| number.fract().abs() < f64::EPSILON)
            .unwrap_or(false),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    }
}

fn expected_type_description(schema: &Value) -> Option<String> {
    for branch_key in ["anyOf", "oneOf"] {
        if let Some(branches) = schema.get(branch_key).and_then(Value::as_array) {
            let mut expected = branches
                .iter()
                .filter_map(expected_type_description)
                .collect::<Vec<_>>();
            expected.sort();
            expected.dedup();
            return (!expected.is_empty()).then(|| expected.join("|"));
        }
    }
    match schema.get("type") {
        Some(Value::String(expected)) => Some(expected.clone()),
        Some(Value::Array(expected)) => {
            let mut expected = expected
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            expected.sort();
            expected.dedup();
            (!expected.is_empty()).then(|| expected.join("|"))
        }
        _ => None,
    }
}

pub(crate) fn unknown_argument_violations(
    input_schema: &Value,
    args: &Value,
) -> Vec<UnknownArgumentViolation> {
    if input_schema
        .get("additionalProperties")
        .is_some_and(|value| value == &Value::Bool(true) || value.is_object())
    {
        return Vec::new();
    }
    let Some(properties) = input_schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let Some(args) = args.as_object() else {
        return Vec::new();
    };

    let mut violations = args
        .keys()
        .filter(|field| {
            field.as_str() != "action"
                && !field.starts_with("_clawd_")
                && !properties.contains_key(*field)
        })
        .map(|field| UnknownArgumentViolation {
            field: field.clone(),
        })
        .collect::<Vec<_>>();
    violations.sort_by(|left, right| left.field.cmp(&right.field));
    violations
}

#[cfg(test)]
#[path = "schema_contract_tests.rs"]
mod tests;
