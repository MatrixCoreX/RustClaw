use std::fs::{create_dir_all, OpenOptions};
use std::io::{IsTerminal, Write as IoWrite};
use std::path::Path;
use std::sync::Arc;

use chrono::{Local, TimeZone};
use serde_json::{json, Value};
use tracing::warn;

use super::client::LlmUsageSnapshot;
use crate::{
    llm_model_kind, llm_vendor_name, now_ts_u64, truncate_for_log, AppState, ClaimedTask,
    LlmProviderRuntime,
};

fn strip_think_blocks(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    loop {
        if let Some(start) = rest.find("<think") {
            out.push_str(&rest[..start]);
            let after_start = &rest[start..];
            if let Some(close) = after_start.find("</think>") {
                rest = &after_start[close + "</think>".len()..];
                continue;
            }
            break;
        }
        out.push_str(rest);
        break;
    }
    out
}

fn strip_markdown_json_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed.to_string();
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest);
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let Some(body) = rest.strip_suffix("```") else {
        return trimmed.to_string();
    };
    body.trim().to_string()
}

fn sanitize_llm_text_output(raw: &str) -> String {
    let stripped = strip_think_blocks(raw);
    let without_think_tags = stripped.replace("<think>", "").replace("</think>", "");
    strip_markdown_json_fence(&without_think_tags)
        .trim()
        .to_string()
}

pub(crate) fn maybe_sanitize_llm_text_output(vendor: &str, raw: &str) -> (String, bool) {
    if vendor.eq_ignore_ascii_case("minimax") {
        let cleaned = sanitize_llm_text_output(raw);
        let sanitized = cleaned != raw.trim();
        return (cleaned, sanitized);
    }
    (raw.to_string(), false)
}

pub(crate) fn append_model_io_log(
    state: &AppState,
    task: &ClaimedTask,
    provider: &Arc<LlmProviderRuntime>,
    status: &str,
    prompt_file: &str,
    prompt: &str,
    request_payload: &Value,
    raw_response: Option<&str>,
    clean_response: Option<&str>,
    usage: Option<&LlmUsageSnapshot>,
    sanitized: bool,
    error: Option<&str>,
) {
    let logs_dir = state.workspace_root.join("logs");
    if let Err(err) = create_dir_all(&logs_dir) {
        warn!("create model io logs dir failed: {err}");
        return;
    }
    let file_path = logs_dir.join("model_io.log");
    let mut file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
    {
        Ok(f) => f,
        Err(err) => {
            warn!("open model io log file failed: {err}");
            return;
        }
    };

    let line = json!({
        "ts": now_ts_u64(),
        "call_id": task.task_id,
        "task_id": task.task_id,
        "user_id": task.user_id,
        "chat_id": task.chat_id,
        "vendor": llm_vendor_name(provider),
        "provider": provider.config.name,
        "provider_type": provider.config.provider_type,
        "model": provider.config.model,
        "model_kind": llm_model_kind(provider),
        "status": status,
        "prompt_file": prompt_file,
        "prompt": truncate_for_log(prompt),
        "request_payload": request_payload,
        "response": clean_response.map(truncate_for_log),
        "raw_response": raw_response.map(truncate_for_log),
        "clean_response": clean_response.map(truncate_for_log),
        "usage": usage,
        "sanitized": sanitized,
        "error": error.map(truncate_for_log),
    })
    .to_string();

    if let Err(err) = writeln!(file, "{line}") {
        warn!("write model io log failed: {err}");
        return;
    }
    drop(file);
    if let Err(err) = prune_model_io_log_to_today(&file_path) {
        warn!("prune model io log failed: {err}");
    }
}

fn prune_model_io_log_to_today(file_path: &Path) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(file_path)?;
    if raw.trim().is_empty() {
        return Ok(());
    }
    let today = Local::now().date_naive();
    let mut kept_lines = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let ts = value
            .get("ts")
            .and_then(|item| item.as_i64())
            .filter(|v| *v > 0);
        let Some(ts) = ts else {
            continue;
        };
        let Some(dt) = Local.timestamp_opt(ts, 0).single() else {
            continue;
        };
        if dt.date_naive() == today {
            kept_lines.push(trimmed.to_string());
        }
    }
    let mut rewritten = kept_lines.join("\n");
    if !rewritten.is_empty() {
        rewritten.push('\n');
    }
    std::fs::write(file_path, rewritten)?;
    Ok(())
}

pub(crate) fn log_color_enabled() -> bool {
    let is_tty = std::io::stdout().is_terminal() || std::io::stderr().is_terminal();
    if let Ok(v) = std::env::var("RUSTCLAW_LOG_COLOR") {
        let s = v.trim().to_ascii_lowercase();
        if matches!(s.as_str(), "0" | "false" | "no" | "off") {
            return false;
        }
        if matches!(s.as_str(), "1" | "true" | "yes" | "on") {
            return is_tty;
        }
    }
    if std::env::var("NO_COLOR").is_ok() {
        return false;
    }
    is_tty
}

pub(crate) fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let mut out = utf8_safe_prefix(text, max_chars).to_string();
    out.push_str("...(truncated)");
    out
}

pub(crate) fn utf8_safe_prefix(text: &str, max_len: usize) -> &str {
    if text.len() <= max_len {
        return text;
    }
    if max_len == 0 {
        return "";
    }
    let mut cut = 0usize;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if next > max_len {
            break;
        }
        cut = next;
    }
    &text[..cut]
}
