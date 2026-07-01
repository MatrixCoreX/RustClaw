use serde_json::Value;

use super::{
    normalize_schema_token, scalar_json_value_text, schema_key_is_structured_scalar_field_selector,
};

pub(super) fn output_recipe_value_declares_execution(value: Option<&Value>) -> bool {
    if execution_recipe_value_explicitly_declares_none_kind(value) {
        return false;
    }
    execution_recipe_value_has_text(value, schema_text_declares_execution_recipe)
}

pub(super) fn execution_recipe_value_declares_structured_read_observation(
    value: Option<&Value>,
) -> bool {
    execution_recipe_value_structured_locator_hint(value).is_some()
        && (execution_recipe_value_declares_structured_read_action(value)
            || execution_recipe_value_declares_structured_scalar_field_request(value))
}

fn execution_recipe_value_declares_structured_read_action(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            let key = normalize_schema_token(key);
            (matches!(
                key.as_str(),
                "kind" | "action" | "operation" | "op" | "tool"
            ) && value_has_schema_token(value, schema_token_is_read_observation_action))
                || execution_recipe_value_declares_structured_read_action(Some(value))
        }),
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_declares_structured_read_action(Some(value))),
        _ => false,
    }
}

pub(super) fn execution_recipe_value_declares_structured_scalar_extraction(
    value: Option<&Value>,
) -> bool {
    if execution_recipe_value_declares_structured_scalar_field_request(value) {
        return true;
    }
    match value {
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            let key = normalize_schema_token(key);
            (matches!(
                key.as_str(),
                "action"
                    | "kind"
                    | "operation"
                    | "op"
                    | "method"
                    | "extract"
                    | "extraction"
                    | "extractor"
                    | "schema"
                    | "output"
                    | "content"
            ) && value_has_schema_token(value, schema_token_is_scalar_extraction_action))
                || execution_recipe_value_declares_structured_scalar_extraction(Some(value))
        }),
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_declares_structured_scalar_extraction(Some(value))),
        _ => false,
    }
}

fn execution_recipe_value_declares_structured_scalar_field_request(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            let key = normalize_schema_token(key);
            (schema_key_is_structured_scalar_field_selector(&key)
                && value_has_nonempty_scalar_text(value))
                || execution_recipe_value_declares_structured_scalar_field_request(Some(value))
        }),
        Some(Value::Array(items)) => items.iter().any(|value| {
            execution_recipe_value_declares_structured_scalar_field_request(Some(value))
        }),
        _ => false,
    }
}

pub(super) fn execution_recipe_value_declares_package_detect_manager_capability(
    value: Option<&Value>,
) -> bool {
    match value {
        Some(Value::Object(map)) => {
            for (key, value) in map {
                let key = normalize_schema_token(key);
                if matches!(
                    key.as_str(),
                    "capability" | "capability_name" | "planner_capability"
                ) && value_has_schema_token(
                    value,
                    schema_token_is_package_manager_detect_capability,
                ) {
                    return true;
                }
                if execution_recipe_value_declares_package_detect_manager_capability(Some(value)) {
                    return true;
                }
            }
            false
        }
        Some(Value::Array(items)) => items.iter().any(|value| {
            execution_recipe_value_declares_package_detect_manager_capability(Some(value))
        }),
        _ => false,
    }
}

pub(super) fn execution_recipe_value_declares_service_status_observation(
    value: Option<&Value>,
) -> bool {
    match value {
        Some(Value::Object(map)) => {
            let mut has_service_status_tool = false;
            let mut has_service_status_action = false;
            for (key, value) in map {
                let key = normalize_schema_token(key);
                if matches!(
                    key.as_str(),
                    "capability" | "capability_name" | "planner_capability"
                ) && value_has_schema_token(value, schema_token_is_service_status_capability)
                {
                    return true;
                }
                if matches!(
                    key.as_str(),
                    "name" | "skill" | "skill_name" | "runner" | "runner_name" | "tool"
                ) && value_has_schema_token(value, schema_token_is_service_status_tool)
                {
                    has_service_status_tool = true;
                    if value_has_schema_token(value, schema_token_is_standalone_service_status_tool)
                    {
                        return true;
                    }
                }
                if matches!(
                    key.as_str(),
                    "name" | "skill" | "skill_name" | "runner" | "runner_name" | "tool"
                ) && value_has_schema_token(value, schema_token_is_port_status_tool)
                {
                    return true;
                }
                if matches!(
                    key.as_str(),
                    "action" | "operation" | "op" | "method" | "intent"
                ) && value_has_schema_token(value, schema_token_is_service_status_action)
                {
                    has_service_status_action = true;
                }
                if execution_recipe_value_declares_service_status_observation(Some(value)) {
                    return true;
                }
            }
            has_service_status_tool && has_service_status_action
        }
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_declares_service_status_observation(Some(value))),
        _ => false,
    }
}

