use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Component, Path};
use toml::Value as TomlValue;
use tracing::{debug, info, warn};

use crate::{execution_adapters, llm_gateway, repo, AgentAction, AppState, AskReply, ClaimedTask};

const AGENT_TOOL_SPEC_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/agent_tool_spec.md");
const AGENT_TOOL_SPEC_PATH: &str = "prompts/agent_tool_spec.md";
const SINGLE_PLAN_EXECUTION_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/single_plan_execution_prompt.md");
const SINGLE_PLAN_EXECUTION_PROMPT_PATH: &str = "prompts/single_plan_execution_prompt.md";
const LOOP_INCREMENTAL_PLAN_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/loop_incremental_plan_prompt.md");
const LOOP_INCREMENTAL_PLAN_PROMPT_PATH: &str = "prompts/loop_incremental_plan_prompt.md";

fn load_rss_categories_for_prompt(state: &AppState) -> Vec<String> {
    let path = state.workspace_root.join("configs/rss.toml");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = toml::from_str::<TomlValue>(&raw) else {
        return Vec::new();
    };
    let mut out = value
        .get("rss")
        .and_then(|v| v.get("categories"))
        .and_then(|v| v.as_table())
        .map(|tbl| tbl.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    out.sort_unstable();
    out
}

fn build_rss_skill_prompt_with_categories(state: &AppState, base_prompt: &str) -> String {
    let base = base_prompt.trim();
    let categories = load_rss_categories_for_prompt(state);
    if categories.is_empty() {
        return base.to_string();
    }
    format!(
        "{base}\n\n## Category Guardrails\n- Allowed `category` values come from `configs/rss.toml` `[rss.categories]`: {categories}\n- When calling `rss_fetch`, `category` must be one of the allowed values.\n- If user intent cannot be mapped confidently, use `general`.\n- Do not invent unseen category strings.",
        categories = categories.join(", ")
    )
}

/// Phase 2+: Planner 可见技能按 task/agent 动态收敛：
/// （execution-enabled）∩（agent allowed_skills）。
/// 每个可见技能需在 registry 中提供 prompt_file 才会注入 playbook。
fn build_skill_playbooks_text(state: &AppState, task: &ClaimedTask) -> String {
    let enabled = state.planner_visible_skills_for_task(task);
    let enabled_count = enabled.len();
    let agent_id = state.task_agent_id(task);
    info!(
        "planner skill playbooks: agent_id={} planner_visible_skills_count={} skills=[{}]",
        agent_id,
        enabled_count,
        enabled.join(", ")
    );

    let mut sections = Vec::new();
    let mut skipped_no_prompt: Vec<String> = Vec::new();

    for skill in &enabled {
        let Some(registry_path) = state.skill_prompt_file(skill) else {
            warn!(
                "planner skill playbook: skill={} prompt_file missing in registry, skipping",
                skill
            );
            skipped_no_prompt.push(skill.clone());
            continue;
        };

        let prompt_body = crate::load_prompt_template_for_state(state, &registry_path, "").0;

        debug!(
            "planner skill playbook: skill={} prompt_file={} source=registry",
            skill, registry_path
        );

        let content = if skill == "rss_fetch" {
            build_rss_skill_prompt_with_categories(state, &prompt_body)
        } else {
            prompt_body
        };
        let trimmed = content.trim();
        if trimmed.is_empty() {
            continue;
        }
        sections.push(format!("### {skill}\n{trimmed}"));
    }

    if !skipped_no_prompt.is_empty() {
        warn!(
            "planner skill playbooks: skipped_no_prompt_count={} skills=[{}]",
            skipped_no_prompt.len(),
            skipped_no_prompt.join(", ")
        );
    }

    let included_count = sections.len();
    info!(
        "planner skill playbooks: included_count={} (enabled={} skipped={})",
        included_count,
        enabled_count,
        enabled_count.saturating_sub(included_count)
    );

    if sections.is_empty() {
        "No skill playbooks configured.".to_string()
    } else {
        sections.join("\n\n")
    }
}

#[derive(Debug, Clone)]
struct AgentLoopGuardPolicy {
    max_steps: usize,
    max_rounds: usize,
    repeat_action_limit: usize,
    no_progress_limit: usize,
    multi_round_enabled: bool,
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

fn load_agent_loop_guard_policy(state: &AppState) -> AgentLoopGuardPolicy {
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
const PROGRESS_ARGS_SUMMARY_MAX_LEN: usize = 160;

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
        // Truncate single value if very long (e.g. pasted text)
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
fn append_progress_hint(
    state: &AppState,
    task: &ClaimedTask,
    progress_messages: &mut Vec<String>,
    hint: String,
) {
    progress_messages.push(hint);
    publish_progress(state, task, progress_messages);
}

/// Append to final delivery only. This is the only path that feeds user-visible result. No progress publish.
fn append_delivery_message(task_id: &str, delivery_messages: &mut Vec<String>, message: String) {
    delivery_messages.push(message.clone());
    info!(
        "delivery appended task_id={} len={} content={}",
        task_id,
        delivery_messages.len(),
        crate::truncate_for_log(&message)
    );
}

#[derive(Debug, Deserialize)]
struct SinglePlanEnvelope {
    #[serde(default)]
    steps: Vec<Value>,
}

fn build_single_plan_prompt(
    prompt_template: &str,
    user_request: &str,
    goal: &str,
    tool_spec: &str,
    skill_playbooks: &str,
) -> String {
    crate::render_prompt_template(
        prompt_template,
        &[
            ("__USER_REQUEST__", user_request),
            ("__GOAL__", goal),
            ("__TOOL_SPEC__", tool_spec),
            ("__SKILL_PLAYBOOKS__", skill_playbooks),
        ],
    )
}

fn build_incremental_plan_prompt(
    prompt_template: &str,
    user_request: &str,
    goal: &str,
    tool_spec: &str,
    skill_playbooks: &str,
    round: usize,
    history_compact: &str,
    last_round_output: &str,
) -> String {
    crate::render_prompt_template(
        prompt_template,
        &[
            ("__USER_REQUEST__", user_request),
            ("__GOAL__", goal),
            ("__TOOL_SPEC__", tool_spec),
            ("__SKILL_PLAYBOOKS__", skill_playbooks),
            ("__ROUND__", &round.to_string()),
            ("__HISTORY_COMPACT__", history_compact),
            ("__LAST_ROUND_OUTPUT__", last_round_output),
        ],
    )
}

fn is_meta_respond_instruction(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let starts_with_meta = [
        "请基于",
        "基于",
        "根据上述",
        "根据上面的输出",
        "请根据",
        "based on the",
        "please analyze",
        "please explain",
        "use the above output",
    ]
    .iter()
    .any(|marker| {
        if marker.is_ascii() {
            lower.starts_with(marker)
        } else {
            trimmed.starts_with(marker)
        }
    });
    let has_instruction_phrases = [
        "请考虑以下因素",
        "不需要询问",
        "请查看上述",
        "重点关注",
        "如果需要我",
        "判断标准",
        "without seeing the actual output",
        "please run the command first",
        "please provide",
        "do not ask",
        "consider the following factors",
    ]
    .iter()
    .any(|marker| {
        if marker.is_ascii() {
            lower.contains(marker)
        } else {
            trimmed.contains(marker)
        }
    });
    starts_with_meta || has_instruction_phrases
}

fn parse_single_plan_actions(raw: &str, state: &AppState) -> Option<Vec<AgentAction>> {
    let value = crate::parse_llm_json_raw_or_any::<Value>(raw)?;
    let env = serde_json::from_value::<SinglePlanEnvelope>(value).ok()?;
    if env.steps.is_empty() {
        return None;
    }
    let mut actions = Vec::new();
    for step in env.steps {
        let raw_step = serde_json::to_string(&step).ok()?;
        let normalized = crate::parse_agent_action_json_with_repair(&raw_step, state).ok()?;
        let action = serde_json::from_value::<AgentAction>(normalized).ok()?;
        match action {
            AgentAction::Think { .. } => {}
            AgentAction::Respond { content }
                if !actions.is_empty() && is_meta_respond_instruction(&content) => {}
            _ => actions.push(action),
        }
    }
    if actions.is_empty() {
        None
    } else {
        Some(actions)
    }
}

/// Progress: short hints only (e.g. "Step 1/3", "Skill X completed"). For "in progress" UI. Not final content.
/// Delivery: final user-facing content only. Only respond and fallback finalizer append here. Channel consumes this.
/// Trace: step output / subtask_results / history_compact for logs and resume; not sent as final delivery.
#[derive(Debug, Default)]
struct LoopState {
    round_no: usize,
    max_rounds: usize,
    tool_calls_total: usize,
    total_steps_executed: usize,
    /// Progress hints only; published to task progress for "processing..." display. Must not contain full raw output.
    progress_messages: Vec<String>,
    /// Final delivery to user. Only respond and fallback finalizer write here. Sole source for AskReply.messages.
    delivery_messages: Vec<String>,
    subtask_results: Vec<String>,
    history_compact: Vec<String>,
    last_actions_fingerprint: Option<String>,
    repeat_action_counts: HashMap<String, usize>,
    successful_action_fingerprints: HashMap<String, usize>,
    consecutive_no_progress: usize,
    last_output: Option<String>,
    output_vars: HashMap<String, String>,
    has_tool_or_skill_output: bool,
    written_file_aliases: HashMap<String, String>,
    last_written_file_path: Option<String>,
    /// Last user-visible respond text (final or publishable). Used when delivery_messages was not filled so we do not fall back to subtask summary.
    last_user_visible_respond: Option<String>,
}

impl LoopState {
    fn new(max_rounds: usize) -> Self {
        Self {
            max_rounds,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
struct RoundOutcome {
    executed_actions: usize,
    had_error: bool,
    stop_signal: Option<String>,
    next_goal_hint: Option<String>,
    no_progress: bool,
}

fn action_fingerprint(state: &AppState, action: &AgentAction) -> String {
    match action {
        // LEGACY: CallTool normalized to skill view so capability/fingerprint is unified.
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
    // Normalize whole-number floats (e.g. 100.0) to integers so action
    // fingerprints treat semantically identical args as duplicates.
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

fn build_loop_history_compact(loop_state: &LoopState) -> String {
    if loop_state.history_compact.is_empty() {
        "(empty)".to_string()
    } else {
        loop_state.history_compact.join("\n")
    }
}

/// Trace only: updates loop_state for planner/resume. Does not write to progress or delivery.
fn register_step_output(
    loop_state: &mut LoopState,
    global_step: usize,
    round_step: usize,
    key_prefix: &str,
    output: &str,
) {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return;
    }
    debug!(
        "trace_only step_output key={} global_step={} round_step={}",
        key_prefix, global_step, round_step
    );
    let value = trimmed.to_string();
    loop_state.last_output = Some(value.clone());
    loop_state
        .output_vars
        .insert("last_output".to_string(), value.clone());
    loop_state
        .output_vars
        .insert(format!("s{global_step}.output"), value.clone());
    loop_state
        .output_vars
        .insert(format!("s{round_step}.output"), value.clone());
    loop_state
        .output_vars
        .insert(format!("{key_prefix}.last_output"), value);
}

fn remember_written_file_alias(
    loop_state: &mut LoopState,
    original_path: &str,
    effective_path: &str,
) {
    let original = original_path.trim();
    let effective = effective_path.trim();
    if original.is_empty() || effective.is_empty() || original == effective {
        return;
    }
    loop_state
        .written_file_aliases
        .insert(original.to_string(), effective.to_string());
    if let Some(name) = Path::new(original).file_name().and_then(|v| v.to_str()) {
        loop_state
            .written_file_aliases
            .entry(name.to_string())
            .or_insert_with(|| effective.to_string());
    }
    loop_state.last_written_file_path = Some(effective.to_string());
}

fn register_file_path_output(
    loop_state: &mut LoopState,
    global_step: usize,
    round_step: usize,
    key_prefix: &str,
    path: &str,
) {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return;
    }
    let value = trimmed.to_string();
    loop_state
        .output_vars
        .insert("last_file_path".to_string(), value.clone());
    loop_state
        .output_vars
        .insert("last_saved_file_path".to_string(), value.clone());
    loop_state
        .output_vars
        .insert("last_written_file_path".to_string(), value.clone());
    loop_state
        .output_vars
        .insert(format!("s{global_step}.path"), value.clone());
    loop_state
        .output_vars
        .insert(format!("s{round_step}.path"), value.clone());
    loop_state
        .output_vars
        .insert(format!("{key_prefix}.path"), value);
}

fn register_failed_step_output(
    loop_state: &mut LoopState,
    global_step: usize,
    round_step: usize,
    key_prefix: &str,
    failed_action: &str,
    err: &str,
) {
    let trimmed = err.trim();
    if !trimmed.is_empty() {
        loop_state.last_output = Some(trimmed.to_string());
        loop_state
            .output_vars
            .insert("last_output".to_string(), trimmed.to_string());
        loop_state
            .output_vars
            .insert("last_error".to_string(), trimmed.to_string());
        loop_state
            .output_vars
            .insert("failed_step.error".to_string(), trimmed.to_string());
        loop_state
            .output_vars
            .insert(format!("s{global_step}.error"), trimmed.to_string());
        loop_state
            .output_vars
            .insert(format!("s{round_step}.error"), trimmed.to_string());
        loop_state
            .output_vars
            .insert(format!("{key_prefix}.error"), trimmed.to_string());
    }
    loop_state.output_vars.insert(
        "failed_step.action".to_string(),
        failed_action.trim().to_string(),
    );
    loop_state
        .output_vars
        .insert("failed_step.index".to_string(), round_step.to_string());
}

fn rewrite_response_with_written_aliases(text: &str, loop_state: &LoopState) -> String {
    let mut out = text.to_string();
    for (alias, effective) in &loop_state.written_file_aliases {
        let file_alias = format!("FILE:{alias}");
        let file_effective = format!("FILE:{effective}");
        let image_alias = format!("IMAGE_FILE:{alias}");
        let image_effective = format!("IMAGE_FILE:{effective}");
        out = out.replace(&file_alias, &file_effective);
        out = out.replace(&image_alias, &image_effective);
        let trimmed = out.trim();
        if trimmed == alias {
            return effective.clone();
        }
        if trimmed == format!("`{alias}`") {
            return effective.clone();
        }
    }
    if let Some(saved_path) = loop_state.last_written_file_path.as_deref() {
        let trimmed = out.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("saved path:") && !trimmed.contains(saved_path) {
            return format!("Saved path: {saved_path}");
        }
        if (trimmed.starts_with("保存路径") || trimmed.starts_with("文件路径"))
            && !trimmed.contains(saved_path)
        {
            return format!("保存路径：{saved_path}");
        }
        if lower.contains("saved path to ")
            && lower.contains(": written ")
            && !trimmed.contains(saved_path)
        {
            return format!("Saved path: {saved_path}");
        }
    }
    out
}

fn has_remaining_action_after(
    actions: &[AgentAction],
    current_idx: usize,
    max_steps: usize,
) -> bool {
    actions
        .iter()
        .take(max_steps.max(1))
        .skip(current_idx + 1)
        .any(|action| !matches!(action, AgentAction::Think { .. }))
}

fn remaining_actions_are_discussion_only(
    state: &AppState,
    actions: &[AgentAction],
    current_idx: usize,
    max_steps: usize,
) -> bool {
    let remaining = actions
        .iter()
        .take(max_steps.max(1))
        .skip(current_idx + 1)
        .filter(|action| !matches!(action, AgentAction::Think { .. }))
        .collect::<Vec<_>>();
    !remaining.is_empty()
        && remaining.iter().all(|action| match action {
            AgentAction::Respond { .. } => true,
            AgentAction::CallSkill { skill, .. } => state
                .resolve_canonical_skill_name(skill)
                .eq_ignore_ascii_case("chat"),
            AgentAction::Think { .. } => true,
            _ => false,
        })
}

/// Parameters extracted from a trade_preview call for consistency check with the next trade_submit.
#[derive(Debug, Clone)]
pub(crate) struct TradePreviewParamsForConsistency {
    exchange: String,
    symbol: String,
    side: String,
    order_type: String,
    quote_qty_usd: Option<f64>,
    qty: f64,
    price: Option<f64>,
    stop_price: Option<f64>,
    time_in_force: Option<String>,
}

const TRADE_PARAMS_FLOAT_EPS: f64 = 1e-9;

/// Default exchange when preview/submit args omit it; must match crypto skill resolve_exchange fallback (binance).
const DEFAULT_CRYPTO_EXCHANGE: &str = "binance";

fn value_to_f64_opt(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
}

pub(crate) fn extract_trade_preview_params_for_consistency(
    args: &Value,
) -> Option<TradePreviewParamsForConsistency> {
    let obj = args.as_object()?;
    let exchange = obj
        .get("exchange")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| DEFAULT_CRYPTO_EXCHANGE.to_string());
    let symbol = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .map(str::to_string)?;
    let side = obj
        .get("side")
        .and_then(|v| v.as_str())
        .unwrap_or("buy")
        .trim()
        .to_ascii_lowercase();
    let order_type = obj
        .get("order_type")
        .and_then(|v| v.as_str())
        .unwrap_or("market")
        .trim()
        .to_ascii_lowercase();
    let quote_qty_usd = obj.get("quote_qty_usd").and_then(value_to_f64_opt);
    let qty = obj.get("qty").and_then(value_to_f64_opt).unwrap_or(0.0);
    let price = obj.get("price").and_then(value_to_f64_opt);
    let stop_price = obj
        .get("stop_price")
        .and_then(value_to_f64_opt)
        .or_else(|| obj.get("stopPrice").and_then(value_to_f64_opt));
    let time_in_force = obj
        .get("time_in_force")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(TradePreviewParamsForConsistency {
        exchange,
        symbol,
        side,
        order_type,
        quote_qty_usd,
        qty,
        price,
        stop_price,
        time_in_force,
    })
}

fn floats_near(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => (x - y).abs() <= TRADE_PARAMS_FLOAT_EPS,
        _ => false,
    }
}

fn trade_submit_params_consistent_with_preview(
    preview: &TradePreviewParamsForConsistency,
    submit_args: &Value,
) -> bool {
    let obj = match submit_args.as_object() {
        Some(o) => o,
        None => return false,
    };
    let submit_exchange = obj
        .get("exchange")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| DEFAULT_CRYPTO_EXCHANGE.to_string());
    if submit_exchange != preview.exchange {
        return false;
    }
    let symbol_ok = obj
        .get("symbol")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_uppercase())
        .as_ref()
        .map(|s| s == &preview.symbol.to_ascii_uppercase())
        .unwrap_or(false);
    if !symbol_ok {
        return false;
    }
    let side_ok = obj
        .get("side")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .as_ref()
        .map(|s| s == &preview.side)
        .unwrap_or(false);
    if !side_ok {
        return false;
    }
    let order_type_ok = obj
        .get("order_type")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .as_ref()
        .map(|s| s == &preview.order_type)
        .unwrap_or(false);
    if !order_type_ok {
        return false;
    }
    let quote_ok = floats_near(
        preview.quote_qty_usd,
        obj.get("quote_qty_usd").and_then(value_to_f64_opt),
    );
    if !quote_ok && preview.quote_qty_usd.is_some() {
        return false;
    }
    let qty_ok = floats_near(Some(preview.qty), obj.get("qty").and_then(value_to_f64_opt));
    if !qty_ok && preview.quote_qty_usd.is_none() {
        return false;
    }
    if !floats_near(preview.price, obj.get("price").and_then(value_to_f64_opt)) {
        return false;
    }
    if !floats_near(
        preview.stop_price,
        obj.get("stop_price")
            .and_then(value_to_f64_opt)
            .or_else(|| obj.get("stopPrice").and_then(value_to_f64_opt)),
    ) {
        return false;
    }
    let tif_submit = obj
        .get("time_in_force")
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_uppercase());
    if preview
        .time_in_force
        .as_deref()
        .map(|s| s.to_ascii_uppercase())
        != tif_submit
    {
        return false;
    }
    true
}

/// Result of checking whether to continue after trade_preview.
/// `Allow` = same-round matching trade_submit present; do not stop.
/// `Reject(reason)` = stop and wait for confirmation; reason is for logging.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TradePreviewContinue {
    Allow,
    Reject(&'static str),
}

/// Core check: given the next step's normalized skill and resolved args, decide Allow or Reject.
/// Used by check_continue_after_trade_preview and by unit tests.
pub(crate) fn check_continue_after_trade_preview_core(
    next_normalized_skill: Option<&str>,
    next_resolved_args: Option<&Value>,
    preview_params: &TradePreviewParamsForConsistency,
) -> TradePreviewContinue {
    let normalized = match next_normalized_skill {
        Some(s) => s,
        None => return TradePreviewContinue::Reject("preview_only_no_next_step"),
    };
    if normalized != "crypto" {
        return TradePreviewContinue::Reject("preview_only_next_not_crypto");
    }
    let resolved = match next_resolved_args {
        Some(v) => v,
        None => return TradePreviewContinue::Reject("preview_only_next_invalid_args"),
    };
    let obj = match resolved.as_object() {
        Some(o) => o,
        None => return TradePreviewContinue::Reject("preview_only_next_invalid_args"),
    };
    if obj.get("action").and_then(|v| v.as_str()) != Some("trade_submit") {
        return TradePreviewContinue::Reject("preview_only_next_not_submit");
    }
    if obj.get("confirm").and_then(|v| v.as_bool()) != Some(true) {
        return TradePreviewContinue::Reject("submit_missing_confirm_true");
    }
    if !trade_submit_params_consistent_with_preview(preview_params, resolved) {
        return TradePreviewContinue::Reject("submit_params_inconsistent_with_preview");
    }
    TradePreviewContinue::Allow
}

/// Returns Allow when the same-round next plan step is a valid trade_submit that matches the
/// preview (same exchange/symbol/side/order_type/qty/price/confirm=true). When Allow, the
/// executor should NOT stop after trade_preview and may continue to execute submit in the same round.
fn check_continue_after_trade_preview(
    state: &AppState,
    actions: &[AgentAction],
    idx: usize,
    loop_state: &LoopState,
    preview_params: &TradePreviewParamsForConsistency,
) -> TradePreviewContinue {
    let next_idx = idx + 1;
    let next_action = match actions.get(next_idx) {
        Some(a) => a,
        None => return TradePreviewContinue::Reject("preview_only_no_next_step"),
    };
    let (next_skill, next_args) = match next_action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return TradePreviewContinue::Reject("preview_only_next_not_skill"),
    };
    let normalized = state.resolve_canonical_skill_name(next_skill);
    let resolved = resolve_arg_value(next_args, loop_state);
    check_continue_after_trade_preview_core(
        Some(normalized.as_str()),
        Some(&resolved),
        preview_params,
    )
}

