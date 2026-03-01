use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
    let default_path = root.join("logs/clawd.log");
    let path = obj
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or(default_path);
    let max_matches = obj
        .get("max_matches")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .min(200) as usize;

    let text = std::fs::read_to_string(&path).map_err(|err| format!("read log failed: {err}"))?;
    let default_keywords = [
        "error",
        "failed",
        "timeout",
        "panic",
        "queue full",
        "unauthorized",
    ];
    let keywords: Vec<String> = obj
        .get("keywords")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_ascii_lowercase())
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| default_keywords.iter().map(|s| s.to_string()).collect());

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut matches = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        let mut hit = false;
        for key in &keywords {
            if lower.contains(key) {
                *counts.entry(key.clone()).or_insert(0) += 1;
                hit = true;
            }
        }
        if hit {
            matches.push(format!("{}: {}", idx + 1, line));
        }
    }
    if matches.len() > max_matches {
        matches = matches[matches.len().saturating_sub(max_matches)..].to_vec();
    }

    Ok(json!({
        "path": path.display().to_string(),
        "total_lines": text.lines().count(),
        "keyword_counts": counts,
        "recent_matches": matches
    })
    .to_string())
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}