pub(super) fn execution_recipe_value_declares_health_check_observation(
    value: Option<&Value>,
) -> bool {
    match value {
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            let key = normalize_schema_token(key);
            (matches!(
                key.as_str(),
                "name" | "skill" | "skill_name" | "runner" | "runner_name" | "tool"
            ) && value_has_schema_token(value, schema_token_is_standalone_service_status_tool))
                || (matches!(
                    key.as_str(),
                    "capability" | "capability_name" | "planner_capability"
                ) && value_has_schema_token(
                    value,
                    schema_token_is_standalone_service_status_tool,
                ))
                || execution_recipe_value_declares_health_check_observation(Some(value))
        }),
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_declares_health_check_observation(Some(value))),
        _ => false,
    }
}

fn schema_token_is_service_status_tool(token: &str) -> bool {
    matches!(token, "health_check" | "process_basic" | "service_control")
}

fn schema_token_is_standalone_service_status_tool(token: &str) -> bool {
    matches!(token, "health_check")
}

fn schema_token_is_port_status_tool(token: &str) -> bool {
    matches!(token, "netstat_ss_ports")
}

fn schema_token_is_service_status_action(token: &str) -> bool {
    matches!(
        token,
        "status" | "health_check" | "ps" | "port_list" | "diagnose_runtime"
    )
}

fn schema_token_is_service_status_capability(token: &str) -> bool {
    matches!(
        token,
        "health_check"
            | "service_status"
            | "service.status"
            | "runtime_health"
            | "process.ps"
            | "process.port_list"
    )
}

fn schema_token_is_package_manager_detect_capability(token: &str) -> bool {
    matches!(
        token,
        "package.detect_manager" | "capability_ref=package.detect_manager"
    )
}

pub(super) fn value_has_schema_token(value: &Value, predicate: fn(&str) -> bool) -> bool {
    match value {
        Value::String(raw) => predicate(&normalize_schema_token(raw)),
        Value::Array(items) => items
            .iter()
            .any(|value| value_has_schema_token(value, predicate)),
        Value::Object(map) => map
            .values()
            .any(|value| value_has_schema_token(value, predicate)),
        other => scalar_json_value_text(other)
            .is_some_and(|text| predicate(&normalize_schema_token(&text))),
    }
}

fn schema_token_is_read_observation_action(token: &str) -> bool {
    matches!(
        token,
        "read"
            | "file_read"
            | "read_file"
            | "read_text"
            | "read_range"
            | "read_text_range"
            | "file_read_title"
            | "file_read_extract_title"
            | "read_file_title"
            | "read_file_extract_title"
            | "read_file_and_extract_title"
    )
}

fn schema_token_is_scalar_extraction_action(token: &str) -> bool {
    matches!(
        token,
        "extract_scalar"
            | "scalar"
            | "file_read_title"
            | "file_read_extract_title"
            | "read_file_title"
            | "read_file_extract_title"
            | "read_file_and_extract_title"
            | "extract_title"
            | "title"
            | "title_only"
            | "first_heading_line"
            | "markdown_heading"
    )
}

pub(super) fn execution_recipe_value_structured_locator_hint(
    value: Option<&Value>,
) -> Option<String> {
    let mut hints = Vec::new();
    collect_execution_recipe_locator_hints(value?, &mut hints);
    hints.sort();
    hints.dedup();
    (hints.len() == 1).then(|| hints.remove(0))
}

