use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Component, Path};
use toml::Value as TomlValue;
use tracing::{info, warn};

use crate::{execution_adapters, llm_gateway, repo, AgentAction, AppState, AskReply, ClaimedTask};

const SKILL_PROMPT_ARCHIVE_BASIC: &str = include_str!("../../../prompts/skills/archive_basic.md");
const SKILL_PROMPT_AUDIO_SYNTHESIZE: &str = include_str!("../../../prompts/skills/audio_synthesize.md");
const SKILL_PROMPT_AUDIO_TRANSCRIBE: &str = include_str!("../../../prompts/skills/audio_transcribe.md");
const SKILL_PROMPT_CONFIG_GUARD: &str = include_str!("../../../prompts/skills/config_guard.md");
const SKILL_PROMPT_CRYPTO: &str = include_str!("../../../prompts/skills/crypto.md");
const SKILL_PROMPT_CHAT: &str = include_str!("../../../prompts/skills/chat.md");
const SKILL_PROMPT_DB_BASIC: &str = include_str!("../../../prompts/skills/db_basic.md");
const SKILL_PROMPT_DOCKER_BASIC: &str = include_str!("../../../prompts/skills/docker_basic.md");
const SKILL_PROMPT_FS_SEARCH: &str = include_str!("../../../prompts/skills/fs_search.md");
const SKILL_PROMPT_GIT_BASIC: &str = include_str!("../../../prompts/skills/git_basic.md");
const SKILL_PROMPT_HEALTH_CHECK: &str = include_str!("../../../prompts/skills/health_check.md");
const SKILL_PROMPT_HTTP_BASIC: &str = include_str!("../../../prompts/skills/http_basic.md");
const SKILL_PROMPT_IMAGE_EDIT: &str = include_str!("../../../prompts/skills/image_edit.md");
const SKILL_PROMPT_IMAGE_GENERATE: &str = include_str!("../../../prompts/skills/image_generate.md");
const SKILL_PROMPT_IMAGE_VISION: &str = include_str!("../../../prompts/skills/image_vision.md");
const SKILL_PROMPT_INSTALL_MODULE: &str = include_str!("../../../prompts/skills/install_module.md");
const SKILL_PROMPT_LOG_ANALYZE: &str = include_str!("../../../prompts/skills/log_analyze.md");
const SKILL_PROMPT_PACKAGE_MANAGER: &str = include_str!("../../../prompts/skills/package_manager.md");
const SKILL_PROMPT_PROCESS_BASIC: &str = include_str!("../../../prompts/skills/process_basic.md");
const SKILL_PROMPT_RSS_FETCH: &str = include_str!("../../../prompts/skills/rss_fetch.md");
const SKILL_PROMPT_SERVICE_CONTROL: &str = include_str!("../../../prompts/skills/service_control.md");
const SKILL_PROMPT_SYSTEM_BASIC: &str = include_str!("../../../prompts/skills/system_basic.md");
const SKILL_PROMPT_X: &str = include_str!("../../../prompts/skills/x.md");
const AGENT_TOOL_SPEC_TEMPLATE: &str = include_str!("../../../prompts/agent_tool_spec.md");
const SINGLE_PLAN_EXECUTION_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/single_plan_execution_prompt.md");
const LOOP_INCREMENTAL_PLAN_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/loop_incremental_plan_prompt.md");

