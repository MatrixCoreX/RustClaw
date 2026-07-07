use super::*;

pub(super) fn service_control_summary_candidate(value: &serde_json::Value) -> Option<String> {
    service_control_status_payload_value(value)?
        .get("summary")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn service_control_status_payload_value(value: &serde_json::Value) -> Option<serde_json::Value> {
    if value_has_service_control_status_shape(value) {
        return Some(value.clone());
    }
    if let Some(extra) = value
        .get("extra")
        .filter(|extra| value_has_service_control_status_shape(extra))
    {
        return Some(extra.clone());
    }
    None
}

fn value_has_service_control_status_shape(value: &serde_json::Value) -> bool {
    value
        .get("service_name")
        .or_else(|| value.get("target"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        && (value.get("post_state").is_some()
            || value.get("pre_state").is_some()
            || value.get("status").is_some()
            || value.get("summary").is_some())
}

fn service_control_state_value(value: &serde_json::Value) -> Option<&str> {
    value
        .get("post_state")
        .or_else(|| value.get("pre_state"))
        .or_else(|| value.get("status"))
        .or_else(|| value.get("summary"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn service_control_json_scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.trim().to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
    .filter(|value| !value.is_empty())
}

fn push_service_control_field(
    fields: &mut Vec<String>,
    value: &serde_json::Value,
    field_name: &'static str,
    output_name: &'static str,
) {
    if fields.iter().any(|field| {
        field
            .split_once('=')
            .is_some_and(|(key, _)| key == output_name)
    }) {
        return;
    }
    let Some(field_value) = value
        .get(field_name)
        .and_then(service_control_json_scalar_to_string)
    else {
        return;
    };
    fields.push(format!("{output_name}={field_value}"));
}

fn service_control_status_fields_answer(value: &serde_json::Value) -> Option<String> {
    let mut fields = Vec::new();
    push_service_control_field(&mut fields, value, "target", "target");
    push_service_control_field(&mut fields, value, "service_name", "target");
    push_service_control_field(&mut fields, value, "service_name", "service_name");
    push_service_control_field(&mut fields, value, "post_state", "post_state");
    push_service_control_field(&mut fields, value, "pre_state", "pre_state");
    push_service_control_field(&mut fields, value, "status", "status");
    push_service_control_field(&mut fields, value, "verified", "verified");
    push_service_control_field(&mut fields, value, "manager_type", "manager_type");
    if fields.is_empty() {
        return None;
    }
    fields.push("source=service_control".to_string());
    Some(fields.join(" "))
}

pub(super) fn service_control_status_direct_answer_candidate(
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
) -> Option<String> {
    let value = service_control_status_payload_value(value)?;
    let service_state = service_control_state_value(&value)?;
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(service_state.to_string());
    }
    if matches!(
        response_shape,
        Some(crate::OutputResponseShape::Strict | crate::OutputResponseShape::Free)
    ) {
        return service_control_status_fields_answer(&value);
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessBasicPsStatusObservation {
    status: String,
    running: bool,
    match_count: u64,
    target: Option<String>,
    filter: Option<String>,
    process_name: Option<String>,
    exit_code: Option<i64>,
}

pub(super) fn latest_process_basic_service_status_direct_answer_candidate(
    _state: Option<&AppState>,
    loop_state: &LoopState,
    response_shape: Option<crate::OutputResponseShape>,
    _prefer_english: bool,
) -> Option<String> {
    let idx = latest_successful_step_index(loop_state, |step| step.skill == "process_basic")?;
    let body = loop_state.executed_step_results[idx]
        .output
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty())?;
    let body = normalized_success_body_for_observed_output(body);
    process_basic_service_status_structured_scalar_candidate(&body, response_shape)
        .or_else(|| {
            process_basic_port_list_structured_direct_answer_candidate(&body, response_shape)
        })
        .or_else(|| {
            matches!(
                response_shape,
                Some(crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict)
            )
            .then(|| {
                process_basic_port_list_direct_answer_candidate(None, &body, response_shape, false)
            })
            .flatten()
        })
}

fn process_basic_service_status_structured_scalar_candidate(
    body: &str,
    response_shape: Option<crate::OutputResponseShape>,
) -> Option<String> {
    let observation = process_basic_ps_status_observation(body)?;
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(observation.status);
    }
    None
}

fn process_basic_ps_status_observation(body: &str) -> Option<ProcessBasicPsStatusObservation> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(trimmed).ok();
    let ps_value = value.as_ref().and_then(process_basic_ps_observation_value);
    let output = ps_value
        .and_then(|value| value.get("output").or_else(|| value.get("text")))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|output| !output.is_empty())
        .unwrap_or(trimmed);
    let rows = process_basic_table_rows(output);
    let no_match_filter = process_basic_no_match_filter(output);
    if ps_value.is_none() && rows.is_empty() && no_match_filter.is_none() {
        return None;
    }
    let filter = ps_value
        .and_then(|value| value.get("filter"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|filter| !filter.is_empty())
        .map(ToOwned::to_owned);
    let process_name = rows
        .first()
        .and_then(|row| process_basic_ps_row(row))
        .map(|row| row.comm)
        .filter(|comm| !comm.trim().is_empty());
    let match_count = ps_value
        .and_then(|value| {
            value
                .get("match_count")
                .or_else(|| value.get("process_count"))
        })
        .and_then(|value| value.as_u64())
        .unwrap_or(rows.len() as u64);
    let running = ps_value
        .and_then(|value| value.get("running"))
        .and_then(|value| value.as_bool())
        .unwrap_or(match_count > 0);
    let status = ps_value
        .and_then(|value| value.get("status"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|status| matches!(*status, "running" | "not_running"))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            if running {
                "running".to_string()
            } else {
                "not_running".to_string()
            }
        });
    let target = filter
        .clone()
        .or_else(|| no_match_filter.clone())
        .or_else(|| process_name.clone());
    let exit_code = ps_value
        .and_then(|value| value.get("exit_code"))
        .and_then(|value| value.as_i64());
    Some(ProcessBasicPsStatusObservation {
        status,
        running,
        match_count,
        target,
        filter,
        process_name,
        exit_code,
    })
}

fn process_basic_ps_observation_value(value: &serde_json::Value) -> Option<&serde_json::Value> {
    let action = value
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim);
    if action == Some("ps") {
        return Some(value);
    }
    value
        .get("extra")
        .and_then(process_basic_ps_observation_value)
}

fn process_basic_port_list_structured_direct_answer_candidate(
    body: &str,
    response_shape: Option<crate::OutputResponseShape>,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body.trim()).ok()?;
    let value = process_basic_port_list_observation_value(&value)?;
    let listener_count = value
        .get("listener_count")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let public_listener_count = value
        .get("public_listener_count")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    if listener_count == 0 && public_listener_count == 0 {
        return None;
    }
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(public_listener_count.max(listener_count).to_string());
    }
    if response_shape.is_some() {
        return None;
    }
    let public_ports = value
        .get("public_ports")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let ports = value
        .get("ports")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let public_listeners = value
        .get("public_listeners")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let listeners = value
        .get("listeners")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let notable_listeners = if public_listeners.is_empty() {
        listeners.iter().take(8).cloned().collect::<Vec<_>>()
    } else {
        public_listeners.iter().take(8).cloned().collect::<Vec<_>>()
    };
    let process_names = process_basic_listener_process_names(&notable_listeners);
    let brief_explanation = process_basic_port_brief_explanation(&notable_listeners);
    let field_value = if public_ports.is_empty() {
        serde_json::Value::Array(ports.clone())
    } else {
        serde_json::Value::Array(public_ports.clone())
    };
    let mut payload = serde_json::Map::new();
    payload.insert(
        "message_key".to_string(),
        serde_json::json!("clawd.msg.service_status.port_list"),
    );
    payload.insert(
        "status_source".to_string(),
        serde_json::json!("process_basic.port_list"),
    );
    payload.insert("field_value".to_string(), field_value);
    payload.insert(
        "listener_count".to_string(),
        serde_json::json!(listener_count),
    );
    payload.insert(
        "public_listener_count".to_string(),
        serde_json::json!(public_listener_count),
    );
    if let Some(count) = value
        .get("localhost_listener_count")
        .and_then(|value| value.as_u64())
    {
        payload.insert(
            "localhost_listener_count".to_string(),
            serde_json::json!(count),
        );
    }
    if !public_ports.is_empty() {
        if let Some(first_port) = public_ports.first().and_then(|value| value.as_str()) {
            payload.insert("port".to_string(), serde_json::json!(first_port));
        }
        payload.insert(
            "public_ports".to_string(),
            serde_json::Value::Array(public_ports),
        );
    }
    if !ports.is_empty() {
        payload.insert("ports".to_string(), serde_json::Value::Array(ports));
    }
    if !notable_listeners.is_empty() {
        payload.insert(
            "public_listeners".to_string(),
            serde_json::Value::Array(notable_listeners),
        );
    }
    if !brief_explanation.is_empty() {
        payload.insert(
            "brief_explanation".to_string(),
            serde_json::Value::Array(brief_explanation.clone()),
        );
        payload.insert(
            "notable_ports".to_string(),
            serde_json::Value::Array(brief_explanation),
        );
    }
    if !process_names.is_empty() {
        if let Some(first_name) = process_names.first() {
            payload.insert("process_name".to_string(), serde_json::json!(first_name));
        }
        payload.insert(
            "process_names".to_string(),
            serde_json::json!(process_names),
        );
    }
    Some(serde_json::Value::Object(payload).to_string())
}

