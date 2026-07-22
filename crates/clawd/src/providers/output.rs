use std::fs::{create_dir_all, OpenOptions};
use std::io::{IsTerminal, Write as IoWrite};
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{Local, TimeZone};
use serde_json::{json, Value};
use tracing::warn;

use super::client::LlmUsageSnapshot;
use crate::{
    llm_model_kind, llm_vendor_name, now_ts_u64, truncate_for_log, AppState, ClaimedTask,
    LlmProviderRuntime,
};

static MODEL_IO_LOG_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn strip_think_blocks(raw: &str) -> String {
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

fn sanitize_provider_raw_response(raw: &str) -> (String, bool) {
    if let Ok(mut value) = serde_json::from_str::<Value>(raw) {
        sanitize_provider_raw_value(&mut value);
        let safe = value.to_string();
        return (safe.clone(), safe != raw.trim());
    }

    let mut values = Vec::new();
    let mut saw_record = false;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(mut value) = serde_json::from_str::<Value>(line) else {
            let safe = strip_think_blocks(raw)
                .replace("<think>", "")
                .replace("</think>", "");
            return (safe.clone(), safe != raw);
        };
        saw_record = true;
        sanitize_provider_raw_value(&mut value);
        values.push(value.to_string());
    }
    if saw_record {
        let safe = values.join("\n");
        return (safe.clone(), safe != raw.trim());
    }
    (raw.to_string(), false)
}

fn sanitize_provider_raw_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for key in [
                "reasoning",
                "reasoning_content",
                "reasoning_details",
                "reasoning_text",
                "thinking",
            ] {
                map.remove(key);
            }
            if let Some(Value::String(content)) = map.get_mut("content") {
                *content = strip_think_blocks(content)
                    .replace("<think>", "")
                    .replace("</think>", "");
            }
            for child in map.values_mut() {
                sanitize_provider_raw_value(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                sanitize_provider_raw_value(child);
            }
        }
        _ => {}
    }
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
    logical_call_index: u64,
    status: &str,
    prompt_source: &str,
    prompt: &str,
    request_payload: &Value,
    raw_response: Option<&str>,
    clean_response: Option<&str>,
    usage: Option<&LlmUsageSnapshot>,
    sanitized: bool,
    error: Option<&str>,
) {
    // 审计 H3：以前 `debug_log_prompt=false` 时这里直接 return，导致生产环境
    // 完全没有 LLM 审计日志。现在改成无条件写入：
    //   * `debug_log_prompt=true`  → "verbose" 行，包含 prompt / response / payload
    //   * `debug_log_prompt=false` → "slim" 行，只有 metadata + 字符数 + usage
    // 这样即使生产关闭 prompt 调试，也保留事件链路，方便事后追溯。
    let verbose = state.policy.routing.debug_log_prompt;
    let logs_dir = state.skill_rt.workspace_root.join("logs");
    let file_path = logs_dir.join("model_io.log");
    let (safe_raw_response, raw_response_sanitized) = raw_response
        .map(sanitize_provider_raw_response)
        .map(|(safe, changed)| (Some(safe), changed))
        .unwrap_or((None, false));
    let sanitized = sanitized || raw_response_sanitized;

    let line = if verbose {
        json!({
            "ts": now_ts_u64(),
            "mode": "verbose",
            "call_id": task.task_id,
            "task_id": task.task_id,
            "logical_call_index": logical_call_index,
            "user_id": task.user_id,
            "chat_id": task.chat_id,
            "vendor": llm_vendor_name(provider),
            "provider": provider.config.name,
            "provider_type": provider.config.provider_type,
            "model": provider.config.model,
            "model_kind": llm_model_kind(provider),
            "status": status,
            "prompt_source": prompt_source,
            // §7.5 Step 2.b：FNV-1a 64-bit hex(完整未截断 prompt)。
            //
            // §7.5 后 MODEL_IO_LOG_MAX_CHARS 提到 128_000，正常 prompt 不会再被
            // 截断，但 prompt_hash 字段仍保留作为：(a) 离线分析 / dashboard 的
            // O(1) 反查索引列；(b) 读老版本（pre-§7.5 / 阈值还是 16K）日志时
            // convert_model_io_log_to_fixture 仍能稳定命中，避免一次"prompt 被
            // 截断后 hash 反算不出"的回归。
            //
            // FNV 计算成本 ~微秒级，对 LLM 路径完全可忽略；slim 模式不加（slim
            // 本来就没 prompt，没法回放）。
            "prompt_hash": crate::providers::fixture_replay::fnv1a_64_hex(prompt),
            "prompt": truncate_for_log(prompt),
            "request_payload": request_payload,
            "response": clean_response.map(truncate_for_log),
            "raw_response": safe_raw_response.as_deref().map(truncate_for_log),
            "clean_response": clean_response.map(truncate_for_log),
            "usage": usage,
            "sanitized": sanitized,
            "error": error.map(truncate_for_log),
        })
    } else {
        json!({
            "ts": now_ts_u64(),
            "mode": "slim",
            "task_id": task.task_id,
            "logical_call_index": logical_call_index,
            "user_id": task.user_id,
            "chat_id": task.chat_id,
            "vendor": llm_vendor_name(provider),
            "provider": provider.config.name,
            "model": provider.config.model,
            "model_kind": llm_model_kind(provider),
            "status": status,
            "prompt_source": prompt_source,
            "prompt_chars": prompt.chars().count() as u64,
            "response_chars": clean_response.map(|s| s.chars().count() as u64),
            "usage": usage,
            "sanitized": sanitized,
            "error": error.map(truncate_for_log),
        })
    }
    .to_string();

    if let Err(err) = append_model_io_line(&file_path, &line) {
        warn!("write model io log failed: {err}");
    }
    // NOTE: 之前这里每写一行都会全量 read_to_string + JSON 解析 + write 整个
    // `model_io.log`，高频 LLM 调用下是 O(N²) 的磁盘放大。现在改由
    // `spawn_cleanup_worker`（默认 300s 一次）调用 `rotate_model_io_log_daily`
    // 完成"按日归档 + 旧档过期"，append 侧保持 O(1)。
}