const SKILL_PLAYBOOKS: &[(&str, &str)] = &[
    ("archive_basic", SKILL_PROMPT_ARCHIVE_BASIC),
    ("audio_synthesize", SKILL_PROMPT_AUDIO_SYNTHESIZE),
    ("audio_transcribe", SKILL_PROMPT_AUDIO_TRANSCRIBE),
    ("config_guard", SKILL_PROMPT_CONFIG_GUARD),
    ("crypto", SKILL_PROMPT_CRYPTO),
    ("chat", SKILL_PROMPT_CHAT),
    ("db_basic", SKILL_PROMPT_DB_BASIC),
    ("docker_basic", SKILL_PROMPT_DOCKER_BASIC),
    ("fs_search", SKILL_PROMPT_FS_SEARCH),
    ("git_basic", SKILL_PROMPT_GIT_BASIC),
    ("health_check", SKILL_PROMPT_HEALTH_CHECK),
    ("http_basic", SKILL_PROMPT_HTTP_BASIC),
    ("image_edit", SKILL_PROMPT_IMAGE_EDIT),
    ("image_generate", SKILL_PROMPT_IMAGE_GENERATE),
    ("image_vision", SKILL_PROMPT_IMAGE_VISION),
    ("install_module", SKILL_PROMPT_INSTALL_MODULE),
    ("log_analyze", SKILL_PROMPT_LOG_ANALYZE),
    ("package_manager", SKILL_PROMPT_PACKAGE_MANAGER),
    ("process_basic", SKILL_PROMPT_PROCESS_BASIC),
    ("rss_fetch", SKILL_PROMPT_RSS_FETCH),
    ("service_control", SKILL_PROMPT_SERVICE_CONTROL),
    ("system_basic", SKILL_PROMPT_SYSTEM_BASIC),
    ("x", SKILL_PROMPT_X),
];

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

fn build_rss_skill_prompt_with_categories(state: &AppState) -> String {
    let base = SKILL_PROMPT_RSS_FETCH.trim();
    let categories = load_rss_categories_for_prompt(state);
    if categories.is_empty() {
        return base.to_string();
    }
    format!(
        "{base}\n\n## Category Guardrails\n- Allowed `category` values come from `configs/rss.toml` `[rss.categories]`: {categories}\n- When calling `rss_fetch`, `category` must be one of the allowed values.\n- If user intent cannot be mapped confidently, use `general`.\n- Do not invent unseen category strings.",
        categories = categories.join(", ")
    )
}

