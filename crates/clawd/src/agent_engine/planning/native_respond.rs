use super::*;

pub(super) fn action_from_native_respond_call(
    call: &ModelToolCall,
    loop_state: Option<&LoopState>,
) -> Result<AgentAction, String> {
    let arguments = call
        .arguments
        .as_object()
        .ok_or_else(|| "native_respond_arguments_not_object".to_string())?;
    validate_native_respond_control_fields(arguments)?;
    let shape = arguments
        .get("shape")
        .and_then(Value::as_str)
        .ok_or_else(|| "native_respond_shape_missing".to_string())?;
    let content = match arguments.get("content") {
        Some(value) => value
            .as_str()
            .ok_or_else(|| "native_respond_content_missing".to_string())?,
        None => "",
    };
    let items = match arguments.get("items") {
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| "native_respond_items_not_array".to_string())?,
        None => &[],
    };
    let exact_item_count = match arguments.get("exact_item_count") {
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value <= MAX_NATIVE_RESPONSE_ITEMS)
                .ok_or_else(|| "native_respond_exact_item_count_invalid".to_string())?,
        ),
        None => None,
    };
    let fields = match arguments.get("fields") {
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| "native_respond_fields_not_array".to_string())?,
        None => &[],
    };
    let observed_fields = match arguments.get("observed_fields") {
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| "native_respond_observed_fields_not_array".to_string())?,
        None => &[],
    };
    let exact_field_count = match arguments.get("exact_field_count") {
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value <= MAX_NATIVE_RESPONSE_FIELDS)
                .ok_or_else(|| "native_respond_exact_field_count_invalid".to_string())?,
        ),
        None => None,
    };

    match shape {
        "free_text" => {
            if content.trim().is_empty() {
                return Err("native_respond_free_text_empty".to_string());
            }
            if !items.is_empty()
                || exact_item_count.unwrap_or(0) != 0
                || !fields.is_empty()
                || !observed_fields.is_empty()
                || exact_field_count.unwrap_or(0) != 0
            {
                return Err("native_respond_free_text_contract_mismatch".to_string());
            }
            Ok(AgentAction::Respond {
                content: content.trim().to_string(),
            })
        }
        "list" => {
            let exact_item_count = exact_item_count
                .ok_or_else(|| "native_respond_exact_item_count_invalid".to_string())?;
            if !content.trim().is_empty() {
                return Err("native_respond_list_content_not_empty".to_string());
            }
            if !fields.is_empty()
                || !observed_fields.is_empty()
                || exact_field_count.unwrap_or(0) != 0
            {
                return Err("native_respond_list_fields_not_empty".to_string());
            }
            if exact_item_count == 0 || items.len() != exact_item_count {
                return Err("native_respond_list_count_mismatch".to_string());
            }
            let items = items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::trim)
                        .filter(|item| {
                            !item.is_empty() && !item.contains('\r') && !item.contains('\n')
                        })
                        .map(ToString::to_string)
                        .ok_or_else(|| "native_respond_list_item_invalid".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let content = items
                .iter()
                .enumerate()
                .map(|(index, item)| format!("{}. {item}", index + 1))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(AgentAction::Respond { content })
        }
        "object" => {
            let exact_field_count = exact_field_count
                .ok_or_else(|| "native_respond_exact_field_count_invalid".to_string())?;
            if !items.is_empty() || exact_item_count.unwrap_or(0) != 0 {
                return Err("native_respond_object_non_field_payload".to_string());
            }
            if !observed_fields.is_empty() {
                return Err("native_respond_object_observed_fields_not_empty".to_string());
            }
            if exact_field_count == 0 || fields.len() != exact_field_count {
                return Err("native_respond_object_count_mismatch".to_string());
            }
            let mut object = Map::new();
            for field in fields {
                let field = field
                    .as_object()
                    .ok_or_else(|| "native_respond_object_field_invalid".to_string())?;
                let name = field
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|name| {
                        !name.is_empty()
                            && name.len() <= 128
                            && !name.contains('\r')
                            && !name.contains('\n')
                    })
                    .ok_or_else(|| "native_respond_object_field_name_invalid".to_string())?;
                let value_json = field
                    .get("value_json")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty() && value.len() <= 65536)
                    .ok_or_else(|| "native_respond_object_field_value_invalid".to_string())?;
                let value = parse_native_authored_json_value(value_json)?;
                if object.insert(name.to_string(), value).is_some() {
                    return Err("native_respond_object_field_duplicate".to_string());
                }
            }
            let object = Value::Object(object);
            if !content.trim().is_empty() {
                let redundant_content: Value = serde_json::from_str(content.trim())
                    .map_err(|_| "native_respond_object_non_field_payload".to_string())?;
                if redundant_content != object {
                    return Err("native_respond_object_non_field_payload".to_string());
                }
            }
            let object =
                project_exact_object_from_observations(&object, loop_state).unwrap_or(object);
            let content = serde_json::to_string(&object)
                .map_err(|_| "native_respond_object_serialize_failed".to_string())?;
            Ok(AgentAction::Respond { content })
        }
        "observed_object" => {
            if !content.trim().is_empty()
                || !items.is_empty()
                || exact_item_count.unwrap_or(0) != 0
                || !fields.is_empty()
            {
                return Err("native_respond_observed_object_non_reference_payload".to_string());
            }
            let exact_field_count = match exact_field_count {
                None | Some(0) => observed_fields.len(),
                Some(count) => count,
            };
            if exact_field_count == 0 || observed_fields.len() != exact_field_count {
                return Err("native_respond_observed_object_count_mismatch".to_string());
            }
            let loop_state =
                loop_state.ok_or_else(|| "native_respond_observation_state_missing".to_string())?;
            let mut object = Map::new();
            for field in observed_fields {
                let field = field
                    .as_object()
                    .ok_or_else(|| "native_respond_observed_field_invalid".to_string())?;
                let name = machine_response_field_name(field.get("name"))?;
                let capability = machine_observation_reference(
                    field.get("capability"),
                    "native_respond_observed_capability_invalid",
                )?;
                let path = machine_observation_reference(
                    field.get("path"),
                    "native_respond_observed_path_invalid",
                )?;
                let result = loop_state
                    .capability_results
                    .iter()
                    .rev()
                    .find(|result| {
                        result.capability == capability
                            && observed_projection_path_allowed(result.status, path)
                    })
                    .ok_or_else(|| {
                        "native_respond_observed_capability_result_missing".to_string()
                    })?;
                let value = crate::capability_result::selected_result_machine_value(result, path)
                    .ok_or_else(|| "native_respond_observed_path_missing".to_string())?;
                if object.insert(name.to_string(), value).is_some() {
                    return Err("native_respond_object_field_duplicate".to_string());
                }
            }
            let content = serde_json::to_string(&Value::Object(object))
                .map_err(|_| "native_respond_object_serialize_failed".to_string())?;
            Ok(AgentAction::Respond { content })
        }
        _ => Err("native_respond_shape_unsupported".to_string()),
    }
}

