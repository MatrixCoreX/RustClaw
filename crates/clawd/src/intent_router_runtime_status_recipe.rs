use super::*;

pub(super) fn execution_recipe_value_declares_command_payload(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Object(map)) => map.iter().any(|(key, value)| {
            (matches!(
                normalize_schema_token(key).as_str(),
                "command" | "commands" | "cmd" | "cmds" | "shell_command" | "shell_commands"
            ) && value_has_nonempty_scalar_text(value))
                || execution_recipe_value_declares_command_payload(Some(value))
        }),
        Some(Value::Array(items)) => items
            .iter()
            .any(|value| execution_recipe_value_declares_command_payload(Some(value))),
        _ => false,
    }
}

pub(super) fn execution_recipe_value_declares_scalar_runtime_tool_observation(
    value: Option<&Value>,
    output_contract: Option<&Value>,
) -> bool {
    if !output_contract_declares_scalar_locatorless_observation(output_contract) {
        return false;
    }
    let Some(map) = value.and_then(Value::as_object) else {
        return false;
    };
    if map
        .get("action")
        .or_else(|| map.get("operation"))
        .or_else(|| map.get("op"))
        .or_else(|| map.get("method"))
        .is_some_and(|value| {
            value_has_nonempty_scalar_text(value)
                && !value_has_schema_token(value, schema_token_is_runtime_status_operation)
        })
    {
        return false;
    }
    [
        "name",
        "tool",
        "tool_name",
        "runner",
        "runner_name",
        "skill",
        "skill_name",
        "capability",
        "capability_name",
    ]
    .iter()
    .any(|key| {
        map.get(*key).is_some_and(|value| {
            value_has_schema_token(value, schema_token_is_runtime_observation_tool)
        })
    })
}

pub(super) fn upsert_runtime_status_query_state_patch(
    obj: &mut serde_json::Map<String, Value>,
    kind: &'static str,
) {
    let value = obj
        .entry("state_patch".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    let Some(patch) = value.as_object_mut() else {
        return;
    };
    patch.insert(
        "runtime_status_query".to_string(),
        serde_json::json!({
            "kind": kind,
            "scope": "system"
        }),
    );
}

pub(super) fn scalar_runtime_status_kind_from_execution_recipe(
    value: Option<&Value>,
) -> Option<&'static str> {
    let mut tokens = Vec::new();
    collect_runtime_status_operation_tokens(value?, &mut tokens);
    tokens
        .into_iter()
        .find_map(|token| runtime_status_kind_for_operation_token(&token))
}

pub(super) fn scalar_runtime_status_kind_from_output_contract(
    value: Option<&Value>,
) -> Option<&'static str> {
    let contract = value?.as_object()?;
    let locator_kind = contract
        .get("locator_kind")
        .and_then(scalar_json_value_text)
        .map(|value| parse_output_locator_kind(&value))
        .unwrap_or_default();
    if locator_kind != OutputLocatorKind::CurrentWorkspace {
        return None;
    }
    let hint = contract
        .get("locator_hint")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    let hint_path = Path::new(hint.trim());
    if !hint_path.is_absolute() {
        return None;
    }
    std::env::var("HOME").ok().and_then(|home| {
        let home_path = Path::new(home.trim());
        (home_path.is_absolute() && hint_path == home_path).then_some("current_user")
    })
}

fn collect_runtime_status_operation_tokens(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let key = normalize_schema_token(key);
                if matches!(
                    key.as_str(),
                    "action"
                        | "operation"
                        | "op"
                        | "method"
                        | "intent"
                        | "query_kind"
                        | "field"
                        | "field_name"
                        | "target_field"
                ) {
                    if let Some(token) = scalar_json_value_text(value)
                        .map(|raw| normalize_schema_token(&raw))
                        .filter(|token| !token.is_empty())
                    {
                        out.push(token);
                    }
                }
                if matches!(
                    key.as_str(),
                    "arg"
                        | "args"
                        | "argument"
                        | "arguments"
                        | "param"
                        | "params"
                        | "parameter"
                        | "parameters"
                ) {
                    collect_runtime_status_arg_tokens(value, out);
                }
                collect_runtime_status_operation_tokens(value, out);
            }
        }
        Value::Array(items) => {
            for value in items {
                collect_runtime_status_operation_tokens(value, out);
            }
        }
        _ => {}
    }
}

fn collect_runtime_status_arg_tokens(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let token = normalize_schema_token(text);
            if runtime_status_kind_for_operation_token(&token).is_some() {
                out.push(token);
            }
        }
        Value::Array(items) => {
            for value in items {
                collect_runtime_status_arg_tokens(value, out);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_runtime_status_arg_tokens(value, out);
            }
        }
        _ => {}
    }
}

fn schema_token_is_runtime_status_operation(token: &str) -> bool {
    runtime_status_kind_for_operation_token(token).is_some()
}

fn runtime_status_kind_for_operation_token(token: &str) -> Option<&'static str> {
    match normalize_schema_token(token).as_str() {
        "whoami" | "current_user" | "current_username" | "os_user" | "system_user"
        | "runtime_user" => Some("current_user"),
        "hostname" | "host_name" | "current_hostname" | "current_host" | "machine_name" => {
            Some("host_name")
        }
        "kernel" | "kernel_name" | "kernel_release" | "os_kernel" | "system_kernel" | "uname"
        | "uname_r" => Some("kernel_release"),
        "pwd"
        | "cwd"
        | "current_working_directory"
        | "current_directory"
        | "process_cwd"
        | "current_process_cwd" => Some("current_working_directory"),
        _ => None,
    }
}

fn output_contract_declares_scalar_locatorless_observation(value: Option<&Value>) -> bool {
    let Some(contract) = value.and_then(Value::as_object) else {
        return false;
    };
    let response_shape = contract
        .get("response_shape")
        .and_then(scalar_json_value_text)
        .map(|value| parse_output_response_shape(&value))
        .unwrap_or_default();
    let locator_kind = contract
        .get("locator_kind")
        .and_then(scalar_json_value_text)
        .map(|value| parse_output_locator_kind(&value))
        .unwrap_or_default();
    let delivery_intent = contract
        .get("delivery_intent")
        .and_then(scalar_json_value_text)
        .map(|value| parse_output_delivery_intent(&value))
        .unwrap_or_default();
    let semantic_kind = contract
        .get("semantic_kind")
        .and_then(scalar_json_value_text)
        .map(|value| parse_output_semantic_kind(&value))
        .unwrap_or_default();
    matches!(response_shape, OutputResponseShape::Scalar)
        && contract
            .get("requires_content_evidence")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && !contract
            .get("delivery_required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && matches!(
            locator_kind,
            OutputLocatorKind::None | OutputLocatorKind::CurrentWorkspace
        )
        && matches!(delivery_intent, OutputDeliveryIntent::None)
        && matches!(
            semantic_kind,
            OutputSemanticKind::None | OutputSemanticKind::ScalarPathOnly
        )
}

fn schema_token_is_runtime_observation_tool(token: &str) -> bool {
    matches!(
        token,
        "system_basic" | "system" | "system_query" | "run_cmd"
    )
}
