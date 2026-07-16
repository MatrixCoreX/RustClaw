use crate::agent_engine::LoopState;

pub(crate) fn service_status_system_basic_info_answer(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> Option<String> {
    if !crate::finalize::route_matches_service_status_output_contract(route) {
        return service_control_status_observed_answer_without_route_contract(loop_state);
    }
    let system_health_only = service_status_selector_is_system_health_only(route);
    if should_preserve_health_check_summary_synthesis(route, loop_state, system_health_only) {
        return None;
    }
    if !system_health_only {
        if should_preserve_service_control_one_sentence(route, loop_state) {
            return None;
        }
        if let Some(answer) = service_control_status_observed_answer(loop_state) {
            return Some(answer);
        }
    }
    if !system_health_only {
        if let Some(value) = loop_state
            .executed_step_results
            .iter()
            .rev()
            .find(|step| step.is_ok() && step.skill == "system_basic")
            .and_then(|step| step.output.as_deref())
            .and_then(system_basic_info_payload_from_output)
        {
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
            if !fields.is_empty() {
                return Some(fields.join("\n"));
            }
        }
    }

    let value = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.is_ok() && step.skill == "health_check")
        .and_then(|step| step.output.as_deref())
        .and_then(health_check_payload_from_output)?;
    let mut fields = Vec::new();
    push_json_path_field(&mut fields, &value, "system_health.os_family");
    push_json_path_field(&mut fields, &value, "system_health.kernel_release");
    push_json_path_field(&mut fields, &value, "system_health.arch");
    push_json_path_field(&mut fields, &value, "system_health.hostname");
    push_json_path_field(&mut fields, &value, "system_health.service_manager");
    push_json_path_field(&mut fields, &value, "system_health.cpu_count");
    push_json_path_field(&mut fields, &value, "system_health.memory_total_bytes");
    push_json_path_field(&mut fields, &value, "system_health.memory_available_bytes");
    push_json_path_field(&mut fields, &value, "system_health.disk_root_total_bytes");
    push_json_path_field(
        &mut fields,
        &value,
        "system_health.disk_root_available_bytes",
    );
    push_json_path_field(&mut fields, &value, "system_health.load_avg_1m");
    push_json_path_field(&mut fields, &value, "system_health.load_avg_5m");
    push_json_path_field(&mut fields, &value, "system_health.load_avg_15m");
    push_json_path_field(&mut fields, &value, "system_health.uptime_seconds");
    push_json_path_field(&mut fields, &value, "system_health.warnings");
    if !system_health_only {
        push_json_path_field(&mut fields, &value, "clawd_process_count");
        push_json_path_field(&mut fields, &value, "clawd_health_port_open");
        push_json_path_field(&mut fields, &value, "telegramd_process_count");
    }
    (!fields.is_empty()).then(|| fields.join("\n"))
}

fn should_preserve_health_check_summary_synthesis(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    system_health_only: bool,
) -> bool {
    if system_health_only {
        return false;
    }
    if matches!(
        route.response_shape,
        crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
    ) {
        return false;
    }
    if !has_successful_health_check_observation(loop_state) {
        return false;
    }
    loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .is_some_and(structurally_publishable_health_check_summary)
}

fn has_successful_health_check_observation(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().rev().any(|step| {
        step.is_ok()
            && step.skill == "health_check"
            && step
                .output
                .as_deref()
                .and_then(health_check_payload_from_output)
                .is_some()
    })
}

fn structurally_publishable_health_check_summary(candidate: &str) -> bool {
    let candidate = candidate.trim();
    !candidate.is_empty()
        && !candidate.starts_with('{')
        && !candidate.starts_with('[')
        && !looks_like_machine_field_dump(candidate)
}

fn looks_like_machine_field_dump(candidate: &str) -> bool {
    let mut non_empty_lines = 0usize;
    let mut assignment_lines = 0usize;
    for line in candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        non_empty_lines += 1;
        if line.contains('=')
            && line
                .split_once('=')
                .is_some_and(|(left, _)| looks_like_machine_field_path(left.trim()))
        {
            assignment_lines += 1;
        }
    }
    non_empty_lines > 0 && assignment_lines >= 2 && assignment_lines * 2 >= non_empty_lines
}

fn looks_like_machine_field_path(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
        && value
            .chars()
            .any(|ch| matches!(ch, '_' | '.') || ch.is_ascii_lowercase())
}

fn should_preserve_service_control_one_sentence(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> bool {
    if route.response_shape != crate::OutputResponseShape::OneSentence {
        return false;
    }
    if !has_successful_service_control_status_observation(loop_state) {
        return false;
    }
    loop_state
        .delivery_messages
        .last()
        .or(loop_state.last_user_visible_respond.as_ref())
        .is_some_and(|candidate| structurally_publishable_one_sentence(candidate))
}

fn has_successful_service_control_status_observation(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().rev().any(|step| {
        step.is_ok()
            && step.skill == "service_control"
            && step
                .output
                .as_deref()
                .and_then(service_control_payload_from_output)
                .is_some()
    })
}

fn structurally_publishable_one_sentence(candidate: &str) -> bool {
    let candidate = candidate.trim();
    !candidate.is_empty()
        && !candidate.contains('\n')
        && !candidate.starts_with('{')
        && !candidate.starts_with('[')
        && !candidate.contains('=')
}

fn service_control_status_observed_answer(loop_state: &LoopState) -> Option<String> {
    let value = latest_service_control_status_payload(loop_state)?;
    service_control_status_observed_answer_from_payload(&value)
}

fn service_control_status_observed_answer_without_route_contract(
    loop_state: &LoopState,
) -> Option<String> {
    let value = latest_service_control_status_payload(loop_state)?;
    let answer = service_control_status_observed_answer_from_payload(&value)?;
    if should_preserve_current_service_control_delivery_without_route_contract(
        loop_state, &value, &answer,
    ) {
        return None;
    }
    Some(answer)
}

fn latest_service_control_status_payload(loop_state: &LoopState) -> Option<serde_json::Value> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.is_ok() && step.skill == "service_control")
        .and_then(|step| step.output.as_deref())
        .and_then(service_control_payload_from_output)
}

