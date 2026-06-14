use std::cmp::Ordering;
use std::fs::{create_dir_all, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
                Ok((text, extra)) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    extra: Some(extra),
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    extra: None,
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: None,
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn execute(args: Value) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("ps")
        .trim();

    let result = match action {
        "ps" => {
            let limit = obj
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(30)
                .min(200);
            let filter = string_arg(obj, &["filter", "query", "name"]);
            run_ps_snapshot(limit as usize, filter.as_deref()).map(|(text, match_count)| {
                let extra = ps_extra(limit, filter, &text, match_count);
                (text, extra)
            })
        }
        "port_list" => {
            let filter = string_arg(obj, &["filter", "query", "port"]);
            run_port_list_snapshot().map(|(command_tool, text)| {
                let text = filter_command_output(&text, filter.as_deref());
                let extra = port_list_extra(command_tool, &text, filter);
                (text.clone(), extra)
            })
        }
        "kill" => {
            let pid = obj
                .get("pid")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| "pid is required".to_string())?;
            let signal = obj.get("signal").and_then(|v| v.as_str()).unwrap_or("TERM");
            run_command("kill", &["-s", signal, &pid.to_string()], None).map(|text| {
                (
                    text.clone(),
                    json!({"action":"kill","exit_code":0,"pid":pid,"signal":signal,"platform":std::env::consts::OS,"output":text}),
                )
            })
        }
        "tail_log" => {
            let path = obj
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "path is required".to_string())?;
            let n = obj
                .get("n")
                .and_then(|v| v.as_u64())
                .unwrap_or(100)
                .min(1000) as usize;
            let root = workspace_root();
            let full = resolve_path(&root, path)?;
            tail_file(&full, n).map(|text| {
                (
                    text.clone(),
                    json!({"action":"tail_log","path":path,"n":n,"platform":std::env::consts::OS,"output":text}),
                )
            })
        }
        _ => Err("unsupported action; use ps|port_list|kill|tail_log".to_string()),
    };

    append_service_log(action, obj, &result);
    result
}

fn string_arg(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        obj.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn run_command(bin: &str, args: &[&str], limit_lines: Option<usize>) -> Result<String, String> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|err| format!("run {bin} failed: {err}"))?;

    let mut text = format_command_output(&output.stdout, &output.stderr);

    if let Some(limit) = limit_lines {
        let mut lines: Vec<&str> = text.lines().collect();
        if lines.len() > limit {
            lines.truncate(limit);
            lines.push("...truncated...");
            text = lines.join("\n");
        }
    }

    let exit_code = output.status.code().unwrap_or(-1);
    if output.status.success() {
        Ok(format!("exit={exit_code}\n{text}"))
    } else {
        Err(format!("process command failed: exit={exit_code}\n{text}"))
    }
}

fn ps_extra(limit: u64, filter: Option<String>, text: &str, match_count: usize) -> Value {
    let running = match_count > 0;
    json!({
        "action": "ps",
        "exit_code": 0,
        "limit": limit,
        "filter": filter,
        "platform": std::env::consts::OS,
        "output": text,
        "match_count": match_count,
        "process_count": match_count,
        "running": running,
        "status": if running { "running" } else { "not_running" },
    })
}

fn run_ps_snapshot(limit: usize, filter: Option<&str>) -> Result<(String, usize), String> {
    let output = Command::new("ps")
        .args(["-Ao", "pid=,ppid=,pcpu=,pmem=,comm="])
        .output()
        .map_err(|err| format!("run ps failed: {err}"))?;

    let stderr_text = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(format!(
            "process command failed: exit={}\n{}",
            output.status.code().unwrap_or(-1),
            stderr_text
        ));
    }

    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let mut rows = stdout_text
        .lines()
        .filter_map(parse_ps_row)
        .filter(|row| ps_row_matches_filter(row, filter))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.pid.cmp(&b.pid))
    });
    let match_count = rows.len();

    let mut lines = vec!["PID PPID %CPU %MEM COMM".to_string()];
    for row in rows.iter().take(limit) {
        lines.push(format!(
            "{} {} {:.1} {:.1} {}",
            row.pid, row.ppid, row.cpu, row.mem, row.comm
        ));
    }
    if lines.len() == 1 {
        if let Some(filter) = filter.map(str::trim).filter(|value| !value.is_empty()) {
            lines.push(format!("no matching processes for filter: {filter}"));
        }
    }
    Ok((
        format!(
            "exit={}\n{}",
            output.status.code().unwrap_or(0),
            lines.join("\n")
        ),
        match_count,
    ))
}

