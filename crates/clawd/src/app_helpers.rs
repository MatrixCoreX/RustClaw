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
        .policy
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
    let configured_locale = state.policy.schedule.locale.trim().to_ascii_lowercase();
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
        let requested_locale = if prefer_english { "en-US" } else { "zh-CN" };
        i18n_t_for_locale_with_default(state, requested_locale, key, default_text)
    }
}

pub(crate) fn localized_t_with_default(
    state: &AppState,
    key: &str,
    default_text: &str,
    locale_hint: &str,
) -> String {
    let locale = match locale_hint.trim().to_ascii_lowercase() {
        hint if hint.starts_with("en") => Some("en-US"),
        hint if hint.starts_with("zh") => Some("zh-CN"),
        hint if hint.starts_with("ja") => Some("ja"),
        hint if hint.starts_with("ko") => Some("ko"),
        _ => None,
    };
    if let Some(locale) = locale {
        let text = i18n_t_for_locale_with_default(state, locale, key, default_text);
        if text != default_text {
            return text;
        }
    }
    i18n_t_with_default(state, key, default_text)
}

fn i18n_t_for_locale_with_default(
    state: &AppState,
    locale: &str,
    key: &str,
    default_text: &str,
) -> String {
    let i18n_dir = state.policy.schedule.i18n_dir.trim();
    let i18n_dir = if i18n_dir.is_empty() {
        std::path::PathBuf::from("configs/i18n")
    } else {
        std::path::PathBuf::from(i18n_dir)
    };
    let i18n_dir = if i18n_dir.is_absolute() {
        i18n_dir
    } else {
        state.skill_rt.workspace_root.join(i18n_dir)
    };
    let suffix = format!(".{locale}.toml");
    let mut paths = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&i18n_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if file_name.ends_with(&suffix) {
                paths.push(path);
            }
        }
    }
    if paths.is_empty() {
        paths.push(i18n_dir.join(format!("schedule.{locale}.toml")));
    }
    paths.sort();
    for path in paths {
        let path = path.to_string_lossy();
        let text = claw_core::channel_i18n::text_from_path(path.as_ref(), key, default_text);
        if text != default_text {
            return text;
        }
    }
    default_text.to_string()
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
            .skill_rt
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