fn collect_execution_recipe_locator_hints(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let normalized_key = normalize_schema_token(key);
                if matches!(
                    normalized_key.as_str(),
                    "target"
                        | "path"
                        | "file_path"
                        | "target_path"
                        | "input_path"
                        | "source_path"
                        | "read_path"
                        | "filepath"
                ) {
                    if let Some(hint) = scalar_json_value_text(value)
                        .and_then(|text| {
                            crate::intent::locator_extractor::extract_explicit_locator_for_fallback(
                                &text,
                            )
                            .map(|locator| locator.locator_hint)
                        })
                        .filter(|hint| !hint.trim().is_empty())
                    {
                        out.push(hint);
                    }
                }
                collect_execution_recipe_locator_hints(value, out);
            }
        }
        Value::Array(items) => {
            for value in items {
                collect_execution_recipe_locator_hints(value, out);
            }
        }
        _ => {}
    }
}

fn execution_recipe_value_explicitly_declares_none_kind(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_object)
        .and_then(|map| map.get("kind"))
        .and_then(scalar_json_value_text)
        .is_some_and(|kind| {
            matches!(
                crate::execution_recipe::parse_execution_recipe_kind_text(&kind),
                crate::execution_recipe::ExecutionRecipeKind::None
            )
        })
}

pub(super) fn execution_recipe_value_has_text(
    value: Option<&Value>,
    predicate: fn(&str) -> bool,
) -> bool {
    match value {
        Some(Value::String(raw)) => predicate(raw),
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_has_text(Some(value), predicate)),
        Some(Value::Object(map)) => map
            .iter()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "target_scope"
                        | "turn_type"
                        | "target_task_policy"
                        | "should_interrupt_active_run"
                        | "state_patch"
                        | "attachment_processing_required"
                )
            })
            .any(|(_, value)| execution_recipe_value_has_text(Some(value), predicate)),
        Some(other) => scalar_json_value_text(other).is_some_and(|text| predicate(&text)),
        None => false,
    }
}

pub(super) fn execution_recipe_value_locator_hint(value: Option<&Value>) -> Option<String> {
    let map = value?.as_object()?;
    for key in [
        "path",
        "file_path",
        "target_path",
        "input_path",
        "source_path",
        "read_path",
        "filepath",
    ] {
        let Some(hint) = map
            .get(key)
            .and_then(scalar_json_value_text)
            .map(|hint| hint.trim().to_string())
            .filter(|hint| !hint.is_empty())
        else {
            continue;
        };
        return Some(hint);
    }
    None
}

pub(super) fn normalizer_object_declares_tool_action_payload(
    obj: &serde_json::Map<String, Value>,
) -> bool {
    let has_action_args_payload = obj
        .get("action")
        .and_then(Value::as_str)
        .is_some_and(|action| !action.trim().is_empty())
        && obj.get("args").is_some_and(value_has_nonempty_scalar_text);
    if has_action_args_payload {
        return true;
    }

    obj.get("steps")
        .and_then(Value::as_array)
        .is_some_and(|steps| {
            steps.iter().any(|step| {
                let Some(step) = step.as_object() else {
                    return false;
                };
                step.get("type")
                    .or_else(|| step.get("action"))
                    .and_then(Value::as_str)
                    .is_some_and(|kind| !kind.trim().is_empty())
                    && (step.get("args").is_some_and(value_has_nonempty_scalar_text)
                        || step.get("tool").is_some_and(value_has_nonempty_scalar_text)
                        || step
                            .get("skill")
                            .is_some_and(value_has_nonempty_scalar_text))
            })
        })
}

pub(super) fn value_has_nonempty_scalar_text(value: &Value) -> bool {
    match value {
        Value::String(raw) => !raw.trim().is_empty(),
        Value::Array(items) => items.iter().any(value_has_nonempty_scalar_text),
        Value::Object(map) => map.values().any(value_has_nonempty_scalar_text),
        other => scalar_json_value_text(other).is_some_and(|text| !text.trim().is_empty()),
    }
}

pub(super) fn schema_text_declares_execution_recipe(raw: &str) -> bool {
    !matches!(
        crate::execution_recipe::parse_execution_recipe_kind_text(raw),
        crate::execution_recipe::ExecutionRecipeKind::None
    ) || !matches!(
        crate::execution_recipe::parse_execution_recipe_profile_text(raw),
        crate::execution_recipe::ExecutionRecipeProfile::None
    )
}
