use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
    let config_path = resolve_config_path(&root, obj);

    let raw = std::fs::read_to_string(&config_path)
        .map_err(|err| format!("read config failed: {err}"))?;
    let v: TomlValue = toml::from_str(&raw).map_err(|err| format!("parse toml failed: {err}"))?;

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
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rustclaw_config_guard_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("configs")).expect("create temp configs");
        root
    }

    #[test]
    fn resolve_config_path_uses_existing_requested_file() {
        let root = temp_root("existing_requested");
        let requested = root.join("custom.toml");
        std::fs::write(&requested, "[tools]\n").expect("write requested config");
        let obj = json!({ "path": requested.display().to_string() });
        let resolved = resolve_config_path(&root, obj.as_object().expect("object"));

        assert_eq!(resolved, requested);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resolve_config_path_falls_back_for_missing_configs_config_toml() {
        let root = temp_root("missing_requested");
        let default_path = root.join("configs/config.toml");
        std::fs::write(&default_path, "[tools]\n").expect("write default config");
        let obj =
            json!({ "path": root.join("rustclaw/configs/config.toml").display().to_string() });
        let resolved = resolve_config_path(&root, obj.as_object().expect("object"));

        assert_eq!(resolved, default_path);
        let _ = std::fs::remove_dir_all(root);
    }
}