fn ps_row_matches_filter(row: &PsRow, filter: Option<&str>) -> bool {
    let Some(filter) = filter.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    let filter_lower = filter.to_ascii_lowercase();
    row.comm.to_ascii_lowercase().contains(&filter_lower)
        || row.pid.to_string() == filter
        || row.ppid.to_string() == filter
}

fn filter_command_output(text: &str, filter: Option<&str>) -> String {
    let Some(filter) = filter.map(str::trim).filter(|value| !value.is_empty()) else {
        return text.to_string();
    };
    let filter_lower = filter.to_ascii_lowercase();
    let mut kept = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if idx == 0 && line.trim_start().starts_with("exit=") {
            kept.push(line.to_string());
            continue;
        }
        if line.to_ascii_lowercase().contains(&filter_lower) {
            kept.push(line.to_string());
        }
    }
    if kept.len() == 1 {
        kept.push(format!("no matching rows for filter: {filter}"));
    }
    kept.join("\n")
}

fn run_port_list_command(
    command_tool: &'static str,
    bin: &str,
    args: &[&str],
) -> Result<(&'static str, String), String> {
    run_command(bin, args, None).map(|text| (command_tool, text))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PortListener {
    local_endpoint: String,
    local_address: String,
    port: String,
    bind_scope: String,
    is_wildcard: bool,
    is_loopback: bool,
    process_name: Option<String>,
    pid: Option<i64>,
}

fn port_list_extra(command_tool: &'static str, text: &str, filter: Option<String>) -> Value {
    let listeners = parse_port_listeners(command_tool, text);
    let listener_count = listeners.len();
    let public_listener_count = listeners
        .iter()
        .filter(|listener| listener.is_wildcard)
        .count();
    let localhost_listener_count = listeners
        .iter()
        .filter(|listener| listener.is_loopback)
        .count();
    let listener_sample = prioritized_listener_sample(&listeners, 64);
    let public_listener_sample = prioritized_listener_sample(
        &listeners
            .iter()
            .filter(|listener| listener.is_wildcard)
            .cloned()
            .collect::<Vec<_>>(),
        32,
    );

    json!({
        "action": "port_list",
        "exit_code": 0,
        "filter": filter,
        "platform": std::env::consts::OS,
        "command_tool": command_tool,
        "output": text,
        "listener_count": listener_count,
        "public_listener_count": public_listener_count,
        "localhost_listener_count": localhost_listener_count,
        "ports": unique_ports(&listeners),
        "public_ports": unique_ports(
            &listeners
                .iter()
                .filter(|listener| listener.is_wildcard)
                .cloned()
                .collect::<Vec<_>>()
        ),
        "listeners": listener_sample,
        "listeners_truncated": listener_count > 64,
        "public_listeners": public_listener_sample,
        "public_listeners_truncated": public_listener_count > 32,
    })
}

fn parse_port_listeners(command_tool: &str, text: &str) -> Vec<PortListener> {
    text.lines()
        .filter_map(|line| match command_tool {
            "ss" => parse_ss_listener_line(line),
            "lsof" => parse_lsof_listener_line(line),
            "netstat" => parse_netstat_listener_line(line),
            _ => None,
        })
        .collect()
}

fn parse_ss_listener_line(line: &str) -> Option<PortListener> {
    let trimmed = line.trim();
    if !trimmed.starts_with("LISTEN") {
        return None;
    }
    let parts = trimmed.split_whitespace().collect::<Vec<_>>();
    let local_endpoint = parts.get(3)?;
    let process_text = if parts.len() > 5 {
        parts[5..].join(" ")
    } else {
        String::new()
    };
    listener_from_endpoint(local_endpoint, &process_text)
}

fn parse_netstat_listener_line(line: &str) -> Option<PortListener> {
    let trimmed = line.trim();
    if !(trimmed.starts_with("tcp") || trimmed.starts_with("udp")) || !trimmed.contains("LISTEN") {
        return None;
    }
    let parts = trimmed.split_whitespace().collect::<Vec<_>>();
    let local_endpoint = parts.get(3)?;
    let process_text = parts
        .iter()
        .position(|part| *part == "LISTEN")
        .and_then(|idx| parts.get(idx + 1..))
        .map(|tail| tail.join(" "))
        .unwrap_or_default();
    listener_from_endpoint(local_endpoint, &process_text)
}

fn parse_lsof_listener_line(line: &str) -> Option<PortListener> {
    let trimmed = line.trim();
    if trimmed.starts_with("COMMAND") || !trimmed.contains("(LISTEN)") {
        return None;
    }
    let parts = trimmed.split_whitespace().collect::<Vec<_>>();
    let command = parts.first().copied().unwrap_or_default();
    let pid = parts.get(1).and_then(|part| part.parse::<i64>().ok());
    let endpoint = parts
        .iter()
        .position(|part| *part == "TCP" || *part == "UDP")
        .and_then(|idx| parts.get(idx + 1))
        .copied()?;
    let mut listener = listener_from_endpoint(endpoint, command)?;
    listener.pid = pid.or(listener.pid);
    if listener.process_name.is_none() && !command.is_empty() {
        listener.process_name = Some(command.to_string());
    }
    Some(listener)
}

fn listener_from_endpoint(endpoint: &str, process_text: &str) -> Option<PortListener> {
    let clean_endpoint = endpoint.trim().trim_end_matches(',');
    let (local_address, port) = split_local_addr_port(clean_endpoint)?;
    let (bind_scope, is_wildcard, is_loopback) = classify_bind_scope(&local_address);
    Some(PortListener {
        local_endpoint: clean_endpoint.to_string(),
        local_address,
        port,
        bind_scope,
        is_wildcard,
        is_loopback,
        process_name: process_name_from_text(process_text),
        pid: pid_from_text(process_text),
    })
}

fn split_local_addr_port(endpoint: &str) -> Option<(String, String)> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return None;
    }
    if let Some(rest) = endpoint.strip_prefix('[') {
        if let Some((addr, port)) = rest.rsplit_once("]:") {
            return clean_addr_port(addr, port);
        }
    }
    let (addr, port) = endpoint.rsplit_once(':')?;
    clean_addr_port(addr, port)
}

