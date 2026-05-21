use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MATCH_LINE_MAX_CHARS: usize = 240;

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

#[derive(Debug, Clone)]
struct LogAnalysis {
    requested_path: String,
    path: String,
    total_lines: usize,
    keyword_counts: BTreeMap<String, usize>,
    recent_matches: Vec<String>,
    level_counts: BTreeMap<String, usize>,
    recent_notable_lines: Vec<String>,
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

    let default_keywords = [
        "error",
        "warn",
        "warning",
        "failed",
        "timeout",
        "panic",
        "latency",
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

    let analysis = analyze_log_target(&path, &keywords, max_matches)?;
    Ok(json!({
        "requested_path": analysis.requested_path,
        "path": analysis.path,
        "total_lines": analysis.total_lines,
        "keyword_counts": analysis.keyword_counts,
        "recent_matches": analysis.recent_matches,
        "level_counts": analysis.level_counts,
        "recent_notable_lines": analysis.recent_notable_lines
    })
    .to_string())
}

fn resolve_log_path(path: &PathBuf) -> Result<PathBuf, String> {
    if path.is_file() {
        return Ok(path.clone());
    }
    if !path.exists() {
        return Err(format!("log path not found: {}", path.display()));
    }
    if !path.is_dir() {
        return Err(format!(
            "log path is neither file nor directory: {}",
            path.display()
        ));
    }

    let mut candidates: Vec<(u8, SystemTime, PathBuf)> = Vec::new();
    let entries = fs::read_dir(path).map_err(|err| format!("read log dir failed: {err}"))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read log dir entry failed: {err}"))?;
        let candidate_path = entry.path();
        if !candidate_path.is_file() {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|err| format!("read log file metadata failed: {err}"))?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        candidates.push((
            candidate_priority(&candidate_path),
            modified,
            candidate_path,
        ));
    }
    if candidates.is_empty() {
        return Err(format!(
            "log directory contains no readable files: {}",
            path.display()
        ));
    }
    candidates.sort_by(|a, b| b.cmp(a));
    Ok(candidates.remove(0).2)
}

fn analyze_log_target(
    path: &PathBuf,
    keywords: &[String],
    max_matches: usize,
) -> Result<LogAnalysis, String> {
    if path.is_dir() {
        return analyze_log_directory(path, keywords, max_matches);
    }
    let resolved = resolve_log_path(path)?;
    analyze_log_file(&resolved, path.display().to_string(), keywords, max_matches)
}

fn analyze_log_directory(
    path: &PathBuf,
    keywords: &[String],
    max_matches: usize,
) -> Result<LogAnalysis, String> {
    let entries = fs::read_dir(path).map_err(|err| format!("read log dir failed: {err}"))?;
    let mut best: Option<(usize, u8, SystemTime, LogAnalysis)> = None;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read log dir entry failed: {err}"))?;
        let candidate_path = entry.path();
        if !candidate_path.is_file() {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|err| format!("read log file metadata failed: {err}"))?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let analysis = match analyze_log_file(
            &candidate_path,
            path.display().to_string(),
            keywords,
            max_matches,
        ) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let total_hits = analysis.keyword_counts.values().copied().sum::<usize>();
        let priority = candidate_priority(&candidate_path);
        let replace = best
            .as_ref()
            .map(|(best_hits, best_priority, best_modified, _)| {
                (total_hits, priority, modified) > (*best_hits, *best_priority, *best_modified)
            })
            .unwrap_or(true);
        if replace {
            best = Some((total_hits, priority, modified, analysis));
        }
    }
    best.map(|(_, _, _, analysis)| analysis).ok_or_else(|| {
        format!(
            "log directory contains no readable files: {}",
            path.display()
        )
    })
}

