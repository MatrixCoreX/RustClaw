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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequiredConstraintViolation {
    pub(crate) field: String,
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
    let mut violations = Vec::new();
    collect_enum_constraint_violations(input_schema, args, "", &mut violations);
    violations.sort_by(|left, right| left.field.cmp(&right.field));
    violations
}

fn collect_enum_constraint_violations(
    schema: &Value,
    value: &Value,
    path: &str,
    violations: &mut Vec<EnumConstraintViolation>,
) {
    if let (Some(properties), Some(object)) = (
        schema.get("properties").and_then(Value::as_object),
        value.as_object(),
    ) {
        for (field, field_value) in object {
            let Some(field_schema) = properties.get(field) else {
                continue;
            };
            let field_path = object_field_path(path, field);
            if field_schema
                .get("enum")
                .and_then(Value::as_array)
                .is_some_and(|allowed| !allowed.iter().any(|candidate| candidate == field_value))
            {
                violations.push(EnumConstraintViolation {
                    field: field_path.clone(),
                });
            }
            collect_enum_constraint_violations(field_schema, field_value, &field_path, violations);
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (index, item) in array.iter().enumerate() {
            collect_enum_constraint_violations(
                items,
                item,
                &array_item_path(path, index),
                violations,
            );
        }
    }
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
    let mut violations = Vec::new();
    collect_type_constraint_violations(input_schema, args, "", &mut violations);
    violations.sort_by(|left, right| left.field.cmp(&right.field));
    violations
}

fn collect_type_constraint_violations(
    schema: &Value,
    value: &Value,
    path: &str,
    violations: &mut Vec<TypeConstraintViolation>,
) {
    if let (Some(properties), Some(object)) = (
        schema.get("properties").and_then(Value::as_object),
        value.as_object(),
    ) {
        for (field, field_value) in object {
            let Some(field_schema) = properties.get(field) else {
                continue;
            };
            let field_path = object_field_path(path, field);
            if let Some(expected) = expected_type_description(field_schema) {
                if !schema_accepts_value_type(field_schema, field_value) {
                    violations.push(TypeConstraintViolation {
                        field: field_path,
                        expected,
                    });
                    continue;
                }
            }
            collect_type_constraint_violations(field_schema, field_value, &field_path, violations);
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (index, item) in array.iter().enumerate() {
            let item_path = array_item_path(path, index);
            if let Some(expected) = expected_type_description(items) {
                if !schema_accepts_value_type(items, item) {
                    violations.push(TypeConstraintViolation {
                        field: item_path,
                        expected,
                    });
                    continue;
                }
            }
            collect_type_constraint_violations(items, item, &item_path, violations);
        }
    }
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
    let mut violations = Vec::new();
    collect_unknown_argument_violations(input_schema, args, "", true, &mut violations);
    violations.sort_by(|left, right| left.field.cmp(&right.field));
    violations
}

fn collect_unknown_argument_violations(
    schema: &Value,
    value: &Value,
    path: &str,
    root: bool,
    violations: &mut Vec<UnknownArgumentViolation>,
) {
    if let (Some(properties), Some(object)) = (
        schema.get("properties").and_then(Value::as_object),
        value.as_object(),
    ) {
        let open = schema
            .get("additionalProperties")
            .is_some_and(|value| value == &Value::Bool(true) || value.is_object());
        for (field, field_value) in object {
            if let Some(field_schema) = properties.get(field) {
                collect_unknown_argument_violations(
                    field_schema,
                    field_value,
                    &object_field_path(path, field),
                    false,
                    violations,
                );
            } else if !open
                && !(root && field == "action")
                && !(root && field.starts_with("_clawd_"))
            {
                violations.push(UnknownArgumentViolation {
                    field: object_field_path(path, field),
                });
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (index, item) in array.iter().enumerate() {
            collect_unknown_argument_violations(
                items,
                item,
                &array_item_path(path, index),
                false,
                violations,
            );
        }
    }
}

pub(crate) fn executable_nested_required_constraint_violations(
    state: &AppState,
    executable: &str,
    args: &Value,
) -> Vec<RequiredConstraintViolation> {
    let input_schema = state
        .mcp_tool(executable)
        .map(|tool| tool.input_schema)
        .or_else(|| {
            state
                .skill_manifest(executable)
                .and_then(|manifest| manifest.input_schema)
        });
    let mut violations = Vec::new();
    if let Some(schema) = input_schema.as_ref() {
        collect_required_constraint_violations(schema, args, "", 0, &mut violations);
    }
    violations.sort_by(|left, right| left.field.cmp(&right.field));
    violations
}

fn collect_required_constraint_violations(
    schema: &Value,
    value: &Value,
    path: &str,
    depth: usize,
    violations: &mut Vec<RequiredConstraintViolation>,
) {
    if let Some(object) = value.as_object() {
        if depth > 0 {
            for required in schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if !object.contains_key(required) {
                    violations.push(RequiredConstraintViolation {
                        field: object_field_path(path, required),
                    });
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (field, field_value) in object {
                if let Some(field_schema) = properties.get(field) {
                    collect_required_constraint_violations(
                        field_schema,
                        field_value,
                        &object_field_path(path, field),
                        depth + 1,
                        violations,
                    );
                }
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (index, item) in array.iter().enumerate() {
            collect_required_constraint_violations(
                items,
                item,
                &array_item_path(path, index),
                depth + 1,
                violations,
            );
        }
    }
}

fn object_field_path(path: &str, field: &str) -> String {
    if path.is_empty() {
        field.to_string()
    } else {
        format!("{path}.{field}")
    }
}

fn array_item_path(path: &str, index: usize) -> String {
    format!("{path}[{index}]")
}

#[cfg(test)]
#[path = "schema_contract_tests.rs"]
mod tests;
