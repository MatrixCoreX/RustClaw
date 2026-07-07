use std::io::ErrorKind;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use toml::Value as TomlValue;

const SKILL_NAME: &str = "config_guard";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    error_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
}

#[derive(Debug)]
struct SkillError {
    kind: &'static str,
    message: String,
    extra: Option<Value>,
}

impl SkillError {
    fn new(kind: &'static str, message: impl Into<String>, extra: Option<Value>) -> Self {
        Self {
            kind,
            message: message.into(),
            extra,
        }
    }

    fn io(operation: &'static str, path: &std::path::Path, err: io::Error) -> Self {
        let kind = match err.kind() {
            ErrorKind::NotFound => "not_found",
            ErrorKind::PermissionDenied => "permission_denied",
            ErrorKind::InvalidInput => "invalid_input",
            ErrorKind::InvalidData => "invalid_data",
            _ => "io_error",
        };
        let path_text = path.display().to_string();
        Self::new(
            kind,
            format!("{operation} failed for {path_text}: {err}"),
            Some(json!({
                "error_kind": kind,
                "operation": operation,
                "path": path_text
            })),
        )
    }
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
                Ok(extra) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text: extra.to_string(),
                    extra: Some(extra),
                    error_text: None,
                    error_kind: None,
                    platform: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    extra: Some(error_extra_with_details(err.kind, err.extra)),
                    error_text: Some(err.message),
                    error_kind: Some(err.kind.to_string()),
                    platform: Some(std::env::consts::OS.to_string()),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra_with_details(
                    "invalid_input",
                    Some(json!({ "operation": "parse_request" })),
                )),
                error_text: Some(format!("invalid input: {err}")),
                error_kind: Some("invalid_input".to_string()),
                platform: Some(std::env::consts::OS.to_string()),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn error_extra_with_details(error_kind: &str, details: Option<Value>) -> Value {
    let mut extra = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    });
    if let Some(details) = details {
        if let (Some(base), Some(details_obj)) = (extra.as_object_mut(), details.as_object()) {
            for (key, value) in details_obj {
                base.entry(key.clone()).or_insert_with(|| value.clone());
            }
        } else if let Some(base) = extra.as_object_mut() {
            base.insert("details".to_string(), details);
        }
    }
    extra
}

fn execute(args: Value) -> Result<Value, SkillError> {
    let obj = args.as_object().ok_or_else(|| {
        SkillError::new(
            "invalid_input",
            "args must be object",
            Some(json!({
                "error_kind": "invalid_input",
                "operation": "parse_args"
            })),
        )
    })?;
    let root = workspace_root();
    let config_path = resolve_config_path(&root, obj);

    let raw = std::fs::read_to_string(&config_path)
        .map_err(|err| SkillError::io("read_config", &config_path, err))?;
    let v: TomlValue = toml::from_str(&raw).map_err(|err| {
        SkillError::new(
            "invalid_data",
            format!("parse_toml failed for {}: {err}", config_path.display()),
            Some(json!({
                "error_kind": "invalid_data",
                "operation": "parse_toml",
                "path": config_path.display().to_string()
            })),
        )
    })?;

    let mut risks = Vec::new();
    if has_real_token(
        v.get("telegram")
            .and_then(|x| x.get("bot_token"))
            .and_then(|x| x.as_str()),
    ) {
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
        "action": "scan",
        "path": config_path.display().to_string(),
        "risk_count": risks.len(),
        "risks": risks
    }))
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

fn discover_default_config_path(root: &PathBuf) -> PathBuf {
    let mut candidates = Vec::new();
    candidates.push(root.join("configs/config.toml"));

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("configs/config.toml"));
    }

    if let Ok(exe) = std::env::current_exe() {
        let mut parent = exe.parent().map(PathBuf::from);
        for _ in 0..8 {
            let Some(dir) = parent.clone() else { break };
            candidates.push(dir.join("configs/config.toml"));
            parent = dir.parent().map(PathBuf::from);
        }
    }

    candidates
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| root.join("configs/config.toml"))
}

fn resolve_config_path(root: &PathBuf, obj: &serde_json::Map<String, Value>) -> PathBuf {
    let default_path = discover_default_config_path(root);
    let Some(raw_path) = obj.get("path").and_then(|v| v.as_str()).map(str::trim) else {
        return default_path;
    };
    if raw_path.is_empty() {
        return default_path;
    }
    let requested = PathBuf::from(raw_path);
    if requested.is_file() {
        return requested;
    }
    if default_path.is_file() && looks_like_rustclaw_config_path(&requested) {
        return default_path;
    }
    requested
}

fn looks_like_rustclaw_config_path(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "config.toml")
        && path.components().any(|component| {
            matches!(
                component,
                std::path::Component::Normal(name) if name == "configs"
            )
        })
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