const NATIVE_RESPOND_CLARIFY_FIELDS: [&str; 5] = [
    "clarify_reason_code",
    "missing_slot",
    "message_key",
    "field_path",
    "locator_kind",
];

fn native_respond_control_string<'a>(
    arguments: &'a Map<String, Value>,
    key: &str,
    max_len: usize,
) -> Result<Option<&'a str>, String> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .map(str::trim)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= max_len
                && !value.contains('\r')
                && !value.contains('\n')
        })
        .ok_or_else(|| format!("native_respond_{key}_invalid"))?;
    Ok(Some(value))
}

fn validate_native_respond_control_fields(arguments: &Map<String, Value>) -> Result<(), String> {
    let terminal_intent = native_respond_control_string(arguments, "terminal_intent", 32)?
        .ok_or_else(|| "native_respond_terminal_intent_missing".to_string())?;
    if !matches!(terminal_intent, "answer" | "clarify") {
        return Err("native_respond_terminal_intent_invalid".to_string());
    }

    let limits = [128, 128, 192, 256, 64];
    let clarify_values = NATIVE_RESPOND_CLARIFY_FIELDS
        .iter()
        .zip(limits)
        .map(|(key, max_len)| native_respond_control_string(arguments, key, max_len))
        .collect::<Result<Vec<_>, _>>()?;
    if terminal_intent == "clarify" && clarify_values[1].is_none() {
        return Err("native_respond_clarify_missing_slot_required".to_string());
    }
    if terminal_intent == "answer" && clarify_values.iter().any(Option::is_some) {
        return Err("native_respond_answer_clarify_fields_forbidden".to_string());
    }
    Ok(())
}

