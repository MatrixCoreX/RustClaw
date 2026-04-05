use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Component, Path};
use toml::Value as TomlValue;
use tracing::{debug, info, warn};

use crate::{repo, AgentAction, AppState, ClaimedTask};

#[derive(Debug, Clone)]
pub(super) struct AgentLoopGuardPolicy {
    pub(super) max_steps: usize,
    pub(super) max_rounds: usize,
    pub(super) repeat_action_limit: usize,
    pub(super) no_progress_limit: usize,
    pub(super) multi_round_enabled: bool,
}

fn parse_usize_from_toml(root: &TomlValue, path: &[&str], fallback: usize) -> usize {
    let mut cursor = root;
    for key in path {
        let Some(next) = cursor.get(*key) else {
            return fallback;
        };
        cursor = next;
    }
    cursor
        .as_integer()
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v >= 1)
        .unwrap_or(fallback)
}

fn parse_bool_from_toml(root: &TomlValue, path: &[&str], fallback: bool) -> bool {
    let mut cursor = root;
    for key in path {
        let Some(next) = cursor.get(*key) else {
            return fallback;
        };
        cursor = next;
    }
    cursor.as_bool().unwrap_or(fallback)
}

pub(super) fn load_agent_loop_guard_policy(state: &AppState) -> AgentLoopGuardPolicy {
    let path = state.workspace_root.join("configs/agent_guard.toml");
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| toml::from_str::<TomlValue>(&raw).ok())
        .unwrap_or(TomlValue::Table(Default::default()));
    AgentLoopGuardPolicy {
        max_steps: parse_usize_from_toml(
            &parsed,
            &["agent", "loop_guard", "max_steps"],
            crate::AGENT_MAX_STEPS,
        ),
        max_rounds: parse_usize_from_toml(&parsed, &["agent", "loop_guard", "max_rounds"], 2),
        repeat_action_limit: parse_usize_from_toml(
            &parsed,
            &["agent", "loop_guard", "repeat_action_limit"],
            4,
        ),
        no_progress_limit: parse_usize_from_toml(
            &parsed,
            &["agent", "loop_guard", "no_progress_limit"],
            1,
        ),
        multi_round_enabled: parse_bool_from_toml(
            &parsed,
            &["agent", "loop_guard", "multi_round_enabled"],
            true,
        ),
    }
}

/// Publish progress hints only. Used for "in progress" UI. Must not contain full raw tool/skill output.
fn publish_progress(state: &AppState, task: &ClaimedTask, progress_messages: &[String]) {
    if progress_messages.is_empty() {
        return;
    }
    let payload = json!({
        "progress_messages": progress_messages,
    });
    if let Err(err) = repo::update_task_progress_result(state, &task.task_id, &payload.to_string())
    {
        warn!(
            "run_agent_with_tools: task_id={} publish progress failed: {}",
            task.task_id, err
        );
    } else {
        debug!(
            "progress published task_id={} count={} last={}",
            task.task_id,
            progress_messages.len(),
            crate::truncate_for_log(progress_messages.last().map(|s| s.as_str()).unwrap_or(""))
        );
    }
}

/// Max length for args summary in progress hint. Longer summaries are truncated with "...".
pub(super) const PROGRESS_ARGS_SUMMARY_MAX_LEN: usize = 160;

/// Keys allowed in progress hint args summary (fixed order). Any other key is omitted.
const PROGRESS_ARGS_WHITELIST: &[&str] = &[
    "action",
    "exchange",
    "symbol",
    "side",
    "order_type",
    "quote_qty_usd",
    "qty",
    "price",
    "stop_price",
    "time_in_force",
    "limit",
    "order_id",
    "client_order_id",
];

/// Keys that must never appear in progress hint (case-insensitive substring match).
const PROGRESS_ARGS_SENSITIVE: &[&str] = &[
    "api_key",
    "api_secret",
    "passphrase",
    "user_key",
    "authorization",
    "token",
    "credential",
    "secret",
    "password",
];

fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_lowercase();
    PROGRESS_ARGS_SENSITIVE
        .iter()
        .any(|s| k.contains(&s.to_lowercase()))
}

fn value_to_short_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.as_str().trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        _ => v.to_string(),
    }
}

/// Build a safe, whitelisted args summary for progress hint. No sensitive keys; truncated to max_len.
pub(crate) fn build_safe_skill_args_summary(args: &Value, max_len: usize) -> String {
    let obj = match args.as_object() {
        Some(o) => o,
        None => return String::new(),
    };
    let mut parts: Vec<String> = Vec::new();
    for &key in PROGRESS_ARGS_WHITELIST {
        if is_sensitive_key(key) {
            continue;
        }
        let Some(v) = obj.get(key) else { continue };
        let s = value_to_short_string(v);
        if s.is_empty() {
            continue;
        }
        let val_display = if s.len() > 40 {
            format!("{}...", &s[..37])
        } else {
            s
        };
        parts.push(format!("{key}={val_display}"));
    }
    let summary = parts.join(", ");
    if summary.len() <= max_len {
        summary
    } else {
        format!(
            "{}...",
            summary
                .chars()
                .take(max_len.saturating_sub(3))
                .collect::<String>()
        )
    }
}

