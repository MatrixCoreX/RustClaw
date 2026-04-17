use std::sync::OnceLock;

use anyhow::anyhow;
use claw_core::hard_rules::main_flow::load_main_flow_rules;
use claw_core::hard_rules::types::MainFlowRules;
use claw_core::types::TaskStatus;
use rusqlite::Connection;
use serde_json::Value;

use crate::AppState;

pub(crate) const TASK_STATUS_QUEUED: &str = "queued";
pub(crate) const TASK_STATUS_RUNNING: &str = "running";
pub(crate) const TASK_STATUS_SUCCEEDED: &str = "succeeded";
pub(crate) const TASK_STATUS_FAILED: &str = "failed";
pub(crate) const TASK_STATUS_CANCELED: &str = "canceled";
pub(crate) const TASK_STATUS_TIMEOUT: &str = "timeout";

pub(crate) const CLASSIFIER_DIRECT_SOURCES: &[&str] = &[
    "voice_mode_intent_detect",
    "voice_mode_intent_detect_regression",
];

pub(crate) const RESUME_CONTINUE_SOURCES: &[&str] = &["resume_continue_execute"];

pub(crate) fn parse_task_status(raw: &str) -> TaskStatus {
    let s = raw.trim().to_ascii_lowercase();
    if s == TASK_STATUS_QUEUED {
        TaskStatus::Queued
    } else if s == TASK_STATUS_RUNNING {
        TaskStatus::Running
    } else if s == TASK_STATUS_SUCCEEDED {
        TaskStatus::Succeeded
    } else if s == TASK_STATUS_FAILED {
        TaskStatus::Failed
    } else if s == TASK_STATUS_CANCELED {
        TaskStatus::Canceled
    } else if s == TASK_STATUS_TIMEOUT {
        TaskStatus::Timeout
    } else {
        TaskStatus::Failed
    }
}

pub(crate) fn parse_resume_context_error(error_text: &str) -> Option<(String, Value)> {
    let trimmed = error_text.trim();
    let payload = trimmed.strip_prefix(crate::RESUME_CONTEXT_ERROR_PREFIX)?;
    let value: Value = serde_json::from_str(payload).ok()?;
    let user_error = value
        .get("user_error")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Task execution failed")
        .to_string();
    Some((user_error, value))
}

pub(crate) fn i18n_t_with_default(state: &AppState, key: &str, default_text: &str) -> String {
    state
        .schedule
        .i18n_dict
        .get(key)
        .cloned()
        .unwrap_or_else(|| default_text.to_string())
}

fn render_i18n_vars(mut text: String, vars: &[(&str, &str)]) -> String {
    for (name, value) in vars {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

pub(crate) fn i18n_t_with_default_vars(
    state: &AppState,
    key: &str,
    default_text: &str,
    vars: &[(&str, &str)],
) -> String {
    render_i18n_vars(i18n_t_with_default(state, key, default_text), vars)
}

pub(crate) fn bilingual_t_with_default(
    state: &AppState,
    key: &str,
    default_zh: &str,
    default_en: &str,
    prefer_english: bool,
) -> String {
    let configured_locale = state.schedule.locale.trim().to_ascii_lowercase();
    let configured_matches_requested = if prefer_english {
        configured_locale.starts_with("en")
    } else {
        configured_locale.starts_with("zh")
    };
    let default_text = if prefer_english {
        default_en
    } else {
        default_zh
    };
    if configured_matches_requested {
        i18n_t_with_default(state, key, default_text)
    } else {
        default_text.to_string()
    }
}

pub(crate) fn bilingual_t_with_default_vars(
    state: &AppState,
    key: &str,
    default_zh: &str,
    default_en: &str,
    prefer_english: bool,
    vars: &[(&str, &str)],
) -> String {
    render_i18n_vars(
        bilingual_t_with_default(state, key, default_zh, default_en, prefer_english),
        vars,
    )
}

pub(crate) fn ensure_column_exists(
    db: &Connection,
    table_name: &str,
    column_name: &str,
    alter_sql: &str,
) -> anyhow::Result<()> {
    let pragma = format!("PRAGMA table_info({table_name})");
    let mut stmt = db.prepare(&pragma)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for r in rows {
        if r?.eq_ignore_ascii_case(column_name) {
            return Ok(());
        }
    }
    db.execute(alter_sql, [])?;
    Ok(())
}

pub(crate) fn now_ts_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn now_ts() -> String {
    now_ts_u64().to_string()
}

pub(crate) fn main_flow_rules(state: &AppState) -> &'static MainFlowRules {
    static RULES: OnceLock<MainFlowRules> = OnceLock::new();
    RULES.get_or_init(|| {
        let path = state
            .workspace_root
            .join("configs/hard_rules/main_flow.toml");
        let path_str = path.to_string_lossy().to_string();
        load_main_flow_rules(&path_str)
    })
}

pub(crate) fn normalize_affirmation_text(text: &str) -> String {
    text.trim().to_ascii_lowercase()
}

pub(crate) fn is_affirmation_click_text(_state: &AppState, text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    matches!(t.as_str(), "y" | "yes")
}

pub(crate) fn mask_secret(raw: &str) -> String {
    let value = raw.trim();
    if value.is_empty() {
        return "-".to_string();
    }
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len().max(4));
    }
    let head: String = chars.iter().take(4).collect();
    let tail: String = chars
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}****{tail}")
}

pub(crate) fn normalize_exchange_name(raw: &str) -> anyhow::Result<String> {
    let exchange = raw.trim().to_ascii_lowercase();
    match exchange.as_str() {
        "binance" | "okx" => Ok(exchange),
        _ => Err(anyhow!("unsupported exchange: {exchange}")),
    }
}

pub(crate) fn normalize_external_id_opt(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}
