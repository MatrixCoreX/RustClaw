use std::io::Read;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct XRequest {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
struct XResponse {
    request_id: String,
    status: String,
    text: String,
    error_text: Option<String>,
}

#[derive(Debug)]
struct XActionInput {
    text: String,
    dry_run: bool,
    send: bool,
}

#[derive(Debug, Deserialize, Default)]
struct XFileConfig {
    xurl_bin: Option<String>,
    xurl_app: Option<String>,
    xurl_auth: Option<String>,
    xurl_username: Option<String>,
    xurl_timeout_seconds: Option<u64>,
    require_explicit_send: Option<bool>,
    max_text_chars: Option<usize>,
}

#[derive(Debug, Clone)]
struct XRuntimeConfig {
    xurl_bin: String,
    xurl_app: Option<String>,
    xurl_auth: Option<String>,
    xurl_username: Option<String>,
    xurl_timeout_seconds: u64,
    require_explicit_send: bool,
    max_text_chars: usize,
}

#[derive(Debug)]
struct ChildOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<XRequest, _> = serde_json::from_str(&line);

        let resp = match parsed {
            Ok(req) => match parse_input(req.args) {
                Ok(input) => match run_x_post(input) {
                    Ok(text) => XResponse {
                        request_id: req.request_id,
                        status: "ok".to_string(),
                        text,
                        error_text: None,
                    },
                    Err(err) => XResponse {
                        request_id: req.request_id,
                        status: "error".to_string(),
                        text: String::new(),
                        error_text: Some(err),
                    },
                },
                Err(err) => XResponse {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    error_text: Some(err),
                },
            },
            Err(err) => XResponse {
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

fn parse_input(args: Value) -> Result<XActionInput, String> {
    match args {
        Value::String(s) => {
            let text = s.trim().to_string();
            if text.is_empty() {
                return Err("x skill text is empty".to_string());
            }
            Ok(XActionInput {
                text,
                dry_run: false,
                send: false,
            })
        }
        Value::Object(map) => {
            let text = map
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "x skill args.text must be string".to_string())?
                .trim()
                .to_string();
            if text.is_empty() {
                return Err("x skill text is empty".to_string());
            }

            let dry_run = map.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);
            let send = map
                .get("send")
                .or_else(|| map.get("confirm_send"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(XActionInput {
                text,
                dry_run,
                send,
            })
        }
        other => Err(format!(
            "x skill args must be string or object, got {}",
            other
        )),
    }
}

fn run_x_post(input: XActionInput) -> Result<String, String> {
    let runtime_cfg = load_runtime_config()?;

    if input.text.chars().count() > runtime_cfg.max_text_chars {
        return Err(format!(
            "x skill text exceeds max chars ({}), got {}",
            runtime_cfg.max_text_chars,
            input.text.chars().count()
        ));
    }

    if input.dry_run {
        return Ok(format!("x skill dry_run=1, preview post: {}", input.text));
    }

    if runtime_cfg.require_explicit_send && !input.send {
        return Ok(format!(
            "x skill safety: preview only (set send=true to publish). preview post: {}",
            input.text
        ));
    }

    run_x_post_via_xurl(&input.text, &runtime_cfg)
}

fn run_x_post_via_xurl(text: &str, runtime_cfg: &XRuntimeConfig) -> Result<String, String> {
    let mut cmd = Command::new(&runtime_cfg.xurl_bin);
    if let Some(app) = non_empty(runtime_cfg.xurl_app.as_deref()) {
        cmd.arg("--app").arg(app);
    }
    if let Some(auth) = non_empty(runtime_cfg.xurl_auth.as_deref()) {
        cmd.arg("--auth").arg(auth);
    }
    if let Some(username) = non_empty(runtime_cfg.xurl_username.as_deref()) {
        cmd.arg("--username").arg(username);
    }
    let body = serde_json::json!({ "text": text }).to_string();
    cmd.arg("-X")
        .arg("POST")
        .arg("/2/tweets")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-d")
        .arg(body);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|err| format!("spawn xurl failed (bin={}): {}", runtime_cfg.xurl_bin, err))?;
    let output = wait_with_timeout(&mut child, Duration::from_secs(runtime_cfg.xurl_timeout_seconds))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        let mut detail = String::new();
        if !stderr.is_empty() {
            detail.push_str(&stderr);
        }
        if !stdout.is_empty() {
            if !detail.is_empty() {
                detail.push_str(" | ");
            }
            detail.push_str(&stdout);
        }
        let msg = if detail.is_empty() {
            "xurl returned non-zero exit".to_string()
        } else {
            detail
        };
        return Err(format!(
            "xurl post failed. ensure `xurl auth oauth2` is completed with tweet.write scope; detail={}",
            msg
        ));
    }

    if let Ok(v) = serde_json::from_str::<Value>(&stdout) {
        let id = v
            .get("data")
            .and_then(|d| d.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let posted_text = v
            .get("data")
            .and_then(|d| d.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or(text);
        if !id.is_empty() {
            return Ok(format!("x post success via xurl: id={} text={}", id, posted_text));
        }
    }

    if stdout.is_empty() {
        return Ok("x post success via xurl".to_string());
    }
    Ok(format!("x post success via xurl: {}", truncate_text(&stdout, 600)))
}

fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> Result<ChildOutput, String> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return collect_child_output(child);
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "xurl timed out after {} seconds",
                        timeout.as_secs().max(1)
                    ));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(format!("wait xurl process failed: {err}")),
        }
    }
}

