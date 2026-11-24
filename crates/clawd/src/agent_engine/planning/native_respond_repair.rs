use super::*;

/// Narrow only the representation of already-empty, inactive response slots.
/// The fresh model response still goes through the normal action parser and verifier.
pub(super) fn inactive_response_collection_repair(
    error_code: &str,
    call: &ModelToolCall,
    schema: &Value,
    loop_state: Option<&LoopState>,
) -> Option<(Value, Value)> {
    if call.name != NATIVE_RESPOND_TOOL {
        return None;
    }
    let failed_field = match error_code {
        "native_respond_items_not_array" => "items",
        "native_respond_fields_not_array" => "fields",
        "native_respond_observed_fields_not_array" => "observed_fields",
        _ => return None,
    };
    let args = call.arguments.as_object()?;
    let shape = args.get("shape")?.as_str()?;
    let inactive: &[&str] = match shape {
        "free_text" => &["items", "fields", "observed_fields"],
        "list" => &["fields", "observed_fields"],
        "object" => &["items", "observed_fields"],
        "observed_object" => &["items", "fields"],
        _ => return None,
    };
    if !inactive.contains(&failed_field) || !args.get(failed_field)?.as_object()?.is_empty() {
        return None;
    }
    let mut canonical = call.clone();
    for field in inactive {
        match args.get(*field) {
            None => {}
            Some(Value::Array(value)) if value.is_empty() => {}
            Some(Value::Object(value)) if value.is_empty() => {}
            _ => return None,
        }
        canonical.arguments[*field] = json!([]);
    }
    // Do not discard nonempty data, repair counts, invent an observed value, or
    // reinterpret an invalid intent/relation/shape as an approved response.
    action_from_native_respond_call(&canonical, loop_state).ok()?;
    let mut projected = schema.clone();
    let properties = projected.get_mut("properties")?.as_object_mut()?;
    for field in inactive {
        properties.remove(*field)?;
    }
    for field in ["shape", "terminal_intent", "conversation_relation"] {
        properties
            .get_mut(field)?
            .as_object_mut()?
            .insert("enum".into(), json!([args.get(field)?.clone()]));
    }
    let required = projected.get_mut("required")?.as_array_mut()?;
    required.retain(|field| field.as_str().is_none_or(|name| !inactive.contains(&name)));
    // The selected shape is pinned for this retry. Its active payload remains
    // mandatory even though the initial multi-shape schema allows omission.
    let active_collection = match shape {
        "list" => Some("items"),
        "object" => Some("fields"),
        "observed_object" => Some("observed_fields"),
        _ => None,
    };
    if let Some(field) = active_collection {
        if !required.contains(&json!(field)) {
            required.push(json!(field));
        }
    }
    Some((
        projected,
        json!({
            "schema_version": 1,
            "kind": "inactive_empty_response_collections",
            "selected_shape": shape,
            "omitted_inactive_fields": inactive,
            "host_parser_default": [],
            "effect_scope": "answer_format_only",
            "semantic_control_source": "validated_rejected_call"
        }),
    ))
}

pub(super) fn annotate_response_collection_repair(signal: String, projection: Value) -> String {
    let Ok(mut signal) = serde_json::from_str::<Value>(&signal) else {
        return signal;
    };
    let shape = projection["selected_shape"].as_str().unwrap_or_default();
    if let Some(shapes) = signal
        .pointer_mut("/protocol_observation/argument_constraints/response_shape_contract")
        .and_then(Value::as_object_mut)
    {
        shapes.retain(|name, _| name.as_str() == shape);
        if let Some(fields) = shapes.get_mut(shape).and_then(Value::as_object_mut) {
            for field in projection["omitted_inactive_fields"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                fields.remove(field);
            }
        }
    }
    signal["protocol_observation"]["response_schema_projection"] = projection;
    signal.to_string()
}

#[cfg(test)]
#[path = "native_respond_repair_tests.rs"]
mod tests;