/// Encode a progress hint for telegramd to render with its i18n. Format: "I18N:key:json_vars".
pub(crate) fn encode_progress_i18n(key: &str, vars: &[(&str, &str)]) -> String {
    let obj: HashMap<String, String> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let vars_json = serde_json::to_string(&obj).unwrap_or_else(|_| "{}".to_string());
    format!("I18N:{}:{}", key, vars_json)
}

/// Append a short progress hint and publish. For "processing..." display only. Do not pass full raw output.
pub(super) fn append_progress_hint(
    state: &AppState,
    task: &ClaimedTask,
    progress_messages: &mut Vec<String>,
    hint: String,
) {
    progress_messages.push(hint);
    publish_progress(state, task, progress_messages);
}

/// Append to final delivery only. This is the only path that feeds user-visible result. No progress publish.
pub(super) fn append_delivery_message(
    task_id: &str,
    delivery_messages: &mut Vec<String>,
    message: String,
) {
    delivery_messages.push(message.clone());
    info!(
        "delivery appended task_id={} len={} content={}",
        task_id,
        delivery_messages.len(),
        crate::truncate_for_log(&message)
    );
}

pub(super) fn action_fingerprint(state: &AppState, action: &AgentAction) -> String {
    match action {
        AgentAction::CallTool { tool, args } => {
            let normalized_skill = state
                .resolve_canonical_skill_name(tool.trim())
                .to_ascii_lowercase();
            let normalized_args = normalize_args_for_fingerprint(&normalized_skill, args);
            format!(
                "skill:{}:{}",
                normalized_skill,
                canonical_json_string(&normalized_args)
            )
        }
        AgentAction::CallSkill { skill, args } => {
            let normalized_skill = state
                .resolve_canonical_skill_name(skill)
                .to_ascii_lowercase();
            let normalized_args = normalize_args_for_fingerprint(&normalized_skill, args);
            format!(
                "skill:{}:{}",
                normalized_skill,
                canonical_json_string(&normalized_args)
            )
        }
        AgentAction::Respond { content } => {
            format!("respond:{}", content.trim().to_ascii_lowercase())
        }
        AgentAction::Think { .. } => "think".to_string(),
    }
}

fn normalize_run_cmd_command_for_fingerprint(command: &str) -> String {
    let tokens = command
        .split_whitespace()
        .map(normalize_command_token_for_fingerprint)
        .collect::<Vec<_>>();
    tokens.join(" ")
}

fn normalize_command_token_for_fingerprint(token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }
    if token.starts_with('-') || token.contains('$') || token.contains('*') {
        return token.to_string();
    }
    if token.starts_with("./") || token.contains("/./") || token.contains("//") {
        return normalize_path_string_for_fingerprint(token);
    }
    token.to_string()
}

fn normalize_path_string_for_fingerprint(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    let mut quote_prefix = String::new();
    let mut quote_suffix = String::new();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        quote_prefix = s[..1].to_string();
        quote_suffix = s[s.len() - 1..].to_string();
        s = s[1..s.len().saturating_sub(1)].to_string();
    }

    while s.starts_with("./") {
        s = s[2..].to_string();
    }
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    s = s.replace("/./", "/");

    let path = Path::new(&s);
    let mut parts = Vec::new();
    let mut absolute = false;
    for comp in path.components() {
        match comp {
            Component::RootDir => absolute = true,
            Component::CurDir => {}
            Component::Normal(p) => parts.push(p.to_string_lossy().to_string()),
            Component::ParentDir => parts.push("..".to_string()),
            Component::Prefix(_) => {}
        }
    }
    let mut out = if absolute {
        format!("/{}", parts.join("/"))
    } else {
        parts.join("/")
    };
    if out.is_empty() {
        out = ".".to_string();
    }
    format!("{quote_prefix}{out}{quote_suffix}")
}

fn normalize_args_for_fingerprint(action_name: &str, args: &Value) -> Value {
    let mut out = args.clone();
    if action_name == "run_cmd" {
        if let Some(obj) = out.as_object_mut() {
            if let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) {
                obj.insert(
                    "command".to_string(),
                    Value::String(normalize_run_cmd_command_for_fingerprint(cmd)),
                );
            }
            if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
                obj.insert(
                    "cwd".to_string(),
                    Value::String(normalize_path_string_for_fingerprint(cwd)),
                );
            }
        }
    }
    out
}

fn canonicalize_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut out = serde_json::Map::new();
            for key in keys {
                if let Some(v) = map.get(&key) {
                    out.insert(key, canonicalize_json_value(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_json_value).collect()),
        Value::Number(num) => canonicalize_json_number(num),
        _ => value.clone(),
    }
}

fn canonicalize_json_number(num: &serde_json::Number) -> Value {
    if num.is_i64() || num.is_u64() {
        return Value::Number(num.clone());
    }
    let Some(float_value) = num.as_f64() else {
        return Value::Number(num.clone());
    };
    if !float_value.is_finite() {
        return Value::Number(num.clone());
    }
    let rounded = float_value.round();
    if (float_value - rounded).abs() <= 1e-12 {
        if rounded >= 0.0 && rounded <= u64::MAX as f64 {
            return Value::Number(serde_json::Number::from(rounded as u64));
        }
        if rounded >= i64::MIN as f64 && rounded <= i64::MAX as f64 {
            return Value::Number(serde_json::Number::from(rounded as i64));
        }
    }
    Value::Number(num.clone())
}

fn canonical_json_string(value: &Value) -> String {
    serde_json::to_string(&canonicalize_json_value(value)).unwrap_or_else(|_| value.to_string())
}