fn process_basic_port_list_observation_value(
    value: &serde_json::Value,
) -> Option<&serde_json::Value> {
    let action = value
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim);
    if action == Some("port_list") {
        return Some(value);
    }
    value
        .get("extra")
        .and_then(process_basic_port_list_observation_value)
}

fn process_basic_listener_process_names(listeners: &[serde_json::Value]) -> Vec<String> {
    let mut names = Vec::new();
    for listener in listeners {
        let Some(name) = listener
            .get("process_name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    }
    names
}

fn process_basic_port_brief_explanation(listeners: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut items = Vec::new();
    let mut seen_ports = Vec::new();
    for listener in listeners {
        let Some(port) = listener
            .get("port")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|port| !port.is_empty())
        else {
            continue;
        };
        if seen_ports.iter().any(|seen| seen == port) {
            continue;
        }
        seen_ports.push(port.to_string());
        let mut item = serde_json::Map::new();
        item.insert("port".to_string(), serde_json::json!(port));
        if let Some(endpoint) = listener
            .get("local_endpoint")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|endpoint| !endpoint.is_empty())
        {
            item.insert("local_endpoint".to_string(), serde_json::json!(endpoint));
        }
        if let Some(bind_scope) = listener
            .get("bind_scope")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|scope| !scope.is_empty())
        {
            item.insert("bind_scope".to_string(), serde_json::json!(bind_scope));
            item.insert(
                "exposure".to_string(),
                serde_json::json!(process_basic_bind_scope_exposure(bind_scope)),
            );
        }
        if let Some(process_name) = listener
            .get("process_name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|process_name| !process_name.is_empty())
        {
            item.insert("process_name".to_string(), serde_json::json!(process_name));
        }
        if let Some(service_hint) = well_known_port_service_hint(port) {
            item.insert("service_hint".to_string(), serde_json::json!(service_hint));
        }
        item.insert(
            "source".to_string(),
            serde_json::json!("process_basic.port_list"),
        );
        items.push(serde_json::Value::Object(item));
    }
    items
}