fn append_model_io_line(file_path: &Path, line: &str) -> std::io::Result<()> {
    let _guard = MODEL_IO_LOG_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(parent) = file_path.parent() {
        create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)?;
    writeln!(file, "{line}")
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;

/// 默认保留多少天的 `model_io.log` 归档（含当天）。
pub(crate) const MODEL_IO_LOG_KEEP_DAYS: u64 = 7;

/// 按日轮转 `logs/model_io.log`：把非当天的行追加到 `model_io.log.YYYY-MM-DD`
/// 归档，主文件只保留当天的行；同时清理超过 `keep_days` 的旧归档。
///
/// 由后台 cleanup worker 周期调用，避免热路径上的全量重写。
/// 旧的 `prune_model_io_log_to_today` 会**直接丢弃**前一天日志，对生产事故复盘
/// 不友好；本函数把跨天的行迁到 dated archive 后再裁剪，保留 N 天可追溯窗口。
pub(crate) fn rotate_model_io_log_daily(file_path: &Path, keep_days: u64) -> anyhow::Result<()> {
    use std::collections::BTreeMap;
    let _guard = MODEL_IO_LOG_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !file_path.exists() {
        // 文件不存在，仍然要做一次旧归档清理。
        cleanup_model_io_log_archives(file_path, keep_days)?;
        return Ok(());
    }
    let raw = std::fs::read_to_string(file_path)?;
    if raw.trim().is_empty() {
        cleanup_model_io_log_archives(file_path, keep_days)?;
        return Ok(());
    }
    let today = Local::now().date_naive();
    let mut today_lines: Vec<String> = Vec::new();
    let mut by_archive_date: BTreeMap<chrono::NaiveDate, Vec<String>> = BTreeMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            today_lines.push(trimmed.to_string());
            continue;
        };
        let ts = value
            .get("ts")
            .and_then(|item| item.as_i64())
            .filter(|v| *v > 0);
        let Some(ts) = ts else {
            today_lines.push(trimmed.to_string());
            continue;
        };
        let Some(dt) = Local.timestamp_opt(ts, 0).single() else {
            today_lines.push(trimmed.to_string());
            continue;
        };
        let date = dt.date_naive();
        if date == today {
            today_lines.push(trimmed.to_string());
        } else {
            by_archive_date
                .entry(date)
                .or_default()
                .push(trimmed.to_string());
        }
    }

    let parent = file_path.parent().unwrap_or_else(|| Path::new("."));
    for (date, lines) in by_archive_date {
        if lines.is_empty() {
            continue;
        }
        let archive_name = format!("model_io.log.{}", date.format("%Y-%m-%d"));
        let archive_path = parent.join(archive_name);
        let mut archive_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&archive_path)?;
        for line in &lines {
            writeln!(archive_file, "{line}")?;
        }
    }

    let mut rewritten = today_lines.join("\n");
    if !rewritten.is_empty() {
        rewritten.push('\n');
    }
    let rewrite_path = file_path.with_extension("log.rotate.tmp");
    std::fs::write(&rewrite_path, rewritten)?;
    std::fs::rename(&rewrite_path, file_path)?;

    cleanup_model_io_log_archives(file_path, keep_days)?;
    Ok(())
}

fn cleanup_model_io_log_archives(file_path: &Path, keep_days: u64) -> anyhow::Result<()> {
    let parent = match file_path.parent() {
        Some(parent) => parent,
        None => return Ok(()),
    };
    if !parent.exists() {
        return Ok(());
    }
    let today = Local::now().date_naive();
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(err) => {
            warn!("read model io archives dir failed: {err}");
            return Ok(());
        }
    };
    for entry in entries.flatten() {
        let name_owned = entry.file_name();
        let Some(name) = name_owned.to_str() else {
            continue;
        };
        let Some(date_str) = name.strip_prefix("model_io.log.") else {
            continue;
        };
        let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
            continue;
        };
        let age_days = (today - date).num_days();
        if age_days >= keep_days as i64 {
            let path = entry.path();
            if let Err(err) = std::fs::remove_file(&path) {
                warn!(
                    "remove expired model io archive failed path={} err={err}",
                    path.display()
                );
            }
        }
    }
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
