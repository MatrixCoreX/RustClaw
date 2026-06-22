use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    context: Option<Value>,
    user_key: Option<String>,
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
            Ok(req) => {
                let req_ui_key = request_ui_key(&req);
                match execute(req.args, req_ui_key.as_deref()) {
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
                }
            }
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

fn request_ui_key(req: &Req) -> Option<String> {
    req.user_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            req.context
                .as_ref()
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("user_key"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

fn should_inject_rustclaw_key(url: &str) -> bool {
    url.starts_with("http://127.0.0.1:8787/")
        || url.starts_with("http://localhost:8787/")
        || url.starts_with("http://0.0.0.0:8787/")
}

fn execute(args: Value, req_user_key: Option<&str>) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;

    let action = obj.get("action").and_then(|v| v.as_str()).unwrap_or("get");
    let url = obj
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "url is required".to_string())?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("url must start with http:// or https://".to_string());
    }

    let timeout_seconds = obj
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .max(1)
        .min(120);

    let mut headers = HashMap::new();
    if let Some(map) = obj.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in map {
            if let Some(s) = v.as_str() {
                headers.insert(k.to_string(), s.to_string());
            }
        }
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|err| format!("build client failed: {err}"))?;

    let mut req = match action {
        "get" => client.get(url),
        "post_json" => client.post(url),
        _ => return Err("unsupported action; use get or post_json".to_string()),
    };

    // GitHub and some APIs require a User-Agent header.
    req = req.header("User-Agent", "RustClaw/1.0");

    for (k, v) in headers {
        req = req.header(k, v);
    }

    if should_inject_rustclaw_key(url) {
        if let Some(user_key) = req_user_key.map(str::trim).filter(|v| !v.is_empty()) {
            req = req.header("X-RustClaw-Key", user_key);
        }
    }

    if action == "post_json" {
        let body = obj.get("body").cloned().unwrap_or(Value::Null);
        req = req.json(&body);
    }

    let resp = req
        .send()
        .map_err(|err| format!("http request failed: {err}"))?;
    let status = resp.status().as_u16();
    let success = resp.status().is_success();
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = resp
        .bytes()
        .map_err(|err| format!("read response failed: {err}"))?;
    let text = String::from_utf8_lossy(&body);
    let preview = bounded_preview(&text, 8000);
    let artifact = if optional_bool(obj, "download").unwrap_or(false)
        || obj
            .get("output_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    {
        let workspace =
            workspace_root().map_err(|err| format!("resolve workspace failed: {err}"))?;
        let output_path = resolve_output_path(
            &workspace,
            "document/http/download",
            obj.get("output_path").and_then(Value::as_str),
        )?;
        ensure_parent_dir(&output_path)?;
        std::fs::write(&output_path, &body).map_err(|err| format!("write output failed: {err}"))?;
        Some(HttpArtifact {
            output_path: output_path.to_string_lossy().to_string(),
            size_bytes: body.len() as u64,
            content_type,
        })
    } else {
        None
    };

    Ok(http_observation(
        action,
        url,
        status,
        success,
        &preview,
        artifact.as_ref(),
    ))
}

#[derive(Debug)]
struct HttpArtifact {
    output_path: String,
    size_bytes: u64,
    content_type: Option<String>,
}

fn http_observation(
    action: &str,
    url: &str,
    status: u16,
    success_status: bool,
    preview: &str,
    artifact: Option<&HttpArtifact>,
) -> (String, Value) {
    let output = match artifact {
        Some(artifact) => format!(
            "status={status}\noutput_path={}\n{preview}",
            artifact.output_path
        ),
        None => format!("status={status}\n{preview}"),
    };
    let mut extra = json!({
        "action": action,
        "url": url,
        "status_code": status,
        "success_status": success_status,
        "body_preview": preview,
    });
    if let (Some(obj), Some(artifact)) = (extra.as_object_mut(), artifact) {
        obj.insert("downloaded".to_string(), json!(true));
        obj.insert("output_path".to_string(), json!(artifact.output_path));
        obj.insert("artifact_path".to_string(), json!(artifact.output_path));
        obj.insert("size_bytes".to_string(), json!(artifact.size_bytes));
        if let Some(content_type) = artifact.content_type.as_deref() {
            obj.insert("content_type".to_string(), json!(content_type));
        }
    }
    (output.clone(), extra)
}

fn optional_bool(obj: &serde_json::Map<String, Value>, key: &str) -> Option<bool> {
    obj.get(key).and_then(Value::as_bool)
}

fn bounded_preview(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn workspace_root() -> Result<PathBuf, std::io::Error> {
    std::env::current_dir()
}

fn resolve_output_path(
    workspace_root: &Path,
    default_dir: &str,
    requested: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(path) = requested.map(str::trim).filter(|value| !value.is_empty()) {
        let out = normalize_workspace_path(workspace_root, path)?;
        return Ok(out);
    }
    Ok(workspace_root
        .join(default_dir)
        .join(format!("http-{}.body", unix_ts())))
}

fn normalize_workspace_path(workspace_root: &Path, raw_path: &str) -> Result<PathBuf, String> {
    let p = Path::new(raw_path);
    let out = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    };
    if !out.starts_with(workspace_root) {
        return Err("output_path is outside workspace".to_string());
    }
    Ok(out)
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "output path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|err| format!("create output dir failed: {err}"))
}

fn unix_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
