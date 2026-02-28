use std::io::{self, BufRead, Write};
use std::fs::{OpenOptions, create_dir_all};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
                Ok(text) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn execute(args: Value) -> Result<String, String> {
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
            run_command(
                "ps",
                &[
                    "-eo",
                    "pid,ppid,%cpu,%mem,comm",
                    "--sort=-%cpu",
                ],
                Some(limit as usize + 1),
            )
        }
        "port_list" => run_command("ss", &["-ltnp"], None).or_else(|_| run_command("netstat", &["-ltnp"], None)),
        "kill" => {
            let pid = obj
                .get("pid")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| "pid is required".to_string())?;
            let signal = obj
                .get("signal")
                .and_then(|v| v.as_str())
                .unwrap_or("TERM");
            run_command("kill", &["-s", signal, &pid.to_string()], None)
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
            tail_file(&full, n)
        }
        _ => Err("unsupported action; use ps|port_list|kill|tail_log".to_string()),
    };

    append_service_log(action, obj, &result);
    result
}

fn run_command(bin: &str, args: &[&str], limit_lines: Option<usize>) -> Result<String, String> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|err| format!("run {bin} failed: {err}"))?;

    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }

    if let Some(limit) = limit_lines {
        let mut lines: Vec<&str> = text.lines().collect();
        if lines.len() > limit {
            lines.truncate(limit);
            lines.push("...truncated...");
            text = lines.join("\n");
        }
    }

    Ok(format!("exit={}\n{}", output.status.code().unwrap_or(-1), text))
}

fn tail_file(path: &Path, n: usize) -> Result<String, String> {
    let content = std::fs::read_to_string(path).map_err(|err| format!("read file failed: {err}"))?;
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
    let base = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        workspace_root.join(input)
    };
    if base.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("path with '..' is not allowed".to_string());
    }
    if !base.starts_with(workspace_root) {
        return Err("path is outside workspace".to_string());
    }
    Ok(base)
}

fn append_service_log(
    action: &str,
    args: &serde_json::Map<String, Value>,
    result: &Result<String, String>,
) {
    let root = workspace_root();
    let log_dir = root.join("logs");
    if let Err(err) = create_dir_all(&log_dir) {
        eprintln!("create service logs dir failed: {err}");
        return;
    }
    let file_path = log_dir.join("service_ops.log");
    let mut file = match OpenOptions::new().create(true).append(true).open(&file_path) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("open service log failed: {err}");
            return;
        }
    };

    let (status, output, error) = match result {
        Ok(text) => ("ok", Some(truncate_for_log(text)), None),
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
