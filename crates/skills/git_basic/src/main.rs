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
    let root = std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    if !is_git_repo(&root) {
        return Ok("当前目录不是 git 仓库。请在 git 仓库目录中使用 git_basic。".to_string());
    }

    let (subcmd, mut extra): (&str, Vec<String>) = match action {
        "status" => ("status", vec!["--short".to_string(), "--branch".to_string()]),
        "log" => (
            "log",
            vec![
                "--oneline".to_string(),
                "-n".to_string(),
                obj.get("n")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20)
                    .min(100)
                    .to_string(),
            ],
        ),
        "diff" => ("diff", vec![]),
        "branch" => ("branch", vec!["--all".to_string()]),
        "show" => {
            let target = obj
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("HEAD");
            ("show", vec!["--stat".to_string(), target.to_string()])
        }
        "rev_parse" => ("rev-parse", vec!["HEAD".to_string()]),
        _ => {
            return Err(
                "unsupported action; use status|log|diff|branch|show|rev_parse".to_string(),
            );
        }
    };

    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .arg(subcmd)
        .args(extra.drain(..))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let out = cmd
        .output()
        .map_err(|err| format!("run git failed: {err}"))?;

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
    }

    Ok(format!("exit={}\n{}", out.status.code().unwrap_or(-1), text))
}

fn is_git_repo(root: &PathBuf) -> bool {
    root.join(".git").exists()
}