pub(super) fn preserve_native_respond_control_fields(
    turn: &ModelTurnResponse,
    plan_result: &mut crate::PlanResult,
) {
    let Some(call) = turn
        .tool_calls
        .iter()
        .find(|call| call.name == NATIVE_RESPOND_TOOL)
    else {
        return;
    };
    let Some(arguments) = call.arguments.as_object() else {
        return;
    };
    let Some(step) = plan_result
        .steps
        .iter_mut()
        .find(|step| step.action_type == "respond")
    else {
        return;
    };
    let Some(step_args) = step.args.as_object_mut() else {
        return;
    };
    for key in std::iter::once("terminal_intent").chain(NATIVE_RESPOND_CLARIFY_FIELDS) {
        if let Some(value) = arguments.get(key) {
            step_args.insert(key.to_string(), value.clone());
        }
    }
}

fn parse_native_authored_json_value(value_json: &str) -> Result<Value, String> {
    if let Ok(value) = serde_json::from_str(value_json) {
        return Ok(value);
    }

    let trimmed = value_json.trim();
    if let Some(without_extra_quote) = trimmed.strip_suffix('"') {
        if matches!(without_extra_quote.as_bytes().first(), Some(b'[' | b'{')) {
            if let Ok(value) = serde_json::from_str(without_extra_quote) {
                return Ok(value);
            }
        }
    }

    let plain_scalar = !trimmed.is_empty()
        && !trimmed.contains(['\r', '\n'])
        && !matches!(trimmed.as_bytes().first(), Some(b'"' | b'[' | b'{'));
    if plain_scalar {
        return Ok(Value::String(trimmed.to_string()));
    }

    Err("native_respond_object_field_json_invalid".to_string())
}

fn project_exact_object_from_observations(
    object: &Value,
    loop_state: Option<&LoopState>,
) -> Option<Value> {
    let Some(object) = object.as_object().filter(|object| !object.is_empty()) else {
        return None;
    };
    let Some(loop_state) = loop_state else {
        return None;
    };
    loop_state
        .capability_results
        .iter()
        .rev()
        .find_map(|result| {
            crate::capability_result::exact_object_projection_from_result(result, object)
        })
}

fn observed_projection_path_allowed(
    status: claw_core::capability_result::CapabilityResultStatus,
    path: &str,
) -> bool {
    match status {
        claw_core::capability_result::CapabilityResultStatus::Ok => {
            path == "data" || path.starts_with("data.")
        }
        claw_core::capability_result::CapabilityResultStatus::Error => {
            path == "status" || path.starts_with("error.")
        }
        claw_core::capability_result::CapabilityResultStatus::Waiting
        | claw_core::capability_result::CapabilityResultStatus::NeedsUser => false,
    }
}

fn machine_response_field_name(value: Option<&Value>) -> Result<&str, String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| {
            !name.is_empty() && name.len() <= 128 && !name.contains('\r') && !name.contains('\n')
        })
        .ok_or_else(|| "native_respond_object_field_name_invalid".to_string())
}

fn machine_observation_reference<'a>(
    value: Option<&'a Value>,
    error_code: &str,
) -> Result<&'a str, String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            value.len() <= MAX_NATIVE_RESPONSE_SOURCE_PATH
                && claw_core::capability_result::is_machine_ref(value)
        })
        .ok_or_else(|| error_code.to_string())
}
