use std::fs::{create_dir_all, OpenOptions};
use std::io::Write as IoWrite;

use serde_json::json;
use tracing::warn;

use crate::{AppState, ClaimedTask};

pub(crate) fn truncate_for_log(text: &str) -> String {
    if text.len() <= crate::MODEL_IO_LOG_MAX_CHARS {
        return text.to_string();
    }
    let mut out = crate::utf8_safe_prefix(text, crate::MODEL_IO_LOG_MAX_CHARS).to_string();
    out.push_str("...(truncated)");
    out
}

pub(crate) fn highlight_tag(kind: &str) -> String {
    let upper = kind.to_ascii_uppercase();
    if !crate::log_color_enabled() {
        return format!("[{upper}]");
    }
    let code = match kind {
        "prompt" => "38;5;214",
        "skill" => "38;5;45",
        "tool" => "38;5;39",
        "loop" => "38;5;141",
        "llm" => "38;5;226",
        "skill_llm" => "38;5;49",
        "routing" => "38;5;198",
        _ => "1",
    };
    format!("\x1b[{code}m[{upper}]\x1b[0m")
}

pub(crate) fn append_subtask_result(
    subtask_results: &mut Vec<String>,
    index: usize,
    action_label: &str,
    success: bool,
    detail: &str,
) {
    let status = if success { "success" } else { "failed" };
    let detail_trimmed = detail.trim();
    if detail_trimmed.is_empty() {
        subtask_results.push(format!("subtask#{index} {action_label}: {status}"));
    } else {
        let header = format!("subtask#{index} {action_label}: {status}");
        subtask_results.push(format!("{}\n{}", header, detail_trimmed));
    }
}

pub(crate) fn append_act_plan_log(
    state: &AppState,
    task: &ClaimedTask,
    phase: &str,
    planned_steps: usize,
    action_steps_executed: usize,
    tool_calls: usize,
    detail: &str,
) {
    let logs_dir = state.workspace_root.join("logs");
    if let Err(err) = create_dir_all(&logs_dir) {
        warn!("create act plan logs dir failed: {err}");
        return;
    }
    let file_path = logs_dir.join("act_plan.log");
    let mut file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
    {
        Ok(f) => f,
        Err(err) => {
            warn!("open act plan log file failed: {err}");
            return;
        }
    };
    let line = json!({
        "ts": crate::now_ts_u64(),
        "call_id": task.task_id,
        "task_id": task.task_id,
        "user_id": task.user_id,
        "chat_id": task.chat_id,
        "phase": phase,
        "planned_steps": planned_steps,
        "action_steps_executed": action_steps_executed,
        "tool_calls": tool_calls,
        "detail": truncate_for_log(detail),
    })
    .to_string();
    if let Err(err) = writeln!(file, "{line}") {
        warn!("write act plan log failed: {err}");
    }
}

pub(crate) fn truncate_for_agent_trace(text: &str) -> String {
    if text.len() <= crate::AGENT_TRACE_LOG_MAX_CHARS {
        return text.to_string();
    }
    let mut out = crate::utf8_safe_prefix(text, crate::AGENT_TRACE_LOG_MAX_CHARS).to_string();
    out.push_str("...(truncated)");
    out
}
