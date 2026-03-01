use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use toml::Value as TomlValue;

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
    let config_path = obj
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("configs/config.toml"));

    let raw =
        std::fs::read_to_string(&config_path).map_err(|err| format!("read config failed: {err}"))?;
    let v: TomlValue = toml::from_str(&raw).map_err(|err| format!("parse toml failed: {err}"))?;

    let mut risks = Vec::new();
    if has_real_token(v.get("telegram").and_then(|x| x.get("bot_token")).and_then(|x| x.as_str())) {
        risks.push("telegram.bot_token looks like a real token".to_string());
    }
    for vendor in ["openai", "google", "anthropic", "grok"] {
        if has_real_token(
            v.get("llm")
                .and_then(|x| x.get(vendor))
                .and_then(|x| x.get("api_key"))
                .and_then(|x| x.as_str()),
        ) {
            risks.push(format!("llm.{vendor}.api_key looks like a real key"));
        }
    }

    if v.get("tools")
        .and_then(|x| x.get("allow_sudo"))
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
    {
        risks.push("tools.allow_sudo=true".to_string());
    }
    if v.get("tools")
        .and_then(|x| x.get("allow_path_outside_workspace"))
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
    {
        risks.push("tools.allow_path_outside_workspace=true".to_string());
    }

    if v.get("telegram")
        .and_then(|x| x.get("sendfile"))
        .and_then(|x| x.get("full_access"))
        .and_then(|x| x.as_bool())
        .unwrap_or(true)
    {
        risks.push("telegram.sendfile.full_access=true".to_string());
    }

    Ok(json!({
        "path": config_path.display().to_string(),
        "risk_count": risks.len(),
        "risks": risks
    })
    .to_string())
}

fn has_real_token(v: Option<&str>) -> bool {
    let Some(s) = v else { return false };
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    !t.starts_with("REPLACE_ME_")
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}
