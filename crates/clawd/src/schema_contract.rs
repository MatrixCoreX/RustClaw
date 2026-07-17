use serde_json::Value;

use crate::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnumConstraintViolation {
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

#[cfg(test)]
#[path = "schema_contract_tests.rs"]
mod tests;
