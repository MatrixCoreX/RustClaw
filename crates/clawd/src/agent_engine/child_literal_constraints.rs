//! Compile explicit structured literals before freezing the ordinary narrowing-only grant.
use serde_json::{json, Value};

pub(super) fn normalize(value: &Value) -> Result<Value, Vec<Value>> {
    if value.to_string().len() > 16 * 1024 {
        return Err(vec![issue("", "argument_constraint_size_limit")]);
    }
    let mut normalized = value.clone();
    if let Some(rules) = normalized.as_array_mut() {
        if rules.len() > 32 {
            return Err(vec![issue("", "argument_constraint_count_limit")]);
        }
        for (i, rule) in rules.iter_mut().enumerate() {
            if let Some(schemas) = rule.get_mut("allowed_args").and_then(Value::as_array_mut) {
                if schemas.len() > 8 {
                    return Err(vec![issue(
                        &format!("/{i}/allowed_args"),
                        "argument_schema_count_limit",
                    )]);
                }
                for (j, schema) in schemas.iter_mut().enumerate() {
                    normalize_schema(schema, &format!("/{i}/allowed_args/{j}"), 0)
                        .map_err(|error| vec![error])?;
                }
            }
        }
    }
    if normalized.to_string().len() > 16 * 1024 {
        return Err(vec![issue("", "argument_constraint_size_limit")]);
    }
    Ok(normalized)
}

fn issue(path: &str, reason: &str) -> Value {
    json!({"path":path.chars().take(1024).collect::<String>(),"reason":reason})
}

fn normalize_schema(schema: &mut Value, path: &str, depth: usize) -> Result<(), Value> {
    if depth > 8 {
        return Err(issue(path, "argument_schema_depth_limit"));
    }
    if path.len() > 1024 {
        return Err(issue(path, "argument_schema_path_limit"));
    }
    let Some(object) = schema.as_object_mut() else {
        return Ok(());
    };
    if let Some(literal) = object.get("literal_text") {
        // Preserve every other narrowing constraint; never replace an existing value grant.
        if object.get("type").and_then(Value::as_str) != Some("string")
            || object.contains_key("enum")
        {
            return Err(issue(path, "literal_text_requires_unbound_string_schema"));
        }
        let text = materialize(literal, &format!("{path}/literal_text"))?;
        object.remove("literal_text");
        object.insert("enum".into(), json!([text]));
    }
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        for (key, property) in properties {
            let escaped = key.replace('~', "~0").replace('/', "~1");
            normalize_schema(property, &format!("{path}/properties/{escaped}"), depth + 1)?;
        }
    }
    if let Some(items) = object.get_mut("items") {
        normalize_schema(items, &format!("{path}/items"), depth + 1)?;
    }
    Ok(())
}

fn materialize(value: &Value, path: &str) -> Result<String, Value> {
    let Some(object) = value.as_object() else {
        return Err(issue(path, "literal_text_object_required"));
    };
    if object.len() != 4
        || object.keys().any(|key| {
            !["schema_version", "lines", "line_ending", "trailing_newline"].contains(&key.as_str())
        })
    {
        return Err(issue(path, "literal_text_fields_invalid"));
    }
    if object.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err(issue(
            &format!("{path}/schema_version"),
            "literal_text_version_unsupported",
        ));
    }
    let ending = match object.get("line_ending").and_then(Value::as_str) {
        Some("lf") => "\n",
        Some("crlf") => "\r\n",
        Some("cr") => "\r",
        _ => {
            return Err(issue(
                &format!("{path}/line_ending"),
                "literal_text_line_ending_invalid",
            ))
        }
    };
    let Some(trailing) = object.get("trailing_newline").and_then(Value::as_bool) else {
        return Err(issue(
            &format!("{path}/trailing_newline"),
            "literal_text_boolean_required",
        ));
    };
    let Some(lines) = object
        .get("lines")
        .and_then(Value::as_array)
        .filter(|lines| !lines.is_empty() && lines.len() <= 512)
    else {
        return Err(issue(
            &format!("{path}/lines"),
            "literal_text_bounded_lines_required",
        ));
    };
    let lines = lines
        .iter()
        .map(|line| {
            line.as_str()
                .filter(|line| !line.contains('\r') && !line.contains('\n'))
                .ok_or_else(|| {
                    issue(
                        &format!("{path}/lines"),
                        "literal_text_single_line_strings_required",
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut text = lines.join(ending);
    if trailing {
        text.push_str(ending);
    }
    Ok(text)
}

#[cfg(test)]
#[path = "child_literal_constraints_tests.rs"]
mod tests;