fn rewrite_run_cmd_with_written_aliases(command: &str, loop_state: &LoopState) -> String {
    if loop_state.written_file_aliases.is_empty() {
        return command.to_string();
    }
    let mut changed = false;
    let rewritten = command
        .split_whitespace()
        .map(|token| {
            let mut candidate = token.to_string();
            for (alias, effective) in &loop_state.written_file_aliases {
                let quoted = candidate.trim_matches(|c| c == '"' || c == '\'');
                let normalized = quoted.strip_prefix("./").unwrap_or(quoted);
                if normalized == alias {
                    changed = true;
                    if quoted.starts_with("./") {
                        candidate = candidate.replacen(&format!("./{normalized}"), effective, 1);
                    } else {
                        candidate = candidate.replacen(normalized, effective, 1);
                    }
                    break;
                }
            }
            candidate
        })
        .collect::<Vec<_>>()
        .join(" ");
    if changed {
        rewritten
    } else {
        command.to_string()
    }
}

fn rewrite_tool_path_with_written_aliases(tool: &str, args: &mut Value, loop_state: &LoopState) {
    if !matches!(tool, "read_file" | "remove_file") || loop_state.written_file_aliases.is_empty() {
        return;
    }
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    let Some(path) = obj.get("path").and_then(|v| v.as_str()) else {
        return;
    };
    let quoted = path.trim_matches(|c| c == '"' || c == '\'');
    let normalized = quoted.strip_prefix("./").unwrap_or(quoted);
    let Some(effective) = loop_state.written_file_aliases.get(normalized) else {
        return;
    };
    obj.insert("path".to_string(), Value::String(effective.clone()));
}