fn process_basic_bind_scope_exposure(bind_scope: &str) -> &'static str {
    match bind_scope {
        "all_interfaces" => "public_bind",
        "localhost" => "loopback",
        _ => "bound",
    }
}

fn well_known_port_service_hint(port: &str) -> Option<&'static str> {
    match port {
        "22" => Some("ssh"),
        "53" => Some("dns"),
        "80" => Some("http"),
        "443" => Some("https"),
        "631" => Some("ipp"),
        _ => None,
    }
}

pub(super) fn process_basic_observed_candidate(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(port_value) = process_basic_port_list_observation_value(&value) {
            return process_basic_port_list_observed_candidate(port_value);
        }
        if let Some(ps_value) = process_basic_ps_observation_value(&value) {
            return process_basic_ps_observed_candidate(ps_value);
        }
    }
    if !process_basic_port_rows(trimmed).is_empty() {
        return process_basic_port_text_observed_candidate(trimmed);
    }
    process_basic_ps_text_observed_candidate(trimmed)
}

fn process_basic_json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn process_basic_json_u64(value: &serde_json::Value, key: &str) -> Option<u64> {
    value.get(key).and_then(serde_json::Value::as_u64)
}

fn push_process_basic_optional_field(
    lines: &mut Vec<String>,
    prefix: &str,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value {
        lines.push(format!("{prefix}.{key}={value}"));
    }
}

