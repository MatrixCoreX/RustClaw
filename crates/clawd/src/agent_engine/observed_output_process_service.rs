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
        "all_interface_listener_count",
        "localhost_listener_count",
    ] {
        if let Some(count) = process_basic_json_u64(value, key) {
            lines.push(format!("port_list.{key}={count}"));
        }
    }
    push_process_basic_optional_field(
        &mut lines,
        "port_list",
        "internet_reachability",
        process_basic_json_string(value, "internet_reachability"),
    );
    for key in ["all_interface_ports", "ports"] {
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
    let all_interface_listeners = value
        .get("all_interface_listeners")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let listeners = value
        .get("listeners")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let source = if all_interface_listeners.is_empty() {
        listeners
    } else {
        all_interface_listeners
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

#[derive(Debug, Clone, PartialEq)]
struct ProcessBasicPsRow {
    pid: String,
    cpu: String,
    mem: String,
    comm: String,
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