fn attach_recent_execution_context_to_chat_args(args: &mut Value, loop_state: &LoopState) {
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    let mut context_lines = loop_state
        .subtask_results
        .iter()
        .rev()
        .take(3)
        .cloned()
        .collect::<Vec<_>>();
    context_lines.reverse();
    if let Some(last_output) = loop_state
        .last_output
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        context_lines.push(format!(
            "latest_raw_output:\n{}",
            crate::truncate_for_agent_trace(last_output)
        ));
    }
    if context_lines.is_empty() {
        return;
    }
    let execution_context = format!(
        "Recent execution context for this same user request (use it when relevant; do not claim the user failed to provide it if it already appears below).\nStay grounded in the supplied execution context. If the subtask says to base the answer on a directory listing / file content / command output, do not invent unseen files, directories, paths, lines, or results:\n{}",
        context_lines.join("\n")
    );
    let merged_system_prompt = obj
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|existing| format!("{existing}\n\n{execution_context}"))
        .unwrap_or(execution_context);
    obj.insert(
        "system_prompt".to_string(),
        Value::String(merged_system_prompt),
    );
}

fn replace_double_brace_placeholders(input: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut cursor = input;
    loop {
        let Some(start) = cursor.find("{{") else {
            out.push_str(cursor);
            break;
        };
        out.push_str(&cursor[..start]);
        let remain = &cursor[start + 2..];
        let Some(end) = remain.find("}}") else {
            out.push_str(&cursor[start..]);
            break;
        };
        let key = remain[..end].trim();
        if let Some(v) = vars.get(key) {
            out.push_str(v);
        } else {
            out.push_str("{{");
            out.push_str(key);
            out.push_str("}}");
        }
        cursor = &remain[end + 2..];
    }
    out
}

fn single_brace_key(input: &str) -> Option<&str> {
    if !input.starts_with('{') || !input.ends_with('}') || input.starts_with("{{") {
        return None;
    }
    let key = &input[1..input.len().saturating_sub(1)];
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        Some(trimmed)
    } else {
        None
    }
}

fn angle_bracket_key(input: &str) -> Option<&str> {
    if !input.starts_with('<') || !input.ends_with('>') || input.len() < 3 {
        return None;
    }
    let key = input[1..input.len() - 1].trim();
    if key.is_empty() {
        return None;
    }
    Some(key)
}

fn resolve_arg_string(input: &str, loop_state: &LoopState) -> String {
    let replaced = replace_double_brace_placeholders(input, &loop_state.output_vars);
    if let Some(key) = single_brace_key(replaced.trim()) {
        if let Some(v) = loop_state.output_vars.get(key) {
            return v.clone();
        }
        if let Some(v) = &loop_state.last_output {
            return v.clone();
        }
    }
    if let Some(key) = angle_bracket_key(replaced.trim()) {
        if let Some(v) = loop_state.output_vars.get(key) {
            return v.clone();
        }
        let normalized_key = key.to_ascii_lowercase();
        if let Some(v) = loop_state.output_vars.get(&normalized_key) {
            return v.clone();
        }
        if normalized_key.contains("output") {
            if let Some(v) = &loop_state.last_output {
                return v.clone();
            }
        }
    }
    replaced
}

