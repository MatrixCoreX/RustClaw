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
            run_ps_snapshot(limit as usize, filter.as_deref()).map(|text| {
                (
                    text.clone(),
                    json!({"action":"ps","exit_code":0,"limit":limit,"filter":filter,"platform":std::env::consts::OS,"output":text}),
                )
            })
        }
        "port_list" => {
            let filter = string_arg(obj, &["filter", "query", "port"]);
            run_port_list_snapshot()
                .map(|(command_tool, text)| {
                    let text = filter_command_output(&text, filter.as_deref());
                (
                    text.clone(),
                        json!({"action":"port_list","exit_code":0,"filter":filter,"platform":std::env::consts::OS,"command_tool":command_tool,"output":text}),
                )
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

fn run_ps_snapshot(limit: usize, filter: Option<&str>) -> Result<String, String> {
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

    let mut lines = vec!["PID PPID %CPU %MEM COMM".to_string()];
    for row in rows.into_iter().take(limit) {
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
    Ok(format!(
        "exit={}\n{}",
        output.status.code().unwrap_or(0),
        lines.join("\n")
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
mod tests {
    use super::*;

    #[test]
    fn ps_filter_matches_command_case_insensitively() {
        let row = PsRow {
            pid: 42,
            ppid: 1,
            cpu: 0.0,
            mem: 0.0,
            comm: "clawd".to_string(),
        };

        assert!(ps_row_matches_filter(&row, Some("CLAWD")));
        assert!(!ps_row_matches_filter(&row, Some("telegramd")));
    }

    #[test]
    fn command_output_filter_keeps_exit_and_matching_rows() {
        let text = "exit=0\nLISTEN 0 128 0.0.0.0:8787 users:((\"clawd\",pid=1))\nLISTEN 0 128 0.0.0.0:5432";

        let filtered = filter_command_output(text, Some("8787"));

        assert!(filtered.starts_with("exit=0"));
        assert!(filtered.contains("8787"));
        assert!(!filtered.contains("5432"));
    }
}
