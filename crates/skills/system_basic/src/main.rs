use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_READ_FILE_BYTES: usize = 64 * 1024;
const MAX_WRITE_FILE_BYTES: usize = 128 * 1024;

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
            Ok(req) => handle(req),
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

fn handle(req: Req) -> Resp {
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let result = execute_action(&workspace_root, req.args);
    match result {
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
    }
}

fn execute_action(workspace_root: &Path, args: Value) -> Result<String, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("info");

    match action {
        "info" => system_info(),
        "list_dir" => {
            let path = obj.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            let real = resolve_path(workspace_root, path)?;
            list_dir(&real)
        }
        "make_dir" => {
            let path = obj
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "path is required".to_string())?;
            let real = resolve_path(workspace_root, path)?;
            std::fs::create_dir_all(&real).map_err(|err| format!("create_dir failed: {err}"))?;
            Ok(format!("directory created: {}", real.display()))
        }
        "read_file" => {
            let path = obj
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "path is required".to_string())?;
            let real = resolve_path(workspace_root, path)?;
            let data = std::fs::read(&real).map_err(|err| format!("read_file failed: {err}"))?;
            let clip = if data.len() > MAX_READ_FILE_BYTES {
                &data[..MAX_READ_FILE_BYTES]
            } else {
                &data
            };
            Ok(String::from_utf8_lossy(clip).to_string())
        }
        "write_file" => {
            let path = obj
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "path is required".to_string())?;
            let content = obj
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "content is required".to_string())?;
            if content.len() > MAX_WRITE_FILE_BYTES {
                return Err(format!("content too large: {} bytes", content.len()));
            }
            let real = resolve_path(workspace_root, path)?;
            if let Some(parent) = real.parent() {
                std::fs::create_dir_all(parent).map_err(|err| format!("mkdir failed: {err}"))?;
            }
            std::fs::write(&real, content).map_err(|err| format!("write_file failed: {err}"))?;
            Ok(format!("file written: {} ({} bytes)", real.display(), content.len()))
        }
        "remove_file" => {
            let path = obj
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "path is required".to_string())?;
            let real = resolve_path(workspace_root, path)?;
            let meta = std::fs::metadata(&real).map_err(|err| format!("stat failed: {err}"))?;
            if meta.is_dir() {
                return Err("remove_file only supports file".to_string());
            }
            std::fs::remove_file(&real).map_err(|err| format!("remove_file failed: {err}"))?;
            Ok(format!("file removed: {}", real.display()))
        }
        other => Err(format!(
            "unknown action: {other}; allowed: info|list_dir|make_dir|read_file|write_file|remove_file"
        )),
    }
}

fn system_info() -> Result<String, String> {
    let hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "unknown".to_string())
        .trim()
        .to_string();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let uptime = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(|v| v.to_string()))
        .unwrap_or_else(|| "-".to_string());
    let mem = memory_rss_bytes().unwrap_or(0);
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "-".to_string());

    Ok(json!({
        "hostname": hostname,
        "now_ts": now,
        "uptime_seconds": uptime,
        "process_rss_bytes": mem,
        "cwd": cwd
    })
    .to_string())
}

fn memory_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

fn list_dir(path: &Path) -> Result<String, String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|err| format!("read_dir failed: {err}"))? {
        let e = entry.map_err(|err| format!("dir entry failed: {err}"))?;
        let mut name = e.file_name().to_string_lossy().to_string();
        if e.path().is_dir() {
            name.push('/');
        }
        out.push(name);
        if out.len() >= 200 {
            break;
        }
    }
    out.sort();
    Ok(out.join("\n"))
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
