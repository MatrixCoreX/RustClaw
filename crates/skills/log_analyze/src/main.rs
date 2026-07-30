use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use skill_sdk::{BoundedResult, ContinuationDescriptor};

const MATCH_LINE_MAX_CHARS: usize = 240;
const RECOVERY_KEYWORDS: &[&str] = &["retry", "recover", "recovered", "succeeded", "success"];
const SKILL_NAME: &str = "log_analyze";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
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
    recovery_counts: BTreeMap<String, usize>,
    recent_recovery_lines: Vec<String>,
    tail_lines_requested: usize,
    tail_lines: Vec<String>,
    page_cursor: usize,
    snapshot_sha256: String,
    match_total: usize,
    notable_total: usize,
    recovery_total: usize,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
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
                    extra: Some(error_extra("execution_failed")),
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra("invalid_input")),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn error_extra(error_kind: &str) -> Value {
    json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_code": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

fn execute(args: Value) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let root = workspace_root();
    let default_path = root.join("logs/clawd.log");
    let requested_path = obj
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or(default_path);
    let requested_path_display = requested_path.display().to_string();
    let path = resolve_input_path(&root, requested_path);
    let max_matches = obj
        .get("max_matches")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .min(200) as usize;
    let tail_lines = requested_tail_lines(obj);
    let continuation = obj
        .get("continuation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

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
        "retry",
        "recovered",
        "succeeded",
        "success",
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

    let mut analysis = analyze_log_target(&path, &keywords, max_matches, tail_lines, continuation)?;
    analysis.requested_path = requested_path_display;
    let extra = log_analysis_extra(analysis);
    Ok((extra.to_string(), extra))
}

fn resolve_input_path(workspace_root: &Path, requested_path: PathBuf) -> PathBuf {
    if requested_path.is_absolute() {
        requested_path
    } else {
        workspace_root.join(requested_path)
    }
}

fn log_analysis_extra(analysis: LogAnalysis) -> Value {
    let match_page = bounded_log_page(
        analysis.recent_matches.clone(),
        analysis.match_total,
        analysis.page_cursor,
        &analysis.snapshot_sha256,
    );
    let notable_page = bounded_log_page(
        analysis.recent_notable_lines.clone(),
        analysis.notable_total,
        analysis.page_cursor,
        &analysis.snapshot_sha256,
    );
    let recovery_page = bounded_log_page(
        analysis.recent_recovery_lines.clone(),
        analysis.recovery_total,
        analysis.page_cursor,
        &analysis.snapshot_sha256,
    );
    json!({
        "action": "analyze_log",
        "requested_path": analysis.requested_path,
        "path": analysis.path,
        "total_lines": analysis.total_lines,
        "keyword_counts": analysis.keyword_counts,
        "recent_matches": analysis.recent_matches,
        "level_counts": analysis.level_counts,
        "recent_notable_lines": analysis.recent_notable_lines,
        "recovery_counts": analysis.recovery_counts,
        "recent_recovery_lines": analysis.recent_recovery_lines,
        "tail_lines_requested": analysis.tail_lines_requested,
        "tail_lines": analysis.tail_lines,
        "tail_excerpt": analysis.tail_lines.join("\n"),
        "snapshot_sha256": analysis.snapshot_sha256,
        "page_cursor": analysis.page_cursor,
        "bounded_results": {
            "matches": match_page,
            "notable_lines": notable_page,
            "recovery_lines": recovery_page,
        },
        "line_excerpt": {
            "complete": false,
            "max_chars_per_line": MATCH_LINE_MAX_CHARS,
            "partial_reason": "display_excerpt",
            "recovery": {
                "capability": "filesystem.read_range",
                "path": analysis.path,
                "unit": "line",
                "line_number_prefix_in_result": true,
            }
        }
    })
}

fn bounded_log_page(
    values: Vec<String>,
    total: usize,
    cursor: usize,
    snapshot_sha256: &str,
) -> BoundedResult<Vec<String>> {
    let next_cursor = cursor.saturating_add(values.len());
    let continuation = (next_cursor < total).then(|| ContinuationDescriptor {
        kind: "opaque".to_string(),
        token: Some(encode_log_continuation(next_cursor, snapshot_sha256)),
        state: json!({"direction": "older", "cursor": next_cursor}),
    });
    BoundedResult::page(
        values,
        next_cursor.saturating_sub(cursor) as u64,
        total as u64,
        continuation,
    )
}

fn requested_tail_lines(obj: &serde_json::Map<String, Value>) -> usize {
    ["tail_lines", "tail", "n"]
        .iter()
        .find_map(|key| obj.get(*key).and_then(|value| value.as_u64()))
        .unwrap_or(0)
        .min(200) as usize
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
    select_log_candidate(candidates).ok_or_else(|| {
        format!(
            "log directory contains no readable files: {}",
            path.display()
        )
    })
}

fn select_log_candidate(candidates: Vec<(u8, SystemTime, PathBuf)>) -> Option<PathBuf> {
    candidates
        .into_iter()
        .max_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| a.0.cmp(&b.0))
                .then_with(|| a.2.cmp(&b.2))
        })
        .map(|(_, _, path)| path)
}