fn resolve_arg_value(value: &Value, loop_state: &LoopState) -> Value {
    match value {
        Value::String(s) => Value::String(resolve_arg_string(s, loop_state)),
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|v| resolve_arg_value(v, loop_state))
                .collect::<Vec<_>>(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), resolve_arg_value(v, loop_state));
            }
            Value::Object(out)
        }
        _ => value.clone(),
    }
}

async fn plan_round_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &LoopState,
) -> Result<Vec<AgentAction>, String> {
    let skill_playbooks = build_skill_playbooks_text(state, task);
    let (tool_spec_template, _) = crate::load_prompt_template_for_state(
        state,
        AGENT_TOOL_SPEC_PATH,
        AGENT_TOOL_SPEC_TEMPLATE,
    );
    let (prompt_name, prompt_file, prompt_text) = if loop_state.round_no <= 1 {
        let (prompt_template, prompt_file) = crate::load_prompt_template_for_state(
            state,
            SINGLE_PLAN_EXECUTION_PROMPT_PATH,
            SINGLE_PLAN_EXECUTION_PROMPT_TEMPLATE,
        );
        (
            "single_plan_execution_prompt",
            prompt_file,
            build_single_plan_prompt(
                &prompt_template,
                user_text,
                goal,
                &tool_spec_template,
                &skill_playbooks,
            ),
        )
    } else {
        let history_compact = build_loop_history_compact(loop_state);
        let last_output = loop_state
            .delivery_messages
            .last()
            .cloned()
            .unwrap_or_else(|| "(none)".to_string());
        let (prompt_template, prompt_file) = crate::load_prompt_template_for_state(
            state,
            LOOP_INCREMENTAL_PLAN_PROMPT_PATH,
            LOOP_INCREMENTAL_PLAN_PROMPT_TEMPLATE,
        );
        (
            "loop_incremental_plan_prompt",
            prompt_file,
            build_incremental_plan_prompt(
                &prompt_template,
                user_text,
                goal,
                &tool_spec_template,
                &skill_playbooks,
                loop_state.round_no,
                &history_compact,
                &last_output,
            ),
        )
    };
    crate::log_prompt_render(
        &task.task_id,
        prompt_name,
        &prompt_file,
        Some(loop_state.round_no),
    );
    info!(
        "{} loop_round_plan task_id={} round={} max_rounds={} max_steps={} multi_round_enabled={}",
        crate::highlight_tag("loop"),
        task.task_id,
        loop_state.round_no,
        policy.max_rounds,
        policy.max_steps,
        policy.multi_round_enabled
    );
    info!(
        "plan_llm_request task_id={} round={} user_request={}",
        task.task_id,
        loop_state.round_no,
        crate::truncate_for_log(user_text)
    );
    let plan_raw =
        llm_gateway::run_with_fallback_with_prompt_file(state, task, &prompt_text, &prompt_file)
            .await?;
    info!(
        "plan_llm_response task_id={} round={} raw={}",
        task.task_id,
        loop_state.round_no,
        crate::truncate_for_log(&plan_raw)
    );
    let plan_actions = parse_single_plan_actions(&plan_raw, state)
        .ok_or_else(|| "single plan parser failed: no executable steps".to_string())?;
    let labels: Vec<String> = plan_actions.iter().map(plan_step_label).collect();
    info!(
        "act_split_trace task_id={} round={} split_steps={}",
        task.task_id,
        loop_state.round_no,
        serde_json::to_string(&labels).unwrap_or_else(|_| "[]".to_string())
    );
    Ok(plan_actions)
}

fn is_numbered_subtask_summary(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return false;
    }
    let mut matched = 0usize;
    for (idx, line) in lines.iter().take(6).enumerate() {
        let no = idx + 1;
        let numbered_prefixes = [
            format!("{no}."),
            format!("{no})"),
            format!("{no}、"),
            format!("{no}："),
            format!("{no}:"),
        ];
        if numbered_prefixes
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            matched += 1;
        }
    }
    matched >= 2
}

fn is_summary_like_response(state: &AppState, text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if is_numbered_subtask_summary(trimmed) {
        return true;
    }
    let lines = trimmed
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return false;
    }
    let first_line = lines.first().copied().unwrap_or("");
    let lower = first_line.to_ascii_lowercase();
    crate::main_flow_rules(state)
        .summary_like_response_markers
        .iter()
        .any(|marker| {
            let marker = marker.trim();
            !marker.is_empty()
                && if marker.is_ascii() {
                    lower.contains(&marker.to_ascii_lowercase())
                } else {
                    first_line.contains(marker)
                }
        })
}

fn has_explicit_summary_request(state: &AppState, user_text: &str) -> bool {
    let trimmed = user_text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    crate::main_flow_rules(state)
        .explicit_summary_markers
        .iter()
        .any(|marker| {
            let marker = marker.trim();
            !marker.is_empty()
                && if marker.is_ascii() {
                    lower.contains(&marker.to_ascii_lowercase())
                } else {
                    trimmed.contains(marker)
                }
        })
}

/// Decide if this respond should enter delivery. Avoid duplicate: do not publish when respond
/// merely repeats the last delivery or the last raw step output.
fn should_publish_respond_message(
    state: &AppState,
    loop_state: &LoopState,
    user_text: &str,
    text: &str,
) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if !loop_state.has_tool_or_skill_output {
        return true;
    }
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|last| last.trim() == trimmed)
    {
        return false;
    }
    if loop_state
        .last_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|last| last == trimmed)
    {
        return false;
    }
    !is_summary_like_response(state, trimmed) || has_explicit_summary_request(state, user_text)
}

