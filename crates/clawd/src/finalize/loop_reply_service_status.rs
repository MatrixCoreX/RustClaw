use crate::agent_engine::LoopState;

pub(super) fn service_status_system_basic_info_answer(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<String> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ServiceStatus {
        return None;
    }
    let value = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.is_ok() && step.skill == "system_basic")
        .and_then(|step| step.output.as_deref())
        .and_then(system_basic_info_payload_from_output)?;
    let mut fields = Vec::new();
    push_system_basic_info_field(&mut fields, &value, "hostname");
    push_system_basic_info_field(&mut fields, &value, "os");
    push_system_basic_info_field(&mut fields, &value, "arch");
    push_system_basic_info_field(&mut fields, &value, "current_user");
    push_system_basic_info_field(&mut fields, &value, "pid");
    push_system_basic_info_field(&mut fields, &value, "cwd");
    push_system_basic_info_field(&mut fields, &value, "workspace_root");
    push_system_basic_info_field(&mut fields, &value, "uptime_seconds");
    push_system_basic_info_field(&mut fields, &value, "process_rss_bytes");
    (!fields.is_empty()).then(|| fields.join("\n"))
}

fn system_basic_info_payload_from_output(output: &str) -> Option<serde_json::Value> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    if json_value_has_system_basic_info_shape(&value) {
        return Some(value);
    }
    if let Some(extra) = value
        .get("extra")
        .filter(|extra| json_value_has_system_basic_info_shape(extra))
    {
        return Some(extra.clone());
    }
    value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .filter(json_value_has_system_basic_info_shape)
}

fn json_value_has_system_basic_info_shape(value: &serde_json::Value) -> bool {
    value
        .get("hostname")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        && value
            .get("os")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
}

fn push_system_basic_info_field(
    fields: &mut Vec<String>,
    value: &serde_json::Value,
    field_name: &'static str,
) {
    let Some(field_value) = value.get(field_name).and_then(json_scalar_to_string) else {
        return;
    };
    if field_value.trim().is_empty() {
        return;
    }
    fields.push(format!("{field_name}={field_value}"));
}

fn json_scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}