fn service_control_status_observed_answer_from_payload(
    value: &serde_json::Value,
) -> Option<String> {
    let mut fields = Vec::new();
    push_json_path_alias_field(&mut fields, &value, "target", "target");
    push_json_path_alias_field(&mut fields, &value, "service_name", "target");
    push_json_path_field(&mut fields, &value, "service_name");
    push_json_path_field(&mut fields, &value, "post_state");
    push_json_path_field(&mut fields, &value, "pre_state");
    push_json_path_field(&mut fields, &value, "status");
    push_json_path_field(&mut fields, &value, "verified");
    push_json_path_field(&mut fields, &value, "manager_type");
    if fields.is_empty() {
        return None;
    }
    fields.push("source=service_control".to_string());
    Some(fields.join(" "))
}

fn should_preserve_current_service_control_delivery_without_route_contract(
    loop_state: &LoopState,
    value: &serde_json::Value,
    observed_answer: &str,
) -> bool {
    let Some(current) = latest_service_control_delivery(loop_state) else {
        return false;
    };
    let current = current.trim();
    if current.is_empty()
        || current == observed_answer.trim()
        || current_matches_service_control_observed_scalar(current, value)
    {
        return false;
    }
    true
}

fn latest_service_control_delivery(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .delivery_messages
        .last()
        .map(String::as_str)
        .or(loop_state.last_user_visible_respond.as_deref())
        .or(loop_state.last_publishable_synthesis_output.as_deref())
}

fn current_matches_service_control_observed_scalar(
    current: &str,
    value: &serde_json::Value,
) -> bool {
    let observed_fields = [
        "summary",
        "post_state",
        "pre_state",
        "status",
        "service_name",
        "target",
    ];
    if observed_fields
        .iter()
        .copied()
        .filter_map(|field| value.get(field).and_then(json_scalar_to_string))
        .any(|scalar| current == scalar.trim())
    {
        return true;
    }
    let Some((key, field_value)) = current.split_once('=') else {
        return false;
    };
    let key = key.trim();
    let field_value = field_value.trim();
    observed_fields.iter().copied().any(|field| {
        key == field
            && value
                .get(field)
                .and_then(json_scalar_to_string)
                .is_some_and(|scalar| field_value == scalar.trim())
    })
}

fn service_control_payload_from_output(output: &str) -> Option<serde_json::Value> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    if json_value_has_service_control_status_shape(&value) {
        return Some(value);
    }
    if let Some(extra) = value
        .get("extra")
        .filter(|extra| json_value_has_service_control_status_shape(extra))
    {
        return Some(extra.clone());
    }
    None
}

fn json_value_has_service_control_status_shape(value: &serde_json::Value) -> bool {
    value
        .get("service_name")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        && (value.get("post_state").is_some()
            || value.get("pre_state").is_some()
            || value.get("summary").is_some())
}

fn service_status_selector_is_system_health_only(route: &crate::IntentOutputContract) -> bool {
    route
        .self_extension
        .structured_field_selector
        .as_deref()
        .map(str::trim)
        .is_some_and(|selector| {
            selector == "system_health"
                || selector == "system_health.*"
                || selector.starts_with("system_health.")
        })
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
    None
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

fn health_check_payload_from_output(output: &str) -> Option<serde_json::Value> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    if json_value_has_health_check_shape(&value) {
        return Some(value);
    }
    if let Some(extra) = value
        .get("extra")
        .filter(|extra| json_value_has_health_check_shape(extra))
    {
        return Some(extra.clone());
    }
    None
}

fn json_value_has_health_check_shape(value: &serde_json::Value) -> bool {
    value
        .get("system_health")
        .is_some_and(serde_json::Value::is_object)
        && value.get("clawd_process_count").is_some()
        && value.get("clawd_health_port_open").is_some()
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

fn push_json_path_field(
    fields: &mut Vec<String>,
    value: &serde_json::Value,
    field_path: &'static str,
) {
    let Some(field_value) = json_value_at_path(value, field_path).and_then(json_scalar_to_string)
    else {
        return;
    };
    if field_value.trim().is_empty() {
        return;
    }
    fields.push(format!("{field_path}={field_value}"));
}

fn push_json_path_alias_field(
    fields: &mut Vec<String>,
    value: &serde_json::Value,
    field_path: &'static str,
    output_field: &'static str,
) {
    if fields.iter().any(|field| {
        field
            .split_once('=')
            .is_some_and(|(key, _)| key == output_field)
    }) {
        return;
    }
    let Some(field_value) = json_value_at_path(value, field_path).and_then(json_scalar_to_string)
    else {
        return;
    };
    if field_value.trim().is_empty() {
        return;
    }
    fields.push(format!("{output_field}={field_value}"));
}

fn json_value_at_path<'a>(
    value: &'a serde_json::Value,
    field_path: &str,
) -> Option<&'a serde_json::Value> {
    field_path
        .split('.')
        .try_fold(value, |current, part| current.get(part))
}

fn json_scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Array(values) => values
            .iter()
            .map(json_scalar_to_string)
            .collect::<Option<Vec<_>>>()
            .map(|values| values.join(",")),
        _ => None,
    }
}