fn process_basic_port_list_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let mut lines = vec!["process_basic.port_list".to_string()];
    push_process_basic_optional_field(
        &mut lines,
        "port_list",
        "platform",
        process_basic_json_string(value, "platform"),
    );
    push_process_basic_optional_field(
        &mut lines,
        "port_list",
        "command_tool",
        process_basic_json_string(value, "command_tool"),
    );
    for key in [
        "listener_count",
        "public_listener_count",
        "localhost_listener_count",
    ] {
        if let Some(count) = process_basic_json_u64(value, key) {
            lines.push(format!("port_list.{key}={count}"));
        }
    }
    for key in ["public_ports", "ports"] {
        if let Some(values) = process_basic_string_array(value, key) {
            lines.push(format!("port_list.{key}={}", values.join(",")));
        }
    }
    let listeners = process_basic_notable_listener_values(value);
    for (idx, listener) in listeners.iter().enumerate() {
        let row = idx + 1;
        let prefix = format!("listener.{row}");
        push_process_basic_optional_field(
            &mut lines,
            &prefix,
            "port",
            process_basic_json_string(listener, "port"),
        );
        push_process_basic_optional_field(
            &mut lines,
            &prefix,
            "endpoint",
            process_basic_json_string(listener, "local_endpoint"),
        );
        push_process_basic_optional_field(
            &mut lines,
            &prefix,
            "bind_scope",
            process_basic_json_string(listener, "bind_scope"),
        );
        push_process_basic_optional_field(
            &mut lines,
            &prefix,
            "process",
            process_basic_json_string(listener, "process_name"),
        );
        if let Some(pid) = listener.get("pid").and_then(serde_json::Value::as_i64) {
            lines.push(format!("{prefix}.pid={pid}"));
        }
    }
    (lines.len() > 1).then(|| lines.join("\n"))
}

fn process_basic_notable_listener_values(value: &serde_json::Value) -> Vec<serde_json::Value> {
    let public_listeners = value
        .get("public_listeners")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let listeners = value
        .get("listeners")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let source = if public_listeners.is_empty() {
        listeners
    } else {
        public_listeners
    };
    source.into_iter().take(8).collect()
}

