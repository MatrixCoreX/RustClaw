//! A deliberately bounded JSON Schema subset, versioned by the result contract.
use serde_json::Value;
const KEYS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "minimum",
    "maximum",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "description",
];
fn kind(value: &Value, name: &str) -> bool {
    match name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        _ => false,
    }
}
pub(super) fn supported(schema: &Value, depth: usize) -> bool {
    let Some(map) = schema.as_object() else {
        return false;
    };
    if depth > 8
        || schema.to_string().len() > 16 * 1024
        || map.keys().any(|key| !KEYS.contains(&key.as_str()))
    {
        return false;
    }
    let valid_kind = |name: &str| {
        [
            "null", "boolean", "string", "object", "array", "number", "integer",
        ]
        .contains(&name)
    };
    if let Some(types) = map.get("type") {
        if !types.as_str().is_some_and(valid_kind)
            && !types.as_array().is_some_and(|types| {
                !types.is_empty() && types.iter().all(|t| t.as_str().is_some_and(valid_kind))
            })
        {
            return false;
        }
    }
    if map.get("properties").is_some_and(|p| {
        !p.as_object()
            .is_some_and(|p| p.values().all(|s| supported(s, depth + 1)))
    }) {
        return false;
    }
    if map.get("items").is_some_and(|s| !supported(s, depth + 1)) {
        return false;
    }
    if map.get("required").is_some_and(|r| {
        !r.as_array()
            .is_some_and(|r| r.iter().all(|v| v.as_str().is_some_and(|s| !s.is_empty())))
    }) {
        return false;
    }
    if map
        .get("additionalProperties")
        .is_some_and(|v| !v.is_boolean())
    {
        return false;
    }
    if map
        .get("enum")
        .is_some_and(|v| !v.as_array().is_some_and(|v| !v.is_empty()))
    {
        return false;
    }
    for key in ["minLength", "maxLength", "minItems", "maxItems"] {
        if map.get(key).is_some_and(|v| v.as_u64().is_none()) {
            return false;
        }
    }
    for key in ["minimum", "maximum"] {
        if map.get(key).is_some_and(|v| v.as_f64().is_none()) {
            return false;
        }
    }
    true
}
pub(super) fn matches(schema: &Value, value: &Value) -> bool {
    if let Some(types) = schema.get("type") {
        let valid = types.as_str().is_some_and(|t| kind(value, t))
            || types.as_array().is_some_and(|ts| {
                ts.iter()
                    .any(|t| t.as_str().is_some_and(|t| kind(value, t)))
            });
        if !valid {
            return false;
        }
    }
    if schema
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|e| !e.contains(value))
    {
        return false;
    }
    if let Some(number) = value.as_f64() {
        if schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|min| number < min)
            || schema
                .get("maximum")
                .and_then(Value::as_f64)
                .is_some_and(|max| number > max)
        {
            return false;
        }
    }
    let bounds = |length: usize, min: &str, max: &str| {
        !schema
            .get(min)
            .and_then(Value::as_u64)
            .is_some_and(|n| (length as u64) < n)
            && !schema
                .get(max)
                .and_then(Value::as_u64)
                .is_some_and(|n| length as u64 > n)
    };
    if let Some(text) = value.as_str() {
        if !bounds(text.chars().count(), "minLength", "maxLength") {
            return false;
        }
    }
    if let Some(items) = value.as_array() {
        if !bounds(items.len(), "minItems", "maxItems")
            || schema
                .get("items")
                .is_some_and(|s| items.iter().any(|v| !matches(s, v)))
        {
            return false;
        }
    }
    if let Some(object) = value.as_object() {
        if schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|r| {
                r.iter()
                    .any(|k| !object.contains_key(k.as_str().unwrap_or("")))
            })
        {
            return false;
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, value) in object {
            match properties.and_then(|p| p.get(key)) {
                Some(s) if !matches(s, value) => return false,
                None if schema.get("additionalProperties").and_then(Value::as_bool)
                    == Some(false) =>
                {
                    return false
                }
                _ => {}
            }
        }
    }
    true
}
