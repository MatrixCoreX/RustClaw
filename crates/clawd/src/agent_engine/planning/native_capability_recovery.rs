use claw_core::model_turn::{ModelToolCall, ModelTurnResponse};
use serde_json::{json, Value};

use super::super::capability_discovery::{
    is_capability_group_token, RUNTIME_CAPABILITY_LOADER_TOOL,
};

pub(super) fn exact_schema_branch_for_call(schema: &Value, call: &ModelToolCall) -> Value {
    let capability = call.arguments.get("capability").and_then(Value::as_str);
    schema
        .get("oneOf")
        .and_then(Value::as_array)
        .and_then(|branches| {
            branches.iter().find(|branch| {
                let Some(capability) = capability else {
                    return false;
                };
                branch
                    .pointer("/properties/capability/enum")
                    .and_then(Value::as_array)
                    .is_some_and(|values| {
                        values
                            .iter()
                            .any(|value| value.as_str() == Some(capability))
                    })
            })
        })
        .cloned()
        .unwrap_or_else(|| schema.clone())
}

pub(super) fn normalize_exact_capability_group_repair(
    turn: &mut ModelTurnResponse,
    repair_signal: &str,
) -> Option<Vec<String>> {
    if turn.tool_calls.len() != 1 {
        return None;
    }
    let repair_observation = serde_json::from_str::<Value>(repair_signal).ok()?;
    if !matches!(
        repair_observation
            .pointer("/protocol_observation/error_code")
            .and_then(Value::as_str),
        Some("native_plan_unknown_tool" | "native_plan_required_companion_capability_missing")
    ) || repair_observation
        .pointer("/protocol_observation/tool_name")
        .and_then(Value::as_str)
        != Some(RUNTIME_CAPABILITY_LOADER_TOOL)
    {
        return None;
    }
    let suggested_groups = repair_observation
        .pointer("/protocol_observation/suggested_capability_groups")?
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if suggested_groups.is_empty()
        || suggested_groups
            .iter()
            .any(|group| !is_capability_group_token(group))
    {
        return None;
    }

    let call = empty_capability_loader_call(turn)?;
    call.arguments = json!({
        "op": "load_groups",
        "groups": suggested_groups,
    });
    Some(suggested_groups)
}

pub(super) fn normalize_empty_capability_loader_search(
    turn: &mut ModelTurnResponse,
    user_text: &str,
) -> Option<&'static str> {
    let (query, source) = if !turn.text.trim().is_empty() {
        (turn.text.trim().to_string(), "model_turn_text")
    } else if !user_text.trim().is_empty() {
        (user_text.trim().to_string(), "user_request")
    } else {
        return None;
    };
    let call = empty_capability_loader_call(turn)?;
    call.arguments = json!({"op": "search", "query": query});
    Some(source)
}

fn empty_capability_loader_call(turn: &mut ModelTurnResponse) -> Option<&mut ModelToolCall> {
    if turn.tool_calls.len() != 1 {
        return None;
    }
    let call = turn.tool_calls.first_mut()?;
    if call.name.as_str() != RUNTIME_CAPABILITY_LOADER_TOOL {
        return None;
    }
    let arguments = call.arguments.as_object()?;
    if arguments.keys().any(|key| key != "op" && key != "groups")
        || arguments
            .get("op")
            .and_then(Value::as_str)
            .is_some_and(|operation| operation != "load_groups")
    {
        return None;
    }
    match arguments.get("groups") {
        None => Some(call),
        Some(Value::Array(groups)) if groups.is_empty() => Some(call),
        Some(_) => None,
    }
}

#[cfg(test)]
#[path = "native_capability_recovery_tests.rs"]
mod tests;