fn clean_addr_port(addr: &str, port: &str) -> Option<(String, String)> {
    let addr = addr.trim().trim_matches(['[', ']']);
    let port = port
        .trim()
        .trim_matches(|ch: char| ch == ',' || ch == ')' || ch == '(');
    if addr.is_empty() || port.is_empty() || port == "*" {
        return None;
    }
    Some((addr.to_string(), port.to_string()))
}

fn classify_bind_scope(addr: &str) -> (String, bool, bool) {
    let base_addr = addr.split('%').next().unwrap_or(addr).trim();
    let is_wildcard = matches!(base_addr, "0.0.0.0" | "::" | "*" | ":::");
    let is_loopback = base_addr == "::1"
        || base_addr.eq_ignore_ascii_case("localhost")
        || base_addr == "127.0.0.1"
        || base_addr.starts_with("127.");
    let bind_scope = if is_wildcard {
        "all_interfaces"
    } else if is_loopback {
        "localhost"
    } else {
        "specific_address"
    };
    (bind_scope.to_string(), is_wildcard, is_loopback)
}

fn process_name_from_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return None;
    }
    if let Some(start) = trimmed.find('"') {
        let rest = &trimmed[start + 1..];
        if let Some(end) = rest.find('"') {
            let name = rest[..end].trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    trimmed
        .split_whitespace()
        .next()
        .map(|value| value.trim_matches(['"', ',']).to_string())
        .filter(|value| !value.is_empty() && value != "-")
}

fn pid_from_text(text: &str) -> Option<i64> {
    let marker = "pid=";
    let start = text.find(marker)? + marker.len();
    let digits = text[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<i64>().ok()
}

fn prioritized_listener_sample(listeners: &[PortListener], limit: usize) -> Vec<PortListener> {
    let mut sample = listeners.to_vec();
    sample.sort_by(|a, b| {
        listener_priority(a)
            .cmp(&listener_priority(b))
            .then_with(|| port_sort_key(&a.port).cmp(&port_sort_key(&b.port)))
            .then_with(|| a.local_endpoint.cmp(&b.local_endpoint))
    });
    sample.truncate(limit);
    sample
}

fn listener_priority(listener: &PortListener) -> u8 {
    if listener.is_wildcard {
        0
    } else if !listener.is_loopback {
        1
    } else {
        2
    }
}

fn unique_ports(listeners: &[PortListener]) -> Vec<String> {
    let mut ports = listeners
        .iter()
        .map(|listener| listener.port.clone())
        .collect::<Vec<_>>();
    ports.sort_by(|a, b| {
        port_sort_key(a)
            .cmp(&port_sort_key(b))
            .then_with(|| a.cmp(b))
    });
    ports.dedup();
    ports
}

fn port_sort_key(port: &str) -> u32 {
    port.parse::<u32>().unwrap_or(u32::MAX)
}

#[cfg(target_os = "macos")]
fn run_port_list_snapshot() -> Result<(&'static str, String), String> {
    run_port_list_command("lsof", "lsof", &["-nP", "-iTCP", "-sTCP:LISTEN"])
        .or_else(|_| run_port_list_command("netstat", "netstat", &["-anv", "-p", "tcp"]))
}

#[cfg(not(target_os = "macos"))]
fn run_port_list_snapshot() -> Result<(&'static str, String), String> {
    run_port_list_command("ss", "ss", &["-ltnp"])
        .or_else(|_| run_port_list_command("lsof", "lsof", &["-nP", "-iTCP", "-sTCP:LISTEN"]))
        .or_else(|_| run_port_list_command("netstat", "netstat", &["-ltnp"]))
}

#[derive(Debug)]
struct PsRow {
    pid: i64,
    ppid: i64,
    cpu: f64,
    mem: f64,
    comm: String,
}

fn parse_ps_row(line: &str) -> Option<PsRow> {
    let mut parts = line.split_whitespace();
    let pid = parts.next()?.parse::<i64>().ok()?;
    let ppid = parts.next()?.parse::<i64>().ok()?;
    let cpu = parts.next()?.parse::<f64>().ok()?;
    let mem = parts.next()?.parse::<f64>().ok()?;
    let comm = parts.collect::<Vec<_>>().join(" ");
    if comm.is_empty() {
        return None;
    }
    Some(PsRow {
        pid,
        ppid,
        cpu,
        mem,
        comm,
    })
}

fn format_command_output(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(stdout));
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(stderr));
    }
    text
}