fn analyze_log_target(
    path: &PathBuf,
    keywords: &[String],
    max_matches: usize,
    tail_lines: usize,
    continuation: Option<&str>,
) -> Result<LogAnalysis, String> {
    if path.is_dir() {
        return analyze_log_directory(path, keywords, max_matches, tail_lines, continuation);
    }
    let resolved = resolve_log_path(path)?;
    analyze_log_file_page(
        &resolved,
        path.display().to_string(),
        keywords,
        max_matches,
        tail_lines,
        continuation,
    )
}

fn analyze_log_directory(
    path: &PathBuf,
    keywords: &[String],
    max_matches: usize,
    tail_lines: usize,
    continuation: Option<&str>,
) -> Result<LogAnalysis, String> {
    let selected = resolve_log_path(path)?;
    analyze_log_file_page(
        &selected,
        path.display().to_string(),
        keywords,
        max_matches,
        tail_lines,
        continuation,
    )
}

#[cfg(test)]
fn analyze_log_file(
    resolved_path: &PathBuf,
    requested_path: String,
    keywords: &[String],
    max_matches: usize,
    tail_lines: usize,
) -> Result<LogAnalysis, String> {
    analyze_log_file_page(
        resolved_path,
        requested_path,
        keywords,
        max_matches,
        tail_lines,
        None,
    )
}

fn analyze_log_file_page(
    resolved_path: &PathBuf,
    requested_path: String,
    keywords: &[String],
    max_matches: usize,
    tail_lines: usize,
    continuation: Option<&str>,
) -> Result<LogAnalysis, String> {
    let text =
        std::fs::read_to_string(resolved_path).map_err(|err| format!("read log failed: {err}"))?;
    let snapshot_sha256 = format!("{:x}", Sha256::digest(text.as_bytes()));
    let page_cursor = continuation
        .map(|token| decode_log_continuation(token, &snapshot_sha256))
        .transpose()?
        .unwrap_or(0);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut level_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut matches = Vec::new();
    let mut notable_lines = Vec::new();
    let mut recovery_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut recovery_lines = Vec::new();
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
        let mut recovery_hit = false;
        for key in RECOVERY_KEYWORDS {
            if lower.contains(key) {
                *recovery_counts.entry((*key).to_string()).or_insert(0) += 1;
                recovery_hit = true;
            }
        }
        if recovery_hit {
            recovery_lines.push(format!(
                "{}: {}",
                idx + 1,
                sanitize_match_line(line, MATCH_LINE_MAX_CHARS)
            ));
        }
    }
    let match_total = matches.len();
    let notable_total = notable_lines.len();
    let recovery_total = recovery_lines.len();
    matches = recent_page(&matches, page_cursor, max_matches);
    notable_lines = recent_page(&notable_lines, page_cursor, max_matches);
    recovery_lines = recent_page(&recovery_lines, page_cursor, max_matches);
    Ok(LogAnalysis {
        requested_path,
        path: resolved_path.display().to_string(),
        total_lines: text.lines().count(),
        keyword_counts: counts,
        recent_matches: matches,
        level_counts,
        recent_notable_lines: notable_lines,
        recovery_counts,
        recent_recovery_lines: recovery_lines,
        tail_lines_requested: tail_lines,
        tail_lines: tail_excerpt_lines(&text, tail_lines),
        page_cursor,
        snapshot_sha256,
        match_total,
        notable_total,
        recovery_total,
    })
}

fn recent_page(values: &[String], cursor: usize, limit: usize) -> Vec<String> {
    let end = values.len().saturating_sub(cursor.min(values.len()));
    let start = end.saturating_sub(limit);
    values[start..end].to_vec()
}

fn encode_log_continuation(cursor: usize, snapshot_sha256: &str) -> String {
    format!("log_analyze_v1:{cursor}:sha256:{snapshot_sha256}")
}

fn decode_log_continuation(token: &str, snapshot_sha256: &str) -> Result<usize, String> {
    let mut parts = token.splitn(4, ':');
    if parts.next() != Some("log_analyze_v1") {
        return Err("invalid_continuation".to_string());
    }
    let cursor = parts
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| "invalid_continuation".to_string())?;
    if parts.next() != Some("sha256") || parts.next() != Some(snapshot_sha256) {
        return Err("stale_snapshot".to_string());
    }
    Ok(cursor)
}

fn tail_excerpt_lines(text: &str, requested: usize) -> Vec<String> {
    if requested == 0 {
        return Vec::new();
    }
    let lines = text.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(requested);
    lines
        .iter()
        .enumerate()
        .skip(start)
        .map(|(idx, line)| {
            format!(
                "{}: {}",
                idx + 1,
                sanitize_match_line(line, MATCH_LINE_MAX_CHARS)
            )
        })
        .collect()
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
#[path = "main_tests.rs"]
mod tests;