async fn execute_actions_once(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[AgentAction],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
) -> Result<RoundOutcome, String> {
    let mut executed_actions = 0usize;
    let mut stop_signal: Option<String> = None;
    let actionable_count = actions.iter().take(policy.max_steps.max(1)).count();
    let before_delivery_count = loop_state.delivery_messages.len();
    let before_progress_count = loop_state.progress_messages.len();
    let before_subtask_count = loop_state.subtask_results.len();
    let mut ended_with_user_visible_output = false;
    let round_steps: Vec<String> = actions.iter().map(plan_step_label).collect();
    for (idx, action) in actions.iter().take(policy.max_steps.max(1)).enumerate() {
        let step_in_round = idx + 1;
        let global_step = loop_state.total_steps_executed + 1;
        let fingerprint = action_fingerprint(state, action);
        let repeat_count = loop_state
            .repeat_action_counts
            .entry(fingerprint.clone())
            .or_insert(0);
        *repeat_count += 1;
        if let Some(success_count) = loop_state.successful_action_fingerprints.get(&fingerprint) {
            stop_signal = Some("repeat_completed_action".to_string());
            info!(
                "executor_result_error task_id={} round={} step={} type=guard error={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                format!(
                    "skip repeated successful action: count={} action={}",
                    success_count,
                    crate::truncate_for_log(&fingerprint)
                )
            );
            break;
        }
        if *repeat_count > policy.repeat_action_limit {
            stop_signal = Some("repeat_action_limit".to_string());
            info!(
                "executor_result_error task_id={} round={} step={} type=guard error={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                format!(
                    "repeat action guard triggered: count={} limit={} action={}",
                    *repeat_count,
                    policy.repeat_action_limit,
                    crate::truncate_for_log(&fingerprint)
                )
            );
            break;
        }

        info!(
            "executor_step_start task_id={} round={} step={} global_step={} action={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            global_step,
            plan_step_label(action)
        );
        loop_state.last_actions_fingerprint = Some(fingerprint.clone());
        // Main path: CallSkill (planner outputs only call_skill). CallTool is legacy fallback for parsed old input only.
        match action {
            // LEGACY COMPATIBILITY: CallTool only from old plans/history; normalizer now outputs call_skill only. Same dispatch as CallSkill (run_skill).
            AgentAction::CallTool { tool, args } => {
                let mut resolved_args = resolve_arg_value(args, loop_state);
                let normalized_skill = state.resolve_canonical_skill_name(tool);
                if normalized_skill == "chat" {
                    attach_recent_execution_context_to_chat_args(&mut resolved_args, loop_state);
                }
                let crypto_action = if normalized_skill == "crypto" {
                    resolved_args
                        .get("action")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                } else {
                    None
                };
                let write_file_effective_path = if normalized_skill == "write_file" {
                    resolved_args
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(|path| {
                            let effective =
                                crate::ensure_default_file_path(&state.workspace_root, path);
                            let user_visible = if Path::new(&effective).is_absolute() {
                                effective.clone()
                            } else {
                                state.workspace_root.join(&effective).display().to_string()
                            };
                            (path.to_string(), effective, user_visible)
                        })
                } else {
                    None
                };
                if normalized_skill == "run_cmd" {
                    if let Some(obj) = resolved_args.as_object_mut() {
                        if let Some(command) = obj.get("command").and_then(|v| v.as_str()) {
                            let rewritten =
                                rewrite_run_cmd_with_written_aliases(command, loop_state);
                            if rewritten != command {
                                obj.insert("command".to_string(), Value::String(rewritten));
                            }
                        }
                    }
                }
                rewrite_tool_path_with_written_aliases(
                    &normalized_skill,
                    &mut resolved_args,
                    loop_state,
                );
                loop_state.tool_calls_total += 1;
                let args_summary =
                    build_safe_skill_args_summary(&resolved_args, PROGRESS_ARGS_SUMMARY_MAX_LEN);
                info!(
                    "{} executor_step_execute task_id={} round={} step={} type=call_skill(legacy_tool) skill={} args={}",
                    crate::highlight_tag("skill"),
                    task.task_id,
                    loop_state.round_no,
                    step_in_round,
                    normalized_skill,
                    crate::truncate_for_log(&resolved_args.to_string())
                );
                match execution_adapters::run_skill(
                    state,
                    task,
                    &normalized_skill,
                    resolved_args.clone(),
                )
                .await
                {
                    Ok(out) => {
                        if let Some((original_path, _effective_path, user_visible_path)) =
                            &write_file_effective_path
                        {
                            remember_written_file_alias(
                                loop_state,
                                original_path,
                                user_visible_path,
                            );
                            register_file_path_output(
                                loop_state,
                                global_step,
                                step_in_round,
                                &format!("skill.{normalized_skill}"),
                                user_visible_path,
                            );
                        } else if tool == "read_file" {
                            if let Some(path) = resolved_args.get("path").and_then(|v| v.as_str()) {
                                register_file_path_output(
                                    loop_state,
                                    global_step,
                                    step_in_round,
                                    &format!("skill.{normalized_skill}"),
                                    path,
                                );
                            }
                        }
                        crate::append_subtask_result(
                            &mut loop_state.subtask_results,
                            global_step,
                            &format!("skill({normalized_skill})"),
                            true,
                            &out,
                        );
                        if !out.trim().is_empty() {
                            loop_state.has_tool_or_skill_output = true;
                            ended_with_user_visible_output = true;
                            let hint = if args_summary.is_empty() {
                                encode_progress_i18n(
                                    "telegram.progress.skill_completed",
                                    &[("skill", normalized_skill.as_str())],
                                )
                            } else {
                                encode_progress_i18n(
                                    "telegram.progress.skill_completed_with_args",
                                    &[
                                        ("skill", normalized_skill.as_str()),
                                        ("args_summary", args_summary.as_str()),
                                    ],
                                )
                            };
                            append_progress_hint(
                                state,
                                task,
                                &mut loop_state.progress_messages,
                                hint,
                            );
                        }
                        register_step_output(
                            loop_state,
                            global_step,
                            step_in_round,
                            &format!("skill.{normalized_skill}"),
                            &out,
                        );
                        *loop_state
                            .successful_action_fingerprints
                            .entry(fingerprint.clone())
                            .or_insert(0) += 1;
                        info!(
                            "executor_result_ok task_id={} round={} step={} type=call_skill(legacy_tool) output={} trace_only=raw_not_delivery",
                            task.task_id,
                            loop_state.round_no,
                            step_in_round,
                            crate::truncate_for_log(&out)
                        );
                        loop_state.history_compact.push(format!(
                            "round={} step={} skill={} ok",
                            loop_state.round_no, step_in_round, normalized_skill
                        ));
                        if crypto_action.as_deref() == Some("trade_preview") {
                            let decision =
                                extract_trade_preview_params_for_consistency(&resolved_args)
                                    .as_ref()
                                    .map(|p| {
                                        check_continue_after_trade_preview(
                                            state, actions, idx, loop_state, p,
                                        )
                                    })
                                    .unwrap_or(TradePreviewContinue::Reject(
                                        "preview_params_extract_failed",
                                    ));
                            match &decision {
                                TradePreviewContinue::Allow => {
                                    info!(
                                        "trade_preview_followed_by_submit_continue task_id={} round={} step={} reason=same_round_submit_matching",
                                        task.task_id, loop_state.round_no, step_in_round
                                    );
                                }
                                TradePreviewContinue::Reject(reason) => {
                                    info!(
                                        "trade_preview_awaiting_confirmation task_id={} round={} step={} reason={}",
                                        task.task_id, loop_state.round_no, step_in_round, reason
                                    );
                                    executed_actions += 1;
                                    loop_state.total_steps_executed += 1;
                                    stop_signal =
                                        Some("trade_preview_awaiting_confirmation".to_string());
                                    break;
                                }
                            }
                        }
                    }
                    Err(err) => {
                        crate::append_subtask_result(
                            &mut loop_state.subtask_results,
                            global_step,
                            &format!("skill({normalized_skill})"),
                            false,
                            &err,
                        );
                        info!(
                            "executor_result_error task_id={} round={} step={} type=call_skill(legacy_tool) error={}",
                            task.task_id,
                            loop_state.round_no,
                            step_in_round,
                            crate::truncate_for_log(&err)
                        );
                        if remaining_actions_are_discussion_only(
                            state,
                            actions,
                            idx,
                            policy.max_steps,
                        ) {
                            register_failed_step_output(
                                loop_state,
                                global_step,
                                step_in_round,
                                &format!("skill.{normalized_skill}"),
                                &format!("skill({normalized_skill})"),
                                &err,
                            );
                            loop_state.history_compact.push(format!(
                                "round={} step={} skill={} failed error={}",
                                loop_state.round_no,
                                step_in_round,
                                normalized_skill,
                                crate::truncate_for_agent_trace(&err)
                            ));
                            executed_actions += 1;
                            loop_state.total_steps_executed += 1;
                            stop_signal = Some("recoverable_failure_continue_round".to_string());
                            break;
                        }
                        let resume_err = build_resume_context_error(
                            actions,
                            &round_steps,
                            user_text,
                            goal,
                            &loop_state.subtask_results,
                            &loop_state.delivery_messages,
                            step_in_round,
                            &format!("skill({normalized_skill})"),
                            &err,
                        );
                        return Err(resume_err);
                    }
                }
            }
            AgentAction::CallSkill { skill, args } => {
                let mut resolved_args = resolve_arg_value(args, loop_state);
                loop_state.tool_calls_total += 1;
                let normalized_skill = state.resolve_canonical_skill_name(skill);
                let write_file_effective_path = if normalized_skill == "write_file" {
                    resolved_args
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(|path| {
                            let effective =
                                crate::ensure_default_file_path(&state.workspace_root, path);
                            let user_visible = if Path::new(&effective).is_absolute() {
                                effective.clone()
                            } else {
                                state.workspace_root.join(&effective).display().to_string()
                            };
                            (path.to_string(), effective, user_visible)
                        })
                } else {
                    None
                };
                if normalized_skill == "chat" {
                    attach_recent_execution_context_to_chat_args(&mut resolved_args, loop_state);
                }
                // Capture action name before resolved_args is moved into run_skill (e.g. for trade_preview stop).
                let crypto_action = if normalized_skill == "crypto" {
                    resolved_args
                        .get("action")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                } else {
                    None
                };
                // Extract preview params for same-round submit check (must be before run_skill consumes resolved_args).
                let preview_params = if normalized_skill == "crypto"
                    && crypto_action.as_deref() == Some("trade_preview")
                {
                    extract_trade_preview_params_for_consistency(&resolved_args)
                } else {
                    None
                };
                let args_summary =
                    build_safe_skill_args_summary(&resolved_args, PROGRESS_ARGS_SUMMARY_MAX_LEN);
                // Whether to require user confirmation before trade_submit is decided by the planner; no hard block here.
                info!(
                    "{} executor_step_execute task_id={} round={} step={} type=call_skill skill={} args={}",
                    crate::highlight_tag("skill"),
                    task.task_id,
                    loop_state.round_no,
                    step_in_round,
                    normalized_skill,
                    crate::truncate_for_log(&resolved_args.to_string())
                );
                match execution_adapters::run_skill(state, task, &normalized_skill, resolved_args)
                    .await
                {
                    Ok(out) => {
                        if let Some((original_path, _effective_path, user_visible_path)) =
                            &write_file_effective_path
                        {
                            remember_written_file_alias(
                                loop_state,
                                original_path,
                                user_visible_path,
                            );
                            register_file_path_output(
                                loop_state,
                                global_step,
                                step_in_round,
                                &format!("skill.{normalized_skill}"),
                                user_visible_path,
                            );
                        }
                        crate::append_subtask_result(
                            &mut loop_state.subtask_results,
                            global_step,
                            &format!("skill({normalized_skill})"),
                            true,
                            &out,
                        );
                        if !out.trim().is_empty() {
                            loop_state.has_tool_or_skill_output = true;
                            ended_with_user_visible_output = true;
                            let hint = if args_summary.is_empty() {
                                encode_progress_i18n(
                                    "telegram.progress.skill_completed",
                                    &[("skill", normalized_skill.as_str())],
                                )
                            } else {
                                encode_progress_i18n(
                                    "telegram.progress.skill_completed_with_args",
                                    &[
                                        ("skill", normalized_skill.as_str()),
                                        ("args_summary", args_summary.as_str()),
                                    ],
                                )
                            };
                            append_progress_hint(
                                state,
                                task,
                                &mut loop_state.progress_messages,
                                hint,
                            );
                        }
                        register_step_output(
                            loop_state,
                            global_step,
                            step_in_round,
                            &format!("skill.{normalized_skill}"),
                            &out,
                        );
                        *loop_state
                            .successful_action_fingerprints
                            .entry(fingerprint.clone())
                            .or_insert(0) += 1;
                        info!(
                            "executor_result_ok task_id={} round={} step={} type=call_skill output={} trace_only=raw_not_delivery",
                            task.task_id,
                            loop_state.round_no,
                            step_in_round,
                            crate::truncate_for_log(&out)
                        );
                        loop_state.history_compact.push(format!(
                            "round={} step={} skill={} ok",
                            loop_state.round_no, step_in_round, normalized_skill
                        ));
                        // trade_preview: stop only when next step is not a matching same-round trade_submit.
                        if crypto_action.as_deref() == Some("trade_preview") {
                            let decision = preview_params
                                .as_ref()
                                .map(|p| {
                                    check_continue_after_trade_preview(
                                        state, actions, idx, loop_state, p,
                                    )
                                })
                                .unwrap_or(TradePreviewContinue::Reject(
                                    "preview_params_extract_failed",
                                ));
                            match &decision {
                                TradePreviewContinue::Allow => {
                                    info!(
                                        "trade_preview_followed_by_submit_continue task_id={} round={} step={} reason=same_round_submit_matching",
                                        task.task_id, loop_state.round_no, step_in_round
                                    );
                                }
                                TradePreviewContinue::Reject(reason) => {
                                    info!(
                                        "trade_preview_awaiting_confirmation task_id={} round={} step={} reason={}",
                                        task.task_id, loop_state.round_no, step_in_round, reason
                                    );
                                    executed_actions += 1;
                                    loop_state.total_steps_executed += 1;
                                    stop_signal =
                                        Some("trade_preview_awaiting_confirmation".to_string());
                                    break;
                                }
                            }
                        }
                    }
                    Err(err) => {
                        crate::append_subtask_result(
                            &mut loop_state.subtask_results,
                            global_step,
                            &format!("skill({normalized_skill})"),
                            false,
                            &err,
                        );
                        info!(
                            "executor_result_error task_id={} round={} step={} type=call_skill error={}",
                            task.task_id,
                            loop_state.round_no,
                            step_in_round,
                            crate::truncate_for_log(&err)
                        );
                        let resume_err = build_resume_context_error(
                            actions,
                            &round_steps,
                            user_text,
                            goal,
                            &loop_state.subtask_results,
                            &loop_state.delivery_messages,
                            step_in_round,
                            &format!("skill({normalized_skill})"),
                            &err,
                        );
                        return Err(resume_err);
                    }
                }
            }
            AgentAction::Respond { content } => {
                let text = rewrite_response_with_written_aliases(
                    &resolve_arg_string(content, loop_state).trim().to_string(),
                    loop_state,
                )
                .trim()
                .to_string();
                let has_remaining_actions =
                    has_remaining_action_after(actions, idx, policy.max_steps);
                let publish_respond =
                    should_publish_respond_message(state, loop_state, user_text, &text);
                if !text.is_empty() && (publish_respond || !has_remaining_actions) {
                    loop_state.last_user_visible_respond = Some(text.clone());
                }
                if publish_respond {
                    crate::append_subtask_result(
                        &mut loop_state.subtask_results,
                        global_step,
                        "respond",
                        true,
                        &text,
                    );
                    if !has_remaining_actions {
                        ended_with_user_visible_output = !text.is_empty();
                    }
                    append_delivery_message(
                        &task.task_id,
                        &mut loop_state.delivery_messages,
                        text.clone(),
                    );
                    info!(
                        "delivery appended from respond task_id={} len={} has_remaining={}",
                        task.task_id,
                        loop_state.delivery_messages.len(),
                        has_remaining_actions
                    );
                    let hint = encode_progress_i18n("telegram.progress.reply_generated", &[]);
                    append_progress_hint(state, task, &mut loop_state.progress_messages, hint);
                }
                if !publish_respond && !text.is_empty() {
                    debug!(
                        "executor_step_skip task_id={} round={} step={} type=respond reason=respond_not_publishable trace_only",
                        task.task_id,
                        loop_state.round_no,
                        step_in_round
                    );
                }
                register_step_output(loop_state, global_step, step_in_round, "respond", &text);
                *loop_state
                    .successful_action_fingerprints
                    .entry(fingerprint.clone())
                    .or_insert(0) += 1;
                info!(
                    "executor_result_ok task_id={} round={} step={} type=respond output={}",
                    task.task_id,
                    loop_state.round_no,
                    step_in_round,
                    crate::truncate_for_log(&text)
                );
                loop_state.history_compact.push(format!(
                    "round={} step={} respond{}",
                    loop_state.round_no,
                    step_in_round,
                    if has_remaining_actions {
                        "_intermediate"
                    } else {
                        ""
                    }
                ));
                executed_actions += 1;
                loop_state.total_steps_executed += 1;
                if !has_remaining_actions {
                    stop_signal = Some("respond".to_string());
                    break;
                }
                continue;
            }
            AgentAction::Think { .. } => {}
        }
        executed_actions += 1;
        loop_state.total_steps_executed += 1;
    }
    if stop_signal.is_none()
        && executed_actions == actionable_count
        && ended_with_user_visible_output
    {
        stop_signal = Some("plan_exhausted_user_visible".to_string());
    }
    let delivery_grew = loop_state.delivery_messages.len() > before_delivery_count;
    let progress_grew = loop_state.progress_messages.len() > before_progress_count;
    let step_output_grew = loop_state.subtask_results.len() > before_subtask_count;
    let no_progress = !delivery_grew && !progress_grew && !step_output_grew;
    let next_goal_hint = loop_state.delivery_messages.last().cloned();
    Ok(RoundOutcome {
        executed_actions,
        had_error: false,
        stop_signal,
        next_goal_hint,
        no_progress,
    })
}

