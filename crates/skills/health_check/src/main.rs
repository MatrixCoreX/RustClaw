use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
    let root = workspace_root();
    let log_dir = obj
        .get("log_dir")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("logs"));

    let clawd_count = process_count("clawd");
    let telegramd_count = process_count("telegramd");
    let health_port_open = is_port_open("127.0.0.1", 8787);

    let clawd_log = summarize_log_file(&log_dir.join("clawd.log"));
    let telegramd_log = summarize_log_file(&log_dir.join("telegramd.log"));

    Ok(json!({
        "ts": now_ts(),
        "workspace_root": root.display().to_string(),
        "clawd_process_count": clawd_count,
        "telegramd_process_count": telegramd_count,
        "clawd_health_port_open": health_port_open,
        "clawd_log": clawd_log,
        "telegramd_log": telegramd_log
    })
    .to_string())
}

fn summarize_log_file(path: &PathBuf) -> Value {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return json!({"exists": false}),
    };
    let modified_ts = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut err_count = 0usize;
    for line in text.lines() {
        let l = line.to_ascii_lowercase();
        if l.contains("error")
            || l.contains("failed")
            || l.contains("panic")
            || l.contains("timeout")
            || l.contains("unauthorized")
        {
            err_count += 1;
        }
    }
    json!({
        "exists": true,
        "size_bytes": meta.len(),
        "modified_ts": modified_ts,
        "keyword_error_count": err_count
    })
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn process_count(keyword: &str) -> usize {
    let out = Command::new("pgrep")
        .args(["-fc", keyword])
        .output()
        .ok();
    out.and_then(|v| String::from_utf8(v.stdout).ok())
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

fn is_port_open(host: &str, port: u16) -> bool {
    std::net::TcpStream::connect((host, port)).is_ok()
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
