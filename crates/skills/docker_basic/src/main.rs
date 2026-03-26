use std::io::{self, BufRead, Write};
use std::process::Command;

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
    ensure_docker_available()?;
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj.get("action").and_then(|v| v.as_str()).unwrap_or("ps");

    match action {
        "ps" => run_docker("ps", &[
            "ps",
            "--format",
            "table {{.Names}}\t{{.Status}}\t{{.Ports}}",
        ]),
        "images" => run_docker("images", &["images"]),
        "logs" => {
            let container = required(obj, "container")?;
            let tail = obj
                .get("tail")
                .and_then(|v| v.as_u64())
                .unwrap_or(100)
                .min(1000);
            run_docker(
                "logs",
                &["logs", "--tail", &tail.to_string(), container],
            )
        }
        "restart" => run_docker("restart", &["restart", required(obj, "container")?]),
        "start" => run_docker("start", &["start", required(obj, "container")?]),
        "stop" => run_docker("stop", &["stop", required(obj, "container")?]),
        "inspect" => run_docker("inspect", &["inspect", required(obj, "container")?]),
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

fn run_docker(action: &str, args: &[&str]) -> Result<(String, Value), String> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(|err| format!("run docker failed: {err}"))?;

    let mut text = format_command_output(&output.stdout, &output.stderr);
    if text.len() > 12000 {
        text.truncate(12000);
    }
    let exit_code = output.status.code().unwrap_or(-1);
    if output.status.success() {
        let output = format!("exit={exit_code}\n{text}");
        Ok((
            output.clone(),
            json!({
                "action": action,
                "exit_code": exit_code,
                "docker_args": args,
                "output": output,
            }),
        ))
    } else {
        Err(format!("docker command failed: exit={exit_code}\n{text}"))
    }
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