fn tail_file(path: &Path, n: usize) -> Result<String, String> {
    let content =
        std::fs::read_to_string(path).map_err(|err| format!("read file failed: {err}"))?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].join("\n"))
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn resolve_path(workspace_root: &Path, input: &str) -> Result<PathBuf, String> {
    let raw = Path::new(input);
    let mut normalized = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::ParentDir => return Err("path with '..' is not allowed".to_string()),
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if raw.is_absolute() {
        return Ok(normalized);
    }
    Ok(workspace_root.join(normalized))
}

fn append_service_log(
    action: &str,
    args: &serde_json::Map<String, Value>,
    result: &Result<(String, Value), String>,
) {
    let root = workspace_root();
    let log_dir = root.join("logs");
    if let Err(err) = create_dir_all(&log_dir) {
        eprintln!("create service logs dir failed: {err}");
        return;
    }
    let file_path = log_dir.join("service_ops.log");
    let mut file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
    {
        Ok(f) => f,
        Err(err) => {
            eprintln!("open service log failed: {err}");
            return;
        }
    };

    let (status, output, error) = match result {
        Ok((text, _extra)) => ("ok", Some(truncate_for_log(text)), None),
        Err(err) => ("failed", None, Some(truncate_for_log(err))),
    };

    let line = serde_json::json!({
        "ts": now_ts(),
        "action": action,
        "status": status,
        "args": args,
        "output": output,
        "error": error,
    })
    .to_string();

    if let Err(err) = writeln!(file, "{line}") {
        eprintln!("write service log failed: {err}");
    }
}

fn truncate_for_log(s: &str) -> String {
    const MAX: usize = 8000;
    if s.len() <= MAX {
        return s.to_string();
    }
    let mut out = s[..MAX].to_string();
    out.push_str("...(truncated)");
    out
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