fn analyze_log_file(
    resolved_path: &PathBuf,
    requested_path: String,
    keywords: &[String],
    max_matches: usize,
) -> Result<LogAnalysis, String> {
    let text =
        std::fs::read_to_string(resolved_path).map_err(|err| format!("read log failed: {err}"))?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut level_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut matches = Vec::new();
    let mut notable_lines = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let lower = line.to_ascii_lowercase();
        let mut hit = false;
        for key in keywords {
            if lower.contains(key) {
                *counts.entry(key.clone()).or_insert(0) += 1;
                hit = true;
            }
        }
        if let Some(level) = log_level_from_line(line) {
            *level_counts.entry(level.to_string()).or_insert(0) += 1;
            if log_level_is_notable(level) {
                notable_lines.push(format!(
                    "{}: {}",
                    idx + 1,
                    sanitize_match_line(line, MATCH_LINE_MAX_CHARS)
                ));
            }
        }
        if hit {
            matches.push(format!(
                "{}: {}",
                idx + 1,
                sanitize_match_line(line, MATCH_LINE_MAX_CHARS)
            ));
        }
    }
    if matches.len() > max_matches {
        matches = matches[matches.len().saturating_sub(max_matches)..].to_vec();
    }
    if notable_lines.len() > max_matches {
        notable_lines = notable_lines[notable_lines.len().saturating_sub(max_matches)..].to_vec();
    }
    Ok(LogAnalysis {
        requested_path,
        path: resolved_path.display().to_string(),
        total_lines: text.lines().count(),
        keyword_counts: counts,
        recent_matches: matches,
        level_counts,
        recent_notable_lines: notable_lines,
    })
}

fn log_level_from_line(line: &str) -> Option<&'static str> {
    line.split(|ch: char| !ch.is_ascii_alphanumeric())
        .find_map(|token| match token {
            "TRACE" => Some("trace"),
            "DEBUG" => Some("debug"),
            "INFO" => Some("info"),
            "WARN" | "WARNING" => Some("warn"),
            "ERROR" | "ERR" => Some("error"),
            "FATAL" | "CRITICAL" => Some("fatal"),
            "PANIC" => Some("panic"),
            _ => None,
        })
}

fn log_level_is_notable(level: &str) -> bool {
    matches!(level, "warn" | "error" | "fatal" | "panic")
}

fn candidate_priority(path: &std::path::Path) -> u8 {
    let file_name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let ext = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        file_name.as_str(),
        "clawd.log"
            | "telegramd.log"
            | "wechatd.log"
            | "whatsappd.log"
            | "whatsapp-webd.log"
            | "feishud.log"
            | "larkd.log"
            | "webd.log"
    ) {
        5
    } else if file_name.contains("model_io")
        || file_name.contains("task_journal")
        || file_name.contains("provider_request")
    {
        1
    } else if ext == "log" {
        4
    } else if ["txt", "out", "err"].contains(&ext.as_str()) || file_name.contains("log") {
        2
    } else {
        1
    }
}

fn sanitize_match_line(line: &str, max_chars: usize) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    out.push_str(" ...(truncated)");
    out
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

#[cfg(test)]
mod tests {
    use super::{analyze_log_file, candidate_priority, log_level_from_line, sanitize_match_line};
    use std::path::Path;

    #[test]
    fn candidate_priority_prefers_operational_logs_over_model_io() {
        assert!(
            candidate_priority(Path::new("clawd.log"))
                > candidate_priority(Path::new("model_io.log"))
        );
        assert!(
            candidate_priority(Path::new("telegramd.log"))
                > candidate_priority(Path::new("model_io.log"))
        );
    }

    #[test]
    fn sanitize_match_line_truncates_oversized_lines() {
        let long = "a".repeat(400);
        let out = sanitize_match_line(&long, 32);
        assert!(out.len() < long.len());
        assert!(out.ends_with("...(truncated)"));
    }

    #[test]
    fn detects_standard_log_levels_as_structured_notable_lines() {
        assert_eq!(
            log_level_from_line("2026-04-01 10:02:20 WARN upstream latency increased"),
            Some("warn")
        );
        assert_eq!(
            log_level_from_line("2026-04-01 10:08:44 ERROR provider timeout"),
            Some("error")
        );
    }

    #[test]
    fn default_analysis_keeps_warn_latency_visible() {
        let dir = std::env::temp_dir().join(format!(
            "rustclaw-log-analyze-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("app.log");
        std::fs::write(
            &path,
            "2026-04-01 10:00:00 INFO boot\n2026-04-01 10:02:20 WARN upstream latency increased to 820ms\n",
        )
        .expect("write log");
        let keywords = [
            "error",
            "warn",
            "warning",
            "failed",
            "timeout",
            "panic",
            "latency",
            "queue full",
            "unauthorized",
        ]
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

        let analysis = analyze_log_file(&path, path.display().to_string(), &keywords, 20)
            .expect("analysis");

        assert_eq!(analysis.level_counts.get("warn"), Some(&1));
        assert_eq!(analysis.keyword_counts.get("warn"), Some(&1));
        assert_eq!(analysis.keyword_counts.get("latency"), Some(&1));
        assert!(
            analysis
                .recent_notable_lines
                .iter()
                .any(|line| line.contains("latency increased"))
        );
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(dir);
    }
}
