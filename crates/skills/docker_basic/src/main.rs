use std::io::{self, BufRead, Write};
use std::process::Command;

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
    ensure_docker_available()?;
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("ps");

    match action {
        "ps" => run_docker(&["ps", "--format", "table {{.Names}}\t{{.Status}}\t{{.Ports}}"]),
        "images" => run_docker(&["images"]),
        "logs" => {
            let container = required(obj, "container")?;
            let tail = obj.get("tail").and_then(|v| v.as_u64()).unwrap_or(100).min(1000);
            run_docker(&["logs", "--tail", &tail.to_string(), container])
        }
        "restart" => run_docker(&["restart", required(obj, "container")?]),
        "start" => run_docker(&["start", required(obj, "container")?]),
        "stop" => run_docker(&["stop", required(obj, "container")?]),
        "inspect" => run_docker(&["inspect", required(obj, "container")?]),
        _ => Err("unsupported action; use ps|images|logs|restart|start|stop|inspect".to_string()),
    }
}

fn ensure_docker_available() -> Result<(), String> {
    let ok = Command::new("docker")
        .arg("--version")
        .status()
        .map_err(|err| format!("docker not available: {err}"))?;
    if ok.success() {
        Ok(())
    } else {
        Err("docker command is not available".to_string())
    }
}

fn required<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Result<&'a str, String> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{key} is required"))
}

fn run_docker(args: &[&str]) -> Result<String, String> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(|err| format!("run docker failed: {err}"))?;

    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    if text.len() > 12000 {
        text.truncate(12000);
    }
    Ok(format!("exit={}\n{}", output.status.code().unwrap_or(-1), text))
}
