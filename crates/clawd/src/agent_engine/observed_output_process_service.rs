use super::*;

pub(super) fn service_control_summary_candidate(value: &serde_json::Value) -> Option<String> {
    value
        .get("summary")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn service_control_state_value(value: &serde_json::Value) -> Option<&str> {
    value
        .get("post_state")
        .or_else(|| value.get("pre_state"))
        .or_else(|| value.get("summary"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

pub(super) fn service_control_status_direct_answer_candidate(
    value: &serde_json::Value,
    response_shape: Option<crate::OutputResponseShape>,
) -> Option<String> {
    let service_state = service_control_state_value(value)?;
    if matches!(response_shape, Some(crate::OutputResponseShape::Scalar)) {
        return Some(service_state.to_string());
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
    let body = normalized_success_body_for_direct_answer(body);
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
    route.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
        && route_allows_model_language_direct_candidate(route)
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