fn collect_child_output(child: &mut std::process::Child) -> Result<ChildOutput, String> {
    let status = child
        .wait()
        .map_err(|err| format!("wait xurl process failed: {err}"))?;
    let mut stdout = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_end(&mut stdout)
            .map_err(|err| format!("read xurl stdout failed: {err}"))?;
    }
    let mut stderr = Vec::new();
    if let Some(mut err_out) = child.stderr.take() {
        err_out
            .read_to_end(&mut stderr)
            .map_err(|err| format!("read xurl stderr failed: {err}"))?;
    }
    Ok(ChildOutput {
        status,
        stdout,
        stderr,
    })
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let trimmed: String = text.chars().take(max_chars).collect();
    format!("{trimmed}...")
}

fn load_runtime_config() -> Result<XRuntimeConfig, String> {
    let file_cfg = load_file_config()?;
    let xurl_bin = std::env::var("XURL_BIN")
        .ok()
        .or(file_cfg.xurl_bin)
        .unwrap_or_else(|| "xurl".to_string());
    let xurl_app = std::env::var("XURL_APP").ok().or(file_cfg.xurl_app);
    let xurl_auth = std::env::var("XURL_AUTH").ok().or(file_cfg.xurl_auth);
    let xurl_username = std::env::var("XURL_USERNAME")
        .ok()
        .or(file_cfg.xurl_username);
    let xurl_timeout_seconds = std::env::var("XURL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .or(file_cfg.xurl_timeout_seconds)
        .filter(|v| *v > 0)
        .unwrap_or(30);
    let require_explicit_send = env_bool("X_REQUIRE_EXPLICIT_SEND")
        .or(file_cfg.require_explicit_send)
        .unwrap_or(true);
    let max_text_chars = std::env::var("X_MAX_TEXT_CHARS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .or(file_cfg.max_text_chars)
        .filter(|v| *v > 0)
        .unwrap_or(280);

    Ok(XRuntimeConfig {
        xurl_bin,
        xurl_app,
        xurl_auth,
        xurl_username,
        xurl_timeout_seconds,
        require_explicit_send,
        max_text_chars,
    })
}

fn load_file_config() -> Result<XFileConfig, String> {
    let path = std::env::var("X_CONFIG_PATH").unwrap_or_else(|_| "configs/x.toml".to_string());
    if !Path::new(&path).exists() {
        return Ok(XFileConfig::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|err| format!("read x config file failed ({path}): {err}"))?;
    toml::from_str::<XFileConfig>(&content)
        .map_err(|err| format!("parse x config file failed ({path}): {err}"))
}

fn env_bool(key: &str) -> Option<bool> {
    let raw = std::env::var(key).ok()?;
    let v = raw.trim().to_ascii_lowercase();
    if matches!(v.as_str(), "1" | "true" | "yes" | "on") {
        return Some(true);
    }
    if matches!(v.as_str(), "0" | "false" | "no" | "off") {
        return Some(false);
    }
    None
}

fn non_empty(v: Option<&str>) -> Option<&str> {
    let s = v?.trim();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
