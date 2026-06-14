use serde_json::Value;

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn schema_type_matches(value: &Value, expected: &str) -> bool {
    match expected {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => value
            .as_f64()
            .map(|n| n.fract().abs() < f64::EPSILON)
            .unwrap_or(false),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    }
}

fn schema_declared_type_matches(value: &Value, schema: &Value) -> bool {
    match schema.get("type") {
        Some(Value::String(kind)) => schema_type_matches(value, kind),
        Some(Value::Array(kinds)) => kinds
            .iter()
            .filter_map(|kind| kind.as_str())
            .any(|kind| schema_type_matches(value, kind)),
        Some(_) => false,
        None => true,
    }
}

fn schema_expected_types(schema: &Value) -> Option<String> {
    match schema.get("type") {
        Some(Value::String(kind)) => Some(kind.clone()),
        Some(Value::Array(kinds)) => Some(
            kinds
                .iter()
                .filter_map(|kind| kind.as_str())
                .collect::<Vec<_>>()
                .join("|"),
        ),
        _ => None,
    }
}

fn schema_ref_target<'a>(schema_root: &'a Value, raw_ref: &str) -> Option<&'a Value> {
    let pointer = raw_ref.strip_prefix('#')?;
    schema_root.pointer(pointer)
}

fn schema_path_key(path: &str, key: &str) -> String {
    if path == "$" {
        format!("$.{key}")
    } else {
        format!("{path}.{key}")
    }
}

fn schema_path_index(path: &str, index: usize) -> String {
    format!("{path}[{index}]")
}

pub(super) fn validate_schema_value(
    schema_root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    if let Some(raw_ref) = schema.get("$ref").and_then(|v| v.as_str()) {
        match schema_ref_target(schema_root, raw_ref) {
            Some(target) => validate_schema_value(schema_root, target, value, path, errors),
            None => errors.push(format!("{path}: unresolved schema ref `{raw_ref}`")),
        }
        return;
    }

    if let Some(branches) = schema.get("oneOf").and_then(|v| v.as_array()) {
        let mut matched = false;
        for branch in branches {
            let mut branch_errors = Vec::new();
            validate_schema_value(schema_root, branch, value, path, &mut branch_errors);
            if branch_errors.is_empty() {
                matched = true;
                break;
            }
        }
        if !matched {
            errors.push(format!(
                "{path}: does not match any allowed schema variant (got {})",
                value_type_name(value)
            ));
        }
        return;
    }

    if !schema_declared_type_matches(value, schema) {
        if let Some(expected) = schema_expected_types(schema) {
            errors.push(format!(
                "{path}: expected type {expected}, got {}",
                value_type_name(value)
            ));
        }
        return;
    }

    if let Some(enum_values) = schema.get("enum").and_then(|v| v.as_array()) {
        if !enum_values.iter().any(|allowed| allowed == value) {
            let allowed = enum_values
                .iter()
                .map(|candidate| candidate.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            errors.push(format!(
                "{path}: expected one of [{allowed}], got {}",
                value
            ));
            return;
        }
    }

    if let Some(const_value) = schema.get("const") {
        if const_value != value {
            errors.push(format!(
                "{path}: expected const {}, got {}",
                const_value, value
            ));
            return;
        }
    }

    if let Some(minimum) = schema.get("minimum").and_then(|v| v.as_f64()) {
        if value.as_f64().map(|n| n < minimum).unwrap_or(false) {
            errors.push(format!("{path}: expected >= {minimum}, got {value}"));
        }
    }
    if let Some(maximum) = schema.get("maximum").and_then(|v| v.as_f64()) {
        if value.as_f64().map(|n| n > maximum).unwrap_or(false) {
            errors.push(format!("{path}: expected <= {maximum}, got {value}"));
        }
    }

    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        if let Some(obj) = value.as_object() {
            for field in required.iter().filter_map(|v| v.as_str()) {
                if !obj.contains_key(field) {
                    errors.push(format!("{path}: missing required field `{field}`"));
                }
            }
        }
    }

    if let Some(obj) = value.as_object() {
        let properties = schema.get("properties").and_then(|v| v.as_object());
        let additional = schema.get("additionalProperties");
        for (key, field_value) in obj {
            if let Some(field_schema) = properties.and_then(|props| props.get(key)) {
                validate_schema_value(
                    schema_root,
                    field_schema,
                    field_value,
                    &schema_path_key(path, key),
                    errors,
                );
                continue;
            }
            match additional {
                Some(Value::Bool(false)) => {
                    errors.push(format!("{}: unexpected property `{}`", path, key));
                }
                Some(extra_schema @ Value::Object(_)) => validate_schema_value(
                    schema_root,
                    extra_schema,
                    field_value,
                    &schema_path_key(path, key),
                    errors,
                ),
                _ => {}
            }
        }
    }

    if let Some(arr) = value.as_array() {
        if let Some(items_schema) = schema.get("items") {
            for (index, item) in arr.iter().enumerate() {
                validate_schema_value(
                    schema_root,
                    items_schema,
                    item,
                    &schema_path_index(path, index),
                    errors,
                );
            }
        }
    }
}
