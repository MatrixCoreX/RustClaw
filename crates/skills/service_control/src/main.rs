use std::io::{self, BufRead, Write};
use std::path::PathBuf;
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
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("status");
    let root = workspace_root();

    match action {
        "status" => status_text(),
        "start" => run_script(&root, "./start-all.sh"),
        "stop" => run_script(&root, "./stop-rustclaw.sh"),
        "restart" => {
            let stop = run_script(&root, "./stop-rustclaw.sh")?;
            let start = run_script(&root, "./start-all.sh")?;
            Ok(format!("{stop}\n\n{start}"))
        }
        _ => Err("unsupported action; use status|start|stop|restart".to_string()),
    }
}

fn status_text() -> Result<String, String> {
    let clawd = process_count("clawd");
    let telegramd = process_count("telegramd");
    Ok(format!(
        "clawd_process_count={clawd}\ntelegramd_process_count={telegramd}"
    ))
}

fn run_script(root: &PathBuf, script: &str) -> Result<String, String> {
    let out = Command::new("bash")
        .arg("-lc")
        .arg(script)
        .current_dir(root)
        .output()
        .map_err(|err| format!("run script failed: {err}"))?;

    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    if !out.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    if text.len() > 12000 {
        text.truncate(12000);
        text.push_str("...(truncated)");
    }
    Ok(format!("exit={}\n{}", out.status.code().unwrap_or(-1), text))
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

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}
