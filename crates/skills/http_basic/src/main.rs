use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::time::Duration;

use reqwest::blocking::Client;
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
        .unwrap_or("get");
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

    if action == "post_json" {
        let body = obj.get("body").cloned().unwrap_or(Value::Null);
        req = req.json(&body);
    }

    let resp = req
        .send()
        .map_err(|err| format!("http request failed: {err}"))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .map_err(|err| format!("read response failed: {err}"))?;
    let preview = if text.len() > 8000 { &text[..8000] } else { &text };

    Ok(format!("status={status}\n{preview}"))
}
