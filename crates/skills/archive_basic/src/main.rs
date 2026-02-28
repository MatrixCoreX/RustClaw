use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
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
        .unwrap_or("list");
    let root = workspace_root();

    match action {
        "list" => {
            let archive = required_str(obj, "archive")?;
            let archive = resolve_path(&root, archive)?;
            list_archive(&archive)
        }
        "pack" => {
            let format = obj
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("zip");
            let source = resolve_path(&root, required_str(obj, "source")?)?;
            let archive = resolve_path(&root, required_str(obj, "archive")?)?;
            pack_archive(format, &source, &archive)
        }
        "unpack" => {
            let archive = resolve_path(&root, required_str(obj, "archive")?)?;
            let dest = resolve_path(&root, required_str(obj, "dest")?)?;
            unpack_archive(&archive, &dest)
        }
        _ => Err("unsupported action; use list|pack|unpack".to_string()),
    }
}

fn list_archive(archive: &Path) -> Result<String, String> {
    let name = archive.to_string_lossy().to_string();
    if name.ends_with(".zip") {
        run("unzip", &[String::from("-l"), name])
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        run("tar", &[String::from("-tzf"), name])
    } else {
        Err("unsupported archive format for list".to_string())
    }
}

fn pack_archive(format: &str, source: &Path, archive: &Path) -> Result<String, String> {
    let src = source.to_string_lossy().to_string();
    let out = archive.to_string_lossy().to_string();
    if let Some(parent) = archive.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("mkdir failed: {err}"))?;
    }

    match format {
        "zip" => run("zip", &[String::from("-r"), out, src]),
        "tar.gz" | "tgz" => run("tar", &[String::from("-czf"), out, src]),
        _ => Err("unsupported format; use zip|tar.gz".to_string()),
    }
}

fn unpack_archive(archive: &Path, dest: &Path) -> Result<String, String> {
    std::fs::create_dir_all(dest).map_err(|err| format!("mkdir failed: {err}"))?;
    let arc = archive.to_string_lossy().to_string();
    let dst = dest.to_string_lossy().to_string();
    if arc.ends_with(".zip") {
        run("unzip", &[arc, String::from("-d"), dst])
    } else if arc.ends_with(".tar.gz") || arc.ends_with(".tgz") {
        run("tar", &[String::from("-xzf"), arc, String::from("-C"), dst])
    } else {
        Err("unsupported archive format for unpack".to_string())
    }
}

fn run(bin: &str, args: &[String]) -> Result<String, String> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|err| format!("run {bin} failed: {err}"))?;
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    if text.len() > 10000 {
        text.truncate(10000);
    }
    Ok(format!("exit={}\n{}", output.status.code().unwrap_or(-1), text))
}

fn required_str<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Result<&'a str, String> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{key} is required"))
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
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