fn process_basic_string_array(value: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    let values = value
        .get(key)?
        .as_array()?
        .iter()
        .filter_map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn process_basic_ps_observed_candidate(value: &serde_json::Value) -> Option<String> {
    let output = value
        .get("output")
        .or_else(|| value.get("text"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or_default();
    let observation = process_basic_ps_status_observation(&value.to_string())?;
    let mut lines = vec!["process_basic.ps".to_string()];
    lines.push(format!("ps.status={}", observation.status));
    lines.push(format!("ps.running={}", observation.running));
    lines.push(format!("ps.match_count={}", observation.match_count));
    if let Some(exit_code) = observation.exit_code {
        lines.push(format!("ps.exit_code={exit_code}"));
    }
    if let Some(filter) = observation.filter {
        lines.push(format!("ps.filter={filter}"));
    }
    if let Some(target) = observation.target {
        lines.push(format!("ps.target={target}"));
    }
    process_basic_ps_rows_observed_lines(output, &mut lines);
    Some(lines.join("\n"))
}

fn process_basic_ps_text_observed_candidate(body: &str) -> Option<String> {
    let observation = process_basic_ps_status_observation(body)?;
    let mut lines = vec!["process_basic.ps".to_string()];
    lines.push(format!("ps.status={}", observation.status));
    lines.push(format!("ps.running={}", observation.running));
    lines.push(format!("ps.match_count={}", observation.match_count));
    if let Some(filter) = observation.filter.or(observation.target) {
        lines.push(format!("ps.target={filter}"));
    }
    process_basic_ps_rows_observed_lines(body, &mut lines);
    Some(lines.join("\n"))
}

fn process_basic_ps_rows_observed_lines(output: &str, lines: &mut Vec<String>) {
    let rows = process_basic_table_rows(output);
    for (idx, row) in rows
        .iter()
        .filter_map(|row| process_basic_ps_row(row))
        .take(8)
        .enumerate()
    {
        let row_no = idx + 1;
        lines.push(format!(
            "process.{row_no}.pid={} process.{row_no}.cpu={} process.{row_no}.mem={} process.{row_no}.comm={}",
            row.pid, row.cpu, row.mem, row.comm
        ));
    }
    if rows.len() > 8 {
        lines.push(format!("ps.rows_truncated_after=8"));
    }
}

fn process_basic_port_text_observed_candidate(body: &str) -> Option<String> {
    let rows = process_basic_port_rows(body);
    if rows.is_empty() {
        return None;
    }
    let mut lines = vec!["process_basic.port_list".to_string()];
    lines.push(format!("port_list.listener_count={}", rows.len()));
    for (idx, row) in rows.iter().take(8).enumerate() {
        let row_no = idx + 1;
        let prefix = format!("listener.{row_no}");
        lines.push(format!("{prefix}.port={}", row.port));
        lines.push(format!("{prefix}.endpoint={}", row.local));
        if let Some(process) = row.process.as_deref().filter(|value| !value.is_empty()) {
            lines.push(format!("{prefix}.process={process}"));
        }
    }
    if rows.len() > 8 {
        lines.push("port_list.rows_truncated_after=8".to_string());
    }
    Some(lines.join("\n"))
}

pub(super) fn process_basic_service_status_direct_answer_candidate(
    state: Option<&AppState>,
    body: &str,
    response_shape: Option<crate::OutputResponseShape>,
    prefer_english: bool,
) -> Option<String> {
    if let Some(answer) =
        process_basic_port_list_direct_answer_candidate(state, body, response_shape, prefer_english)
    {
        return Some(answer);
    }
    let observation = process_basic_ps_status_observation(body);
    let rows = process_basic_table_rows(body);
    let status = if rows.is_empty() {
        "not_running"
    } else {
        "running"
    };
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(
            observation
                .map(|observation| observation.status)
                .unwrap_or_else(|| status.to_string()),
        );
    }
    if let Some(answer) =
        process_basic_ps_inventory_direct_answer_candidate(state, &rows, prefer_english)
    {
        return Some(answer);
    }
    observation?;
    None
}

#[derive(Debug, Clone, PartialEq)]
struct ProcessBasicPsRow {
    pid: String,
    cpu: String,
    mem: String,
    comm: String,
}

fn process_basic_ps_inventory_direct_answer_candidate(
    state: Option<&AppState>,
    rows: &[&str],
    prefer_english: bool,
) -> Option<String> {
    let rows = rows
        .iter()
        .filter_map(|row| process_basic_ps_row(row))
        .collect::<Vec<_>>();
    if rows.len() < 2 {
        return None;
    }
    let top = rows.first()?;
    let list = rows
        .iter()
        .enumerate()
        .map(|(idx, row)| {
            format!(
                "{}. {} CPU {}% MEM {}% PID {}",
                idx + 1,
                row.comm,
                row.cpu,
                row.mem,
                row.pid
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    Some(observed_t_with_vars(
        state,
        "clawd.msg.process_basic_ps_inventory_summary",
        "当前 CPU 占用最高的 {count} 个进程：{list}。最值得注意的是 {top_comm}（CPU {top_cpu}%，PID {top_pid}）。",
        "Top {count} processes by CPU: {list}. Most notable: {top_comm} (CPU {top_cpu}%, PID {top_pid}).",
        prefer_english,
        &[
            ("count", &rows.len().to_string()),
            ("list", &list),
            ("top_comm", top.comm.as_str()),
            ("top_cpu", top.cpu.as_str()),
            ("top_pid", top.pid.as_str()),
        ],
    ))
}

fn process_basic_ps_row(row: &str) -> Option<ProcessBasicPsRow> {
    let columns = row.split_whitespace().collect::<Vec<_>>();
    if columns.len() < 5 || !columns[0].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(ProcessBasicPsRow {
        pid: columns[0].to_string(),
        cpu: columns[2].to_string(),
        mem: columns[3].to_string(),
        comm: columns[4..].join(" "),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessBasicPortRow {
    local: String,
    port: String,
    process: Option<String>,
}

fn process_basic_port_list_direct_answer_candidate(
    _state: Option<&AppState>,
    body: &str,
    response_shape: Option<crate::OutputResponseShape>,
    _prefer_english: bool,
) -> Option<String> {
    let rows = process_basic_port_rows(body);
    if rows.is_empty() {
        return None;
    }
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(rows.len().to_string());
    }
    let mut lines = vec![format!("port.count={}", rows.len())];
    lines.extend(
        rows.iter()
            .enumerate()
            .map(|(idx, row)| process_basic_port_row_label(idx, row)),
    );
    Some(lines.join("\n"))
}

fn process_basic_port_row_label(idx: usize, row: &ProcessBasicPortRow) -> String {
    let exposure = if process_basic_local_addr_is_loopback(&row.local) {
        "loopback"
    } else {
        "public_bind"
    };
    match row.process.as_deref().filter(|value| !value.is_empty()) {
        Some(process) => format!(
            "port[{idx}].number={}\nport[{idx}].local={}\nport[{idx}].exposure={}\nport[{idx}].process={}",
            row.port, row.local, exposure, process
        ),
        None => format!(
            "port[{idx}].number={}\nport[{idx}].local={}\nport[{idx}].exposure={}",
            row.port, row.local, exposure
        ),
    }
}

fn process_basic_port_rows(body: &str) -> Vec<ProcessBasicPortRow> {
    let mut rows = Vec::new();
    for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with("exit=")
            || line.contains("Local Address:Port")
            || line.starts_with("COMMAND ")
        {
            continue;
        }
        let Some(local) = process_basic_local_address_from_port_line(line) else {
            continue;
        };
        let Some(port) = process_basic_port_from_local_address(&local) else {
            continue;
        };
        if rows
            .iter()
            .any(|row: &ProcessBasicPortRow| row.port == port && row.local == local)
        {
            continue;
        }
        rows.push(ProcessBasicPortRow {
            local,
            port,
            process: process_basic_process_name_from_port_line(line),
        });
    }
    rows
}

fn process_basic_local_address_from_port_line(line: &str) -> Option<String> {
    let columns = line.split_whitespace().collect::<Vec<_>>();
    if columns.first().is_some_and(|column| *column == "LISTEN") && columns.len() >= 4 {
        return Some(columns[3].to_string());
    }
    if columns.iter().any(|column| *column == "(LISTEN)") {
        return columns
            .iter()
            .rev()
            .skip_while(|column| **column == "(LISTEN)")
            .find(|column| column.contains(':'))
            .map(|column| column.to_string());
    }
    None
}

fn process_basic_port_from_local_address(local: &str) -> Option<String> {
    let host_port = local.rsplit_once(':')?.1;
    let port = host_port
        .trim()
        .trim_end_matches(']')
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!port.is_empty()).then_some(port)
}

fn process_basic_process_name_from_port_line(line: &str) -> Option<String> {
    let marker = "users:((\"";
    let start = line.find(marker)? + marker.len();
    let rest = line.get(start..)?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string()).filter(|value| !value.trim().is_empty())
}

fn process_basic_local_addr_is_loopback(local: &str) -> bool {
    local.starts_with("127.") || local.starts_with("[::1]") || local.starts_with("::1")
}

fn process_basic_table_rows(body: &str) -> Vec<&str> {
    let mut saw_header = false;
    let mut rows = Vec::new();
    for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with("exit=") {
            continue;
        }
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.iter().any(|column| *column == "PID")
            && columns.iter().any(|column| *column == "COMM")
        {
            saw_header = true;
            continue;
        }
        if saw_header
            && columns.len() >= 2
            && columns
                .first()
                .is_some_and(|column| column.chars().all(|ch| ch.is_ascii_digit()))
        {
            rows.push(line);
        }
    }
    rows
}

fn process_basic_no_match_filter(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("no matching processes for filter:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn process_basic_port_list_should_use_llm_synthesis(
    route: &crate::RouteResult,
    body: &str,
) -> bool {
    super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::ServiceStatus,
    ) && route_allows_model_language_direct_candidate(route)
        && route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        && body_has_process_basic_port_list_observation(body)
}

fn body_has_process_basic_port_list_observation(body: &str) -> bool {
    !process_basic_port_rows(body).is_empty()
        || serde_json::from_str::<serde_json::Value>(body.trim())
            .ok()
            .as_ref()
            .and_then(process_basic_port_list_observation_value)
            .is_some()
}
