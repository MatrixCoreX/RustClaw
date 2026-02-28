use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct SkillRequest {
    request_id: String,
    user_id: i64,
    chat_id: i64,
    skill_name: String,
    args: Value,
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct SkillResponse {
    request_id: String,
    status: String,
    text: String,
    buttons: Option<Value>,
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChildSkillResponse {
    request_id: Option<String>,
    status: Option<String>,
    text: Option<String>,
    buttons: Option<Value>,
    extra: Option<Value>,
    error_text: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<SkillRequest, _> = serde_json::from_str(&line);

        let resp = match parsed {
            Ok(req) => execute_skill(req),
            Err(err) => SkillResponse {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                buttons: None,
                extra: None,
                error_text: Some(format!("invalid request: {err}")),
            },
        };

        let out = serde_json::to_string(&resp)?;
        writeln!(stdout, "{out}")?;
        stdout.flush()?;
    }

    Ok(())
}

fn execute_skill(req: SkillRequest) -> SkillResponse {
    let timeout_secs: u64 = std::env::var("SKILL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(30);

    let child_bin = match skill_binary_path(&req.skill_name) {
        Ok(path) => path,
        Err(err) => {
            return SkillResponse {
                request_id: req.request_id,
                status: "error".to_string(),
                text: String::new(),
                buttons: None,
                extra: None,
                error_text: Some(err),
            }
        }
    };

    let child_req = serde_json::json!({
        "request_id": req.request_id,
        "args": req.args,
        "context": req.context,
        "user_id": req.user_id,
        "chat_id": req.chat_id,
    });

    match run_child_skill(&child_bin, &child_req.to_string(), Duration::from_secs(timeout_secs)) {
        Ok(out) => {
            let parsed: Result<ChildSkillResponse, _> = serde_json::from_str(&out);
            match parsed {
                Ok(v) => SkillResponse {
                    request_id: v.request_id.unwrap_or_else(|| "unknown".to_string()),
                    status: v.status.unwrap_or_else(|| "ok".to_string()),
                    text: v.text.unwrap_or_default(),
                    buttons: v.buttons,
                    extra: v.extra,
                    error_text: v.error_text,
                },
                Err(err) => SkillResponse {
                    request_id: "unknown".to_string(),
                    status: "error".to_string(),
                    text: String::new(),
                    buttons: None,
                    extra: None,
                    error_text: Some(format!("invalid child response: {err}; raw={out}")),
                },
            }
        }
        Err(err) => SkillResponse {
            request_id: req.request_id,
            status: "error".to_string(),
            text: String::new(),
            buttons: None,
            extra: None,
            error_text: Some(err),
        },
    }
}

fn skill_binary_path(skill_name: &str) -> Result<String, String> {
    match skill_name {
        "x" => {
            let path = "target/debug/x-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("x skill binary not found, build it first".to_string())
            }
        }
        "system_basic" => {
            let path = "target/debug/system-basic-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("system_basic skill binary not found, build it first".to_string())
            }
        }
        "http_basic" => {
            let path = "target/debug/http-basic-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("http_basic skill binary not found, build it first".to_string())
            }
        }
        "git_basic" => {
            let path = "target/debug/git-basic-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("git_basic skill binary not found, build it first".to_string())
            }
        }
        "install_module" => {
            let path = "target/debug/install-module-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("install_module skill binary not found, build it first".to_string())
            }
        }
        "process_basic" => {
            let path = "target/debug/process-basic-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("process_basic skill binary not found, build it first".to_string())
            }
        }
        "package_manager" => {
            let path = "target/debug/package-manager-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("package_manager skill binary not found, build it first".to_string())
            }
        }
        "archive_basic" => {
            let path = "target/debug/archive-basic-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("archive_basic skill binary not found, build it first".to_string())
            }
        }
        "db_basic" => {
            let path = "target/debug/db-basic-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("db_basic skill binary not found, build it first".to_string())
            }
        }
        "docker_basic" => {
            let path = "target/debug/docker-basic-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("docker_basic skill binary not found, build it first".to_string())
            }
        }
        "fs_search" => {
            let path = "target/debug/fs-search-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("fs_search skill binary not found, build it first".to_string())
            }
        }
        "rss_fetch" => {
            let path = "target/debug/rss-fetch-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("rss_fetch skill binary not found, build it first".to_string())
            }
        }
        "image_vision" => {
            let path = "target/debug/image-vision-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("image_vision skill binary not found, build it first".to_string())
            }
        }
        "image_generate" => {
            let path = "target/debug/image-generate-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("image_generate skill binary not found, build it first".to_string())
            }
        }
        "image_edit" => {
            let path = "target/debug/image-edit-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("image_edit skill binary not found, build it first".to_string())
            }
        }
        "audio_transcribe" => {
            let path = "target/debug/audio-transcribe-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("audio_transcribe skill binary not found, build it first".to_string())
            }
        }
        "audio_synthesize" => {
            let path = "target/debug/audio-synthesize-skill".to_string();
            if Path::new(&path).exists() {
                Ok(path)
            } else {
                Err("audio_synthesize skill binary not found, build it first".to_string())
            }
        }
        _ => Err(format!("unknown skill: {skill_name}")),
    }
}

fn run_child_skill(child_bin: &str, input_line: &str, timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(child_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("spawn child failed: {err}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("{input_line}\n").as_bytes())
            .map_err(|err| format!("write child stdin failed: {err}"))?;
        stdin
            .flush()
            .map_err(|err| format!("flush child stdin failed: {err}"))?;
    }

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return Err("child skill timeout".to_string());
                }
                thread::sleep(Duration::from_millis(30));
            }
            Err(err) => return Err(format!("wait child failed: {err}")),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|err| format!("collect child output failed: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("child exited with {:?}: {stderr}", output.status.code()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next().unwrap_or_default().trim().to_string();
    if line.is_empty() {
        return Err("child stdout is empty".to_string());
    }

    Ok(line)
}