fn build_skill_playbooks_text(state: &AppState) -> String {
    let mut sections = Vec::new();
    for (skill, body) in SKILL_PLAYBOOKS {
        let is_enabled = state.skills_list.contains(crate::canonical_skill_name(skill));
        let content = if !is_enabled {
            let disabled_text = crate::i18n_t_with_default(
                state,
                "clawd.msg.skill_disabled",
                "Skill is not enabled: {skill}. Please enable it in config and try again.",
            )
            .replace("{skill}", skill);
            format!(
                "Status: disabled.\n\nIf user intent requires this skill, do NOT call this skill.\nReturn `{{\"type\":\"respond\",\"content\":\"{disabled_text}\"}}`."
            )
        } else if *skill == "rss_fetch" {
            build_rss_skill_prompt_with_categories(state)
        } else {
            body.to_string()
        };
        let trimmed = content.trim();
        if trimmed.is_empty() {
            continue;
        }
        sections.push(format!("### {skill}\n{trimmed}"));
    }
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

fn publish_progress_messages(state: &AppState, task: &ClaimedTask, delivery_messages: &[String]) {
    if delivery_messages.is_empty() {
        return;
    }
    let payload = json!({
        "progress_messages": delivery_messages,
    });
    if let Err(err) = repo::update_task_progress_result(state, &task.task_id, &payload.to_string()) {
        warn!(
            "run_agent_with_tools: task_id={} publish progress failed: {}",
            task.task_id, err
        );
    }
}

fn append_and_publish_progress_message(
    state: &AppState,
    task: &ClaimedTask,
    delivery_messages: &mut Vec<String>,
    message: String,
) {
    delivery_messages.push(message);
    publish_progress_messages(state, task, delivery_messages);
}

#[derive(Debug, Deserialize)]
struct SinglePlanEnvelope {
    #[serde(default)]
    steps: Vec<Value>,
}

fn build_single_plan_prompt(
    user_request: &str,
    goal: &str,
    tool_spec: &str,
    skill_playbooks: &str,
) -> String {
    SINGLE_PLAN_EXECUTION_PROMPT_TEMPLATE
        .replace("__USER_REQUEST__", user_request)
        .replace("__GOAL__", goal)
        .replace("__TOOL_SPEC__", tool_spec)
        .replace("__SKILL_PLAYBOOKS__", skill_playbooks)
}

fn build_incremental_plan_prompt(
    user_request: &str,
    goal: &str,
    tool_spec: &str,
    skill_playbooks: &str,
    round: usize,
    history_compact: &str,
    last_round_output: &str,
) -> String {
    LOOP_INCREMENTAL_PLAN_PROMPT_TEMPLATE
        .replace("__USER_REQUEST__", user_request)
        .replace("__GOAL__", goal)
        .replace("__TOOL_SPEC__", tool_spec)
        .replace("__SKILL_PLAYBOOKS__", skill_playbooks)
        .replace("__ROUND__", &round.to_string())
        .replace("__HISTORY_COMPACT__", history_compact)
        .replace("__LAST_ROUND_OUTPUT__", last_round_output)
}

fn parse_single_plan_actions(raw: &str) -> Option<Vec<AgentAction>> {
    let value = crate::extract_json_object(raw)
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .or_else(|| serde_json::from_str::<Value>(raw).ok())?;
    let env = serde_json::from_value::<SinglePlanEnvelope>(value).ok()?;
    if env.steps.is_empty() {
        return None;
    }
    let mut actions = Vec::new();
    for step in env.steps {
        let raw_step = serde_json::to_string(&step).ok()?;
        let normalized = crate::parse_agent_action_json_with_repair(&raw_step).ok()?;
        let action = serde_json::from_value::<AgentAction>(normalized).ok()?;
        match action {
            AgentAction::Think { .. } => {}
            _ => actions.push(action),
        }
    }
    if actions.is_empty() { None } else { Some(actions) }
}

#[derive(Debug, Default)]
struct LoopState {
    round_no: usize,
    max_rounds: usize,
    tool_calls_total: usize,
    total_steps_executed: usize,
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

fn action_fingerprint(action: &AgentAction) -> String {
    match action {
        AgentAction::CallTool { tool, args } => {
            let tool_name = tool.trim().to_ascii_lowercase();
            let normalized_args = normalize_args_for_fingerprint(&tool_name, args);
            format!(
                "tool:{}:{}",
                tool_name,
                canonical_json_string(&normalized_args)
            )
        }
        AgentAction::CallSkill { skill, args } => {
            let normalized_skill = crate::canonical_skill_name(skill).to_ascii_lowercase();
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

fn register_step_output(loop_state: &mut LoopState, global_step: usize, key_prefix: &str, output: &str) {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return;
    }
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
        .insert(format!("{key_prefix}.last_output"), value);
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
    let skill_playbooks = build_skill_playbooks_text(state);
    let (prompt_name, prompt_file, prompt_text) = if loop_state.round_no <= 1 {
        (
            "single_plan_execution_prompt",
            "prompts/single_plan_execution_prompt.md",
            build_single_plan_prompt(user_text, goal, AGENT_TOOL_SPEC_TEMPLATE, &skill_playbooks),
        )
    } else {
        let history_compact = build_loop_history_compact(loop_state);
        let last_output = loop_state
            .delivery_messages
            .last()
            .cloned()
            .unwrap_or_else(|| "(none)".to_string());
        (
            "loop_incremental_plan_prompt",
            "prompts/loop_incremental_plan_prompt.md",
            build_incremental_plan_prompt(
                user_text,
                goal,
                AGENT_TOOL_SPEC_TEMPLATE,
                &skill_playbooks,
                loop_state.round_no,
                &history_compact,
                &last_output,
            ),
        )
    };
    info!(
        "{} prompt_invocation task_id={} prompt_name={} prompt_file={} round={}",
        crate::highlight_tag("prompt"),
        task.task_id,
        prompt_name,
        prompt_file,
        loop_state.round_no
    );
    info!(
        "{} prompt_debug task_id={} prompt_name={} prompt_file={} prompt_dynamic=true note=dynamic_built_prompt round={}",
        crate::highlight_tag("prompt"),
        task.task_id,
        prompt_name,
        prompt_file,
        loop_state.round_no
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
        llm_gateway::run_with_fallback_with_prompt_file(state, task, &prompt_text, prompt_file)
            .await?;
    info!(
        "plan_llm_response task_id={} round={} raw={}",
        task.task_id,
        loop_state.round_no,
        crate::truncate_for_log(&plan_raw)
    );
    let plan_actions = parse_single_plan_actions(&plan_raw)
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
    let before_message_count = loop_state.delivery_messages.len();
    let round_steps: Vec<String> = actions.iter().map(plan_step_label).collect();
    for (idx, action) in actions.iter().take(policy.max_steps.max(1)).enumerate() {
        let step_in_round = idx + 1;
        let global_step = loop_state.total_steps_executed + 1;
        let fingerprint = action_fingerprint(action);
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
                    *repeat_count, policy.repeat_action_limit, crate::truncate_for_log(&fingerprint)
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
        match action {
            AgentAction::CallTool { tool, args } => {
                let resolved_args = resolve_arg_value(args, loop_state);
                loop_state.tool_calls_total += 1;
                info!(
                    "{} executor_step_execute task_id={} round={} step={} type=call_tool tool={} args={}",
                    crate::highlight_tag("tool"),
                    task.task_id,
                    loop_state.round_no,
                    step_in_round,
                    tool,
                    crate::truncate_for_log(&resolved_args.to_string())
                );
                match execution_adapters::run_tool(state, tool, &resolved_args).await {
                    Ok(out) => {
                        crate::append_subtask_result(
                            &mut loop_state.subtask_results,
                            global_step,
                            &format!("tool({tool})"),
                            true,
                            &out,
                        );
                        if !out.trim().is_empty() {
                            loop_state.has_tool_or_skill_output = true;
                            append_and_publish_progress_message(
                        state,
                        task,
                                &mut loop_state.delivery_messages,
                                out.clone(),
                            );
                        }
                        register_step_output(
                            loop_state,
                            global_step,
                            &format!("tool.{tool}"),
                            &out,
                        );
                        *loop_state
                            .successful_action_fingerprints
                            .entry(fingerprint.clone())
                            .or_insert(0) += 1;
                        info!(
                            "executor_result_ok task_id={} round={} step={} type=call_tool output={}",
                            task.task_id,
                            loop_state.round_no,
                            step_in_round,
                            crate::truncate_for_log(&out)
                        );
                        loop_state.history_compact.push(format!(
                            "round={} step={} tool={} ok",
                            loop_state.round_no, step_in_round, tool
                        ));
                    }
                    Err(err) => {
                        crate::append_subtask_result(
                            &mut loop_state.subtask_results,
                            global_step,
                            &format!("tool({tool})"),
                            false,
                            &err,
                        );
                        info!(
                            "executor_result_error task_id={} round={} step={} type=call_tool error={}",
                            task.task_id,
                            loop_state.round_no,
                            step_in_round,
                            crate::truncate_for_log(&err)
                        );
                        let resume_err = build_resume_context_error(
                            &round_steps,
                            user_text,
                            goal,
                            &loop_state.subtask_results,
                            &loop_state.delivery_messages,
                            step_in_round,
                            &format!("tool({tool})"),
                            &err,
                        );
                        return Err(resume_err);
                    }
                }
            }
            AgentAction::CallSkill { skill, args } => {
                let resolved_args = resolve_arg_value(args, loop_state);
                loop_state.tool_calls_total += 1;
                let normalized_skill = crate::canonical_skill_name(skill).to_string();
                // Capture action name before resolved_args is moved into run_skill.
                let crypto_action = if normalized_skill == "crypto" {
                    resolved_args.get("action").and_then(|v| v.as_str()).map(str::to_string)
                } else {
                    None
                };
                // Hard block: crypto/trade_submit must not be executed within an agent turn.
                // Actual order submission is gated behind the user confirmation flow
                // (hard_trade_confirm_route) triggered by explicit button click or Y/YES reply.
                // This mirrors the run_skill-kind guard in main.rs.
                if crypto_action.as_deref() == Some("trade_submit") {
                    info!(
                        "executor_skill_blocked task_id={} round={} step={} skill=crypto action=trade_submit: agent cannot submit directly; awaiting user confirmation",
                        task.task_id, loop_state.round_no, step_in_round
                    );
                    stop_signal = Some("trade_submit_blocked".to_string());
                    break;
                }
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
                crate::append_subtask_result(
                            &mut loop_state.subtask_results,
                            global_step,
                            &format!("skill({normalized_skill})"),
                    true,
                            &out,
                        );
                        if !out.trim().is_empty() {
                            loop_state.has_tool_or_skill_output = true;
                    append_and_publish_progress_message(
                        state,
                        task,
                                &mut loop_state.delivery_messages,
                                out.clone(),
                            );
                        }
                        register_step_output(
                            loop_state,
                            global_step,
                            &format!("skill.{normalized_skill}"),
                            &out,
                        );
                        *loop_state
                            .successful_action_fingerprints
                            .entry(fingerprint.clone())
                            .or_insert(0) += 1;
                        info!(
                            "executor_result_ok task_id={} round={} step={} type=call_skill output={}",
                            task.task_id,
                            loop_state.round_no,
                            step_in_round,
                            crate::truncate_for_log(&out)
                        );
                        loop_state.history_compact.push(format!(
                            "round={} step={} skill={} ok",
                            loop_state.round_no, step_in_round, normalized_skill
                        ));
                        // trade_preview publishes a confirm-gated message; stop the loop
                        // immediately so the agent does not spin into another round and
                        // waste an LLM call planning the same (now-guarded) action again.
                        if crypto_action.as_deref() == Some("trade_preview") {
                            executed_actions += 1;
                            loop_state.total_steps_executed += 1;
                            stop_signal = Some("trade_preview_awaiting_confirmation".to_string());
                            break;
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
                let text = content.trim().to_string();
                let skip_respond_publish = loop_state.has_tool_or_skill_output;
                crate::append_subtask_result(
                    &mut loop_state.subtask_results,
                    global_step,
                    "respond",
                    true,
                    &text,
                );
                if !text.is_empty() && !skip_respond_publish {
                    append_and_publish_progress_message(
                    state,
                    task,
                        &mut loop_state.delivery_messages,
                        text.clone(),
                    );
                }
                if skip_respond_publish {
                    info!(
                        "executor_step_skip task_id={} round={} step={} type=respond reason=tool_or_skill_output_already_published",
                        task.task_id,
                        loop_state.round_no,
                        step_in_round
                    );
                }
                register_step_output(loop_state, global_step, "respond", &text);
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
                loop_state
                    .history_compact
                    .push(format!("round={} step={} respond", loop_state.round_no, step_in_round));
                stop_signal = Some("respond".to_string());
                executed_actions += 1;
                loop_state.total_steps_executed += 1;
                break;
            }
            AgentAction::Think { .. } => {}
        }
        executed_actions += 1;
        loop_state.total_steps_executed += 1;
    }
    let no_progress = loop_state.delivery_messages.len() == before_message_count;
    let next_goal_hint = loop_state.delivery_messages.last().cloned();
    Ok(RoundOutcome {
        executed_actions,
        had_error: false,
        stop_signal,
        next_goal_hint,
        no_progress,
    })
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
        let actions = plan_round_actions(state, task, goal, user_text, &policy, &loop_state).await?;
        let outcome =
            execute_actions_once(state, task, goal, user_text, &actions, &mut loop_state, &policy)
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

    let final_text = loop_state
        .delivery_messages
        .last()
        .cloned()
        .or_else(|| loop_state.subtask_results.last().cloned())
        .unwrap_or_default();
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
    Ok(AskReply::non_llm(final_text).with_messages(loop_state.delivery_messages))
}

fn plan_step_label(action: &AgentAction) -> String {
    match action {
        AgentAction::CallTool { tool, .. } => format!("tool:{tool}"),
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
    plan_steps: &[String],
    user_request: &str,
    goal: &str,
    subtask_results: &[String],
    delivery_messages: &[String],
    failed_index: usize,
    failed_action: &str,
    err: &str,
) -> String {
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
    let resume_context = json!({
        "resume_context_id": format!("ctx-{}", uuid::Uuid::new_v4()),
        "user_request": user_request,
        "goal": goal,
        "plan_steps": plan_steps,
        "completed_steps": completed_steps,
        "completed_messages": delivery_messages,
        "failed_step": {
            "index": failed_index,
            "action": failed_action,
            "error": crate::truncate_for_agent_trace(err),
        },
        "remaining_steps": remaining_steps,
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