/// Only synthesize (chat) when we have raw output but no delivery yet. Prefer raw finalizer first.
fn should_synthesize_final_response(loop_state: &LoopState) -> bool {
    loop_state.has_tool_or_skill_output && loop_state.delivery_messages.is_empty()
}

/// Minimal filter: only allow raw outputs that look like publishable user-facing content.
/// Excludes empty, pure process hints, and obvious internal confirmation text.
fn is_publishable_raw(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.len() <= 2 {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    const INTERNAL_PHRASES: &[&str] = &[
        "ok",
        "done",
        "success",
        "completed",
        "yes",
        "no",
        "完成",
        "成功",
        "已执行",
        "执行完成",
        "好的",
        "command completed",
        "success.",
    ];
    if INTERNAL_PHRASES
        .iter()
        .any(|p| lower == *p || (lower.starts_with(p) && t.len() <= p.len() + 2))
    {
        return false;
    }
    if t.chars()
        .all(|c| c.is_ascii_digit() || c.is_ascii_punctuation() || c.is_whitespace())
    {
        return false;
    }
    true
}

/// Build final delivery list and final_text with priority: last_user_visible_respond > delivery_messages (deduped).
/// Used for testing and by run_agent_with_loop. Does not apply fallback_from_raw.
pub(crate) fn build_final_delivery_with_priority(
    delivery_messages: &[String],
    last_user_visible_respond: Option<&String>,
) -> (Vec<String>, String, bool) {
    let mut delivery_deduped: Vec<String> = Vec::new();
    for m in delivery_messages {
        let t = m.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(pos) = delivery_deduped.iter().position(|x| x.trim() == t) {
            delivery_deduped.remove(pos);
        }
        delivery_deduped.push(m.clone());
    }
    let used_last_respond = if let Some(last_respond) = last_user_visible_respond {
        let trimmed = last_respond.trim();
        if !trimmed.is_empty() {
            delivery_deduped.retain(|x| x.trim() != trimmed);
            delivery_deduped.push(last_respond.clone());
            true
        } else {
            false
        }
    } else {
        false
    };
    let final_text = delivery_deduped.last().cloned().unwrap_or_default();
    (delivery_deduped, final_text, used_last_respond)
}

/// Build a single final delivery string from raw subtask results (no LLM). Only include publishable raw;
/// process/internal lines are filtered out so they are not exposed as final delivery.
fn fallback_finalize_from_raw(subtask_results: &[String]) -> String {
    if subtask_results.is_empty() {
        return String::new();
    }
    const MAX_FALLBACK_ITEMS: usize = 5;
    let take = subtask_results.len().min(MAX_FALLBACK_ITEMS);
    let slice = &subtask_results[subtask_results.len().saturating_sub(take)..];
    let filtered: Vec<String> = slice
        .iter()
        .filter_map(|s| {
            let t = s.trim();
            if t.is_empty() {
                None
            } else if is_publishable_raw(t) {
                Some(t.to_string())
            } else {
                None
            }
        })
        .collect();
    filtered.join("\n\n")
}

async fn synthesize_final_response(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
) -> Result<Option<String>, String> {
    if !should_synthesize_final_response(loop_state) {
        return Ok(None);
    }
    let mut args = json!({
        "text": format!(
            "Original user request:\n{}\n\nWrite the final user-facing answer now. Use the recent execution context above. Complete any still-pending lightweight conclusion requested by the user, but do not invent unseen files, paths, lines, or command results. Keep the reply concise and direct.",
            user_text
        )
    });
    attach_recent_execution_context_to_chat_args(&mut args, loop_state);
    let out = execution_adapters::run_skill(state, task, "chat", args).await?;
    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}

fn evaluate_round_outcome(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    outcome: &RoundOutcome,
) -> bool {
    if outcome.had_error {
        info!(
            "loop_round_stop task_id={} round={} reason=had_error",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if let Some(reason) = &outcome.stop_signal {
        if reason == "recoverable_failure_continue_round" {
            info!(
                "loop_round_continue task_id={} round={} reason={}",
                task.task_id, loop_state.round_no, reason
            );
            return false;
        }
        info!(
            "loop_round_stop task_id={} round={} reason={} next_goal_hint={}",
            task.task_id,
            loop_state.round_no,
            reason,
            crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
        );
        return true;
    }
    if outcome.executed_actions == 0 {
        info!(
            "loop_round_stop task_id={} round={} reason=no_actions",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if outcome.no_progress {
        loop_state.consecutive_no_progress += 1;
    } else {
        loop_state.consecutive_no_progress = 0;
    }
    if loop_state.consecutive_no_progress > policy.no_progress_limit {
        info!(
            "loop_round_stop task_id={} round={} reason=no_progress limit={} count={}",
            task.task_id,
            loop_state.round_no,
            policy.no_progress_limit,
            loop_state.consecutive_no_progress
        );
        return true;
    }
    if !policy.multi_round_enabled {
        info!(
            "loop_round_stop task_id={} round={} reason=multi_round_disabled",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if loop_state.round_no >= loop_state.max_rounds {
        info!(
            "loop_round_stop task_id={} round={} reason=max_rounds reached={}",
            task.task_id, loop_state.round_no, loop_state.max_rounds
        );
        return true;
    }
    false
}

async fn run_agent_with_loop(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
) -> Result<AskReply, String> {
    let policy = load_agent_loop_guard_policy(state);
    let mut loop_state = LoopState::new(policy.max_rounds.max(1));
    for round in 1..=loop_state.max_rounds {
        loop_state.round_no = round;
        info!(
            "loop_round_start task_id={} round={} max_rounds={} total_steps={} tool_calls_total={}",
            task.task_id,
            round,
            loop_state.max_rounds,
            loop_state.total_steps_executed,
            loop_state.tool_calls_total
        );
        let actions =
            plan_round_actions(state, task, goal, user_text, &policy, &loop_state).await?;
        let outcome = execute_actions_once(
            state,
            task,
            goal,
            user_text,
            &actions,
            &mut loop_state,
            &policy,
        )
        .await?;
        info!(
            "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
            task.task_id,
            round,
            outcome.executed_actions,
            outcome.no_progress,
            outcome.stop_signal.as_deref().unwrap_or(""),
            crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
        );
        if evaluate_round_outcome(task, &mut loop_state, &policy, &outcome) {
            break;
        }
    }

    // When delivery is empty, fill from last_user_visible_respond so fallback path still has a candidate.
    if loop_state.delivery_messages.is_empty() {
        if let Some(ref last_respond) = loop_state.last_user_visible_respond {
            if !last_respond.trim().is_empty() {
                append_delivery_message(
                    &task.task_id,
                    &mut loop_state.delivery_messages,
                    last_respond.clone(),
                );
                info!(
                    "final_result_use_last_respond task_id={} (delivery was empty)",
                    task.task_id
                );
            }
        }
    }

    // Fallback finalizer: only when we still have no delivery.
    if loop_state.delivery_messages.is_empty() && !loop_state.subtask_results.is_empty() {
        let fallback = fallback_finalize_from_raw(loop_state.subtask_results.as_slice());
        if !fallback.trim().is_empty() {
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, fallback);
            info!(
                "final_result_fallback_from_raw task_id={} subtask_count={}",
                task.task_id,
                loop_state.subtask_results.len()
            );
        }
    }

    // Last resort: synthesize via chat when still no delivery (e.g. raw was empty or finalizer skipped).
    if loop_state.delivery_messages.is_empty() && should_synthesize_final_response(&loop_state) {
        if let Some(synthesized) =
            synthesize_final_response(state, task, user_text, &loop_state).await?
        {
            append_delivery_message(
                &task.task_id,
                &mut loop_state.delivery_messages,
                synthesized.clone(),
            );
            info!("delivery fallback_from_synthesize task_id={}", task.task_id);
            crate::append_subtask_result(
                &mut loop_state.subtask_results,
                loop_state.total_steps_executed + 1,
                "respond(finalize)",
                true,
                &synthesized,
            );
            loop_state
                .history_compact
                .push("finalize respond".to_string());
        }
    }

    let (delivery_deduped, _, used_last_respond) = build_final_delivery_with_priority(
        &loop_state.delivery_messages,
        loop_state.last_user_visible_respond.as_ref(),
    );

    let final_text = delivery_deduped
        .last()
        .cloned()
        .or_else(|| loop_state.subtask_results.last().cloned())
        .unwrap_or_default();

    if used_last_respond {
        info!(
            "final_result_source=last_respond task_id={} len={}",
            task.task_id,
            delivery_deduped.len()
        );
    } else if !delivery_deduped.is_empty() {
        info!(
            "final_result_source=delivery_messages task_id={} len={}",
            task.task_id,
            delivery_deduped.len()
        );
    } else if !loop_state.subtask_results.is_empty() {
        info!(
            "final_result_source=fallback_from_raw task_id={}",
            task.task_id
        );
    }

    crate::append_act_plan_log(
        state,
        task,
        "loop_done",
        loop_state.total_steps_executed,
        loop_state.subtask_results.len(),
        loop_state.tool_calls_total,
        &format!(
            "rounds={} messages={} no_progress_count={}",
            loop_state.round_no,
            loop_state.delivery_messages.len(),
            loop_state.consecutive_no_progress
        ),
    );
    Ok(AskReply::non_llm(final_text).with_messages(delivery_deduped))
}

fn plan_step_label(action: &AgentAction) -> String {
    match action {
        // LEGACY: CallTool shown as skill for unified capability view.
        AgentAction::CallTool { tool, .. } => format!("skill:{tool}"),
        AgentAction::CallSkill { skill, .. } => format!("skill:{skill}"),
        AgentAction::Respond { content } => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                "respond".to_string()
            } else {
                format!("respond:{}", crate::truncate_for_agent_trace(trimmed))
            }
        }
        AgentAction::Think { .. } => "think".to_string(),
    }
}

fn build_resume_context_error(
    actions: &[AgentAction],
    plan_steps: &[String],
    user_request: &str,
    goal: &str,
    subtask_results: &[String],
    delivery_messages: &[String],
    failed_index: usize,
    failed_action: &str,
    err: &str,
) -> String {
    let completed_messages_for_ctx: Vec<String> = if delivery_messages.is_empty() {
        subtask_results.to_vec()
    } else {
        delivery_messages.to_vec()
    };
    let completed_steps = if failed_index <= 1 {
        Vec::new()
    } else {
        subtask_results
            .iter()
            .take(failed_index.saturating_sub(1))
            .cloned()
            .collect::<Vec<_>>()
    };
    let remaining_steps = if plan_steps.len() > failed_index {
        plan_steps
            .iter()
            .skip(failed_index)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let remaining_actions = if actions.len() > failed_index {
        actions
            .iter()
            .skip(failed_index)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let resume_context = json!({
        "resume_context_id": format!("ctx-{}", uuid::Uuid::new_v4()),
        "user_request": user_request,
        "goal": goal,
        "plan_steps": plan_steps,
        "completed_steps": completed_steps,
        "completed_messages": completed_messages_for_ctx,
        "failed_step": {
            "index": failed_index,
            "action": failed_action,
            "error": crate::truncate_for_agent_trace(err),
        },
        "remaining_steps": remaining_steps,
        "remaining_actions": remaining_actions,
        "hint": "LLM should infer continuation from resume context and user follow-up."
    });
    let user_error = if resume_context
        .get("remaining_steps")
        .and_then(|v| v.as_array())
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        format!(
            "step {failed_index} failed ({failed_action}): {err}. Remaining steps are interrupted. 你可以回复“继续”来执行剩余步骤。"
        )
    } else {
        format!("step {failed_index} failed ({failed_action}): {err}")
    };
    let payload = json!({
        "user_error": user_error,
        "resume_context": resume_context
    });
    format!("{}{}", crate::RESUME_CONTEXT_ERROR_PREFIX, payload)
}

pub(crate) async fn run_agent_with_tools(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_request: &str,
) -> Result<AskReply, String> {
    info!(
        "run_agent_with_tools: task_id={} user_id={} chat_id={} goal={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        crate::truncate_for_log(goal)
    );
    let user_text = user_request.trim();
    if !user_text.is_empty() {
        return run_agent_with_loop(state, task, goal, user_text).await;
    }
    return Ok(AskReply::non_llm(String::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 1. trade_preview 后面没有 next step -> Reject preview_only_no_next_step
    #[test]
    fn test_trade_preview_continue_no_next_step() {
        let preview = json!({
            "action": "trade_preview",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let out = check_continue_after_trade_preview_core(None, None, &params);
        assert_eq!(
            out,
            TradePreviewContinue::Reject("preview_only_no_next_step")
        );
    }

    /// 2. next step 不是 crypto -> Reject
    #[test]
    fn test_trade_preview_continue_next_not_crypto() {
        let preview = json!({
            "action": "trade_preview",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let submit = json!({
            "action": "trade_submit",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09,
            "confirm": true
        });
        let out = check_continue_after_trade_preview_core(Some("chat"), Some(&submit), &params);
        assert_eq!(
            out,
            TradePreviewContinue::Reject("preview_only_next_not_crypto")
        );
    }

    /// 3. next step 是 crypto 但 action 不是 trade_submit -> Reject
    #[test]
    fn test_trade_preview_continue_next_not_submit() {
        let preview = json!({
            "action": "trade_preview",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let next = json!({
            "action": "open_orders",
            "exchange": "binance"
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&next), &params);
        assert_eq!(
            out,
            TradePreviewContinue::Reject("preview_only_next_not_submit")
        );
    }

    /// 4. trade_submit 缺 confirm=true -> Reject
    #[test]
    fn test_trade_preview_continue_submit_missing_confirm() {
        let preview = json!({
            "action": "trade_preview",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let submit = json!({
            "action": "trade_submit",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&submit), &params);
        assert_eq!(
            out,
            TradePreviewContinue::Reject("submit_missing_confirm_true")
        );
    }

    /// 5. trade_submit 与 preview symbol 不一致 -> Reject
    #[test]
    fn test_trade_preview_continue_symbol_mismatch() {
        let preview = json!({
            "action": "trade_preview",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let submit = json!({
            "action": "trade_submit",
            "exchange": "binance",
            "symbol": "BTCUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09,
            "confirm": true
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&submit), &params);
        assert_eq!(
            out,
            TradePreviewContinue::Reject("submit_params_inconsistent_with_preview")
        );
    }

    /// 6. trade_submit 与 preview side 不一致 -> Reject
    #[test]
    fn test_trade_preview_continue_side_mismatch() {
        let preview = json!({
            "action": "trade_preview",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let submit = json!({
            "action": "trade_submit",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "sell",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09,
            "confirm": true
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&submit), &params);
        assert_eq!(
            out,
            TradePreviewContinue::Reject("submit_params_inconsistent_with_preview")
        );
    }

    /// 7. trade_submit 与 preview price 不一致 (limit) -> Reject
    #[test]
    fn test_trade_preview_continue_price_mismatch() {
        let preview = json!({
            "action": "trade_preview",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let submit = json!({
            "action": "trade_submit",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.10,
            "confirm": true
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&submit), &params);
        assert_eq!(
            out,
            TradePreviewContinue::Reject("submit_params_inconsistent_with_preview")
        );
    }

    /// 8. trade_submit 与 preview qty/quote_qty_usd 不一致 -> Reject
    #[test]
    fn test_trade_preview_continue_qty_mismatch() {
        let preview = json!({
            "action": "trade_preview",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let submit = json!({
            "action": "trade_submit",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 10.0,
            "price": 0.09,
            "confirm": true
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&submit), &params);
        assert_eq!(
            out,
            TradePreviewContinue::Reject("submit_params_inconsistent_with_preview")
        );
    }

    /// 9. limit 单参数完全一致且 confirm=true -> Allow
    #[test]
    fn test_trade_preview_continue_limit_allow() {
        let preview = json!({
            "action": "trade_preview",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let submit = json!({
            "action": "trade_submit",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 5.0,
            "price": 0.09,
            "confirm": true
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&submit), &params);
        assert_eq!(out, TradePreviewContinue::Allow);
    }

    /// 10. market 单参数完全一致且 confirm=true -> Allow
    #[test]
    fn test_trade_preview_continue_market_allow() {
        let preview = json!({
            "action": "trade_preview",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "market",
            "quote_qty_usd": 10.0
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let submit = json!({
            "action": "trade_submit",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "market",
            "quote_qty_usd": 10.0,
            "confirm": true
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&submit), &params);
        assert_eq!(out, TradePreviewContinue::Allow);
    }

    /// preview 缺 exchange，submit 也缺 exchange，其他参数一致 -> Allow
    #[test]
    fn test_trade_preview_continue_missing_exchange_both_allow() {
        let preview = json!({
            "action": "trade_preview",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 10.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        assert_eq!(params.exchange, "binance");
        let submit = json!({
            "action": "trade_submit",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 10.0,
            "price": 0.09,
            "confirm": true
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&submit), &params);
        assert_eq!(out, TradePreviewContinue::Allow);
    }

    /// preview 缺 exchange，submit 显式带默认 exchange (binance) -> Allow
    #[test]
    fn test_trade_preview_continue_preview_no_exchange_submit_binance_allow() {
        let preview = json!({
            "action": "trade_preview",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 10.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let submit = json!({
            "action": "trade_submit",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 10.0,
            "price": 0.09,
            "confirm": true
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&submit), &params);
        assert_eq!(out, TradePreviewContinue::Allow);
    }

    /// preview 显式 binance，submit 缺 exchange（默认也是 binance）-> Allow
    #[test]
    fn test_trade_preview_continue_preview_binance_submit_no_exchange_allow() {
        let preview = json!({
            "action": "trade_preview",
            "exchange": "binance",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 10.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let submit = json!({
            "action": "trade_submit",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 10.0,
            "price": 0.09,
            "confirm": true
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&submit), &params);
        assert_eq!(out, TradePreviewContinue::Allow);
    }

    /// preview 缺 exchange，submit 也缺 exchange，但其他参数不一致 (symbol) -> Reject
    #[test]
    fn test_trade_preview_continue_missing_exchange_but_params_mismatch_reject() {
        let preview = json!({
            "action": "trade_preview",
            "symbol": "DOGEUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 10.0,
            "price": 0.09
        });
        let params = extract_trade_preview_params_for_consistency(&preview).unwrap();
        let submit = json!({
            "action": "trade_submit",
            "symbol": "BTCUSDT",
            "side": "buy",
            "order_type": "limit",
            "quote_qty_usd": 10.0,
            "price": 0.09,
            "confirm": true
        });
        let out = check_continue_after_trade_preview_core(Some("crypto"), Some(&submit), &params);
        assert_eq!(
            out,
            TradePreviewContinue::Reject("submit_params_inconsistent_with_preview")
        );
    }

    // --- build_safe_skill_args_summary: progress hint args must be whitelisted and safe ---
    #[test]
    fn test_build_safe_skill_args_summary_whitelist_order() {
        let args = json!({
            "symbol": "DOGEUSDT",
            "action": "open_orders",
            "exchange": "binance"
        });
        let s = build_safe_skill_args_summary(&args, 160);
        assert!(s.contains("action=open_orders"));
        assert!(s.contains("exchange=binance"));
        assert!(s.contains("symbol=DOGEUSDT"));
        assert!(s.starts_with("action="));
    }

    #[test]
    fn test_build_safe_skill_args_summary_sensitive_omitted() {
        let args = json!({
            "action": "trade_submit",
            "symbol": "DOGEUSDT",
            "api_key": "secret123",
            "api_secret": "never-show"
        });
        let s = build_safe_skill_args_summary(&args, 160);
        assert!(!s.contains("api_key"));
        assert!(!s.contains("api_secret"));
        assert!(!s.contains("secret123"));
        assert!(s.contains("action=trade_submit"));
        assert!(s.contains("symbol=DOGEUSDT"));
    }

    #[test]
    fn test_build_safe_skill_args_summary_truncate() {
        let args = json!({
            "action": "trade_history",
            "symbol": "DOGEUSDT",
            "limit": 10
        });
        let s = build_safe_skill_args_summary(&args, 30);
        assert!(s.len() <= 33);
        assert!(s.ends_with("...") || s.len() <= 30);
    }

    #[test]
    fn test_build_safe_skill_args_summary_empty_object() {
        let args = json!({});
        let s = build_safe_skill_args_summary(&args, 160);
        assert!(s.is_empty());
    }

    // --- build_final_delivery_with_priority: last_respond has priority over delivery_messages ---
    #[test]
    fn test_final_delivery_last_respond_priority_when_different() {
        let delivery = vec!["early answer".to_string()];
        let last_respond = "final answer".to_string();
        let (deduped, final_text, used) =
            build_final_delivery_with_priority(&delivery, Some(&last_respond));
        assert!(used);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0], "early answer");
        assert_eq!(deduped[1], "final answer");
        assert_eq!(final_text, "final answer");
    }

    #[test]
    fn test_final_delivery_last_respond_same_as_delivery_no_duplicate() {
        let delivery = vec!["same text".to_string()];
        let last_respond = "same text".to_string();
        let (deduped, final_text, used) =
            build_final_delivery_with_priority(&delivery, Some(&last_respond));
        assert!(used);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0], "same text");
        assert_eq!(final_text, "same text");
    }

    #[test]
    fn test_final_delivery_no_last_respond_uses_delivery() {
        let delivery = vec!["only delivery".to_string()];
        let (deduped, final_text, used) = build_final_delivery_with_priority(&delivery, None);
        assert!(!used);
        assert_eq!(deduped.len(), 1);
        assert_eq!(final_text, "only delivery");
    }

    #[test]
    fn test_final_delivery_both_empty() {
        let delivery: Vec<String> = vec![];
        let (deduped, final_text, used) = build_final_delivery_with_priority(&delivery, None);
        assert!(!used);
        assert!(deduped.is_empty());
        assert!(final_text.is_empty());
    }
}
