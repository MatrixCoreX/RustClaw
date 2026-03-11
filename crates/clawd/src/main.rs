use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{create_dir_all, OpenOptions};
use std::io::IsTerminal;
use std::io::Write as IoWrite;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, get_service, post};
use axum::{Json, Router};
use claw_core::config::{
    AppConfig, ChannelBindingConfig, CommandIntentConfig, LlmProviderConfig, MaintenanceConfig,
    MemoryConfig, PersonaConfig, RoutingConfig, ScheduleConfig, ToolsConfig,
};
use claw_core::hard_rules::main_flow::load_main_flow_rules;
use claw_core::hard_rules::trade as hard_trade;
use claw_core::hard_rules::trade::CompiledTradeRules;
use claw_core::hard_rules::types::MainFlowRules;
use claw_core::types::{
    ApiResponse, AuthIdentity, ChannelKind, ExchangeCredentialStatus, HealthResponse,
    SubmitTaskRequest, SubmitTaskResponse, TaskQueryResponse, TaskStatus,
};
use reqwest::Client;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Semaphore;
use toml::Value as TomlValue;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{debug, error, info, info_span, warn, Instrument};
use uuid::Uuid;

mod agent_engine;
mod execution_adapters;
mod http;
mod intent_router;
mod llm_gateway;
mod memory;
mod repo;
mod routing_context;
mod schedule_service;

const INIT_SQL: &str = include_str!("../../../migrations/001_init.sql");
const MEMORY_UPGRADE_SQL: &str = include_str!("../../../migrations/002_memory_upgrade.sql");
const CHANNEL_UPGRADE_SQL: &str = include_str!("../../../migrations/003_channels_upgrade.sql");
const KEY_AUTH_UPGRADE_SQL: &str = include_str!("../../../migrations/004_key_auth.sql");
const LLM_RETRY_TIMES: usize = 2;
pub(crate) const AGENT_MAX_STEPS: usize = 32;
pub(crate) const RESUME_CONTEXT_ERROR_PREFIX: &str = "__RESUME_CTX__";
const MAX_READ_FILE_BYTES: usize = 64 * 1024;
const MAX_WRITE_FILE_BYTES: usize = 128 * 1024;
const MODEL_IO_LOG_MAX_CHARS: usize = 16000;
const AGENT_TRACE_LOG_MAX_CHARS: usize = 4000;
const LOG_CALL_WRAP: &str = "---- task-call ----";

const CHAT_RESPONSE_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/chat_response_prompt.md");
const CHAT_RESPONSE_PROMPT_PATH: &str = "prompts/chat_response_prompt.md";
const RESUME_CONTINUE_EXECUTE_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/resume_continue_execute_prompt.md");
const RESUME_FOLLOWUP_DISCUSSION_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/resume_followup_discussion_prompt.md");
const RESUME_FOLLOWUP_DISCUSSION_PROMPT_PATH: &str = "prompts/resume_followup_discussion_prompt.md";
const IMAGE_OUTPUT_REWRITE_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/image_output_rewrite_prompt.md");
const LANGUAGE_INFER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/language_infer_prompt.md");
const IMAGE_REFERENCE_RESOLVER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/image_reference_resolver_prompt.md");
const LONG_TERM_SUMMARY_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/long_term_summary_prompt.md");
const SCHEDULE_INTENT_PROMPT_TEMPLATE_DEFAULT: &str =
    include_str!("../../../prompts/vendors/default/schedule_intent_prompt.md");
const SCHEDULE_INTENT_RULES_TEMPLATE_DEFAULT: &str =
    include_str!("../../../prompts/vendors/default/schedule_intent_rules.md");

#[derive(Clone)]
struct AppState {
    started_at: Instant,
    queue_limit: usize,
    db: Arc<Mutex<Connection>>,
    llm_providers: Vec<Arc<LlmProviderRuntime>>,
    skill_timeout_seconds: u64,
    skill_runner_path: PathBuf,
    skills_list: Arc<HashSet<String>>,
    #[allow(dead_code)] // reserved for future use (e.g. capability view / UI)
    configured_skills: Arc<HashSet<String>>,
    skill_semaphore: Arc<Semaphore>,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    maintenance: MaintenanceConfig,
    memory: MemoryConfig,
    workspace_root: PathBuf,
    tools_policy: Arc<ToolsPolicy>,
    active_provider_type: Option<String>,
    cmd_timeout_seconds: u64,
    max_cmd_length: usize,
    allow_path_outside_workspace: bool,
    allow_sudo: bool,
    worker_task_timeout_seconds: u64,
    routing: RoutingConfig,
    persona_prompt: String,
    command_intent: CommandIntentRuntime,
    schedule: ScheduleRuntime,
    telegram_bot_token: String,
    telegram_crypto_confirm_ttl_seconds: i64,
    whatsapp_cloud_enabled: bool,
    whatsapp_api_base: String,
    whatsapp_access_token: String,
    whatsapp_phone_number_id: String,
    whatsapp_web_enabled: bool,
    whatsapp_web_bridge_base_url: String,
    future_adapters_enabled: Arc<Vec<String>>,
    http_client: Client,
}

#[derive(Clone)]
struct LlmProviderRuntime {
    config: LlmProviderConfig,
    client: Client,
    semaphore: Arc<Semaphore>,
}

struct ClaimedTask {
    task_id: String,
    user_id: i64,
    chat_id: i64,
    user_key: Option<String>,
    channel: String,
    external_user_id: Option<String>,
    external_chat_id: Option<String>,
    kind: String,
    payload_json: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeChannel {
    Telegram,
    Whatsapp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhatsappDeliveryRoute {
    Cloud,
    WebBridge,
}

struct AskReply {
    text: String,
    messages: Vec<String>,
    is_llm_reply: bool,
}

pub(crate) fn render_prompt_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut rendered = template.to_string();
    for (key, value) in replacements {
        rendered = rendered.replace(key, value);
    }
    rendered
}

pub(crate) fn log_prompt_render(
    task_id: &str,
    prompt_name: &str,
    prompt_file: &str,
    round: Option<usize>,
) {
    match round {
        Some(round) => {
            info!(
                "{} prompt_invocation task_id={} prompt_name={} prompt_file={} prompt_dynamic=true note=dynamic_built_prompt round={}",
                highlight_tag("prompt"),
                task_id,
                prompt_name,
                prompt_file,
                round
            );
        }
        None => {
            info!(
                "{} prompt_invocation task_id={} prompt_name={} prompt_file={} prompt_dynamic=true note=dynamic_built_prompt",
                highlight_tag("prompt"),
                task_id,
                prompt_name,
                prompt_file
            );
        }
    }
}

pub(crate) fn parse_llm_json_extract_then_raw<T: DeserializeOwned>(raw: &str) -> Option<T> {
    extract_json_object(raw)
        .and_then(|s| serde_json::from_str::<T>(&s).ok())
        .or_else(|| serde_json::from_str::<T>(raw.trim()).ok())
}

pub(crate) fn parse_llm_json_extract_or_any<T: DeserializeOwned>(raw: &str) -> Option<T> {
    extract_json_object(raw)
        .or_else(|| extract_first_json_object_any(raw))
        .and_then(|s| serde_json::from_str::<T>(&s).ok())
}

pub(crate) fn parse_llm_json_raw_or_any<T: DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_str::<T>(raw.trim()).ok().or_else(|| {
        extract_first_json_object_any(raw).and_then(|s| serde_json::from_str::<T>(&s).ok())
    })
}

impl AskReply {
    pub(crate) fn llm(text: String) -> Self {
        Self {
            text,
            messages: Vec::new(),
            is_llm_reply: true,
        }
    }

    pub(crate) fn non_llm(text: String) -> Self {
        Self {
            text,
            messages: Vec::new(),
            is_llm_reply: false,
        }
    }

    pub(crate) fn with_messages(mut self, messages: Vec<String>) -> Self {
        self.messages = messages;
        self
    }
}

struct RateLimiter {
    global_rpm: usize,
    user_rpm: usize,
    global: VecDeque<u64>,
    per_user: HashMap<i64, VecDeque<u64>>,
}

struct ToolsPolicy {
    profile: String,
    allow: Vec<String>,
    deny: Vec<String>,
    by_provider: HashMap<String, ProviderScopedPolicy>,
}

struct ProviderScopedPolicy {
    allow: Vec<String>,
    deny: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AgentAction {
    Think {
        #[allow(dead_code)]
        content: String,
    },
    CallTool {
        tool: String,
        args: Value,
    },
    CallSkill {
        skill: String,
        args: Value,
    },
    Respond {
        content: String,
    },
}

#[derive(Debug, Clone, Copy)]
enum RoutedMode {
    Chat,
    Act,
    ChatAct,
    AskClarify,
}

#[derive(Debug, Clone, Deserialize)]
struct CommandIntentRules {
    #[serde(default)]
    result_suffixes: Vec<String>,
}

#[derive(Clone)]
struct CommandIntentRuntime {
    all_result_suffixes: Vec<String>,
}

#[derive(Clone)]
struct ScheduleRuntime {
    timezone: String,
    intent_prompt_template: String,
    intent_prompt_file: String,
    intent_rules_template: String,
    i18n_dict: HashMap<String, String>,
}

#[derive(serde::Serialize)]
struct LocalInteractionContext {
    user_id: i64,
    chat_id: i64,
    role: String,
}

#[derive(Deserialize)]
struct MemoryConfigFileWrapper {
    memory: MemoryConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct ScheduleIntentOutput {
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) timezone: String,
    #[serde(default)]
    pub(crate) schedule: ScheduleIntentSchedule,
    #[serde(default)]
    pub(crate) task: ScheduleIntentTask,
    #[serde(default)]
    pub(crate) target_job_id: String,
    #[serde(default)]
    pub(crate) confidence: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct ScheduleIntentSchedule {
    #[serde(default)]
    pub(crate) r#type: String,
    #[serde(default)]
    pub(crate) run_at: String,
    #[serde(default)]
    pub(crate) time: String,
    #[serde(default)]
    pub(crate) weekday: i64,
    #[serde(default)]
    pub(crate) every_minutes: i64,
    #[serde(default)]
    pub(crate) cron: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct ScheduleIntentTask {
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) payload: Value,
}

struct ScheduledJobDue {
    job_id: String,
    user_id: i64,
    chat_id: i64,
    user_key: Option<String>,
    channel: String,
    external_user_id: Option<String>,
    external_chat_id: Option<String>,
    task_kind: String,
    task_payload_json: String,
    next_run_at: i64,
    schedule_type: String,
    time_of_day: Option<String>,
    weekday: Option<i64>,
    every_minutes: Option<i64>,
    timezone: String,
}

impl RateLimiter {
    fn new(global_rpm: usize, user_rpm: usize) -> Self {
        Self {
            global_rpm: global_rpm.max(1),
            user_rpm: user_rpm.max(1),
            global: VecDeque::new(),
            per_user: HashMap::new(),
        }
    }

    fn check_and_record(&mut self, user_id: i64) -> Result<(), &'static str> {
        let now = now_ts_u64();
        let min_ts = now.saturating_sub(60);

        while self.global.front().is_some_and(|v| *v < min_ts) {
            self.global.pop_front();
        }

        let user_q = self.per_user.entry(user_id).or_default();
        while user_q.front().is_some_and(|v| *v < min_ts) {
            user_q.pop_front();
        }

        if self.global.len() >= self.global_rpm {
            return Err("global_rpm");
        }
        if user_q.len() >= self.user_rpm {
            return Err("user_rpm");
        }

        self.global.push_back(now);
        user_q.push_back(now);
        Ok(())
    }
}

impl ToolsPolicy {
    fn from_config(cfg: &ToolsConfig) -> Result<Self, String> {
        let profile = cfg.profile.trim().to_ascii_lowercase();
        if !matches!(
            profile.as_str(),
            "full" | "coding" | "minimal" | "messaging"
        ) {
            return Err(format!(
                "invalid tools.profile={}, allowed: full|coding|minimal|messaging",
                cfg.profile
            ));
        }
        // Normalize legacy "tool:" to "skill:" at load so we never store or expose tool: as main semantic.
        let allow: Vec<String> = cfg
            .allow
            .iter()
            .map(|v| normalize_capability_pattern(v.trim()))
            .filter(|v| !v.is_empty())
            .collect();
        let deny: Vec<String> = cfg
            .deny
            .iter()
            .map(|v| normalize_capability_pattern(v.trim()))
            .filter(|v| !v.is_empty())
            .collect();

        for p in allow.iter().chain(deny.iter()) {
            if p != "*" && !p.starts_with("skill:") {
                return Err(format!(
                    "invalid tools pattern: {p}; expected '*' or prefix 'skill:' (legacy 'tool:' is auto-converted to 'skill:')"
                ));
            }
        }

        let mut by_provider = HashMap::new();
        for (provider_key, scoped) in &cfg.by_provider {
            let key = provider_key.trim().to_ascii_lowercase();
            if key.is_empty() {
                return Err("tools.by_provider contains empty key".to_string());
            }
            let allow_scoped: Vec<String> = scoped
                .allow
                .iter()
                .map(|v| normalize_capability_pattern(v.trim()))
                .filter(|v| !v.is_empty())
                .collect();
            let deny_scoped: Vec<String> = scoped
                .deny
                .iter()
                .map(|v| normalize_capability_pattern(v.trim()))
                .filter(|v| !v.is_empty())
                .collect();

            for p in allow_scoped.iter().chain(deny_scoped.iter()) {
                if p != "*" && !p.starts_with("skill:") {
                    return Err(format!(
                        "invalid tools.by_provider.{key} pattern: {p}; expected '*' or prefix 'skill:' (legacy 'tool:' is auto-converted to 'skill:')"
                    ));
                }
            }

            by_provider.insert(
                key,
                ProviderScopedPolicy {
                    allow: allow_scoped,
                    deny: deny_scoped,
                },
            );
        }

        Ok(Self {
            profile,
            allow,
            deny,
            by_provider,
        })
    }

    fn is_allowed(&self, token: &str, provider_type: Option<&str>) -> bool {
        if self.deny.iter().any(|p| wildcard_match(p, token)) {
            return false;
        }

        if !self.allow.is_empty() {
            return self.allow.iter().any(|p| wildcard_match(p, token));
        }

        let mut allowed = self.default_allowed(token);

        if !allowed {
            return false;
        }

        if let Some(provider) = provider_type {
            let keys = provider_policy_keys(provider);
            for key in keys {
                if let Some(scoped) = self.by_provider.get(&key) {
                    if scoped.deny.iter().any(|p| wildcard_match(p, token)) {
                        return false;
                    }
                    if !scoped.allow.is_empty()
                        && !scoped.allow.iter().any(|p| wildcard_match(p, token))
                    {
                        return false;
                    }
                    allowed = true;
                    break;
                }
            }
        }

        allowed
    }

    fn default_allowed(&self, token: &str) -> bool {
        let defaults = match self.profile.as_str() {
            "full" => vec!["*"],
            "coding" => vec![
                "skill:*",
                "skill:system_basic",
                "skill:http_basic",
                "skill:git_basic",
                "skill:install_module",
                "skill:process_basic",
                "skill:package_manager",
                "skill:archive_basic",
                "skill:db_basic",
                "skill:docker_basic",
                "skill:fs_search",
                "skill:rss_fetch",
                "skill:x",
                "skill:image_vision",
                "skill:image_generate",
                "skill:image_edit",
                "skill:crypto",
            ],
            "minimal" => vec![
                "skill:run_cmd",
                "skill:read_file",
                "skill:write_file",
                "skill:list_dir",
                "skill:make_dir",
                "skill:remove_file",
                "skill:system_basic",
            ],
            "messaging" => vec!["skill:system_basic"],
            _ => vec!["*"],
        };
        defaults.into_iter().any(|p| wildcard_match(p, token))
    }
}

fn provider_policy_keys(provider_type: &str) -> Vec<String> {
    let p = provider_type.trim().to_ascii_lowercase();
    let mut keys = vec![p.clone()];
    match p.as_str() {
        "openai_compat" => keys.push("openai".to_string()),
        "google_gemini" => keys.push("google".to_string()),
        "anthropic_claude" => keys.push("anthropic".to_string()),
        _ => {}
    }
    keys
}

fn llm_vendor_name(provider: &LlmProviderRuntime) -> &str {
    provider
        .config
        .name
        .strip_prefix("vendor-")
        .unwrap_or(provider.config.name.as_str())
}

fn llm_model_kind(provider: &LlmProviderRuntime) -> &'static str {
    match provider.config.provider_type.as_str() {
        "openai_compat" => "compat",
        "google_gemini" => "gemini_native",
        "anthropic_claude" => "claude_native",
        _ => "unknown",
    }
}

/// Legacy compatibility: convert "tool:*" to "skill:*" at config load. Policy/capability view is skill-only.
fn normalize_capability_pattern(s: &str) -> String {
    let s = s.trim();
    if s.starts_with("tool:") {
        format!("skill:{}", &s[5..])
    } else {
        s.to_string()
    }
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }

    let mut idx = 0usize;
    let mut first = true;
    for part in &parts {
        if part.is_empty() {
            continue;
        }
        if first && !pattern.starts_with('*') {
            if !text[idx..].starts_with(part) {
                return false;
            }
            idx += part.len();
            first = false;
            continue;
        }
        if let Some(found) = text[idx..].find(part) {
            idx += found + part.len();
        } else {
            return false;
        }
        first = false;
    }
    if !pattern.ends_with('*') {
        if let Some(last) = parts.last() {
            return text.ends_with(last);
        }
    }
    true
}

fn load_command_intent_runtime(
    workspace_root: &Path,
    cfg: &CommandIntentConfig,
) -> CommandIntentRuntime {
    let rules_dir = workspace_root.join(cfg.rules_dir.trim());
    let mut all_result_suffixes: Vec<String> = Vec::new();
    for locale in ["zh-CN", "en-US"] {
        let path = rules_dir.join(format!("{locale}.toml"));
        match std::fs::read_to_string(&path) {
            Ok(raw) => match toml::from_str::<CommandIntentRules>(&raw) {
                Ok(rules) => {
                    for marker in &rules.result_suffixes {
                        let m = marker.trim();
                        if !m.is_empty()
                            && !all_result_suffixes
                                .iter()
                                .any(|x| x.eq_ignore_ascii_case(m))
                        {
                            all_result_suffixes.push(m.to_string());
                        }
                    }
                }
                Err(err) => {
                    warn!(
                        "load command intent rules failed: path={} err={err}",
                        path.display()
                    );
                }
            },
            Err(err) => {
                warn!(
                    "read command intent rules failed: path={} err={err}",
                    path.display()
                );
            }
        }
    }

    CommandIntentRuntime {
        all_result_suffixes,
    }
}

fn load_schedule_runtime(
    workspace_root: &Path,
    cfg: &ScheduleConfig,
    selected_vendor: Option<&str>,
) -> ScheduleRuntime {
    let timezone = if cfg.timezone.trim().is_empty() {
        "Asia/Shanghai".to_string()
    } else {
        cfg.timezone.trim().to_string()
    };

    let prompt_rel = if cfg.intent_prompt_path.trim().is_empty() {
        "prompts/schedule_intent_prompt.md"
    } else {
        cfg.intent_prompt_path.trim()
    };
    let (intent_prompt_template, intent_prompt_file) = load_prompt_template_for_vendor(
        workspace_root,
        selected_vendor,
        prompt_rel,
        SCHEDULE_INTENT_PROMPT_TEMPLATE_DEFAULT,
    );

    let rules_rel = if cfg.intent_rules_path.trim().is_empty() {
        "prompts/schedule_intent_rules.md"
    } else {
        cfg.intent_rules_path.trim()
    };
    let (intent_rules_template, _intent_rules_file) = load_prompt_template_for_vendor(
        workspace_root,
        selected_vendor,
        rules_rel,
        SCHEDULE_INTENT_RULES_TEMPLATE_DEFAULT,
    );

    let locale = if cfg.locale.trim().is_empty() {
        "zh-CN".to_string()
    } else {
        cfg.locale.trim().to_string()
    };
    let i18n_dir = if cfg.i18n_dir.trim().is_empty() {
        "configs/i18n".to_string()
    } else {
        cfg.i18n_dir.trim().to_string()
    };
    let i18n_path = workspace_root
        .join(&i18n_dir)
        .join(format!("schedule.{locale}.toml"));
    let mut i18n_dict = HashMap::new();
    match std::fs::read_to_string(&i18n_path) {
        Ok(raw) => match toml::from_str::<TomlValue>(&raw) {
            Ok(value) => {
                if let Some(table) = value.get("dict").and_then(|v| v.as_table()) {
                    for (k, v) in table {
                        if let Some(text) = v.as_str() {
                            i18n_dict.insert(k.to_string(), text.to_string());
                        }
                    }
                } else {
                    warn!(
                        "schedule i18n file missing [dict]: path={}",
                        i18n_path.display()
                    );
                }
            }
            Err(err) => {
                warn!(
                    "parse schedule i18n file failed: path={} err={err}",
                    i18n_path.display()
                );
            }
        },
        Err(err) => {
            warn!(
                "read schedule i18n file failed: path={} err={err}",
                i18n_path.display()
            );
        }
    }
    if i18n_dict.is_empty() {
        i18n_dict.insert(
            "schedule.desc.daily".to_string(),
            "daily {time}".to_string(),
        );
        i18n_dict.insert(
            "schedule.desc.weekly".to_string(),
            "weekly weekday={weekday} {time}".to_string(),
        );
        i18n_dict.insert(
            "schedule.desc.interval".to_string(),
            "every {minutes}m".to_string(),
        );
        i18n_dict.insert("schedule.desc.once".to_string(), "once".to_string());
        i18n_dict.insert("schedule.status.enabled".to_string(), "enabled".to_string());
        i18n_dict.insert("schedule.status.paused".to_string(), "paused".to_string());
        i18n_dict.insert(
            "schedule.msg.list_empty".to_string(),
            "There are no scheduled jobs right now.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.list_header".to_string(),
            "Scheduled jobs:\n{lines}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.delete_none".to_string(),
            "There are no scheduled jobs to delete.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.job_id_not_found".to_string(),
            "Job ID not found: {job_id}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.delete_all_ok".to_string(),
            "Deleted all scheduled jobs ({count} total).".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.delete_one_ok".to_string(),
            "Deleted scheduled job: {job_id}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.update_none".to_string(),
            "There are no scheduled jobs to update.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.resume_all_ok".to_string(),
            "Resumed all scheduled jobs ({count} total).".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.pause_all_ok".to_string(),
            "Paused all scheduled jobs ({count} total).".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.resume_one_ok".to_string(),
            "Resumed scheduled job: {job_id}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.pause_one_ok".to_string(),
            "Paused scheduled job: {job_id}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_fail_task_kind".to_string(),
            "Create failed: task.kind only supports ask or run_skill.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.cron_not_supported".to_string(),
            "Cron expressions are not supported in this version yet. Please use daily/weekly/every-N-minutes.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.cron_not_supported_with_expr".to_string(),
            "Cron expressions are not supported in this version yet ({cron}). Please use daily/weekly/every-N-minutes.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_fail_invalid_run_at".to_string(),
            "Create failed: invalid run_at for one-time job. Expected YYYY-MM-DD HH:MM[:SS]."
                .to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_fail_run_at_must_be_future".to_string(),
            "Create failed: execution time must be later than now.".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_fail_cannot_compute_next_run".to_string(),
            "Create failed: cannot compute next run time; please check the time format."
                .to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_exists_same".to_string(),
            "An identical scheduled job already exists: {job_id}\nType: {type}\nTimezone: {timezone}\nNext run: {next_run_human}\nTask content: {task_content}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.update_existing_ok".to_string(),
            "Found an existing job for the same symbol; updated it: {job_id}\nType: {type}\nTimezone: {timezone}\nNext run: {next_run_human}\nTask content: {task_content}".to_string(),
        );
        i18n_dict.insert(
            "schedule.msg.create_ok".to_string(),
            "Scheduled job created: {job_id}\nType: {type}\nTimezone: {timezone}\nNext run: {next_run_human}\nTask content: {task_content}".to_string(),
        );
    }

    ScheduleRuntime {
        timezone,
        intent_prompt_template,
        intent_prompt_file,
        intent_rules_template,
        i18n_dict,
    }
}

fn builtin_persona_prompt(profile: &str) -> &'static str {
    match profile {
        "expert" => {
            "Persona profile: expert. Be rigorous and concise. Explain key trade-offs, assumptions, and verification steps. Prefer correctness and safety over speed."
        }
        "companion" => {
            "Persona profile: companion. Be friendly and supportive while staying practical. Keep responses clear and encouraging, but still action-oriented."
        }
        _ => {
            "Persona profile: executor. Be direct and efficient. Give conclusion first, then minimal actionable details. Prioritize execution quality and safety."
        }
    }
}

fn load_persona_prompt(
    workspace_root: &Path,
    selected_vendor: Option<&str>,
    cfg: &PersonaConfig,
) -> String {
    let raw_profile = cfg.profile.trim().to_ascii_lowercase();
    let profile = match raw_profile.as_str() {
        "expert" | "companion" | "executor" => raw_profile,
        other => {
            warn!("unknown persona profile={}, fallback to executor", other);
            "executor".to_string()
        }
    };
    let dir = if cfg.dir.trim().is_empty() {
        "prompts/personas".to_string()
    } else {
        cfg.dir.trim().to_string()
    };
    let rel_path = format!("{dir}/{profile}.md");
    let (template, resolved_path) = load_prompt_template_for_vendor(
        workspace_root,
        selected_vendor,
        &rel_path,
        builtin_persona_prompt(&profile),
    );
    let text = template.trim();
    if text.is_empty() {
        warn!(
            "persona prompt file is empty, fallback to built-in: path={}",
            resolved_path
        );
        builtin_persona_prompt(&profile).to_string()
    } else {
        text.to_string()
    }
}

fn load_runtime_prompt_template(
    workspace_root: &Path,
    rel_path: &str,
    default_template: &str,
) -> String {
    let path = workspace_root.join(rel_path);
    match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => default_template.to_string(),
    }
}

fn normalize_prompt_vendor_name(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => "claude".to_string(),
        "google" | "gemini" => "google".to_string(),
        "openai" => "openai".to_string(),
        "grok" | "xai" => "grok".to_string(),
        "deepseek" => "deepseek".to_string(),
        "qwen" => "qwen".to_string(),
        "minimax" => "minimax".to_string(),
        "custom" => "openai".to_string(),
        _ => "default".to_string(),
    }
}

pub(crate) fn prompt_vendor_name_from_selected_vendor(selected_vendor: Option<&str>) -> String {
    selected_vendor
        .map(normalize_prompt_vendor_name)
        .unwrap_or_else(|| "default".to_string())
}

pub(crate) fn active_prompt_vendor_name(state: &AppState) -> String {
    if let Some(provider) = state.llm_providers.first() {
        return normalize_prompt_vendor_name(llm_vendor_name(provider));
    }
    if let Some(active) = state.active_provider_type.as_deref() {
        return normalize_prompt_vendor_name(active);
    }
    "default".to_string()
}

pub(crate) fn resolve_prompt_rel_path_for_vendor(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
) -> String {
    let trimmed = rel_path.trim();
    if trimmed.is_empty() || !trimmed.starts_with("prompts/") {
        return trimmed.to_string();
    }
    let suffix = trimmed.trim_start_matches("prompts/");
    let vendor_candidate = format!("prompts/vendors/{vendor}/{suffix}");
    if workspace_root.join(&vendor_candidate).is_file() {
        return vendor_candidate;
    }
    let default_candidate = format!("prompts/vendors/default/{suffix}");
    if vendor != "default" && workspace_root.join(&default_candidate).is_file() {
        return default_candidate;
    }
    trimmed.to_string()
}

pub(crate) fn load_prompt_template_for_vendor(
    workspace_root: &Path,
    selected_vendor: Option<&str>,
    rel_path: &str,
    default_template: &str,
) -> (String, String) {
    let vendor = prompt_vendor_name_from_selected_vendor(selected_vendor);
    let resolved_path = resolve_prompt_rel_path_for_vendor(workspace_root, &vendor, rel_path);
    let template = load_runtime_prompt_template(workspace_root, &resolved_path, default_template);
    (template, resolved_path)
}

pub(crate) fn load_prompt_template_for_state(
    state: &AppState,
    rel_path: &str,
    default_template: &str,
) -> (String, String) {
    let vendor = active_prompt_vendor_name(state);
    let resolved_path =
        resolve_prompt_rel_path_for_vendor(&state.workspace_root, &vendor, rel_path);
    let template =
        load_runtime_prompt_template(&state.workspace_root, &resolved_path, default_template);
    (template, resolved_path)
}

fn load_memory_runtime_config(workspace_root: &Path, cfg: &MemoryConfig) -> MemoryConfig {
    let path_raw = cfg.config_path.trim();
    if path_raw.is_empty() {
        return cfg.clone();
    }
    let path = if Path::new(path_raw).is_absolute() {
        PathBuf::from(path_raw)
    } else {
        workspace_root.join(path_raw)
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(v) => v,
        Err(err) => {
            warn!(
                "read memory config failed: path={} err={err}; fallback to main config values",
                path.display()
            );
            return cfg.clone();
        }
    };
    match toml::from_str::<MemoryConfig>(&raw) {
        Ok(mut loaded) => {
            loaded.config_path = path_raw.to_string();
            info!(
                "loaded memory runtime config: path={} recall_limit={} prompt_recall_limit={}",
                path.display(),
                loaded.recall_limit,
                loaded.prompt_recall_limit
            );
            loaded
        }
        Err(_) => match toml::from_str::<MemoryConfigFileWrapper>(&raw) {
            Ok(mut wrapped) => {
                wrapped.memory.config_path = path_raw.to_string();
                info!(
                    "loaded wrapped memory runtime config: path={} recall_limit={} prompt_recall_limit={}",
                    path.display(),
                    wrapped.memory.recall_limit,
                    wrapped.memory.prompt_recall_limit
                );
                wrapped.memory
            }
            Err(err) => {
                warn!(
                    "parse memory config failed: path={} err={err}; fallback to main config values",
                    path.display()
                );
                cfg.clone()
            }
        },
    }
}

fn trim_command_text(mut s: String) -> String {
    s = s.trim().to_string();
    while s.ends_with(|c: char| {
        matches!(
            c,
            '。' | '，' | ',' | ';' | '；' | ':' | '：' | '!' | '！' | '?' | '？'
        )
    }) {
        s.pop();
        s = s.trim_end().to_string();
    }
    if (s.starts_with('`') && s.ends_with('`')) || (s.starts_with('"') && s.ends_with('"')) {
        s = s[1..s.len().saturating_sub(1)].trim().to_string();
    }
    s
}

fn strip_result_suffixes(command: &str, suffixes: &[String]) -> String {
    let mut out = command.trim().to_string();
    if out.is_empty() {
        return out;
    }
    let lowered = out.to_lowercase();
    let mut cut_idx: Option<usize> = None;
    for marker in suffixes {
        let needle = marker.trim().to_lowercase();
        if needle.is_empty() {
            continue;
        }
        if let Some(idx) = lowered.find(&needle) {
            if idx > 0 {
                cut_idx = Some(match cut_idx {
                    Some(old) => old.min(idx),
                    None => idx,
                });
            }
        }
    }
    if let Some(idx) = cut_idx {
        out = out[..idx].trim().to_string();
    }
    trim_command_text(out)
}

fn sanitize_command_before_execute(runtime: &CommandIntentRuntime, command: &str) -> String {
    if runtime.all_result_suffixes.is_empty() {
        return trim_command_text(command.trim().to_string());
    }
    strip_result_suffixes(command, &runtime.all_result_suffixes)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        // 默认用 info 级别，若设置 RUST_LOG 则以环境变量为准。
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .with_target(false)
        .with_ansi(log_color_enabled())
        .compact()
        .init();

    let config = AppConfig::load("configs/config.toml")?;
    let tools_policy = ToolsPolicy::from_config(&config.tools)
        .map_err(|err| anyhow::anyhow!("invalid tools config: {err}"))?;
    let db = init_db(&config)?;
    seed_users(&db, &config)?;
    ensure_schedule_schema(&db)?;
    ensure_memory_schema(&db)?;
    ensure_channel_schema(&db)?;
    ensure_key_auth_schema(&db)?;
    let bootstrap_admin_key = ensure_bootstrap_admin_key(&db)?;
    seed_channel_bindings(&db, &config)?;
    if let Some(user_key) = bootstrap_admin_key.as_deref() {
        warn!("============================================================");
        warn!("No auth key found in database. Generated initial admin key.");
        warn!("Initial admin key: {}", user_key);
        warn!("Please save it now and use it to bind UI / Telegram / WhatsApp.");
        warn!("============================================================");
        eprintln!("============================================================");
        eprintln!("Initial admin key: {}", user_key);
        eprintln!("Please save it now and use it to bind UI / Telegram / WhatsApp.");
        eprintln!("============================================================");
    }
    let recovered_task_ids =
        recover_stale_running_tasks_on_startup(&db, config.worker.task_timeout_seconds.max(1))?;
    if !recovered_task_ids.is_empty() {
        let recovery_detail = json!({
            "reason": "startup_stale_running_recovery",
            "task_timeout_seconds": config.worker.task_timeout_seconds.max(1),
            "recovered_count": recovered_task_ids.len(),
            "task_ids": recovered_task_ids,
        });
        if let Err(err) = insert_audit_log_raw(
            &db,
            None,
            "startup_recover_running_timeout",
            Some(&recovery_detail.to_string()),
            None,
        ) {
            warn!("write startup recovery audit log failed: {err}");
        }
        warn!(
            "startup stale-running recovery applied: converted {} tasks to timeout (threshold={}s)",
            recovery_detail["recovered_count"]
                .as_u64()
                .unwrap_or_default(),
            config.worker.task_timeout_seconds.max(1)
        );
    } else {
        info!(
            "startup stale-running recovery: no stale running tasks found (threshold={}s)",
            config.worker.task_timeout_seconds.max(1)
        );
    }

    let workspace_root = std::env::current_dir()?;
    let memory_runtime = load_memory_runtime_config(&workspace_root, &config.memory);
    let command_intent = load_command_intent_runtime(&workspace_root, &config.command_intent);
    let schedule = load_schedule_runtime(
        &workspace_root,
        &config.schedule,
        config.llm.selected_vendor.as_deref(),
    );
    let routing = config.routing.clone();
    let persona_prompt = load_persona_prompt(
        &workspace_root,
        config.llm.selected_vendor.as_deref(),
        &config.persona,
    );
    let mut preferred_runner = workspace_root.join("target/release/skill-runner");
    if let Ok(exe) = std::env::current_exe() {
        let exe_lc = exe.to_string_lossy().to_ascii_lowercase();
        if exe_lc.contains("/target/debug/") {
            preferred_runner = workspace_root.join("target/debug/skill-runner");
        }
    }
    let release_fallback = workspace_root.join("target/release/skill-runner");
    let debug_fallback = workspace_root.join("target/debug/skill-runner");
    let effective_skill_runner_path = if preferred_runner.exists() {
        preferred_runner
    } else if release_fallback.exists() {
        release_fallback
    } else {
        debug_fallback
    };
    info!(
        "skill_runner_path resolved: {}",
        effective_skill_runner_path.display()
    );

    let llm_providers = llm_gateway::build_providers(&config);
    info!("Loaded LLM providers count={}", llm_providers.len());
    for p in &llm_providers {
        info!(
            "Active provider: name={}, type={}, model={}",
            p.config.name, p.config.provider_type, p.config.model
        );
    }
    info!(
        "run_cmd config: timeout_seconds={}, max_cmd_length={}, allow_outside_workspace={}, allow_sudo={}",
        config.tools.cmd_timeout_seconds.max(1),
        config.tools.max_cmd_length.max(16),
        config.tools.allow_path_outside_workspace,
        config.tools.allow_sudo
    );
    info!(
        "schedule config: timezone={}, prompt_chars={}, rules_chars={}",
        schedule.timezone,
        schedule.intent_prompt_template.chars().count(),
        schedule.intent_rules_template.chars().count()
    );
    info!(
        "persona loaded: profile={} chars={}",
        config.persona.profile.trim(),
        persona_prompt.chars().count()
    );
    let startup_rss = current_rss_bytes();
    info!("Startup memory RSS bytes={}", startup_rss.unwrap_or(0));

    let active_provider_type = llm_providers
        .first()
        .map(|p| p.config.provider_type.clone());

    let ui_dist_dir = resolve_ui_dist_dir(&workspace_root);
    let whatsapp_cloud_enabled = config.whatsapp_cloud.enabled || config.whatsapp.enabled;
    let whatsapp_api_base = if config.whatsapp_cloud.api_base.trim().is_empty() {
        config.whatsapp.api_base.clone()
    } else {
        config.whatsapp_cloud.api_base.clone()
    };
    let whatsapp_access_token = if config.whatsapp_cloud.access_token.trim().is_empty() {
        config.whatsapp.access_token.clone()
    } else {
        config.whatsapp_cloud.access_token.clone()
    };
    let whatsapp_phone_number_id = if config.whatsapp_cloud.phone_number_id.trim().is_empty() {
        config.whatsapp.phone_number_id.clone()
    } else {
        config.whatsapp_cloud.phone_number_id.clone()
    };

    let mut enabled_skills: HashSet<String> = config
        .skills
        .skills_list
        .iter()
        .map(|skill| canonical_skill_name(skill).to_string())
        .collect();
    let mut configured_skills = enabled_skills.clone();
    for (skill, is_enabled) in &config.skills.skill_switches {
        let canonical = canonical_skill_name(skill);
        configured_skills.insert(canonical.to_string());
        if *is_enabled {
            enabled_skills.insert(canonical.to_string());
        } else {
            enabled_skills.remove(canonical);
        }
    }
    for s in claw_core::config::core_skills_always_enabled() {
        enabled_skills.insert(canonical_skill_name(s).to_string());
        configured_skills.insert(canonical_skill_name(s).to_string());
    }
    let mut enabled_skills_for_log: Vec<String> = enabled_skills.iter().cloned().collect();
    enabled_skills_for_log.sort();
    info!(
        "enabled skills resolved count={} skills={}",
        enabled_skills_for_log.len(),
        enabled_skills_for_log.join(", ")
    );

    let state = AppState {
        started_at: Instant::now(),
        queue_limit: config.worker.queue_limit,
        db: Arc::new(Mutex::new(db)),
        llm_providers,
        skill_timeout_seconds: config.skills.skill_timeout_seconds,
        skill_runner_path: effective_skill_runner_path,
        skills_list: Arc::new(enabled_skills),
        configured_skills: Arc::new(configured_skills),
        skill_semaphore: Arc::new(Semaphore::new(config.skills.skill_max_concurrency.max(1))),
        rate_limiter: Arc::new(Mutex::new(RateLimiter::new(
            config.limits.global_rpm,
            config.limits.user_rpm,
        ))),
        maintenance: config.maintenance.clone(),
        memory: memory_runtime,
        workspace_root,
        tools_policy: Arc::new(tools_policy),
        active_provider_type,
        cmd_timeout_seconds: config.tools.cmd_timeout_seconds.max(1),
        max_cmd_length: config.tools.max_cmd_length.max(16),
        allow_path_outside_workspace: config.tools.allow_path_outside_workspace,
        allow_sudo: config.tools.allow_sudo,
        worker_task_timeout_seconds: config.worker.task_timeout_seconds.max(1),
        routing,
        persona_prompt,
        command_intent,
        schedule,
        telegram_bot_token: config.telegram.bot_token.clone(),
        telegram_crypto_confirm_ttl_seconds: (config.telegram.crypto_confirm_ttl_seconds.max(1))
            as i64,
        whatsapp_cloud_enabled,
        whatsapp_api_base,
        whatsapp_access_token,
        whatsapp_phone_number_id,
        whatsapp_web_enabled: config.whatsapp_web.enabled,
        whatsapp_web_bridge_base_url: config.whatsapp_web.bridge_base_url.clone(),
        future_adapters_enabled: Arc::new(
            config
                .adapters
                .iter()
                .filter_map(|(k, v)| if v.enabled { Some(k.clone()) } else { None })
                .collect(),
        ),
        http_client: Client::new(),
    };

    spawn_worker(
        state.clone(),
        config.worker.poll_interval_ms,
        config.worker.concurrency.max(1),
    );
    spawn_cleanup_worker(state.clone());
    spawn_schedule_worker(state.clone());

    let ui_index_path = ui_dist_dir.join("index.html");
    if ui_index_path.exists() {
        info!("UI static assets enabled at {}", ui_dist_dir.display());
    } else {
        warn!(
            "UI static assets missing: {} (run `cd UI && npm run build`)",
            ui_index_path.display()
        );
    }

    let api = Router::new()
        .merge(http::ui_routes::build_ui_router())
        .route("/tasks", post(submit_task))
        .route("/tasks/:task_id", get(get_task))
        .route("/tasks/cancel", post(cancel_tasks))
        .with_state(state.clone());

    let ui_service =
        get_service(ServeDir::new(&ui_dist_dir).not_found_service(ServeFile::new(ui_index_path)));

    let app = Router::new()
        .nest("/v1", api)
        .fallback_service(ui_service)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderName::from_static("x-rustclaw-key"),
                ]),
        );

    let listener = tokio::net::TcpListener::bind(&config.server.listen).await?;
    info!("clawd listening on {}", config.server.listen);
    axum::serve(listener, app).await?;
    Ok(())
}

fn resolve_ui_dist_dir(workspace_root: &Path) -> PathBuf {
    if let Ok(raw) = std::env::var("RUSTCLAW_UI_DIST") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if candidate.is_absolute() {
                return candidate;
            }
            return workspace_root.join(candidate);
        }
    }
    workspace_root.join("UI").join("dist")
}

fn recover_stale_running_tasks_on_startup(
    db: &Connection,
    task_timeout_seconds: u64,
) -> anyhow::Result<Vec<String>> {
    let now = now_ts_u64() as i64;
    let timeout = task_timeout_seconds.max(1) as i64;
    let stale_before = now.saturating_sub(timeout);
    let mut task_ids = Vec::new();
    {
        let mut stmt = db.prepare(
            "SELECT task_id
             FROM tasks
             WHERE status = 'running'
               AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?1
             ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) ASC",
        )?;
        let rows = stmt.query_map(params![stale_before.to_string()], |row| {
            row.get::<_, String>(0)
        })?;
        for row in rows {
            task_ids.push(row?);
        }
    }
    if task_ids.is_empty() {
        return Ok(task_ids);
    }

    let stale_note = format!(
        "auto timeout on startup: exceeded {}s while status=running",
        task_timeout_seconds.max(1)
    );

    let changed = db.execute(
        "UPDATE tasks
         SET status = 'timeout',
             error_text = CASE
                 WHEN error_text IS NULL OR TRIM(error_text) = '' THEN ?2
                 ELSE error_text
             END,
             updated_at = ?3
         WHERE status = 'running'
           AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) <= ?1",
        params![stale_before.to_string(), stale_note, now_ts()],
    )?;
    if changed != task_ids.len() {
        warn!(
            "startup stale-running recovery count mismatch: selected={} updated={}",
            task_ids.len(),
            changed
        );
    }

    Ok(task_ids)
}

fn spawn_worker(state: AppState, poll_interval_ms: u64, concurrency: usize) {
    let worker_count = concurrency.max(1);
    info!(
        "spawn_worker: starting {} worker loop(s), poll_interval_ms={}",
        worker_count,
        poll_interval_ms.max(10)
    );
    for worker_idx in 0..worker_count {
        let state_cloned = state.clone();
        tokio::spawn(async move {
            loop {
                if let Err(err) = worker_once(&state_cloned).await {
                    error!("Worker tick failed (worker_idx={}): {}", worker_idx, err);
                }
                tokio::time::sleep(Duration::from_millis(poll_interval_ms.max(10))).await;
            }
        });
    }
}

fn spawn_cleanup_worker(state: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(
                state.maintenance.cleanup_interval_seconds.max(30),
            ))
            .await;

            if let Err(err) = cleanup_once(&state) {
                error!("Cleanup task failed: {}", err);
            }
        }
    });
}

fn spawn_schedule_worker(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(err) = schedule_once(&state) {
                error!("Schedule worker tick failed: {}", err);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

fn schedule_once(state: &AppState) -> anyhow::Result<()> {
    let now = now_ts_u64() as i64;
    let mut due_jobs: Vec<ScheduledJobDue> = Vec::new();

    {
        let db = state
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
        let mut stmt = db.prepare(
            "SELECT job_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, task_kind, task_payload_json, next_run_at,
                    schedule_type, time_of_day, weekday, every_minutes, timezone
             FROM scheduled_jobs
             WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1
             ORDER BY next_run_at ASC
             LIMIT 16",
        )?;
        let rows = stmt.query_map(params![now], |row| {
            Ok(ScheduledJobDue {
                job_id: row.get(0)?,
                user_id: row.get(1)?,
                chat_id: row.get(2)?,
                user_key: row.get(3)?,
                channel: row.get(4)?,
                external_user_id: row.get(5)?,
                external_chat_id: row.get(6)?,
                task_kind: row.get(7)?,
                task_payload_json: row.get(8)?,
                next_run_at: row.get(9)?,
                schedule_type: row.get(10)?,
                time_of_day: row.get(11)?,
                weekday: row.get(12)?,
                every_minutes: row.get(13)?,
                timezone: row.get(14)?,
            })
        })?;
        for row in rows {
            due_jobs.push(row?);
        }
    }

    if due_jobs.is_empty() {
        return Ok(());
    }

    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

    for job in due_jobs {
        let next_run = schedule_service::compute_next_run_for_schedule(
            &job.schedule_type,
            job.time_of_day.as_deref(),
            job.weekday,
            job.every_minutes,
            &job.timezone,
            now,
        );

        let mut payload =
            serde_json::from_str::<Value>(&job.task_payload_json).unwrap_or_else(|_| json!({}));
        if let Value::Object(map) = &mut payload {
            map.insert("schedule_triggered".to_string(), Value::Bool(true));
            map.insert(
                "schedule_job_id".to_string(),
                Value::String(job.job_id.clone()),
            );
            map.insert("channel".to_string(), Value::String(job.channel.clone()));
            if let Some(v) = job.external_user_id.as_ref() {
                map.insert("external_user_id".to_string(), Value::String(v.clone()));
            }
            if let Some(v) = job.external_chat_id.as_ref() {
                map.insert("external_chat_id".to_string(), Value::String(v.clone()));
            }
        }

        let task_id = Uuid::new_v4().to_string();
        let now_text = now_ts();
        db.execute(
            "INSERT INTO tasks (task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, message_id, kind, payload_json, status, result_json, error_text, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, 'queued', NULL, NULL, ?10, ?10)",
            params![
                task_id,
                job.user_id,
                job.chat_id,
                job.user_key,
                job.channel,
                job.external_user_id,
                job.external_chat_id,
                job.task_kind,
                payload.to_string(),
                now_text
            ],
        )?;

        match next_run {
            Some(ts) => {
                db.execute(
                    "UPDATE scheduled_jobs
                     SET last_run_at = ?2, next_run_at = ?3, updated_at = ?2
                     WHERE job_id = ?1 AND next_run_at = ?4",
                    params![job.job_id, now.to_string(), ts, job.next_run_at],
                )?;
            }
            None => {
                db.execute(
                    "UPDATE scheduled_jobs
                     SET enabled = 0, last_run_at = ?2, next_run_at = NULL, updated_at = ?2
                     WHERE job_id = ?1 AND next_run_at = ?3",
                    params![job.job_id, now.to_string(), job.next_run_at],
                )?;
            }
        }
    }

    Ok(())
}

fn cleanup_once(state: &AppState) -> anyhow::Result<()> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

    let now = now_ts_u64() as i64;

    let task_cutoff = now - (state.maintenance.tasks_retention_days as i64 * 86400);
    db.execute(
        "DELETE FROM tasks WHERE CAST(created_at AS INTEGER) < ?1",
        params![task_cutoff],
    )?;

    db.execute(
        "DELETE FROM tasks WHERE task_id IN (
             SELECT task_id FROM tasks
             ORDER BY CAST(created_at AS INTEGER) DESC
             LIMIT -1 OFFSET ?1
         )",
        params![state.maintenance.tasks_max_rows as i64],
    )?;

    let audit_cutoff = now - (state.maintenance.audit_retention_days as i64 * 86400);
    db.execute(
        "DELETE FROM audit_logs WHERE CAST(ts AS INTEGER) < ?1",
        params![audit_cutoff],
    )?;

    db.execute(
        "DELETE FROM audit_logs WHERE id IN (
             SELECT id FROM audit_logs
             ORDER BY id DESC
             LIMIT -1 OFFSET ?1
         )",
        params![state.maintenance.audit_max_rows as i64],
    )?;

    let memory_cutoff = now - (state.memory.retention_days as i64 * 86400);
    db.execute(
        "DELETE FROM memories
         WHERE COALESCE(created_at_ts, CAST(created_at AS INTEGER)) < ?1",
        params![memory_cutoff],
    )?;

    db.execute(
        "DELETE FROM memories WHERE id IN (
             SELECT id FROM memories
             ORDER BY id DESC
             LIMIT -1 OFFSET ?1
         )",
        params![state.memory.max_rows as i64],
    )?;

    let long_term_cutoff = now - (state.memory.long_term_retention_days as i64 * 86400);
    db.execute(
        "DELETE FROM long_term_memories
         WHERE COALESCE(updated_at_ts, CAST(updated_at AS INTEGER)) < ?1",
        params![long_term_cutoff],
    )?;

    db.execute(
        "DELETE FROM long_term_memories WHERE id IN (
             SELECT id FROM long_term_memories
             ORDER BY id DESC
             LIMIT -1 OFFSET ?1
         )",
        params![state.memory.long_term_max_rows as i64],
    )?;

    Ok(())
}

async fn worker_once(state: &AppState) -> anyhow::Result<()> {
    let Some(task) = repo::claim_next_task(state)? else {
        debug!("worker_once: no queued tasks, idle tick");
        return Ok(());
    };

    let call_id = task.task_id.clone();
    let call_span = info_span!(
        "task_call",
        call_id = %call_id,
        task_id = %task.task_id,
        user_id = task.user_id,
        chat_id = task.chat_id,
        kind = %task.kind,
        channel = %task.channel
    );
    async {
        info!(
            "worker_once: picked task_id={} user_id={} chat_id={} kind={}",
            task.task_id, task.user_id, task.chat_id, task.kind
        );
        info!("{}", LOG_CALL_WRAP);
        info!(
            "task_call_begin call_id={} task_id={} kind={} user_id={} chat_id={}",
            call_id, task.task_id, task.kind, task.user_id, task.chat_id
        );
        info!("{}", LOG_CALL_WRAP);

        let mut payload = serde_json::from_str::<serde_json::Value>(&task.payload_json)
            .map_err(|err| anyhow::anyhow!("invalid payload_json for task {}: {err}", task.task_id))?;

        let task_kind_for_timeout_log = task.kind.clone();
        let worker_timeout_secs = state.worker_task_timeout_seconds.max(1);
        let task_result = tokio::time::timeout(Duration::from_secs(worker_timeout_secs), async {
        match task.kind.as_str() {
        "ask" => {
            let original_prompt = payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let injected_resume_followup_decision = maybe_bind_recent_failed_resume_context(
                state,
                &task,
                &mut payload,
                &original_prompt,
            )
            .await;
            let prompt = payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let source = payload
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let main_rules = main_flow_rules(state);
            let is_resume_continue = is_resume_continue_source(main_rules, source);
            let resume_followup_decision = if is_resume_continue {
                if let Some(decision) = injected_resume_followup_decision.clone() {
                    Some(decision)
                } else if payload.get("resume_context").is_some()
                    && payload.get("resume_user_text").is_some()
                {
                    Some(intent_router::ResumeFollowupDecision {
                        decision: "resume".to_string(),
                        reason: "resume_context_bound".to_string(),
                        confidence: Some(1.0),
                        bind_resume_context: true,
                    })
                } else {
                    let resume_context = payload
                        .get("resume_context")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    Some(
                        intent_router::classify_resume_followup_intent(
                            state,
                            &task,
                            &prompt,
                            &resume_context,
                            &json!({
                                "source": "resume_continue_source",
                                "failed_resume_context_ts": Value::Null,
                                "has_newer_successful_ask_after_failed_task": false,
                            }),
                        )
                        .await,
                    )
                }
            } else {
                None
            };
            let resume_should_apply_context = resume_followup_decision
                .as_ref()
                .map(|d| d.decision == "resume")
                .unwrap_or(false);
            let resume_should_discuss_context = resume_followup_decision
                .as_ref()
                .map(|d| d.decision == "defer")
                .unwrap_or(false);
            let runtime_prompt = if is_resume_continue && resume_should_apply_context {
                build_resume_continue_execute_prompt(state, &payload, prompt)
            } else if is_resume_continue && resume_should_discuss_context {
                build_resume_followup_discussion_prompt(state, &payload, prompt)
            } else {
                prompt.to_string()
            };
            info!(
                "worker_once: ask received_message task_id={} user_id={} chat_id={} text={}",
                task.task_id,
                task.user_id,
                task.chat_id,
                truncate_for_log(&runtime_prompt)
            );
            let agent_mode = payload
                .get("agent_mode")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let trimmed_request = runtime_prompt.trim();
            let is_yes = is_affirmation_click_text(state, trimmed_request);
            let is_no = is_negative_confirmation_click_text(state, trimmed_request);
            let mut auto_cancel_notice: Option<String> = None;
            if !is_resume_continue && agent_mode {
                // Use hard_rules-driven windows to avoid hardcoded timing behavior
                // in the main confirmation routing flow.
                let hard_rules = main_flow_rules(state);
                let effective_window_secs = effective_trade_confirm_window_secs(state, &task.channel);
                let effective_preview_ctx = find_recent_trade_preview_context(
                    state,
                    task.user_id,
                    task.chat_id,
                    effective_window_secs,
                );
                let stale_preview_ctx = if effective_window_secs < hard_rules.recent_trade_preview_window_secs
                    && (is_yes || is_no)
                {
                    find_recent_trade_preview_context(
                        state,
                        task.user_id,
                        task.chat_id,
                        hard_rules.recent_trade_preview_window_secs,
                    )
                } else {
                    None
                };
                if let Some(preview_ctx) = effective_preview_ctx {
                    info!(
                        "worker_once: ask task_id={} hard_trade_confirm_route input={} exchange={} symbol={} side={} qty={}",
                        task.task_id,
                        truncate_for_log(trimmed_request),
                        preview_ctx.exchange,
                        preview_ctx.symbol,
                        preview_ctx.side,
                        preview_ctx.qty
                    );
                    if is_yes || is_no {
                        let hard_text = if is_no {
                            build_trade_confirm_cancelled_text(state, &preview_ctx)
                        } else {
                            let mut submit_args = if let Some(quote_usd) = preview_ctx.quote_qty_usd {
                                json!({
                                    "action": "trade_submit",
                                    "exchange": preview_ctx.exchange,
                                    "symbol": preview_ctx.symbol,
                                    "side": preview_ctx.side,
                                    "order_type": preview_ctx.order_type,
                                    "quote_qty_usd": quote_usd,
                                    "confirm": true
                                })
                            } else {
                                json!({
                                    "action": "trade_submit",
                                    "exchange": preview_ctx.exchange,
                                    "symbol": preview_ctx.symbol,
                                    "side": preview_ctx.side,
                                    "order_type": preview_ctx.order_type,
                                    "qty": preview_ctx.qty,
                                    "confirm": true
                                })
                            };
                            // Restore limit/stop order parameters from preview context
                            if let Some(p) = preview_ctx.price {
                                submit_args["price"] = serde_json::Value::from(p);
                            }
                            if let Some(sp) = preview_ctx.stop_price {
                                submit_args["stop_price"] = serde_json::Value::from(sp);
                            }
                            if let Some(tif) = &preview_ctx.time_in_force {
                                submit_args["time_in_force"] = serde_json::Value::from(tif.as_str());
                            }
                            match run_skill_with_runner(state, &task, "crypto", submit_args).await {
                                Ok(text) => text,
                                Err(err) => {
                                    error!(
                                        "hard_trade_confirm: trade_submit skill failed task_id={} err={}",
                                        task.task_id, err
                                    );
                                    repo::update_task_failure(state, &task.task_id, &err)?;
                                    maybe_notify_schedule_result(state, &task, &payload, false, &err).await;
                                    info!("{}", LOG_CALL_WRAP);
                                    info!(
                                        "task_call_end task_id={} kind=ask status=failed path=hard_trade_confirm_submit error={}",
                                        task.task_id,
                                        truncate_for_log(&err)
                                    );
                                    info!("{}", LOG_CALL_WRAP);
                                    return Ok(());
                                }
                            }
                        };
                        let hard_text = intercept_response_text_for_delivery(&hard_text);
                        let result = json!({ "text": hard_text.clone() });
                        repo::update_task_success(state, &task.task_id, &result.to_string())?;
                        let _ = memory::service::insert_memory(
                            state,
                            task.user_id,
                            task.chat_id,
                            task.user_key.as_deref(),
                            &task.channel,
                            task.external_chat_id.as_deref(),
                            "user",
                            prompt,
                            state.memory.item_max_chars.max(256),
                        );
                        let _ = memory::service::insert_memory(
                            state,
                            task.user_id,
                            task.chat_id,
                            task.user_key.as_deref(),
                            &task.channel,
                            task.external_chat_id.as_deref(),
                            "assistant",
                            &hard_text,
                            state.memory.item_max_chars.max(256),
                        );
                        if let Err(err) = memory::service::maybe_refresh_long_term_summary(state, &task).await {
                            warn!("refresh long-term memory summary failed: {err}");
                        }
                        info!("{}", LOG_CALL_WRAP);
                        info!(
                            "task_call_end task_id={} kind=ask status=success path=hard_trade_confirm result={}",
                            task.task_id,
                            truncate_for_log(&hard_text)
                        );
                        info!("{}", LOG_CALL_WRAP);
                        return Ok(());
                    }
                    // Any non-confirm command while a preview is pending should
                    // cancel that preview and continue executing the new command.
                    auto_cancel_notice = Some(build_trade_confirm_cancelled_text(state, &preview_ctx));
                    info!(
                        "worker_once: ask task_id={} hard_trade_auto_cancel_then_continue input={}",
                        task.task_id,
                        truncate_for_log(trimmed_request)
                    );
                } else if let Some(stale_ctx) = stale_preview_ctx {
                    let hard_text = intercept_response_text_for_delivery(&build_trade_confirm_cancelled_text(state, &stale_ctx));
                    let result = json!({ "text": hard_text.clone() });
                    repo::update_task_success(state, &task.task_id, &result.to_string())?;
                    let _ = memory::service::insert_memory(
                        state,
                        task.user_id,
                        task.chat_id,
                        task.user_key.as_deref(),
                        &task.channel,
                        task.external_chat_id.as_deref(),
                        "user",
                        prompt,
                        state.memory.item_max_chars.max(256),
                    );
                    let _ = memory::service::insert_memory(
                        state,
                        task.user_id,
                        task.chat_id,
                        task.user_key.as_deref(),
                        &task.channel,
                        task.external_chat_id.as_deref(),
                        "assistant",
                        &hard_text,
                        state.memory.item_max_chars.max(256),
                    );
                    if let Err(err) = memory::service::maybe_refresh_long_term_summary(state, &task).await {
                        warn!("refresh long-term memory summary failed: {err}");
                    }
                    info!("{}", LOG_CALL_WRAP);
                    info!(
                        "task_call_end task_id={} kind=ask status=success path=hard_trade_confirm_expired result={}",
                        task.task_id,
                        truncate_for_log(&hard_text)
                    );
                    info!("{}", LOG_CALL_WRAP);
                    return Ok(());
                }
            }
            let direct_resume_execution = is_resume_continue && resume_should_apply_context;
            let direct_resume_discussion = is_resume_continue && resume_should_discuss_context;
            let context_resolution = if direct_resume_execution || direct_resume_discussion {
                intent_router::ContextResolution {
                    resolved_user_intent: runtime_prompt.clone(),
                    needs_clarify: false,
                    confidence: Some(1.0),
                    reason: if direct_resume_execution {
                        "resume_context_execute".to_string()
                    } else {
                        "resume_context_followup_defer".to_string()
                    },
                }
            } else {
                intent_router::resolve_user_request_with_context(state, &task, &runtime_prompt).await
            };
            let resolved_prompt = context_resolution.resolved_user_intent.clone();
            info!(
                "{} worker_once: ask resolved_message task_id={} needs_clarify={} confidence={} reason={} resolved_text={}",
                highlight_tag("routing"),
                task.task_id,
                context_resolution.needs_clarify,
                context_resolution.confidence.unwrap_or(-1.0),
                truncate_for_log(&context_resolution.reason),
                truncate_for_log(&resolved_prompt)
            );
            let memory_ctx = memory::service::prepare_prompt_with_memory(state, &task, &resolved_prompt);
            let long_term_summary = memory_ctx.long_term_summary;
            let preferences = memory_ctx.preferences;
            let recalled = memory_ctx.recalled;
            let prompt_with_memory = memory_ctx.prompt_with_memory;
            let chat_prompt_context =
                memory::service::memory_context_block(None, &preferences, &[], 512);
            let long_term_log = long_term_summary
                .as_deref()
                .map(truncate_for_log)
                .unwrap_or_else(|| "<none>".to_string());
            let recalled_log = if recalled.is_empty() {
                "<none>".to_string()
            } else {
                let merged = recalled
                    .iter()
                    .map(|(role, content)| format!("{role}:{content}"))
                    .collect::<Vec<_>>()
                    .join(" | ");
                truncate_for_log(&merged)
            };
            let preferences_log = if preferences.is_empty() {
                "<none>".to_string()
            } else {
                let merged = preferences
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(" | ");
                truncate_for_log(&merged)
            };
            info!(
                "worker_once: ask memory task_id={} memory.long_term_summary={} memory.preferences={} memory.recalled_recent_count={} memory.recalled_recent={}",
                task.task_id,
                long_term_log,
                preferences_log,
                recalled.len(),
                recalled_log,
            );

            // Source-id based classifier bypass list is hard_rules-driven.
            let classifier_direct_mode = main_flow_rules(state)
                .classifier_direct_sources
                .iter()
                .any(|s| s == &source.to_ascii_lowercase());

            let low_confidence = context_resolution.confidence.unwrap_or(0.0)
                < main_flow_rules(state).context_low_confidence_threshold;
            let force_clarify = context_resolution.needs_clarify && low_confidence;

            let result = if force_clarify {
                let clarify = intent_router::generate_clarify_question(
                    state,
                    &task,
                    &resolved_prompt,
                    &context_resolution.reason,
                )
                .await;
                Ok(AskReply::non_llm(clarify))
            } else if direct_resume_discussion {
                let resume_prompt_file = resolve_prompt_rel_path_for_vendor(
                    &state.workspace_root,
                    &active_prompt_vendor_name(state),
                    RESUME_FOLLOWUP_DISCUSSION_PROMPT_PATH,
                );
                log_prompt_render(
                    &task.task_id,
                    "resume_followup_discussion_prompt",
                    &resume_prompt_file,
                    None,
                );
                llm_gateway::run_with_fallback_with_prompt_file(
                    state,
                    &task,
                    &resolved_prompt,
                    &resume_prompt_file,
                )
                .await
                .map(|s| AskReply::llm(s.trim().to_string()))
            } else if direct_resume_execution {
                agent_engine::run_agent_with_tools(state, &task, &prompt_with_memory, &resolved_prompt)
                    .await
            } else if !is_resume_continue {
                if let Ok(Some(schedule_reply)) =
                    intent_router::try_handle_schedule_request(state, &task, &resolved_prompt).await
                {
                    let schedule_reply = intercept_response_text_for_delivery(&schedule_reply);
                    let result = json!({ "text": schedule_reply.clone() });
                    repo::update_task_success(state, &task.task_id, &result.to_string())?;
                    let _ = memory::service::insert_memory(
                        state,
                        task.user_id,
                        task.chat_id,
                        task.user_key.as_deref(),
                        &task.channel,
                        task.external_chat_id.as_deref(),
                        "user",
                        prompt,
                        state.memory.item_max_chars.max(256),
                    );
                    let _ = memory::service::insert_memory(
                        state,
                        task.user_id,
                        task.chat_id,
                        task.user_key.as_deref(),
                        &task.channel,
                        task.external_chat_id.as_deref(),
                        "assistant",
                        &schedule_reply,
                        state.memory.item_max_chars.max(256),
                    );
                    info!("{}", LOG_CALL_WRAP);
                    info!(
                        "task_call_end task_id={} kind=ask status=success path=schedule_direct result={}",
                        task.task_id,
                        truncate_for_log(&schedule_reply)
                    );
                    info!("{}", LOG_CALL_WRAP);
                    return Ok(());
                }
                if classifier_direct_mode && !resume_should_discuss_context {
                    // Classifier-style sub-requests (like telegram voice mode intent detection)
                    // need raw label outputs, so bypass chat response wrapping.
                    log_prompt_render(
                        &task.task_id,
                        "classifier_direct",
                        "prompts/classifier_direct.md",
                        None,
                    );
                    llm_gateway::run_with_fallback_with_prompt_file(
                        state,
                        &task,
                        &resolved_prompt,
                        "prompts/classifier_direct.md",
                    )
                    .await
                    .map(|s| AskReply::llm(s.trim().to_string()))
                } else {
                    let routed_mode = if resume_should_discuss_context {
                        RoutedMode::Chat
                    } else if agent_mode {
                        intent_router::route_request_mode(state, &task, &resolved_prompt).await
                    } else {
                        RoutedMode::Chat
                    };
                    info!(
                    "{} worker_once: ask task_id={} routed_mode={:?} agent_mode={}",
                    highlight_tag("routing"),
                        task.task_id, routed_mode, agent_mode
                    );

                    match routed_mode {
                        RoutedMode::Chat => {
                            let (chat_prompt_template, chat_prompt_file) = load_prompt_template_for_state(
                                state,
                                CHAT_RESPONSE_PROMPT_PATH,
                                CHAT_RESPONSE_PROMPT_TEMPLATE,
                            );
                            log_prompt_render(
                                &task.task_id,
                                "chat_response_prompt",
                                &chat_prompt_file,
                                None,
                            );
                            let chat_prompt = render_prompt_template(
                                &chat_prompt_template,
                                &[
                                    ("__PERSONA_PROMPT__", &state.persona_prompt),
                                    ("__CONTEXT__", &chat_prompt_context),
                                    ("__REQUEST__", &resolved_prompt),
                                ],
                            );
                            llm_gateway::run_with_fallback_with_prompt_file(
                                state,
                                &task,
                                &chat_prompt,
                                &chat_prompt_file,
                            )
                                .await
                                .map(AskReply::llm)
                        }
                        RoutedMode::Act => {
                            agent_engine::run_agent_with_tools(state, &task, &prompt_with_memory, &resolved_prompt)
                                .await
                        }
                        RoutedMode::ChatAct => {
                            let chat_act_goal = format!(
                                "{}\n\nMode hint: chat_act. Complete required actions first, then return a concise user-facing reply that confirms results naturally.",
                                prompt_with_memory
                            );
                            agent_engine::run_agent_with_tools(state, &task, &chat_act_goal, &resolved_prompt).await
                        }
                        RoutedMode::AskClarify => {
                            let clarify = intent_router::generate_clarify_question(
                                state,
                                &task,
                                &resolved_prompt,
                                "router_selected_ask_clarify",
                            )
                            .await;
                            Ok(AskReply::non_llm(clarify))
                        }
                    }
                }
            } else if classifier_direct_mode {
                // Classifier-style sub-requests (like telegram voice mode intent detection)
                // need raw label outputs, so bypass chat response wrapping.
                log_prompt_render(
                    &task.task_id,
                    "classifier_direct",
                    "prompts/classifier_direct.md",
                    None,
                );
                llm_gateway::run_with_fallback_with_prompt_file(
                    state,
                    &task,
                    &resolved_prompt,
                    "prompts/classifier_direct.md",
                )
                    .await
                    .map(|s| AskReply::llm(s.trim().to_string()))
            } else {
                let routed_mode = if agent_mode {
                    intent_router::route_request_mode(state, &task, &resolved_prompt).await
                } else {
                    RoutedMode::Chat
                };
                info!(
                "{} worker_once: ask task_id={} routed_mode={:?} agent_mode={}",
                highlight_tag("routing"),
                    task.task_id, routed_mode, agent_mode
                );

                match routed_mode {
                    RoutedMode::Chat => {
                        let (chat_prompt_template, chat_prompt_file) = load_prompt_template_for_state(
                            state,
                            CHAT_RESPONSE_PROMPT_PATH,
                            CHAT_RESPONSE_PROMPT_TEMPLATE,
                        );
                        log_prompt_render(
                            &task.task_id,
                            "chat_response_prompt",
                            &chat_prompt_file,
                            None,
                        );
                        let chat_prompt = render_prompt_template(
                            &chat_prompt_template,
                            &[
                                ("__PERSONA_PROMPT__", &state.persona_prompt),
                                ("__CONTEXT__", &chat_prompt_context),
                                ("__REQUEST__", &resolved_prompt),
                            ],
                        );
                        llm_gateway::run_with_fallback_with_prompt_file(
                            state,
                            &task,
                            &chat_prompt,
                            &chat_prompt_file,
                        )
                            .await
                            .map(AskReply::llm)
                    }
                    RoutedMode::Act => {
                        agent_engine::run_agent_with_tools(state, &task, &prompt_with_memory, &resolved_prompt)
                            .await
                    }
                    RoutedMode::ChatAct => {
                        let chat_act_goal = format!(
                            "{}\n\nMode hint: chat_act. Complete required actions first, then return a concise user-facing reply that confirms results naturally.",
                            prompt_with_memory
                        );
                        agent_engine::run_agent_with_tools(state, &task, &chat_act_goal, &resolved_prompt).await
                    }
                    RoutedMode::AskClarify => {
                        let clarify = intent_router::generate_clarify_question(
                            state,
                            &task,
                            &resolved_prompt,
                            "router_selected_ask_clarify",
                        )
                        .await;
                        Ok(AskReply::non_llm(clarify))
                    }
                }
            };

            match result {
                Ok(mut answer) => {
                    if let Some(cancel_notice) = auto_cancel_notice.take() {
                        let prefixed_text = if answer.text.trim().is_empty() {
                            cancel_notice.clone()
                        } else {
                            format!("{cancel_notice}\n{}", answer.text)
                        };
                        answer.text = prefixed_text;
                        if !answer.messages.is_empty() {
                            let mut merged_messages = Vec::with_capacity(answer.messages.len() + 1);
                            merged_messages.push(cancel_notice);
                            merged_messages.extend(answer.messages);
                            answer.messages = merged_messages;
                        }
                    }
                    let (answer_text, answer_messages) = intercept_response_payload_for_delivery(
                        answer.text,
                        answer.messages,
                    );
                    let mut result = if answer_messages.is_empty() {
                        json!({ "text": answer_text.clone() })
                    } else {
                        json!({ "text": answer_text.clone(), "messages": answer_messages })
                    };
                    if let Some(decision) = &resume_followup_decision {
                        if let Some(obj) = result.as_object_mut() {
                            obj.insert(
                                "resume_followup_decision".to_string(),
                                json!({
                                    "decision": decision.decision,
                                    "reason": decision.reason,
                                    "confidence": decision.confidence,
                                }),
                            );
                        }
                    }
                    repo::update_task_success(state, &task.task_id, &result.to_string())?;
                    maybe_notify_schedule_result(state, &task, &payload, true, &answer_text).await;
                    let _ = memory::service::insert_memory(
                        state,
                        task.user_id,
                        task.chat_id,
                        task.user_key.as_deref(),
                        &task.channel,
                        task.external_chat_id.as_deref(),
                        "user",
                        prompt,
                        state.memory.item_max_chars.max(256),
                    );
                    let assistant_memory_text = if answer.is_llm_reply
                        && state.memory.mark_llm_reply_in_short_term
                    {
                        format!("{}{}", memory::LLM_SHORT_TERM_MEMORY_PREFIX, answer_text)
                    } else {
                        answer_text.clone()
                    };
                    let _ = memory::service::insert_memory(
                        state,
                        task.user_id,
                        task.chat_id,
                        task.user_key.as_deref(),
                        &task.channel,
                        task.external_chat_id.as_deref(),
                        "assistant",
                        &assistant_memory_text,
                        state.memory.item_max_chars.max(256),
                    );
                    if let Err(err) = memory::service::maybe_refresh_long_term_summary(state, &task).await {
                        warn!("refresh long-term memory summary failed: {err}");
                    }
                    info!("{}", LOG_CALL_WRAP);
                    info!(
                        "task_call_end task_id={} kind=ask status=success path=normal result={}",
                        task.task_id,
                        truncate_for_log(&answer_text)
                    );
                    info!("{}", LOG_CALL_WRAP);
                }
                Err(err_text) => {
                    if let Some((user_error, resume_ctx)) = parse_resume_context_error(&err_text) {
                        let resume_payload = resume_ctx
                            .get("resume_context")
                            .cloned()
                            .unwrap_or(resume_ctx);
                        let result = json!({
                            "text": user_error.clone(),
                            "resume_context": resume_payload,
                        });
                        repo::update_task_failure_with_result(
                            state,
                            &task.task_id,
                            &result.to_string(),
                            &user_error,
                        )?;
                        maybe_notify_schedule_result(state, &task, &payload, false, &user_error).await;
                        info!("{}", LOG_CALL_WRAP);
                        info!(
                            "task_call_end task_id={} kind=ask status=failed path=normal error={} resume_context=true",
                            task.task_id,
                            truncate_for_log(&user_error)
                        );
                        info!("{}", LOG_CALL_WRAP);
                        return Ok(());
                    }
                    error!(
                        "worker_once: ask task_id={} failed: {}",
                        task.task_id, err_text
                    );
                    repo::update_task_failure(state, &task.task_id, &err_text)?;
                    maybe_notify_schedule_result(state, &task, &payload, false, &err_text).await;
                    info!("{}", LOG_CALL_WRAP);
                    info!(
                        "task_call_end task_id={} kind=ask status=failed path=normal error={}",
                        task.task_id,
                        truncate_for_log(&err_text)
                    );
                    info!("{}", LOG_CALL_WRAP);
                }
            }
        }
        "run_skill" => {
            let skill_name = payload
                .get("skill_name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let args = payload.get("args").cloned().unwrap_or_else(|| json!(""));
            let _action = args
                .as_object()
                .and_then(|m| m.get("action"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            let is_price_alert_action = is_crypto_price_alert_action(state, skill_name, &args);
            let schedule_triggered = payload
                .get("schedule_triggered")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            info!(
                "worker_once: processing run_skill task_id={} user_id={} chat_id={} skill_name={} args={}",
                task.task_id,
                task.user_id,
                task.chat_id,
                skill_name,
                truncate_for_log(&args.to_string())
            );

            // Whether to require user confirmation before crypto trade_submit is decided by the planner; no hard block here.

            match execution_adapters::run_skill(state, &task, skill_name, args).await {
                Ok(text) => {
                    let price_alert_rules = main_flow_rules(state);
                    let no_trigger = text
                        .trim_start()
                        .starts_with(&price_alert_rules.crypto_price_alert_not_triggered_tag);
                    let clean_text = if is_price_alert_action {
                        strip_price_alert_tag(&text, price_alert_rules)
                    } else {
                        text.clone()
                    };
                    let clean_text = intercept_response_text_for_delivery(&clean_text);
                    let result = json!({ "text": clean_text });
                    repo::update_task_success(state, &task.task_id, &result.to_string())?;
                    if !(schedule_triggered && is_price_alert_action && no_trigger) {
                        maybe_notify_schedule_result(state, &task, &payload, true, &clean_text).await;
                    }
                    let _ = memory::service::insert_memory(
                        state,
                        task.user_id,
                        task.chat_id,
                        task.user_key.as_deref(),
                        &task.channel,
                        task.external_chat_id.as_deref(),
                        "assistant",
                        &clean_text,
                        state.memory.item_max_chars.max(256),
                    );
                    let _ = repo::insert_audit_log(
                        state,
                        Some(task.user_id),
                        "run_skill",
                        Some(
                            &json!({
                                "task_id": task.task_id,
                                "chat_id": task.chat_id,
                                "skill_name": skill_name,
                                "status": "ok"
                            })
                            .to_string(),
                        ),
                        None,
                    );
                    info!("{}", LOG_CALL_WRAP);
                    info!(
                        "task_call_end task_id={} kind=run_skill status=success skill={} result={}",
                        task.task_id,
                        skill_name,
                        truncate_for_log(&clean_text)
                    );
                    info!("{}", LOG_CALL_WRAP);
                }
                Err(err_text) => {
                    error!(
                        "worker_once: run_skill task_id={} skill={} failed: {}",
                        task.task_id, skill_name, err_text
                    );
                    repo::update_task_failure(state, &task.task_id, &err_text)?;
                    maybe_notify_schedule_result(state, &task, &payload, false, &err_text).await;
                    let action = if err_text.contains("timeout") {
                        "timeout"
                    } else {
                        "run_skill"
                    };
                    let _ = repo::insert_audit_log(
                        state,
                        Some(task.user_id),
                        action,
                        Some(
                            &json!({
                                "task_id": task.task_id,
                                "chat_id": task.chat_id,
                                "skill_name": skill_name,
                                "status": "failed"
                            })
                            .to_string(),
                        ),
                        Some(&err_text),
                    );
                    info!("{}", LOG_CALL_WRAP);
                    info!(
                        "task_call_end task_id={} kind=run_skill status=failed skill={} error={}",
                        task.task_id,
                        skill_name,
                        truncate_for_log(&err_text)
                    );
                    info!("{}", LOG_CALL_WRAP);
                }
            }
        }
        other => {
            let err = format!("Unsupported task kind: {other}");
            error!(
                "worker_once: unsupported task kind for task_id={}: {}",
                task.task_id, other
            );
            repo::update_task_failure(state, &task.task_id, &err)?;
            info!("{}", LOG_CALL_WRAP);
            info!(
                "task_call_end task_id={} kind={} status=failed error={}",
                task.task_id,
                other,
                truncate_for_log(&err)
            );
            info!("{}", LOG_CALL_WRAP);
        }
    }
        Ok::<(), anyhow::Error>(())
        })
        .await;

        match task_result {
            Ok(inner) => inner?,
            Err(_) => {
                let timeout_err = format!(
                    "worker timeout after {}s while processing kind={}",
                    worker_timeout_secs, task_kind_for_timeout_log
                );
                error!(
                    "worker_once timeout: task_id={} kind={} timeout_seconds={}",
                    task.task_id, task_kind_for_timeout_log, worker_timeout_secs
                );
                update_task_timeout(state, &task.task_id, &timeout_err)?;
                maybe_notify_schedule_result(state, &task, &payload, false, &timeout_err).await;
                info!("{}", LOG_CALL_WRAP);
                info!(
                    "task_call_end task_id={} kind={} status=timeout error={}",
                    task.task_id,
                    task_kind_for_timeout_log,
                    truncate_for_log(&timeout_err)
                );
                info!("{}", LOG_CALL_WRAP);
            }
        }
        Ok(())
    }
    .instrument(call_span)
    .await
}

async fn maybe_notify_schedule_result(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    success: bool,
    text: &str,
) {
    let is_scheduled = payload
        .get("schedule_triggered")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !is_scheduled {
        return;
    }
    let Some(job_id) = payload.get("schedule_job_id").and_then(|v| v.as_str()) else {
        return;
    };
    let prefix = if success {
        i18n_t_with_default(
            state,
            "clawd.msg.schedule_run_success_prefix",
            "Scheduled job executed successfully",
        )
    } else {
        i18n_t_with_default(
            state,
            "clawd.msg.schedule_run_failed_prefix",
            "Scheduled job execution failed",
        )
    };
    let job_id_label = i18n_t_with_default(state, "clawd.msg.schedule_run_job_id_label", "Job ID");
    let status_block = format!("{prefix}\n{job_id_label}: {job_id}");
    let text_trimmed = text.trim();
    let message = if text_trimmed.is_empty() {
        status_block
    } else {
        format!("{text_trimmed}\n\n{status_block}")
    };
    if let Err(err) = send_task_channel_message(state, task, payload, &message).await {
        warn!(
            "schedule notify failed: task_id={} chat_id={} err={}",
            task.task_id, task.chat_id, err
        );
    }
}

fn runtime_channel_from_payload(state: &AppState, payload: &Value) -> RuntimeChannel {
    match payload.get("channel").and_then(|v| v.as_str()) {
        Some(v) if is_whatsapp_channel_value(main_flow_rules(state), v) => RuntimeChannel::Whatsapp,
        _ => RuntimeChannel::Telegram,
    }
}

fn is_whatsapp_channel_value(rules: &MainFlowRules, raw: &str) -> bool {
    let channel = raw.trim().to_ascii_lowercase();
    rules
        .runtime_whatsapp_channel_aliases
        .iter()
        .any(|v| v == &channel)
}

fn is_resume_continue_source(rules: &MainFlowRules, raw: &str) -> bool {
    let source = raw.trim().to_ascii_lowercase();
    rules.resume_continue_sources.iter().any(|v| v == &source)
}

async fn maybe_bind_recent_failed_resume_context(
    state: &AppState,
    task: &ClaimedTask,
    payload: &mut Value,
    user_text: &str,
) -> Option<intent_router::ResumeFollowupDecision> {
    if payload.get("resume_context").is_some() {
        return None;
    }
    let source = payload
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if is_resume_continue_source(main_flow_rules(state), source) {
        return None;
    }
    let (resume_context, resume_context_ts) =
        find_recent_failed_resume_context(state, task.user_id, task.chat_id)?;
    let binding_context = json!({
        "source": "recent_failed_resume_context",
        "failed_resume_context_ts": resume_context_ts,
        "has_newer_successful_ask_after_failed_task": has_newer_successful_ask_after(
            state,
            task.user_id,
            task.chat_id,
            resume_context_ts,
        ),
    });
    let decision = intent_router::classify_resume_followup_intent(
        state,
        task,
        user_text,
        &resume_context,
        &binding_context,
    )
    .await;
    if decision.decision != "resume" && !decision.bind_resume_context {
        return None;
    }
    let obj = payload.as_object_mut()?;
    let resume_source = main_flow_rules(state)
        .resume_continue_sources
        .first()
        .cloned()
        .unwrap_or_else(|| "resume_continue_execute".to_string());
    obj.insert("source".to_string(), Value::String(resume_source));
    obj.insert(
        "resume_user_text".to_string(),
        Value::String(user_text.to_string()),
    );
    obj.insert("resume_context".to_string(), resume_context);
    Some(decision)
}

fn parse_task_status_with_rules(rules: &MainFlowRules, raw: &str) -> TaskStatus {
    let s = raw.trim().to_ascii_lowercase();
    if s == rules.task_status_queued {
        TaskStatus::Queued
    } else if s == rules.task_status_running {
        TaskStatus::Running
    } else if s == rules.task_status_succeeded {
        TaskStatus::Succeeded
    } else if s == rules.task_status_failed {
        TaskStatus::Failed
    } else if s == rules.task_status_canceled {
        TaskStatus::Canceled
    } else if s == rules.task_status_timeout {
        TaskStatus::Timeout
    } else {
        TaskStatus::Failed
    }
}

fn task_payload_value(task: &ClaimedTask) -> Option<Value> {
    serde_json::from_str::<Value>(&task.payload_json).ok()
}

fn is_crypto_price_alert_action(state: &AppState, skill_name: &str, args: &Value) -> bool {
    // Route crypto alert-action aliases via hard_rules instead of inline literals.
    if canonical_skill_name(skill_name) != "crypto" {
        return false;
    }
    let rules = main_flow_rules(state);
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    rules
        .crypto_price_alert_actions
        .iter()
        .any(|a| a == &action)
}

fn strip_price_alert_tag(text: &str, rules: &MainFlowRules) -> String {
    text.trim()
        .trim_start_matches(&rules.crypto_price_alert_triggered_tag)
        .trim_start_matches(&rules.crypto_price_alert_not_triggered_tag)
        .trim()
        .to_string()
}

fn task_runtime_channel(state: &AppState, task: &ClaimedTask) -> RuntimeChannel {
    if is_whatsapp_channel_value(main_flow_rules(state), &task.channel) {
        return RuntimeChannel::Whatsapp;
    }
    let Some(payload) = task_payload_value(task) else {
        return RuntimeChannel::Telegram;
    };
    runtime_channel_from_payload(state, &payload)
}

fn task_external_chat_id(task: &ClaimedTask) -> Option<String> {
    if let Some(v) = task
        .external_chat_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(v);
    }
    let payload = task_payload_value(task)?;
    payload
        .get("external_chat_id")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

async fn send_task_channel_message(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    text: &str,
) -> Result<(), String> {
    match runtime_channel_from_payload(state, payload) {
        RuntimeChannel::Telegram => send_telegram_message(state, task.chat_id, text).await,
        RuntimeChannel::Whatsapp => {
            let to = task_external_chat_id(task)
                .or_else(|| {
                    payload
                        .get("external_chat_id")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                })
                .ok_or_else(|| "missing external_chat_id for whatsapp task".to_string())?;
            match resolve_whatsapp_delivery_route(state, payload) {
                WhatsappDeliveryRoute::WebBridge => {
                    send_whatsapp_web_bridge_text_message(state, &to, text).await
                }
                WhatsappDeliveryRoute::Cloud => {
                    send_whatsapp_cloud_text_message(state, &to, text).await
                }
            }
        }
    }
}

fn resolve_whatsapp_delivery_route(state: &AppState, payload: &Value) -> WhatsappDeliveryRoute {
    // Keep adapter alias mapping in hard_rules (configurable) instead of
    // scattering literal adapter names in main request routing flow.
    let rules = main_flow_rules(state);
    let adapter = payload
        .get("adapter")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if rules.whatsapp_web_adapters.iter().any(|a| a == &adapter) {
        return WhatsappDeliveryRoute::WebBridge;
    }
    if rules.whatsapp_cloud_adapters.iter().any(|a| a == &adapter) {
        return WhatsappDeliveryRoute::Cloud;
    }
    if state.whatsapp_web_enabled && !state.whatsapp_cloud_enabled {
        return WhatsappDeliveryRoute::WebBridge;
    }
    WhatsappDeliveryRoute::Cloud
}

async fn send_telegram_message(state: &AppState, chat_id: i64, text: &str) -> Result<(), String> {
    let token = state.telegram_bot_token.trim();
    if token.is_empty() {
        return Err("telegram bot token is empty".to_string());
    }
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let resp = state
        .http_client
        .post(&url)
        .json(&json!({
            "chat_id": chat_id,
            "text": text
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("status={status} body={body}"));
    }
    Ok(())
}

async fn send_whatsapp_cloud_text_message(
    state: &AppState,
    to: &str,
    text: &str,
) -> Result<(), String> {
    let token = state.whatsapp_access_token.trim();
    if token.is_empty() {
        return Err("whatsapp access_token is empty".to_string());
    }
    let phone_number_id = state.whatsapp_phone_number_id.trim();
    if phone_number_id.is_empty() {
        return Err("whatsapp phone_number_id is empty".to_string());
    }
    let base = state.whatsapp_api_base.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("whatsapp api_base is empty".to_string());
    }
    let url = format!("{base}/v23.0/{phone_number_id}/messages");
    let resp = state
        .http_client
        .post(&url)
        .bearer_auth(token)
        .json(&json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "text",
            "text": {
                "body": text
            }
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("status={status} body={body}"));
    }
    Ok(())
}

async fn send_whatsapp_web_bridge_text_message(
    state: &AppState,
    to: &str,
    text: &str,
) -> Result<(), String> {
    let base = state
        .whatsapp_web_bridge_base_url
        .trim()
        .trim_end_matches('/');
    if base.is_empty() {
        return Err("whatsapp_web.bridge_base_url is empty".to_string());
    }
    let url = format!("{base}/v1/send-text");
    let resp = state
        .http_client
        .post(&url)
        .json(&json!({
            "to": to,
            "text": text
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("wa-web bridge status={status} body={body}"));
    }
    Ok(())
}

async fn run_skill_with_runner(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: serde_json::Value,
) -> Result<String, String> {
    let policy_token = format!("skill:{skill_name}");
    if !state
        .tools_policy
        .is_allowed(&policy_token, state.active_provider_type.as_deref())
    {
        return Err(format!("blocked by policy: {policy_token}"));
    }

    let skill_timeout_secs = match skill_name {
        "image_generate" | "image_edit" => state.skill_timeout_seconds.max(180),
        "image_vision" => state.skill_timeout_seconds.max(90),
        "audio_transcribe" => state.skill_timeout_seconds.max(120),
        "audio_synthesize" => state.skill_timeout_seconds.max(90),
        "crypto" => state.skill_timeout_seconds.max(60),
        _ => state.skill_timeout_seconds,
    };

    if skill_name.is_empty() {
        return Err("skill_name is empty".to_string());
    }

    const BUILTIN_SKILL_NAMES: &[&str] = &[
        "run_cmd",
        "read_file",
        "write_file",
        "list_dir",
        "make_dir",
        "remove_file",
    ];
    if BUILTIN_SKILL_NAMES.contains(&skill_name) {
        if !state.skills_list.contains(skill_name) {
            let mut allowed: Vec<String> = state.skills_list.iter().cloned().collect();
            allowed.sort();
            let enabled = allowed.join(", ");
            let err_text = i18n_t_with_default(
                state,
                "clawd.msg.skill_disabled_with_enabled_list",
                "Skill is not enabled: {skill}. Please enable it in config and try again. (Currently enabled: {enabled_skills})",
            )
            .replace("{skill}", skill_name)
            .replace("{enabled_skills}", &enabled);
            return Err(err_text);
        }
        return execute_builtin_skill(state, skill_name, &args).await;
    }

    if !state.skills_list.contains(skill_name) {
        let mut allowed: Vec<String> = state.skills_list.iter().cloned().collect();
        allowed.sort();
        let enabled = allowed.join(", ");
        let err_text = i18n_t_with_default(
            state,
            "clawd.msg.skill_disabled_with_enabled_list",
            "Skill is not enabled: {skill}. Please enable it in config and try again. (Currently enabled: {enabled_skills})",
        )
        .replace("{skill}", skill_name)
        .replace("{enabled_skills}", &enabled);
        return Err(err_text);
    }

    let _permit = state
        .skill_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| format!("skill semaphore closed: {err}"))?;

    let args = enrich_skill_args_with_memory(state, task, skill_name, args).await;
    let args = inject_skill_memory_context(state, task, skill_name, args);
    let args = ensure_default_output_dir_for_skill_args(&state.workspace_root, skill_name, args);
    let source = match task_runtime_channel(state, task) {
        RuntimeChannel::Whatsapp => "whatsapp",
        RuntimeChannel::Telegram => "telegram",
    };
    let mut value =
        run_skill_with_runner_once(state, task, skill_name, &args, &source, skill_timeout_secs)
            .await?;
    let mut status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("error")
        .to_string();

    if status != "ok" && canonical_skill_name(skill_name) == "crypto" {
        let main_rules = main_flow_rules(state);
        let action = args
            .as_object()
            .and_then(|m| m.get("action"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let err_text = value
            .get("error_text")
            .and_then(|v| v.as_str())
            .unwrap_or("skill execution failed")
            .to_ascii_lowercase();
        let unsupported = main_rules
            .crypto_unsupported_error_keywords
            .iter()
            .any(|k| err_text.contains(k));
        if action == main_rules.crypto_price_alert_primary_action.as_str() && unsupported {
            for legacy_action in &main_rules.crypto_price_alert_fallback_actions {
                let mut retry_args = args.clone();
                if let Some(map) = retry_args.as_object_mut() {
                    map.insert("action".to_string(), Value::String(legacy_action.clone()));
                } else {
                    break;
                }
                let retry_value = run_skill_with_runner_once(
                    state,
                    task,
                    skill_name,
                    &retry_args,
                    &source,
                    skill_timeout_secs,
                )
                .await?;
                let retry_status = retry_value
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("error");
                if retry_status == "ok" {
                    info!(
                        "run_skill_with_runner: fallback action used for crypto task_id={} from={} to={}",
                        task.task_id, main_rules.crypto_price_alert_primary_action, legacy_action
                    );
                    value = retry_value;
                    status = "ok".to_string();
                    break;
                }
            }
        }
    }

    if status != "ok" {
        return Err(value
            .get("error_text")
            .and_then(|v| v.as_str())
            .unwrap_or("skill execution failed")
            .to_string());
    }

    if let Some((provider, model, model_kind)) = extract_skill_provider_model(&value) {
        info!(
            "{} skill_model_selected task_id={} skill={} provider={} model={} model_kind={}",
            highlight_tag("skill_llm"),
            task.task_id,
            skill_name,
            provider,
            model,
            model_kind
        );
    }

    if let Some(llm_meta) = value
        .get("extra")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("llm"))
        .and_then(|v| v.as_object())
    {
        let prompt_name = llm_meta
            .get("prompt_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let model = llm_meta
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        info!(
            "{} skill_llm_call task_id={} skill={} prompt={} model={}",
            highlight_tag("skill_llm"),
            task.task_id,
            skill_name,
            prompt_name,
            model
        );
    }

    let mut text = value
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if canonical_skill_name(skill_name) == "image_vision" {
        let action = args
            .as_object()
            .and_then(|m| m.get("action"))
            .and_then(|v| v.as_str())
            .unwrap_or("describe")
            .to_ascii_lowercase();
        let target_language = args
            .as_object()
            .and_then(|m| m.get("response_language").or_else(|| m.get("language")))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(lang) = target_language {
            if matches!(
                action.as_str(),
                "describe" | "compare" | "screenshot_summary"
            ) {
                match rewrite_image_vision_output_language(state, task, &text, &lang).await {
                    Ok(rewritten) => {
                        info!(
                            "rewrite_image_vision_output_language: task_id={} lang={} action={} status=ok",
                            task.task_id, lang, action
                        );
                        text = rewritten;
                    }
                    Err(err) => {
                        warn!(
                            "rewrite_image_vision_output_language: task_id={} lang={} action={} status=failed err={}",
                            task.task_id, lang, action, err
                        );
                    }
                }
            }
        }
    }
    Ok(text)
}

fn extract_skill_provider_model(value: &Value) -> Option<(String, String, String)> {
    let extra = value.get("extra")?.as_object()?;
    let provider = extra
        .get("provider")
        .or_else(|| extra.get("vendor"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let model = extra
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let model_kind = extra
        .get("model_kind")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("unknown");
    Some((
        provider.to_string(),
        model.to_string(),
        model_kind.to_string(),
    ))
}

async fn run_skill_with_runner_once(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: &serde_json::Value,
    source: &str,
    skill_timeout_secs: u64,
) -> Result<serde_json::Value, String> {
    let credential_context = if canonical_skill_name(skill_name) == "crypto" {
        exchange_credential_context_for_task(state, task)
    } else {
        json!({})
    };
    let llm_skill = canonical_skill_name(skill_name) == "chat";
    let user_key_for_skill = if llm_skill {
        Value::Null
    } else {
        task.user_key
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null)
    };
    let req_line = json!({
        "request_id": task.task_id,
        "user_id": task.user_id,
        "chat_id": task.chat_id,
        "user_key": user_key_for_skill,
        "external_user_id": task.external_user_id,
        "external_chat_id": task_external_chat_id(task),
        "skill_name": skill_name,
        "args": args,
        "context": {
            "source": source,
            "kind": "run_skill",
            "user_key": if llm_skill { Value::Null } else { task.user_key.clone().map(Value::String).unwrap_or(Value::Null) },
                "exchange_credentials": credential_context
        }
    })
    .to_string();

    if !state.skill_runner_path.exists() {
        return Err(format!(
            "skill-runner binary not found: path={} (workspace_root={})",
            state.skill_runner_path.display(),
            state.workspace_root.display()
        ));
    }

    let selected_openai_model = llm_gateway::selected_openai_model(state);
    let mut child = Command::new(&state.skill_runner_path)
        .env("SKILL_TIMEOUT_SECONDS", skill_timeout_secs.to_string())
        .env(
            "OPENAI_API_KEY",
            llm_gateway::selected_openai_api_key(state),
        )
        .env(
            "OPENAI_BASE_URL",
            llm_gateway::selected_openai_base_url(state),
        )
        .env("OPENAI_MODEL", selected_openai_model.clone())
        .env("CHAT_SKILL_MODEL", selected_openai_model)
        .env("WORKSPACE_ROOT", state.workspace_root.display().to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| {
            format!(
                "spawn skill-runner failed: path={} err={}",
                state.skill_runner_path.display(),
                err
            )
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("{req_line}\n").as_bytes())
            .await
            .map_err(|err| format!("write skill-runner stdin failed: {err}"))?;
        stdin
            .flush()
            .await
            .map_err(|err| format!("flush skill-runner stdin failed: {err}"))?;
    }

    let mut out_line = String::new();
    let mut err_line = String::new();

    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout);
        let read_out = tokio::time::timeout(
            Duration::from_secs(skill_timeout_secs.max(1)),
            reader.read_line(&mut out_line),
        )
        .await;

        match read_out {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => return Err(format!("read skill-runner stdout failed: {err}")),
            Err(_) => {
                let _ = child.kill().await;
                return Err("skill-runner timeout".to_string());
            }
        }
    }

    if let Some(stderr) = child.stderr.take() {
        let mut err_reader = BufReader::new(stderr);
        let _ = err_reader.read_line(&mut err_line).await;
    }

    let _ = child.wait().await;

    if out_line.trim().is_empty() {
        return Err(format!("empty skill-runner output: {}", err_line.trim()));
    }

    serde_json::from_str(out_line.trim()).map_err(|err| format!("invalid skill-runner json: {err}"))
}

async fn rewrite_image_vision_output_language(
    state: &AppState,
    task: &ClaimedTask,
    original_text: &str,
    target_language: &str,
) -> Result<String, String> {
    if original_text.trim().is_empty() {
        return Ok(original_text.to_string());
    }
    let (prompt_template, prompt_file) = load_prompt_template_for_state(
        state,
        "prompts/image_output_rewrite_prompt.md",
        IMAGE_OUTPUT_REWRITE_PROMPT_TEMPLATE,
    );
    let prompt = render_prompt_template(
        &prompt_template,
        &[
            ("__TARGET_LANGUAGE__", target_language),
            ("__ORIGINAL_OUTPUT__", original_text),
        ],
    );
    log_prompt_render(
        &task.task_id,
        "image_output_rewrite_prompt",
        &prompt_file,
        None,
    );
    let out = run_llm_with_fallback_with_prompt_file(state, task, &prompt, &prompt_file).await?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return Err("empty rewrite output".to_string());
    }
    Ok(trimmed.to_string())
}

fn inject_skill_memory_context(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: Value,
) -> Value {
    if !state.memory.skill_memory_enabled {
        return args;
    }
    let mut obj = match args {
        Value::Object(map) => map,
        other => return other,
    };
    if canonical_skill_name(skill_name) == "chat" {
        return Value::Object(obj);
    }
    if obj.contains_key("_memory") {
        return Value::Object(obj);
    }
    let anchor = skill_memory_anchor(skill_name, &obj);
    let (long_term_summary, preferences, recalled) = memory::service::recall_memory_context_parts(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        &anchor,
        state.memory.recall_limit.max(1),
        true,
        true,
    );
    let memory_context = memory::service::memory_context_block(
        long_term_summary.as_deref(),
        &preferences,
        &recalled,
        state.memory.skill_memory_max_chars.max(384),
    );
    let mut pref_map = serde_json::Map::new();
    for (k, v) in &preferences {
        pref_map.insert(k.clone(), Value::String(v.clone()));
    }
    let lang_hint = memory::service::preferred_response_language(&preferences).unwrap_or_default();
    obj.insert(
        "_memory".to_string(),
        json!({
            "context": memory_context,
            "long_term_summary": long_term_summary.unwrap_or_default(),
            "preferences": Value::Object(pref_map),
            "lang_hint": lang_hint
        }),
    );
    Value::Object(obj)
}

fn skill_memory_anchor(skill_name: &str, args_obj: &serde_json::Map<String, Value>) -> String {
    let mut parts = vec![format!("skill={skill_name}")];
    for key in [
        "text",
        "query",
        "instruction",
        "goal",
        "prompt",
        "message",
        "content",
        "action",
    ] {
        if let Some(val) = args_obj.get(key).and_then(|v| v.as_str()) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }
    parts.join(" | ")
}

async fn enrich_skill_args_with_memory(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: Value,
) -> Value {
    let canonical = canonical_skill_name(skill_name);
    if canonical == "image_edit" {
        let obj = args.as_object().cloned().unwrap_or_default();
        if image_edit_args_has_image(&obj) {
            return Value::Object(obj);
        }
        let ref_goal = obj
            .get("instruction")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if let Some(path) = resolve_image_for_edit_from_context_llm(state, task, &ref_goal).await {
            info!(
                "image_edit_auto_resolve: task_id={} user_id={} chat_id={} selected_path={} instruction={}",
                task.task_id,
                task.user_id,
                task.chat_id,
                path,
                truncate_for_log(&ref_goal)
            );
            return normalize_image_edit_args(Value::Object(obj), &ref_goal, &path);
        }
        return Value::Object(obj);
    }
    if canonical != "image_vision" {
        return args;
    }
    let mut obj = args.as_object().cloned().unwrap_or_default();
    let has_lang = obj
        .get("response_language")
        .and_then(|v| v.as_str())
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
        || obj
            .get("language")
            .and_then(|v| v.as_str())
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
    if has_lang {
        return Value::Object(obj);
    }
    if let Some(lang) = infer_language_preference_from_memory_llm(state, task).await {
        obj.insert("response_language".to_string(), Value::String(lang));
    }
    Value::Object(obj)
}

async fn infer_language_preference_from_memory_llm(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<String> {
    let preferences = recall_user_preferences(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        state.memory.preference_recall_limit.max(1),
    )
    .ok()?;
    if let Some(lang) = memory::service::preferred_response_language(&preferences) {
        info!(
            "infer_language_preference_from_memory_llm: task_id={} user_id={} chat_id={} source=structured_preferences language={}",
            task.task_id, task.user_id, task.chat_id, lang
        );
        return Some(lang);
    }
    let (long_term_summary, pref_fallback, recalled) = memory::service::recall_memory_context_parts(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        "infer language preference",
        state.memory.recall_limit.max(1),
        state.memory.image_memory_include_long_term,
        state.memory.image_memory_include_preferences,
    );
    let memory_context = memory::service::memory_context_block(
        long_term_summary.as_deref(),
        &pref_fallback,
        &recalled,
        state.memory.image_memory_max_chars.max(384),
    );
    if memory_context == "<none>" {
        return None;
    }
    let (prompt_template, prompt_file) = load_prompt_template_for_state(
        state,
        "prompts/language_infer_prompt.md",
        LANGUAGE_INFER_PROMPT_TEMPLATE,
    );
    let prompt = render_prompt_template(
        &prompt_template,
        &[("__MEMORY_SNIPPETS__", &memory_context)],
    );
    log_prompt_render(&task.task_id, "language_infer_prompt", &prompt_file, None);
    let out = match run_llm_with_fallback_with_prompt_file(state, task, &prompt, &prompt_file).await
    {
        Ok(v) => v,
        Err(err) => {
            warn!(
                "infer_language_preference_from_memory_llm failed: task_id={} user_id={} chat_id={} err={}",
                task.task_id, task.user_id, task.chat_id, err
            );
            return None;
        }
    };
    let parsed = parse_language_from_llm_output(&out);
    info!(
        "infer_language_preference_from_memory_llm: task_id={} user_id={} chat_id={} memory_items={} parsed_language={} llm_out={}",
        task.task_id,
        task.user_id,
        task.chat_id,
        recalled.len(),
        parsed.as_deref().unwrap_or("unknown"),
        truncate_for_log(&out)
    );
    parsed
}

fn parse_language_from_llm_output(raw: &str) -> Option<String> {
    parse_llm_json_raw_or_any::<Value>(raw)
        .and_then(|v| {
            v.get("language")
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty() && s.to_ascii_lowercase() != "unknown")
}

pub(crate) fn extract_first_json_object_any(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i;
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;
            let mut j = i;
            while j < bytes.len() {
                let c = bytes[j];
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        in_string = false;
                    }
                } else if c == b'"' {
                    in_string = true;
                } else if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return Some(text[start..=j].to_string());
                    }
                }
                j += 1;
            }
            i = j;
        }
        i += 1;
    }
    None
}

fn selected_openai_api_key(state: &AppState) -> String {
    if let Some(p) = state
        .llm_providers
        .iter()
        .find(|p| p.config.provider_type == "openai_compat")
    {
        return p.config.api_key.clone();
    }
    String::new()
}

fn selected_openai_base_url(state: &AppState) -> String {
    if let Some(p) = state
        .llm_providers
        .iter()
        .find(|p| p.config.provider_type == "openai_compat")
    {
        return p.config.base_url.clone();
    }
    "https://api.openai.com/v1".to_string()
}

fn selected_openai_model(state: &AppState) -> String {
    state
        .llm_providers
        .iter()
        .find(|p| p.config.provider_type == "openai_compat")
        .map(|p| p.config.model.clone())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "gpt-4o-mini".to_string())
}

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

fn maybe_sanitize_llm_text_output(vendor: &str, raw: &str) -> (String, bool) {
    if vendor.eq_ignore_ascii_case("minimax") {
        let cleaned = sanitize_llm_text_output(raw);
        let sanitized = cleaned != raw.trim();
        return (cleaned, sanitized);
    }
    (raw.to_string(), false)
}

async fn run_llm_with_fallback_with_prompt_file(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    prompt_file: &str,
) -> Result<String, String> {
    let _prompt_debug_enabled = state.routing.debug_log_prompt;
    if state.llm_providers.is_empty() {
        return Err("No available LLM provider configured".to_string());
    }

    let mut last_error = "unknown llm error".to_string();

    for provider in &state.llm_providers {
        let vendor = llm_vendor_name(provider);
        let model = provider.config.model.as_str();
        let model_kind = llm_model_kind(provider);
        let provider_name = format!("{}:{}", provider.config.name, provider.config.model);
        info!(
            "{} [LLM_CALL] stage=request task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_file={}",
            highlight_tag("llm"),
            task.task_id,
            task.user_id,
            task.chat_id,
            vendor,
            model,
            model_kind,
            provider_name,
            prompt_file
        );

        match call_provider_with_retry(provider.clone(), prompt).await {
            Ok(text) => {
                let (cleaned_text, sanitized) = maybe_sanitize_llm_text_output(vendor, &text);
                if sanitized {
                    warn!(
                        "{} [LLM_CALL] stage=cleanup task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_file={} note=removed_think_block",
                        highlight_tag("llm"),
                        task.task_id,
                        task.user_id,
                        task.chat_id,
                        vendor,
                        model,
                        model_kind,
                        provider_name,
                        prompt_file
                    );
                }
                info!(
                    "{} [LLM_CALL] stage=response task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_file={} response={}",
                    highlight_tag("llm"),
                    task.task_id,
                    task.user_id,
                    task.chat_id,
                    vendor,
                    model,
                    model_kind,
                    provider_name,
                    prompt_file,
                    truncate_for_log(&cleaned_text)
                );
                append_model_io_log(
                    state,
                    task,
                    provider,
                    "ok",
                    prompt,
                    Some(&text),
                    Some(&cleaned_text),
                    sanitized,
                    None,
                );
                let _ = insert_audit_log(
                    state,
                    Some(task.user_id),
                    "run_llm",
                    Some(
                        &json!({
                            "task_id": task.task_id,
                            "chat_id": task.chat_id,
                            "vendor": vendor,
                            "provider": provider.config.name,
                            "model": provider.config.model,
                            "model_kind": model_kind,
                            "status": "ok"
                        })
                        .to_string(),
                    ),
                    None,
                );
                return Ok(cleaned_text);
            }
            Err(err) => {
                last_error = format!("provider={provider_name} failed: {err}");
                warn!(
                    "{} [LLM_CALL] stage=error task_id={} user_id={} chat_id={} vendor={} model={} model_kind={} provider={} prompt_file={} error={}",
                    highlight_tag("llm"),
                    task.task_id,
                    task.user_id,
                    task.chat_id,
                    vendor,
                    model,
                    model_kind,
                    provider_name,
                    prompt_file,
                    truncate_for_log(&last_error)
                );
                append_model_io_log(
                    state,
                    task,
                    provider,
                    "failed",
                    prompt,
                    None,
                    None,
                    false,
                    Some(&last_error),
                );
                let _ = insert_audit_log(
                    state,
                    Some(task.user_id),
                    "run_llm",
                    Some(
                        &json!({
                            "task_id": task.task_id,
                            "chat_id": task.chat_id,
                            "vendor": vendor,
                            "provider": provider.config.name,
                            "model": provider.config.model,
                            "model_kind": model_kind,
                            "status": "failed"
                        })
                        .to_string(),
                    ),
                    Some(&last_error),
                );
                warn!("{last_error}");
            }
        }
    }

    Err(last_error)
}

fn append_model_io_log(
    state: &AppState,
    task: &ClaimedTask,
    provider: &Arc<LlmProviderRuntime>,
    status: &str,
    prompt: &str,
    raw_response: Option<&str>,
    clean_response: Option<&str>,
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
        "prompt": truncate_for_log(prompt),
        "response": clean_response.map(truncate_for_log),
        "raw_response": raw_response.map(truncate_for_log),
        "clean_response": clean_response.map(truncate_for_log),
        "sanitized": sanitized,
        "error": error.map(truncate_for_log),
    })
    .to_string();

    if let Err(err) = writeln!(file, "{line}") {
        warn!("write model io log failed: {err}");
    }
}

pub(crate) fn truncate_for_log(text: &str) -> String {
    if text.len() <= MODEL_IO_LOG_MAX_CHARS {
        return text.to_string();
    }
    let mut out = utf8_safe_prefix(text, MODEL_IO_LOG_MAX_CHARS).to_string();
    out.push_str("...(truncated)");
    out
}

fn log_color_enabled() -> bool {
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

/// Returns [TAG] with optional ANSI color when logging to a TTY (see log_color_enabled).
/// When not a TTY or RUSTCLAW_LOG_COLOR=0, returns plain text so log files stay clean.
pub(crate) fn highlight_tag(kind: &str) -> String {
    let upper = kind.to_ascii_uppercase();
    if !log_color_enabled() {
        return format!("[{upper}]");
    }
    let code = match kind {
        "prompt" => "38;5;214",   // orange
        "skill" => "38;5;45",     // cyan
        "tool" => "38;5;39",      // blue
        "loop" => "38;5;141",     // purple
        "llm" => "38;5;226",      // yellow
        "skill_llm" => "38;5;49", // green
        "routing" => "38;5;198",  // magenta-pink
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
    let detail_line = detail
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let detail_trimmed = detail_line.trim();
    if detail_trimmed.is_empty() {
        subtask_results.push(format!("subtask#{index} {action_label}: {status}"));
    } else {
        subtask_results.push(format!(
            "subtask#{index} {action_label}: {status} | {}",
            truncate_for_agent_trace(detail_trimmed)
        ));
    }
}

fn parse_resume_context_error(error_text: &str) -> Option<(String, Value)> {
    let trimmed = error_text.trim();
    let payload = trimmed.strip_prefix(RESUME_CONTEXT_ERROR_PREFIX)?;
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

fn build_resume_continue_execute_prompt(
    state: &AppState,
    payload: &Value,
    fallback_user_text: &str,
) -> String {
    let user_text = payload
        .get("resume_user_text")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_user_text);
    let resume_context = payload
        .get("resume_context")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let resume_instruction = payload
        .get("resume_instruction")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let resume_steps = payload
        .get("resume_steps")
        .cloned()
        .filter(|v| v.as_array().map(|arr| !arr.is_empty()).unwrap_or(false))
        .unwrap_or_else(|| {
            resume_context
                .get("remaining_actions")
                .cloned()
                .filter(|v| v.as_array().map(|arr| !arr.is_empty()).unwrap_or(false))
                .unwrap_or_else(|| {
                    resume_context
                        .get("remaining_steps")
                        .cloned()
                        .unwrap_or_else(|| json!([]))
                })
        });
    let resume_context_json = serde_json::to_string_pretty(&resume_context)
        .unwrap_or_else(|_| resume_context.to_string());
    let resume_steps_json =
        serde_json::to_string_pretty(&resume_steps).unwrap_or_else(|_| resume_steps.to_string());

    let (prompt_template, _) = load_prompt_template_for_state(
        state,
        "prompts/resume_continue_execute_prompt.md",
        RESUME_CONTINUE_EXECUTE_PROMPT_TEMPLATE,
    );
    render_prompt_template(
        &prompt_template,
        &[
            ("__USER_TEXT__", user_text),
            ("__RESUME_CONTEXT__", &resume_context_json),
            ("__RESUME_STEPS__", &resume_steps_json),
            ("__RESUME_INSTRUCTION__", resume_instruction),
        ],
    )
}

fn build_resume_followup_discussion_prompt(
    state: &AppState,
    payload: &Value,
    fallback_user_text: &str,
) -> String {
    let user_text = payload
        .get("resume_user_text")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_user_text)
        .trim();
    let resume_context = payload
        .get("resume_context")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let resume_context_json = serde_json::to_string_pretty(&resume_context)
        .unwrap_or_else(|_| resume_context.to_string());
    let (prompt_template, _) = load_prompt_template_for_state(
        state,
        RESUME_FOLLOWUP_DISCUSSION_PROMPT_PATH,
        RESUME_FOLLOWUP_DISCUSSION_PROMPT_TEMPLATE,
    );
    render_prompt_template(
        &prompt_template,
        &[
            ("__USER_TEXT__", user_text),
            ("__RESUME_CONTEXT__", &resume_context_json),
        ],
    )
}

pub(crate) fn i18n_t_with_default(state: &AppState, key: &str, default_text: &str) -> String {
    state
        .schedule
        .i18n_dict
        .get(key)
        .cloned()
        .unwrap_or_else(|| default_text.to_string())
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
        "ts": now_ts_u64(),
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
    if text.len() <= AGENT_TRACE_LOG_MAX_CHARS {
        return text.to_string();
    }
    let mut out = utf8_safe_prefix(text, AGENT_TRACE_LOG_MAX_CHARS).to_string();
    out.push_str("...(truncated)");
    out
}

fn normalize_image_edit_args(args: Value, fallback_instruction: &str, image_path: &str) -> Value {
    let mut obj = args.as_object().cloned().unwrap_or_default();
    if !obj.contains_key("action") {
        obj.insert("action".to_string(), Value::String("edit".to_string()));
    }
    if !obj.contains_key("instruction") {
        obj.insert(
            "instruction".to_string(),
            Value::String(fallback_instruction.trim().to_string()),
        );
    }
    if !obj.contains_key("image") {
        obj.insert("image".to_string(), json!({"path": image_path}));
    }
    Value::Object(obj)
}

fn image_edit_args_has_image(obj: &serde_json::Map<String, Value>) -> bool {
    let image_obj_has_path = obj
        .get("image")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("path"))
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let image_str = obj
        .get("image")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let images_array_has_path = obj
        .get("images")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().any(|it| {
                it.as_object()
                    .and_then(|m| m.get("path"))
                    .and_then(|v| v.as_str())
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
                    || it.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false)
            })
        })
        .unwrap_or(false);
    image_obj_has_path || image_str || images_array_has_path
}

pub(crate) fn extract_delivery_file_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("FILE:") {
            out.push(format!("FILE:{}", rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("IMAGE_FILE:") {
            // For generated/edited images, enforce document/file delivery.
            out.push(format!("FILE:{}", rest.trim()));
        }
    }
    out
}

fn extract_file_path_from_delivery_token(token: &str) -> Option<String> {
    token
        .strip_prefix("FILE:")
        .or_else(|| token.strip_prefix("IMAGE_FILE:"))
        .map(|s| trim_path_token(s))
        .filter(|s| !s.is_empty())
}

fn trim_path_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '`' | '，' | ',' | ':' | '：' | ';' | '。' | ')' | '(' | '）' | '（'
            )
        })
        .to_string()
}

fn intercept_response_text_for_delivery(text: &str) -> String {
    text.trim().to_string()
}

fn intercept_response_payload_for_delivery(
    text: String,
    messages: Vec<String>,
) -> (String, Vec<String>) {
    let mut seen = HashSet::new();
    let normalized_messages = messages
        .into_iter()
        .map(|msg| intercept_response_text_for_delivery(&msg))
        .filter(|msg| !msg.is_empty())
        .filter(|msg| seen.insert(msg.clone()))
        .collect::<Vec<_>>();
    let normalized_text = intercept_response_text_for_delivery(&text);
    (normalized_text, normalized_messages)
}

fn is_image_file_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
        || lower.ends_with(".bmp")
}

async fn resolve_image_for_edit_from_context_llm(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
) -> Option<String> {
    let candidates = collect_recent_image_candidates(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        200,
    );
    if candidates.is_empty() {
        return None;
    }
    let (long_term_summary, preferences, recalled) = memory::service::recall_memory_context_parts(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        goal,
        state.memory.recall_limit.max(1),
        state.memory.image_memory_include_long_term,
        state.memory.image_memory_include_preferences,
    );
    let memory_text = memory::service::memory_context_block(
        long_term_summary.as_deref(),
        &preferences,
        &recalled,
        state.memory.image_memory_max_chars.max(384),
    );
    let candidate_lines = candidates
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{i}: {p}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (prompt_template, prompt_file) = load_prompt_template_for_state(
        state,
        "prompts/image_reference_resolver_prompt.md",
        IMAGE_REFERENCE_RESOLVER_PROMPT_TEMPLATE,
    );
    let prompt = render_prompt_template(
        &prompt_template,
        &[
            ("__MEMORY_TEXT__", &memory_text),
            ("__GOAL__", goal),
            ("__CANDIDATES__", &candidate_lines),
        ],
    );
    log_prompt_render(
        &task.task_id,
        "image_reference_resolver_prompt",
        &prompt_file,
        None,
    );
    let llm_out = run_llm_with_fallback_with_prompt_file(state, task, &prompt, &prompt_file)
        .await
        .ok()?;
    let idx = parse_image_reference_index_from_llm_output(&llm_out)?;
    if idx < 0 {
        return None;
    }
    let idx = idx as usize;
    let selected = candidates.get(idx).cloned();
    info!(
        "resolve_image_for_edit_from_context_llm: task_id={} selected_index={} selected_path={} llm_out={}",
        task.task_id,
        idx,
        selected.as_deref().unwrap_or("<none>"),
        truncate_for_log(&llm_out)
    );
    selected
}

fn parse_image_reference_index_from_llm_output(raw: &str) -> Option<i64> {
    parse_llm_json_raw_or_any::<Value>(raw)
        .and_then(|v| v.get("selected_index").and_then(|x| x.as_i64()))
}

fn collect_recent_image_candidates(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    limit: usize,
) -> Vec<String> {
    let Some(user_key) = user_key.map(str::trim).filter(|v| !v.is_empty()) else {
        return Vec::new();
    };
    let db = match state.db.lock() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    let mut mem_stmt = match db.prepare(
        "SELECT content
         FROM memories
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3 AND role = 'assistant'
         ORDER BY id DESC
         LIMIT 120",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    if let Ok(rows) = mem_stmt.query_map(params![user_id, chat_id, user_key], |row| {
        row.get::<_, String>(0)
    }) {
        for row in rows.flatten() {
            let tokens = extract_delivery_file_tokens(&row);
            for t in tokens {
                if let Some(path) = extract_file_path_from_delivery_token(&t) {
                    if is_image_file_path(&path) && seen.insert(path.clone()) {
                        out.push(path);
                    }
                }
            }
        }
    }

    let mut task_stmt = match db.prepare(
        "SELECT payload_json, result_json
         FROM tasks
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3 AND kind = 'run_skill' AND status = 'succeeded'
         ORDER BY rowid DESC
         LIMIT ?4",
    ) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = task_stmt
        .query_map(params![user_id, chat_id, user_key, limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
    {
        for row in rows.flatten() {
            let (payload_json, result_json) = row;
            if let Ok(payload) = serde_json::from_str::<Value>(&payload_json) {
                collect_image_paths_from_task_payload(&payload, &mut out, &mut seen);
            }
            if let Some(result) = result_json {
                if let Ok(v) = serde_json::from_str::<Value>(&result) {
                    if let Some(text) = v.get("text").and_then(|x| x.as_str()) {
                        for t in extract_delivery_file_tokens(text) {
                            if let Some(path) = extract_file_path_from_delivery_token(&t) {
                                if is_image_file_path(&path) && seen.insert(path.clone()) {
                                    out.push(path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn collect_image_paths_from_task_payload(
    payload: &Value,
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let skill = payload
        .get("skill_name")
        .and_then(|v| v.as_str())
        .map(canonical_skill_name)
        .unwrap_or_default();
    let args = payload.get("args").and_then(|v| v.as_object());
    if args.is_none() {
        return;
    }
    let args = args.unwrap();
    if skill == "image_vision" {
        if let Some(images) = args.get("images").and_then(|v| v.as_array()) {
            for item in images {
                let path = item
                    .as_object()
                    .and_then(|m| m.get("path"))
                    .and_then(|v| v.as_str())
                    .or_else(|| item.as_str());
                if let Some(path) = path {
                    let p = path.trim().to_string();
                    if is_image_file_path(&p) && seen.insert(p.clone()) {
                        out.push(p);
                    }
                }
            }
        }
    } else if skill == "image_edit" {
        let path = args
            .get("image")
            .and_then(|v| v.as_object())
            .and_then(|m| m.get("path"))
            .and_then(|v| v.as_str())
            .or_else(|| args.get("image").and_then(|v| v.as_str()));
        if let Some(path) = path {
            let p = path.trim().to_string();
            if is_image_file_path(&p) && seen.insert(p.clone()) {
                out.push(p);
            }
        }
    } else if skill == "image_generate" {
        if let Some(path) = args.get("output_path").and_then(|v| v.as_str()) {
            let p = path.trim().to_string();
            if is_image_file_path(&p) && seen.insert(p.clone()) {
                out.push(p);
            }
        }
    }
}

pub(crate) fn extract_json_object(text: &str) -> Option<String> {
    extract_agent_action_objects(text).into_iter().next()
}

fn is_agent_action_candidate(candidate: &str) -> bool {
    if let Ok(v) = serde_json::from_str::<Value>(candidate) {
        return v.get("type").is_some()
            || v.get("action").is_some()
            || v.get("tool").is_some()
            || v.get("skill").is_some();
    }
    candidate.contains("\"type\"")
        || candidate.contains("\"action\"")
        || candidate.contains("\"tool\"")
        || candidate.contains("\"skill\"")
}

pub(crate) fn extract_agent_action_objects(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i;
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;
            let mut j = i;

            while j < bytes.len() {
                let c = bytes[j];
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        in_string = false;
                    }
                } else {
                    if c == b'"' {
                        in_string = true;
                    } else if c == b'{' {
                        depth += 1;
                    } else if c == b'}' {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                        if depth == 0 {
                            let candidate = &text[start..=j];
                            // Prefer objects that look like an agent action payload.
                            if is_agent_action_candidate(candidate) {
                                out.push(candidate.to_string());
                            }
                        }
                    }
                }
                j += 1;
            }
            i = j;
        }
        i += 1;
    }
    out
}

pub(crate) fn parse_agent_action_json_with_repair(raw: &str) -> Result<Value, String> {
    let parsed = match serde_json::from_str::<Value>(raw) {
        Ok(v) => Ok(v),
        Err(first_err) => {
            let repaired = repair_invalid_json_escapes(raw);
            match serde_json::from_str::<Value>(&repaired) {
                Ok(v) => Ok(v),
                Err(second_err) => Err(format!(
                    "initial parse error: {first_err}; repaired parse error: {second_err}"
                )),
            }
        }
    }?;
    Ok(normalize_agent_action_shape(parsed))
}

fn repair_invalid_json_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    let mut in_string = false;
    let mut escaped = false;

    for ch in raw.chars() {
        if !in_string {
            if ch == '"' {
                in_string = true;
            }
            out.push(ch);
            continue;
        }

        if escaped {
            let valid = matches!(ch, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u');
            if !valid {
                // Convert invalid escape like \(... to \\(... so JSON stays valid.
                out.push('\\');
            }
            out.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' => {
                out.push(ch);
                escaped = true;
            }
            '"' => {
                out.push(ch);
                in_string = false;
            }
            _ => out.push(ch),
        }
    }

    out
}

fn normalize_agent_action_shape(value: Value) -> Value {
    let Some(obj) = value.as_object() else {
        return value;
    };
    let Some(raw_type) = obj.get("type").and_then(|v| v.as_str()) else {
        if let Some(skill) = obj.get("skill").and_then(|v| v.as_str()) {
            let normalized_skill = canonical_skill_name(skill.trim()).to_string();
            if is_builtin_skill_name(&normalized_skill) {
                let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
                return json!({
                    "type": "call_skill",
                    "skill": normalized_skill,
                    "args": args,
                });
            }
        }
        if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
            let normalized_tool = tool.trim().to_ascii_lowercase();
            let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
            return json!({
                "type": "call_skill",
                "skill": normalized_tool,
                "args": args,
            });
        }
        if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
            return json!({
                "type": "respond",
                "content": content,
            });
        }
        return value;
    };
    let step_type = raw_type.trim().to_ascii_lowercase();
    if matches!(
        step_type.as_str(),
        "think" | "call_tool" | "call_skill" | "respond"
    ) {
        if step_type == "call_tool" {
            if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
                let normalized_tool = tool.trim().to_ascii_lowercase();
                let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
                return json!({
                    "type": "call_skill",
                    "skill": normalized_tool,
                    "args": args,
                });
            }
        }
        return value;
    }

    let args = collect_bare_action_args(obj);
    if is_builtin_skill_name(&step_type) {
        return json!({
            "type": "call_skill",
            "skill": step_type,
            "args": args,
        });
    }

    let normalized_skill = canonical_skill_name(&step_type).to_string();
    if is_builtin_skill_name(&normalized_skill) {
        return json!({
            "type": "call_skill",
            "skill": normalized_skill,
            "args": args,
        });
    }

    value
}

fn collect_bare_action_args(obj: &serde_json::Map<String, Value>) -> Value {
    let mut args = obj
        .get("args")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    for (key, value) in obj {
        if matches!(key.as_str(), "type" | "args" | "tool" | "skill") {
            continue;
        }
        args.insert(key.clone(), value.clone());
    }
    Value::Object(args)
}

pub(crate) fn canonical_skill_name(name: &str) -> &str {
    match name {
        // file search
        "fs_rearch" | "fs-search" | "filesystem_search" | "file_search" | "search_files" => {
            "fs_search"
        }
        // package / install
        "package_install" | "pkg_manager" | "packages" => "package_manager",
        "module_install" | "install_modules" => "install_module",
        // system ops
        "process" | "process_manager" => "process_basic",
        "archive" | "archive_tool" => "archive_basic",
        "database" | "sqlite_tool" => "db_basic",
        "docker" | "docker_ops" => "docker_basic",
        "rss" | "rss_reader" | "rss_fetcher" => "rss_fetch",
        // image ops
        "image_vision_skill" | "vision" | "vision_image" | "image-analyze" => "image_vision",
        "image_generation" | "generate_image" | "draw_image" | "text_to_image" => "image_generate",
        "image_modify" | "image_editor" | "edit_image" | "image_outpaint" => "image_edit",
        "coin" | "coins" | "crypto_trade" | "market_data" | "crypto_market" => "crypto",
        "talk" | "smalltalk" | "joke" | "chitchat" => "chat",
        "git" => "git_basic",
        "http" => "http_basic",
        "system" => "system_basic",
        _ => name,
    }
}

/// Unified skill registry: base (builtin) + runner skills. All capabilities are skills.
pub(crate) fn is_builtin_skill_name(name: &str) -> bool {
    matches!(
        name,
        "run_cmd" | "read_file" | "write_file" | "list_dir" | "make_dir" | "remove_file"
            | "archive_basic"
            | "audio_synthesize"
            | "audio_transcribe"
            | "config_guard"
            | "crypto"
            | "chat"
            | "db_basic"
            | "docker_basic"
            | "fs_search"
            | "git_basic"
            | "health_check"
            | "http_basic"
            | "image_edit"
            | "image_generate"
            | "image_vision"
            | "install_module"
            | "log_analyze"
            | "package_manager"
            | "process_basic"
            | "rss_fetch"
            | "service_control"
            | "system_basic"
            | "x"
    )
}

fn ensure_default_output_dir_for_skill_args(
    workspace_root: &Path,
    skill_name: &str,
    args: Value,
) -> Value {
    let Some(mut obj) = args.as_object().cloned() else {
        return args;
    };
    match skill_name {
        "image_generate" | "image_edit" => {
            // Force a unified download directory for generated/edited images,
            // even if model/user provided a custom output_path in args.
            let section = if skill_name == "image_edit" {
                "image_edit"
            } else {
                "image_generation"
            };
            let dir = resolve_output_dir_from_config(workspace_root, section);
            let ts = now_ts_u64();
            let prefix = if skill_name == "image_edit" {
                "edit"
            } else {
                "gen"
            };
            let suggested = format!("{dir}/{prefix}-{ts}.png");
            obj.insert("output_path".to_string(), Value::String(suggested));
            Value::Object(obj)
        }
        _ => Value::Object(obj),
    }
}

pub(crate) fn ensure_default_file_path(workspace_root: &Path, input: &str) -> String {
    let default_dir = resolve_file_default_output_dir_from_config(workspace_root);
    let p = input.trim();
    if p.is_empty() {
        return format!("{default_dir}/untitled.txt");
    }
    if Path::new(p).is_absolute()
        || p.contains('/')
        || p.contains('\\')
        || p.starts_with("./")
        || p.starts_with("../")
    {
        return p.to_string();
    }
    format!("{default_dir}/{p}")
}

fn resolve_output_dir_from_config(workspace_root: &Path, section: &str) -> String {
    let cfg_path = workspace_root.join("configs/config.toml");
    let raw = match std::fs::read_to_string(cfg_path) {
        Ok(v) => v,
        Err(_) => return "document".to_string(),
    };
    let value: TomlValue = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return "document".to_string(),
    };
    value
        .get(section)
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("default_output_dir"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("document")
        .to_string()
}

fn resolve_file_default_output_dir_from_config(workspace_root: &Path) -> String {
    resolve_output_dir_from_config(workspace_root, "file_generation")
}

/// Base (builtin) skills: run_cmd, read_file, write_file, list_dir, make_dir, remove_file; executed in-process. Policy uses skill:* token.
async fn execute_builtin_skill(
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> Result<String, String> {
    let policy_token = format!("skill:{skill_name}");
    if !state
        .tools_policy
        .is_allowed(&policy_token, state.active_provider_type.as_deref())
    {
        return Err(format!("blocked by policy: {policy_token}"));
    }

    let map = ensure_args_object(args)?;

    match skill_name {
        "read_file" => {
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            let bytes =
                std::fs::read(&real_path).map_err(|err| format!("read file failed: {err}"))?;
            let clip = if bytes.len() > MAX_READ_FILE_BYTES {
                &bytes[..MAX_READ_FILE_BYTES]
            } else {
                &bytes
            };
            Ok(String::from_utf8_lossy(clip).to_string())
        }
        "write_file" => {
            ensure_only_keys(map, &["path", "content"])?;
            let path = required_string(map, "path")?;
            let content = required_string(map, "content")?;
            if content.len() > MAX_WRITE_FILE_BYTES {
                return Err(format!("content too large: {} bytes", content.len()));
            }
            let effective_path = ensure_default_file_path(&state.workspace_root, path);
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                &effective_path,
                state.allow_path_outside_workspace,
            )?;
            if let Some(parent) = real_path.parent() {
                std::fs::create_dir_all(parent).map_err(|err| format!("mkdir failed: {err}"))?;
            }
            std::fs::write(&real_path, content)
                .map_err(|err| format!("write file failed: {err}"))?;
            Ok(format!(
                "written {} bytes to {}",
                content.len(),
                real_path.display()
            ))
        }
        "list_dir" => {
            ensure_only_keys(map, &["path"])?;
            let path = optional_string(map, "path").unwrap_or(".");
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            let mut items = Vec::new();
            for entry in
                std::fs::read_dir(&real_path).map_err(|err| format!("read_dir failed: {err}"))?
            {
                let e = entry.map_err(|err| format!("dir entry failed: {err}"))?;
                let name = e.file_name();
                let mut label = name.to_string_lossy().to_string();
                if e.path().is_dir() {
                    label.push('/');
                }
                items.push(label);
                if items.len() >= 200 {
                    break;
                }
            }
            items.sort();
            Ok(items.join("\n"))
        }
        "run_cmd" => {
            ensure_only_keys(map, &["command", "cwd"])?;
            let command = required_string(map, "command")?;
            let sanitized_command = sanitize_command_before_execute(&state.command_intent, command);
            if sanitized_command.is_empty() {
                return Err("empty command after sanitize".to_string());
            }
            if sanitized_command != command.trim() {
                info!(
                    "run_cmd sanitized command: before={} after={}",
                    truncate_for_log(command),
                    truncate_for_log(&sanitized_command)
                );
            }
            let cwd = optional_string(map, "cwd").unwrap_or(".");
            let cwd_path = resolve_workspace_path(
                &state.workspace_root,
                cwd,
                state.allow_path_outside_workspace,
            )?;
            run_safe_command(
                &cwd_path,
                &sanitized_command,
                state.max_cmd_length,
                state.cmd_timeout_seconds,
                state.allow_sudo,
            )
            .await
        }
        "make_dir" => {
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            std::fs::create_dir_all(&real_path)
                .map_err(|err| format!("create_dir failed: {err}"))?;
            Ok(format!("created directory {}", real_path.display()))
        }
        "remove_file" => {
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            if real_path.is_dir() {
                return Err("remove_file only supports files; use run_cmd for directory removal".to_string());
            }
            std::fs::remove_file(&real_path)
                .map_err(|err| format!("remove_file failed: {err}"))?;
            Ok(format!("removed {}", real_path.display()))
        }
        _ => Err(format!("unknown skill: {skill_name}")),
    }
}

fn ensure_args_object(args: &Value) -> Result<&serde_json::Map<String, Value>, String> {
    args.as_object()
        .ok_or_else(|| "skill args must be a JSON object".to_string())
}

fn ensure_only_keys(map: &serde_json::Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    for k in map.keys() {
        if !allowed.iter().any(|x| x == k) {
            return Err(format!("unexpected arg key: {k}"));
        }
    }
    Ok(())
}

fn required_string<'a>(
    map: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    map.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{key} must be string"))
}

fn optional_string<'a>(map: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(|v| v.as_str())
}

fn resolve_workspace_path(
    workspace_root: &Path,
    input: &str,
    allow_path_outside_workspace: bool,
) -> Result<PathBuf, String> {
    let base = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        workspace_root.join(input)
    };

    if allow_path_outside_workspace {
        return Ok(base);
    }

    if base.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("path with '..' is not allowed".to_string());
    }

    if !base.starts_with(workspace_root) {
        return Err("path is outside workspace".to_string());
    }

    Ok(base)
}

async fn run_safe_command(
    cwd: &Path,
    command: &str,
    max_cmd_length: usize,
    cmd_timeout_seconds: u64,
    allow_sudo: bool,
) -> Result<String, String> {
    if command.len() > max_cmd_length {
        return Err("command too long".to_string());
    }

    if command.trim().is_empty() {
        return Err("empty command".to_string());
    }

    if !allow_sudo && command.split_whitespace().any(|p| p == "sudo") {
        return Err("sudo is not allowed by tools config".to_string());
    }

    let mut cmd = Command::new("bash");
    cmd.arg("-lc").arg(command);
    cmd.current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let soft_timeout = cmd_timeout_seconds.max(1);
    let mut output_fut = Box::pin(cmd.output());

    let out = match tokio::time::timeout(Duration::from_secs(soft_timeout), &mut output_fut).await {
        Ok(r) => r.map_err(|err| format!("run command failed: {err}"))?,
        Err(_) => {
            info!(
                "run_cmd soft-timeout reached; command still running (soft={}s): {}",
                soft_timeout,
                truncate_for_log(command)
            );
            output_fut
                .await
                .map_err(|err| format!("run command failed: {err}"))?
        }
    };

    let stdout_text = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr_text = String::from_utf8_lossy(&out.stderr).to_string();

    let mut text = String::new();
    text.push_str(&stdout_text);
    if !stderr_text.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&stderr_text);
    }

    if text.len() > 8000 {
        text.truncate(8000);
    }

    let exit_code = out.status.code().unwrap_or(-1);
    if exit_code == 0 {
        if text.trim().is_empty() {
            Ok(format!("exit=0 command={}", command.trim()))
        } else {
            Ok(text)
        }
    } else if text.trim().is_empty() {
        Err(format!("Command failed with exit code {}", exit_code))
    } else {
        let mut detail = String::new();
        if !stderr_text.trim().is_empty() {
            detail.push_str("stderr:\n");
            detail.push_str(stderr_text.trim());
        }
        if !stdout_text.trim().is_empty() {
            if !detail.is_empty() {
                detail.push_str("\n\n");
            }
            detail.push_str("stdout:\n");
            detail.push_str(stdout_text.trim());
        }
        if detail.len() > 8000 {
            detail.truncate(8000);
        }
        Err(format!(
            "Command failed with exit code {}\n{}",
            exit_code, detail
        ))
    }
}

async fn call_provider_with_retry(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<String, String> {
    let mut attempts = 0usize;

    loop {
        attempts += 1;
        match call_provider(provider.clone(), prompt).await {
            Ok(text) => return Ok(text),
            Err(ProviderError::Retryable(err)) => {
                if attempts > LLM_RETRY_TIMES {
                    return Err(err);
                }
                tokio::time::sleep(Duration::from_millis(250 * attempts as u64)).await;
            }
            Err(ProviderError::NonRetryable(err)) => return Err(err),
        }
    }
}

enum ProviderError {
    Retryable(String),
    NonRetryable(String),
}

async fn call_provider(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<String, ProviderError> {
    match provider.config.provider_type.as_str() {
        "openai_compat" => call_openai_compat(provider, prompt).await,
        "google_gemini" => call_google_gemini(provider, prompt).await,
        "anthropic_claude" => call_anthropic_claude(provider, prompt).await,
        other => Err(ProviderError::NonRetryable(format!(
            "unsupported provider type: {other}"
        ))),
    }
}

async fn call_openai_compat(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<String, ProviderError> {
    let _permit = provider
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| ProviderError::NonRetryable(format!("semaphore closed: {err}")))?;

    let url = format!(
        "{}/chat/completions",
        provider.config.base_url.trim_end_matches('/')
    );

    let req_body = json!({
        "model": provider.config.model,
        "messages": [
            { "role": "user", "content": prompt }
        ],
        "stream": false
    });

    let resp = provider
        .client
        .post(url)
        .bearer_auth(&provider.config.api_key)
        .json(&req_body)
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                ProviderError::Retryable(format!("timeout: {err}"))
            } else {
                ProviderError::Retryable(format!("request failed: {err}"))
            }
        })?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|err| ProviderError::Retryable(format!("read response failed: {err}")))?;

    if status.as_u16() == 429 || status.is_server_error() {
        return Err(ProviderError::Retryable(format!(
            "http {}: {}",
            status.as_u16(),
            body_text
        )));
    }

    if !status.is_success() {
        return Err(ProviderError::NonRetryable(format!(
            "http {}: {}",
            status.as_u16(),
            body_text
        )));
    }

    let value: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|err| ProviderError::NonRetryable(format!("parse response failed: {err}")))?;

    if let Some(reason) = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("finish_reason"))
        .and_then(|v| v.as_str())
    {
        if reason == "length" {
            warn!(
                "openai_compat response truncated: finish_reason=length model={}",
                provider.config.model
            );
        }
    }

    let text = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("message"))
        .and_then(|msg| msg.get("content"))
        .and_then(|content| content.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ProviderError::NonRetryable("missing choices[0].message.content".to_string())
        })?;

    Ok(text)
}

async fn call_google_gemini(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<String, ProviderError> {
    let _permit = provider
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| ProviderError::NonRetryable(format!("semaphore closed: {err}")))?;

    let url = format!(
        "{}/models/{}:generateContent?key={}",
        provider.config.base_url.trim_end_matches('/'),
        provider.config.model,
        provider.config.api_key
    );

    let req_body = json!({
        "contents": [{
            "parts": [{ "text": prompt }]
        }]
    });

    let resp = provider
        .client
        .post(url)
        .json(&req_body)
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                ProviderError::Retryable(format!("timeout: {err}"))
            } else {
                ProviderError::Retryable(format!("request failed: {err}"))
            }
        })?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|err| ProviderError::Retryable(format!("read response failed: {err}")))?;

    if status.as_u16() == 429 || status.is_server_error() {
        return Err(ProviderError::Retryable(format!(
            "http {}: {}",
            status.as_u16(),
            body_text
        )));
    }

    if !status.is_success() {
        return Err(ProviderError::NonRetryable(format!(
            "http {}: {}",
            status.as_u16(),
            body_text
        )));
    }

    let value: Value = serde_json::from_str(&body_text)
        .map_err(|err| ProviderError::NonRetryable(format!("parse response failed: {err}")))?;

    if let Some(block_reason) = value
        .get("promptFeedback")
        .and_then(|v| v.get("blockReason"))
        .and_then(|v| v.as_str())
    {
        return Err(ProviderError::NonRetryable(format!(
            "gemini prompt blocked: blockReason={block_reason}"
        )));
    }

    if let Some(finish_reason) = value
        .get("candidates")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("finishReason"))
        .and_then(|v| v.as_str())
    {
        match finish_reason {
            "MAX_TOKENS" => {
                warn!(
                    "gemini response truncated: finishReason=MAX_TOKENS model={}",
                    provider.config.model
                );
            }
            "SAFETY" => {
                return Err(ProviderError::NonRetryable(format!(
                    "gemini response blocked by safety filter: finishReason=SAFETY model={}",
                    provider.config.model
                )));
            }
            "RECITATION" => {
                return Err(ProviderError::NonRetryable(format!(
                    "gemini response blocked: finishReason=RECITATION model={}",
                    provider.config.model
                )));
            }
            _ => {}
        }
    }

    let text = value
        .get("candidates")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|v| v.as_array())
        .and_then(|parts| {
            let mut merged = String::new();
            for p in parts {
                if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                    merged.push_str(t);
                }
            }
            if merged.is_empty() {
                None
            } else {
                Some(merged)
            }
        })
        .ok_or_else(|| {
            ProviderError::NonRetryable("missing candidates[0].content.parts[*].text".to_string())
        })?;

    Ok(text)
}

async fn call_anthropic_claude(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<String, ProviderError> {
    let _permit = provider
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| ProviderError::NonRetryable(format!("semaphore closed: {err}")))?;

    let url = format!(
        "{}/messages",
        provider.config.base_url.trim_end_matches('/')
    );
    let req_body = json!({
        "model": provider.config.model,
        "max_tokens": 4096,
        "messages": [
            { "role": "user", "content": prompt }
        ]
    });

    let resp = provider
        .client
        .post(url)
        .header("x-api-key", &provider.config.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&req_body)
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                ProviderError::Retryable(format!("timeout: {err}"))
            } else {
                ProviderError::Retryable(format!("request failed: {err}"))
            }
        })?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|err| ProviderError::Retryable(format!("read response failed: {err}")))?;

    if status.as_u16() == 429 || status.is_server_error() {
        return Err(ProviderError::Retryable(format!(
            "http {}: {}",
            status.as_u16(),
            body_text
        )));
    }

    if !status.is_success() {
        return Err(ProviderError::NonRetryable(format!(
            "http {}: {}",
            status.as_u16(),
            body_text
        )));
    }

    let value: Value = serde_json::from_str(&body_text)
        .map_err(|err| ProviderError::NonRetryable(format!("parse response failed: {err}")))?;

    if let Some(stop_reason) = value.get("stop_reason").and_then(|v| v.as_str()) {
        if stop_reason == "max_tokens" {
            warn!(
                "anthropic response truncated: stop_reason=max_tokens model={}",
                provider.config.model
            );
        }
    }

    let text = value
        .get("content")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            let mut merged = String::new();
            for item in arr {
                if item.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                        merged.push_str(t);
                    }
                }
            }
            if merged.is_empty() {
                None
            } else {
                Some(merged)
            }
        })
        .ok_or_else(|| ProviderError::NonRetryable("missing content[*].text".to_string()))?;

    Ok(text)
}

fn claim_next_task(state: &AppState) -> anyhow::Result<Option<ClaimedTask>> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

    let mut stmt = db.prepare(
        "SELECT task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, kind, payload_json
         FROM tasks
         WHERE status = 'queued'
         ORDER BY created_at ASC
         LIMIT 1",
    )?;

    let candidate = stmt
        .query_row([], |row| {
            Ok(ClaimedTask {
                task_id: row.get(0)?,
                user_id: row.get(1)?,
                chat_id: row.get(2)?,
                user_key: row.get(3)?,
                channel: row.get(4)?,
                external_user_id: row.get(5)?,
                external_chat_id: row.get(6)?,
                kind: row.get(7)?,
                payload_json: row.get(8)?,
            })
        })
        .optional()?;

    let Some(task) = candidate else {
        return Ok(None);
    };

    let changed = db.execute(
        "UPDATE tasks SET status = 'running', updated_at = ?2 WHERE task_id = ?1 AND status = 'queued'",
        params![task.task_id, now_ts()],
    )?;

    if changed == 0 {
        debug!(
            "claim_next_task: race lost for task_id={}, another worker took it",
            task.task_id
        );
        return Ok(None);
    }

    debug!(
        "claim_next_task: claimed task_id={} user_id={} chat_id={} kind={}",
        task.task_id, task.user_id, task.chat_id, task.kind
    );
    Ok(Some(task))
}

fn update_task_success(state: &AppState, task_id: &str, result_json: &str) -> anyhow::Result<()> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "UPDATE tasks SET status = 'succeeded', result_json = ?2, error_text = NULL, updated_at = ?3 WHERE task_id = ?1",
        params![task_id, result_json, now_ts()],
    )?;
    Ok(())
}

fn update_task_progress_result(
    state: &AppState,
    task_id: &str,
    result_json: &str,
) -> anyhow::Result<()> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "UPDATE tasks SET result_json = ?2, updated_at = ?3 WHERE task_id = ?1 AND status IN ('queued','running')",
        params![task_id, result_json, now_ts()],
    )?;
    Ok(())
}

fn update_task_failure_with_result(
    state: &AppState,
    task_id: &str,
    result_json: &str,
    error_text: &str,
) -> anyhow::Result<()> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "UPDATE tasks SET status = 'failed', result_json = ?2, error_text = ?3, updated_at = ?4 WHERE task_id = ?1",
        params![task_id, result_json, error_text, now_ts()],
    )?;
    Ok(())
}

fn update_task_failure(state: &AppState, task_id: &str, error_text: &str) -> anyhow::Result<()> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "UPDATE tasks SET status = 'failed', result_json = NULL, error_text = ?2, updated_at = ?3 WHERE task_id = ?1",
        params![task_id, error_text, now_ts()],
    )?;
    Ok(())
}

fn update_task_timeout(state: &AppState, task_id: &str, error_text: &str) -> anyhow::Result<()> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "UPDATE tasks SET status = 'timeout', result_json = NULL, error_text = ?2, updated_at = ?3 WHERE task_id = ?1",
        params![task_id, error_text, now_ts()],
    )?;
    Ok(())
}

fn insert_audit_log(
    state: &AppState,
    user_id: Option<i64>,
    action: &str,
    detail_json: Option<&str>,
    error_text: Option<&str>,
) -> anyhow::Result<()> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    insert_audit_log_raw(&db, user_id, action, detail_json, error_text)
}

fn insert_audit_log_raw(
    db: &Connection,
    user_id: Option<i64>,
    action: &str,
    detail_json: Option<&str>,
    error_text: Option<&str>,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO audit_logs (ts, user_id, action, detail_json, error_text) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![now_ts(), user_id, action, detail_json, error_text],
    )?;

    Ok(())
}

fn insert_memory(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
    channel: &str,
    external_chat_id: Option<&str>,
    role: &str,
    content: &str,
    max_chars: usize,
) -> anyhow::Result<()> {
    memory::insert_memory(
        state,
        user_id,
        chat_id,
        user_key,
        channel,
        external_chat_id,
        role,
        content,
        max_chars,
    )
}

fn recall_recent_memories(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    limit: usize,
) -> anyhow::Result<Vec<(String, String)>> {
    memory::recall_recent_memories(state, user_key, user_id, chat_id, limit)
}

fn filter_memories_for_prompt_recall(
    memories: Vec<(String, String)>,
    prefer_llm_assistant_memory: bool,
) -> Vec<(String, String)> {
    memory::filter_memories_for_prompt_recall(memories, prefer_llm_assistant_memory)
}

fn select_relevant_memories_for_prompt(
    memories: Vec<(String, String)>,
    prompt: &str,
    min_score: f32,
) -> Vec<(String, String)> {
    memory::select_relevant_memories_for_prompt(memories, prompt, min_score)
}

fn recall_user_preferences(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    limit: usize,
) -> anyhow::Result<Vec<(String, String)>> {
    memory::recall_user_preferences(state, user_key, user_id, chat_id, limit)
}

fn recall_long_term_summary(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
) -> anyhow::Result<Option<String>> {
    memory::recall_long_term_summary(state, user_key, user_id, chat_id)
}

fn recall_memories_since_id(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    source_memory_id: i64,
    limit: usize,
) -> anyhow::Result<Vec<(i64, String, String, String)>> {
    memory::recall_memories_since_id(state, user_key, user_id, chat_id, source_memory_id, limit)
}

fn read_long_term_source_memory_id(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
) -> anyhow::Result<i64> {
    memory::read_long_term_source_memory_id(state, user_key, user_id, chat_id)
}

fn upsert_long_term_summary(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
    summary: &str,
    source_memory_id: i64,
) -> anyhow::Result<()> {
    memory::upsert_long_term_summary(state, user_id, chat_id, user_key, summary, source_memory_id)
}

async fn maybe_refresh_long_term_summary(
    state: &AppState,
    task: &ClaimedTask,
) -> Result<(), String> {
    if !state.memory.long_term_enabled {
        return Ok(());
    }
    if task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_none()
    {
        return Ok(());
    }
    let rounds = memory::count_chat_memory_rounds(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
    )
    .map_err(|err| format!("count memory rounds failed: {err}"))?;
    if rounds == 0 || rounds % state.memory.long_term_every_rounds.max(1) != 0 {
        return Ok(());
    }
    let source_id = read_long_term_source_memory_id(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
    )
    .map_err(|err| format!("read long-term source id failed: {err}"))?;
    let fetch_limit = state.memory.long_term_source_rounds.max(1) * 2;
    let entries = recall_memories_since_id(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        source_id,
        fetch_limit,
    )
    .map_err(|err| format!("read memories for summary failed: {err}"))?;
    let min_entries = state.memory.long_term_every_rounds.max(1) * 2;
    if entries.len() < min_entries {
        return Ok(());
    }
    let new_chars = entries
        .iter()
        .map(|(_, _, content, _)| content.trim().chars().count())
        .sum::<usize>();
    if new_chars < state.memory.long_term_refresh_min_new_chars.max(1) {
        return Ok(());
    }
    if memory::repeated_entries_ratio(&entries) > state.memory.long_term_refresh_max_repeat_ratio {
        return Ok(());
    }

    let latest_id = entries.last().map(|e| e.0).unwrap_or(source_id);
    if latest_id <= source_id {
        return Ok(());
    }

    let previous_summary =
        recall_long_term_summary(state, task.user_key.as_deref(), task.user_id, task.chat_id)
            .map_err(|err| format!("read previous long-term summary failed: {err}"))?
            .unwrap_or_default();

    let mut convo_lines = Vec::new();
    for (_, role, content, safety_flag) in &entries {
        if state.memory.safety_filter_enabled && safety_flag == "injection_like" {
            convo_lines.push(format!("{role}: [safety_signal content omitted]"));
            continue;
        }
        convo_lines.push(format!("{role}: {content}"));
    }
    if convo_lines.is_empty() {
        return Ok(());
    }
    let (summary_template, summary_prompt_file) = load_prompt_template_for_state(
        state,
        "prompts/long_term_summary_prompt.md",
        LONG_TERM_SUMMARY_PROMPT_TEMPLATE,
    );
    let summary_prompt = render_prompt_template(
        &summary_template,
        &[
            ("__PREVIOUS_SUMMARY__", &previous_summary),
            (
                "__NEW_CONVERSATION_CHUNK__",
                &convo_lines.join(
                    "
",
                ),
            ),
        ],
    );
    log_prompt_render(
        &task.task_id,
        "long_term_summary_prompt",
        &summary_prompt_file,
        None,
    );

    let summary =
        run_llm_with_fallback_with_prompt_file(state, task, &summary_prompt, &summary_prompt_file)
            .await?;
    let trimmed = truncate_text(&summary, state.memory.long_term_summary_max_chars.max(512));
    upsert_long_term_summary(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        &trimmed,
        latest_id,
    )
    .map_err(|err| format!("write long-term summary failed: {err}"))?;
    Ok(())
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let mut out = utf8_safe_prefix(text, max_chars).to_string();
    out.push_str("...(truncated)");
    out
}

fn utf8_safe_prefix(text: &str, max_len: usize) -> &str {
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

fn build_prompt_with_memory(
    prompt: &str,
    long_term_summary: Option<&str>,
    preferences: &[(String, String)],
    memories: &[(String, String)],
    max_chars: usize,
) -> String {
    memory::build_prompt_with_memory(prompt, long_term_summary, preferences, memories, max_chars)
}

fn memory_context_block(
    long_term_summary: Option<&str>,
    preferences: &[(String, String)],
    memories: &[(String, String)],
    max_chars: usize,
) -> String {
    memory::build_memory_context_block(long_term_summary, preferences, memories, max_chars)
}

fn preferred_response_language(preferences: &[(String, String)]) -> Option<String> {
    for (k, v) in preferences.iter().rev() {
        if k.trim() == "response_language" || k.trim() == "language" {
            let lang = v.trim();
            if !lang.is_empty() {
                return Some(lang.to_string());
            }
        }
    }
    None
}

fn recall_memory_context_parts(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    anchor_prompt: &str,
    recent_limit: usize,
    include_long_term: bool,
    include_preferences: bool,
) -> (Option<String>, Vec<(String, String)>, Vec<(String, String)>) {
    let long_term_summary = if include_long_term && state.memory.long_term_enabled {
        recall_long_term_summary(state, user_key, user_id, chat_id)
            .unwrap_or(None)
            .map(|s| truncate_text(&s, state.memory.long_term_recall_max_chars.max(256)))
    } else {
        None
    };
    let preferences = if include_preferences {
        recall_user_preferences(
            state,
            user_key,
            user_id,
            chat_id,
            state.memory.preference_recall_limit.max(1),
        )
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let recalled = recall_recent_memories(state, user_key, user_id, chat_id, recent_limit.max(1))
        .unwrap_or_default();
    let recalled =
        filter_memories_for_prompt_recall(recalled, state.memory.prefer_llm_assistant_memory);
    let recalled = if state.memory.recent_relevance_enabled {
        select_relevant_memories_for_prompt(
            recalled,
            anchor_prompt,
            state.memory.recent_relevance_min_score.clamp(0.0, 1.0),
        )
    } else {
        recalled
    };
    (long_term_summary, preferences, recalled)
}

fn init_db(config: &AppConfig) -> anyhow::Result<Connection> {
    if let Some(parent) = Path::new(&config.database.sqlite_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let db = Connection::open(&config.database.sqlite_path)?;
    db.busy_timeout(Duration::from_millis(config.database.busy_timeout_ms))?;
    db.execute_batch(INIT_SQL)?;
    Ok(db)
}

fn seed_users(db: &Connection, config: &AppConfig) -> anyhow::Result<()> {
    let now = now_ts();

    for admin_id in &config.telegram.admins {
        db.execute(
            "INSERT INTO users (user_id, role, is_allowed, created_at, last_seen)
             VALUES (?1, 'admin', 1, ?2, ?2)
             ON CONFLICT(user_id) DO UPDATE SET role='admin', is_allowed=1, last_seen=excluded.last_seen",
            params![admin_id, now],
        )?;
    }

    for user_id in &config.telegram.allowlist {
        db.execute(
            "INSERT INTO users (user_id, role, is_allowed, created_at, last_seen)
             VALUES (?1, 'user', 1, ?2, ?2)
             ON CONFLICT(user_id) DO UPDATE SET is_allowed=1, last_seen=excluded.last_seen",
            params![user_id, now],
        )?;
    }

    Ok(())
}

fn ensure_schedule_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS scheduled_jobs (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id            TEXT NOT NULL UNIQUE,
            user_id           INTEGER NOT NULL,
            chat_id           INTEGER NOT NULL,
            channel           TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui')),
            external_user_id  TEXT,
            external_chat_id  TEXT,
            schedule_type     TEXT NOT NULL CHECK (schedule_type IN ('once', 'daily', 'weekly', 'interval', 'cron')),
            run_at            INTEGER,
            time_of_day       TEXT,
            weekday           INTEGER,
            every_minutes     INTEGER,
            cron_expr         TEXT,
            timezone          TEXT NOT NULL,
            task_kind         TEXT NOT NULL CHECK (task_kind IN ('ask', 'run_skill')),
            task_payload_json TEXT NOT NULL,
            enabled           INTEGER NOT NULL DEFAULT 1,
            notify_on_success INTEGER NOT NULL DEFAULT 1,
            notify_on_failure INTEGER NOT NULL DEFAULT 1,
            last_run_at       TEXT,
            next_run_at       INTEGER,
            created_at        TEXT NOT NULL,
            updated_at        TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_due ON scheduled_jobs(enabled, next_run_at);
        CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_user_chat ON scheduled_jobs(user_id, chat_id);",
    )?;
    Ok(())
}

fn ensure_memory_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(MEMORY_UPGRADE_SQL)?;
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_user_chat_role_id
         ON memories(user_id, chat_id, role, id DESC);",
    )?;
    ensure_column_exists(
        db,
        "memories",
        "memory_type",
        "ALTER TABLE memories ADD COLUMN memory_type TEXT NOT NULL DEFAULT 'generic'",
    )?;
    ensure_column_exists(
        db,
        "memories",
        "salience",
        "ALTER TABLE memories ADD COLUMN salience REAL NOT NULL DEFAULT 0.5",
    )?;
    ensure_column_exists(
        db,
        "memories",
        "created_at_ts",
        "ALTER TABLE memories ADD COLUMN created_at_ts INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column_exists(
        db,
        "user_preferences",
        "updated_at_ts",
        "ALTER TABLE user_preferences ADD COLUMN updated_at_ts INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column_exists(
        db,
        "long_term_memories",
        "created_at_ts",
        "ALTER TABLE long_term_memories ADD COLUMN created_at_ts INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column_exists(
        db,
        "long_term_memories",
        "updated_at_ts",
        "ALTER TABLE long_term_memories ADD COLUMN updated_at_ts INTEGER NOT NULL DEFAULT 0",
    )?;
    db.execute(
        "UPDATE memories
         SET created_at_ts = CAST(created_at AS INTEGER)
         WHERE created_at_ts = 0 AND created_at GLOB '[0-9]*'",
        [],
    )?;
    db.execute(
        "UPDATE user_preferences
         SET updated_at_ts = CAST(updated_at AS INTEGER)
         WHERE updated_at_ts = 0 AND updated_at GLOB '[0-9]*'",
        [],
    )?;
    db.execute(
        "UPDATE long_term_memories
         SET created_at_ts = CAST(created_at AS INTEGER)
         WHERE created_at_ts = 0 AND created_at GLOB '[0-9]*'",
        [],
    )?;
    db.execute(
        "UPDATE long_term_memories
         SET updated_at_ts = CAST(updated_at AS INTEGER)
         WHERE updated_at_ts = 0 AND updated_at GLOB '[0-9]*'",
        [],
    )?;
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_user_chat_created_at_ts
         ON memories(user_id, chat_id, created_at_ts);
         CREATE INDEX IF NOT EXISTS idx_user_preferences_user_chat_updated_ts
         ON user_preferences(user_id, chat_id, updated_at_ts);
         CREATE INDEX IF NOT EXISTS idx_long_term_memories_updated_at_ts
         ON long_term_memories(updated_at_ts);",
    )?;
    ensure_column_exists(
        db,
        "memories",
        "is_instructional",
        "ALTER TABLE memories ADD COLUMN is_instructional INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column_exists(
        db,
        "memories",
        "safety_flag",
        "ALTER TABLE memories ADD COLUMN safety_flag TEXT NOT NULL DEFAULT 'normal'",
    )?;
    Ok(())
}

fn ensure_channel_schema(db: &Connection) -> anyhow::Result<()> {
    if let Err(err) = db.execute_batch(CHANNEL_UPGRADE_SQL) {
        debug!("channel schema batch skipped: {}", err);
    }
    ensure_column_exists(
        db,
        "tasks",
        "channel",
        "ALTER TABLE tasks ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui'))",
    )?;
    ensure_column_exists(
        db,
        "tasks",
        "external_user_id",
        "ALTER TABLE tasks ADD COLUMN external_user_id TEXT",
    )?;
    ensure_column_exists(
        db,
        "tasks",
        "external_chat_id",
        "ALTER TABLE tasks ADD COLUMN external_chat_id TEXT",
    )?;

    ensure_column_exists(
        db,
        "scheduled_jobs",
        "channel",
        "ALTER TABLE scheduled_jobs ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui'))",
    )?;
    ensure_column_exists(
        db,
        "scheduled_jobs",
        "external_user_id",
        "ALTER TABLE scheduled_jobs ADD COLUMN external_user_id TEXT",
    )?;
    ensure_column_exists(
        db,
        "scheduled_jobs",
        "external_chat_id",
        "ALTER TABLE scheduled_jobs ADD COLUMN external_chat_id TEXT",
    )?;

    ensure_column_exists(
        db,
        "memories",
        "channel",
        "ALTER TABLE memories ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui'))",
    )?;
    ensure_column_exists(
        db,
        "memories",
        "external_chat_id",
        "ALTER TABLE memories ADD COLUMN external_chat_id TEXT",
    )?;
    Ok(())
}

fn rebuild_channel_tables_for_ui(db: &Connection) -> anyhow::Result<()> {
    let tasks_sql: String = db.query_row(
        "SELECT COALESCE(sql, '') FROM sqlite_master WHERE type = 'table' AND name = 'tasks'",
        [],
        |row| row.get(0),
    )?;
    if tasks_sql.contains("'ui'") {
        return Ok(());
    }
    db.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE tasks RENAME TO tasks_old;
         CREATE TABLE tasks (
             task_id       TEXT PRIMARY KEY,
             user_id       INTEGER NOT NULL,
             chat_id       INTEGER NOT NULL,
             channel       TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui')),
             external_user_id TEXT,
             external_chat_id TEXT,
             message_id    INTEGER,
             user_key      TEXT,
             kind          TEXT NOT NULL CHECK (kind IN ('ask', 'run_skill', 'admin')),
             payload_json  TEXT NOT NULL,
             status        TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'canceled', 'timeout')),
             result_json   TEXT,
             error_text    TEXT,
             created_at    TEXT NOT NULL,
             updated_at    TEXT NOT NULL
         );
         INSERT INTO tasks (
             task_id, user_id, chat_id, channel, external_user_id, external_chat_id, message_id, user_key,
             kind, payload_json, status, result_json, error_text, created_at, updated_at
         )
         SELECT
             task_id, user_id, chat_id, channel, external_user_id, external_chat_id, message_id, user_key,
             kind, payload_json, status, result_json, error_text, created_at, updated_at
         FROM tasks_old;
         DROP TABLE tasks_old;
         CREATE INDEX IF NOT EXISTS idx_tasks_status_created_at ON tasks(status, created_at);
         CREATE INDEX IF NOT EXISTS idx_tasks_user_id_created_at ON tasks(user_id, created_at);
         CREATE INDEX IF NOT EXISTS idx_tasks_user_key_created_at ON tasks(user_key, created_at);
         ALTER TABLE scheduled_jobs RENAME TO scheduled_jobs_old;
         CREATE TABLE scheduled_jobs (
             id                INTEGER PRIMARY KEY AUTOINCREMENT,
             job_id            TEXT NOT NULL UNIQUE,
             user_id           INTEGER NOT NULL,
             chat_id           INTEGER NOT NULL,
             channel           TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui')),
             external_user_id  TEXT,
             external_chat_id  TEXT,
             user_key          TEXT,
             schedule_type     TEXT NOT NULL CHECK (schedule_type IN ('once', 'daily', 'weekly', 'interval', 'cron')),
             run_at            INTEGER,
             time_of_day       TEXT,
             weekday           INTEGER,
             every_minutes     INTEGER,
             cron_expr         TEXT,
             timezone          TEXT NOT NULL,
             task_kind         TEXT NOT NULL CHECK (task_kind IN ('ask', 'run_skill')),
             task_payload_json TEXT NOT NULL,
             enabled           INTEGER NOT NULL DEFAULT 1,
             notify_on_success INTEGER NOT NULL DEFAULT 1,
             notify_on_failure INTEGER NOT NULL DEFAULT 1,
             last_run_at       TEXT,
             next_run_at       INTEGER,
             created_at        TEXT NOT NULL,
             updated_at        TEXT NOT NULL
         );
         INSERT INTO scheduled_jobs (
             id, job_id, user_id, chat_id, channel, external_user_id, external_chat_id, user_key,
             schedule_type, run_at, time_of_day, weekday, every_minutes, cron_expr, timezone,
             task_kind, task_payload_json, enabled, notify_on_success, notify_on_failure,
             last_run_at, next_run_at, created_at, updated_at
         )
         SELECT
             id, job_id, user_id, chat_id, channel, external_user_id, external_chat_id, user_key,
             schedule_type, run_at, time_of_day, weekday, every_minutes, cron_expr, timezone,
             task_kind, task_payload_json, enabled, notify_on_success, notify_on_failure,
             last_run_at, next_run_at, created_at, updated_at
         FROM scheduled_jobs_old;
         DROP TABLE scheduled_jobs_old;
         CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_due ON scheduled_jobs(enabled, next_run_at);
         CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_user_chat ON scheduled_jobs(user_id, chat_id);
         CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_user_key_chat ON scheduled_jobs(user_key, chat_id);
         ALTER TABLE memories RENAME TO memories_old;
         CREATE TABLE memories (
             id               INTEGER PRIMARY KEY AUTOINCREMENT,
             user_id          INTEGER NOT NULL,
             chat_id          INTEGER NOT NULL,
             user_key         TEXT,
             channel          TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui')),
             external_chat_id TEXT,
             role             TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
             content          TEXT NOT NULL,
             created_at       TEXT NOT NULL,
             created_at_ts    INTEGER NOT NULL DEFAULT 0,
             memory_type      TEXT NOT NULL DEFAULT 'generic',
             salience         REAL NOT NULL DEFAULT 0.5,
             is_instructional INTEGER NOT NULL DEFAULT 0,
             safety_flag      TEXT NOT NULL DEFAULT 'normal'
         );
         INSERT INTO memories (
             id, user_id, chat_id, user_key, channel, external_chat_id, role, content,
             created_at, created_at_ts, memory_type, salience, is_instructional, safety_flag
         )
         SELECT
             id, user_id, chat_id, user_key, channel, external_chat_id, role, content,
             created_at, created_at_ts, memory_type, salience, is_instructional, safety_flag
         FROM memories_old;
         DROP TABLE memories_old;
         CREATE INDEX IF NOT EXISTS idx_memories_user_chat_created_at ON memories(user_id, chat_id, created_at);
         CREATE INDEX IF NOT EXISTS idx_memories_user_chat_role_id ON memories(user_id, chat_id, role, id DESC);
         CREATE INDEX IF NOT EXISTS idx_memories_user_chat_created_at_ts ON memories(user_id, chat_id, created_at_ts);
         CREATE INDEX IF NOT EXISTS idx_memories_user_key_chat_created_at ON memories(user_key, chat_id, created_at_ts);
         COMMIT;",
    )?;
    Ok(())
}

fn ensure_key_auth_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(KEY_AUTH_UPGRADE_SQL)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS exchange_api_credentials (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            user_key    TEXT NOT NULL,
            exchange    TEXT NOT NULL,
            api_key     TEXT NOT NULL,
            api_secret  TEXT NOT NULL,
            passphrase  TEXT,
            enabled     INTEGER NOT NULL DEFAULT 1,
            updated_at  TEXT NOT NULL,
            UNIQUE(user_key, exchange)
        );
        CREATE INDEX IF NOT EXISTS idx_exchange_api_credentials_user_exchange
        ON exchange_api_credentials(user_key, exchange);",
    )?;
    ensure_column_exists(
        db,
        "tasks",
        "user_key",
        "ALTER TABLE tasks ADD COLUMN user_key TEXT",
    )?;
    ensure_column_exists(
        db,
        "scheduled_jobs",
        "user_key",
        "ALTER TABLE scheduled_jobs ADD COLUMN user_key TEXT",
    )?;
    ensure_column_exists(
        db,
        "memories",
        "user_key",
        "ALTER TABLE memories ADD COLUMN user_key TEXT",
    )?;
    ensure_column_exists(
        db,
        "long_term_memories",
        "user_key",
        "ALTER TABLE long_term_memories ADD COLUMN user_key TEXT",
    )?;
    ensure_column_exists(
        db,
        "audit_logs",
        "user_key",
        "ALTER TABLE audit_logs ADD COLUMN user_key TEXT",
    )?;
    ensure_column_exists(
        db,
        "user_preferences",
        "user_key",
        "ALTER TABLE user_preferences ADD COLUMN user_key TEXT",
    )?;
    rebuild_user_preferences_for_key_scope(db)?;
    rebuild_long_term_memories_for_key_scope(db)?;
    rebuild_channel_tables_for_ui(db)?;
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_tasks_user_key_created_at ON tasks(user_key, created_at);
         CREATE INDEX IF NOT EXISTS idx_memories_user_key_chat_created_at ON memories(user_key, chat_id, created_at_ts);
         CREATE INDEX IF NOT EXISTS idx_scheduled_jobs_user_key_chat ON scheduled_jobs(user_key, chat_id);
         CREATE INDEX IF NOT EXISTS idx_user_preferences_user_key_chat ON user_preferences(user_key, chat_id, updated_at_ts);
         CREATE INDEX IF NOT EXISTS idx_long_term_memories_user_key_chat_updated_ts ON long_term_memories(user_key, chat_id, updated_at_ts);",
    )?;
    Ok(())
}

fn rebuild_user_preferences_for_key_scope(db: &Connection) -> anyhow::Result<()> {
    let table_sql: String = db.query_row(
        "SELECT COALESCE(sql, '') FROM sqlite_master WHERE type = 'table' AND name = 'user_preferences'",
        [],
        |row| row.get(0),
    )?;
    if table_sql.contains("UNIQUE(user_id, chat_id, user_key, pref_key)") {
        return Ok(());
    }
    if !table_sql.contains("UNIQUE(user_id, chat_id, pref_key)") {
        return Ok(());
    }
    db.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE user_preferences RENAME TO user_preferences_old;
         CREATE TABLE user_preferences (
             id           INTEGER PRIMARY KEY AUTOINCREMENT,
             user_id      INTEGER NOT NULL,
             chat_id      INTEGER NOT NULL,
             pref_key     TEXT NOT NULL,
             pref_value   TEXT NOT NULL,
             confidence   REAL NOT NULL DEFAULT 0.8,
             source       TEXT NOT NULL DEFAULT 'memory_extract',
             updated_at   TEXT NOT NULL,
             updated_at_ts INTEGER NOT NULL DEFAULT 0,
             user_key     TEXT,
             UNIQUE(user_id, chat_id, user_key, pref_key)
         );
         INSERT OR REPLACE INTO user_preferences (
             id, user_id, chat_id, pref_key, pref_value, confidence, source, updated_at, updated_at_ts, user_key
         )
         SELECT
             id, user_id, chat_id, pref_key, pref_value, confidence, source, updated_at, updated_at_ts, user_key
         FROM user_preferences_old
         ORDER BY COALESCE(updated_at_ts, CAST(updated_at AS INTEGER)) ASC, id ASC;
         DROP TABLE user_preferences_old;
         CREATE INDEX IF NOT EXISTS idx_user_preferences_user_chat_updated
         ON user_preferences(user_id, chat_id, updated_at);
         CREATE INDEX IF NOT EXISTS idx_user_preferences_user_chat_updated_ts
         ON user_preferences(user_id, chat_id, updated_at_ts);
         CREATE INDEX IF NOT EXISTS idx_user_preferences_user_key_chat
         ON user_preferences(user_key, chat_id, updated_at_ts);
         COMMIT;",
    )?;
    Ok(())
}

fn rebuild_long_term_memories_for_key_scope(db: &Connection) -> anyhow::Result<()> {
    let table_sql: String = db.query_row(
        "SELECT COALESCE(sql, '') FROM sqlite_master WHERE type = 'table' AND name = 'long_term_memories'",
        [],
        |row| row.get(0),
    )?;
    if table_sql.contains("UNIQUE(user_id, chat_id, user_key)") {
        return Ok(());
    }
    if !table_sql.contains("UNIQUE(user_id, chat_id)") {
        return Ok(());
    }
    db.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE long_term_memories RENAME TO long_term_memories_old;
         CREATE TABLE long_term_memories (
             id                INTEGER PRIMARY KEY AUTOINCREMENT,
             user_id           INTEGER NOT NULL,
             chat_id           INTEGER NOT NULL,
             summary           TEXT NOT NULL,
             source_memory_id  INTEGER NOT NULL DEFAULT 0,
             created_at        TEXT NOT NULL,
             updated_at        TEXT NOT NULL,
             created_at_ts     INTEGER NOT NULL DEFAULT 0,
             updated_at_ts     INTEGER NOT NULL DEFAULT 0,
             user_key          TEXT,
             UNIQUE(user_id, chat_id, user_key)
         );
         INSERT OR REPLACE INTO long_term_memories (
             id, user_id, chat_id, summary, source_memory_id, created_at, updated_at, created_at_ts, updated_at_ts, user_key
         )
         SELECT
             id, user_id, chat_id, summary, source_memory_id, created_at, updated_at, created_at_ts, updated_at_ts, user_key
         FROM long_term_memories_old
         ORDER BY COALESCE(updated_at_ts, CAST(updated_at AS INTEGER)) ASC, id ASC;
         DROP TABLE long_term_memories_old;
         CREATE INDEX IF NOT EXISTS idx_long_term_memories_updated_at
         ON long_term_memories(updated_at);
         CREATE INDEX IF NOT EXISTS idx_long_term_memories_updated_at_ts
         ON long_term_memories(updated_at_ts);
         CREATE INDEX IF NOT EXISTS idx_long_term_memories_user_key_chat_updated_ts
         ON long_term_memories(user_key, chat_id, updated_at_ts);
         COMMIT;",
    )?;
    Ok(())
}

fn generate_user_key() -> String {
    format!("rk-{}", Uuid::new_v4().simple())
}

fn ensure_bootstrap_admin_key(db: &Connection) -> anyhow::Result<Option<String>> {
    let existing_count: i64 =
        db.query_row("SELECT COUNT(*) FROM auth_keys", [], |row| row.get(0))?;
    if existing_count > 0 {
        return Ok(None);
    }
    let user_key = generate_user_key();
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, 'admin', 1, ?2, NULL)",
        params![user_key, now_ts()],
    )?;
    Ok(Some(user_key))
}

fn seed_channel_binding_rows(
    db: &Connection,
    channel: &str,
    bindings: &[ChannelBindingConfig],
) -> anyhow::Result<()> {
    let now = now_ts();
    for binding in bindings {
        let user_key = normalize_user_key(&binding.user_key);
        if user_key.is_empty() {
            continue;
        }
        let external_user_id = normalize_external_id_opt(Some(&binding.external_user_id));
        let external_chat_id = normalize_external_id_opt(Some(&binding.external_chat_id))
            .or_else(|| external_user_id.clone());
        if external_user_id.is_none() && external_chat_id.is_none() {
            continue;
        }
        db.execute(
            "INSERT INTO channel_bindings (channel, external_user_id, external_chat_id, user_key, bound_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(channel, external_user_id, external_chat_id)
             DO UPDATE SET user_key=excluded.user_key, updated_at=excluded.updated_at",
            params![channel, external_user_id, external_chat_id, user_key, now],
        )?;
    }
    Ok(())
}

fn seed_channel_bindings(db: &Connection, config: &AppConfig) -> anyhow::Result<()> {
    seed_channel_binding_rows(db, "telegram", &config.telegram.bindings)?;
    seed_channel_binding_rows(db, "whatsapp", &config.whatsapp.bindings)?;
    seed_channel_binding_rows(db, "whatsapp", &config.whatsapp_cloud.bindings)?;
    seed_channel_binding_rows(db, "whatsapp", &config.whatsapp_web.bindings)?;
    Ok(())
}

fn ensure_column_exists(
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

fn current_rss_bytes() -> Option<u64> {
    current_rss_bytes_from_status("/proc/self/status")
}

fn current_rss_bytes_from_status(status_path: &str) -> Option<u64> {
    let status = std::fs::read_to_string(status_path).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())?;
            return Some(kb * 1024);
        }
    }
    None
}

fn telegramd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("telegramd")
}

fn whatsappd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("whatsappd")
}

fn wa_webd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("whatsapp_webd")
}

fn daemon_process_stats(process_name: &str) -> Option<(usize, u64)> {
    let entries = std::fs::read_dir("/proc").ok()?;
    let mut count = 0usize;
    let mut total_rss_bytes = 0u64;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid = name.to_string_lossy();
        if !pid.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let cmdline_path = format!("/proc/{pid}/cmdline");
        let bytes = match std::fs::read(&cmdline_path) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if bytes.is_empty() {
            continue;
        }
        let cmdline = String::from_utf8_lossy(&bytes);
        if cmdline.contains(process_name) {
            count += 1;
            let status_path = format!("/proc/{pid}/status");
            if let Some(rss_bytes) = current_rss_bytes_from_status(&status_path) {
                total_rss_bytes = total_rss_bytes.saturating_add(rss_bytes);
            }
        }
    }

    Some((count, total_rss_bytes))
}

fn task_count_by_status(state: &AppState, status: &str) -> anyhow::Result<usize> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

    let count: i64 = db.query_row(
        "SELECT COUNT(1) FROM tasks WHERE status = ?1",
        params![status],
        |row| row.get(0),
    )?;

    Ok(count as usize)
}

fn oldest_running_task_age_seconds(state: &AppState) -> anyhow::Result<u64> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

    let min_created_at: Option<i64> = db
        .query_row(
            "SELECT MIN(CAST(created_at AS INTEGER)) FROM tasks WHERE status = 'running'",
            [],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(created_ts) = min_created_at {
        let now = now_ts_u64() as i64;
        let age = now.saturating_sub(created_ts).max(0) as u64;
        Ok(age)
    } else {
        Ok(0)
    }
}

fn confirmation_rules(state: &AppState) -> &'static CompiledTradeRules {
    static RULES: OnceLock<CompiledTradeRules> = OnceLock::new();
    RULES.get_or_init(|| {
        let path = state.workspace_root.join("configs/hard_rules/trade.toml");
        let path_str = path.to_string_lossy().to_string();
        hard_trade::load_compiled_trade_rules(&path_str)
    })
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

fn normalize_affirmation_text(text: &str) -> String {
    text.trim().to_ascii_lowercase()
}

fn is_affirmation_click_text(state: &AppState, text: &str) -> bool {
    hard_trade::is_yes_confirmation(text, confirmation_rules(state))
}

fn is_negative_confirmation_click_text(state: &AppState, text: &str) -> bool {
    hard_trade::is_no_confirmation(text, confirmation_rules(state))
}

fn effective_trade_confirm_window_secs(state: &AppState, channel: &str) -> i64 {
    let base_window = main_flow_rules(state)
        .recent_trade_preview_window_secs
        .max(1);
    if is_whatsapp_channel_value(main_flow_rules(state), channel) {
        return base_window;
    }
    base_window
        .min(state.telegram_crypto_confirm_ttl_seconds.max(1))
        .max(1)
}

#[derive(Debug, Clone)]
struct TradePreviewContext {
    exchange: String,
    symbol: String,
    side: String,
    order_type: String,
    qty: f64,
    quote_qty_usd: Option<f64>,
    price: Option<f64>,
    stop_price: Option<f64>,
    time_in_force: Option<String>,
}

fn build_trade_confirm_cancelled_text(
    state: &AppState,
    preview_ctx: &TradePreviewContext,
) -> String {
    i18n_t_with_default(
        state,
        "clawd.msg.trade_confirm_cancelled",
        "Trade confirmation cancelled: {exchange} {symbol} {side} qty={qty}",
    )
    .replace("{exchange}", &preview_ctx.exchange)
    .replace("{symbol}", &preview_ctx.symbol)
    .replace("{side}", &preview_ctx.side)
    .replace("{qty}", &preview_ctx.qty.to_string())
}

fn parse_trade_preview_line(line: &str, rules: &MainFlowRules) -> Option<TradePreviewContext> {
    let trimmed = line.trim();
    if !trimmed.starts_with(&rules.trade_preview_line_prefix) {
        return None;
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }
    let qty = parts.iter().find_map(|p| {
        p.strip_prefix("qty=")
            .or_else(|| p.strip_prefix("est_qty="))
            .and_then(|v| v.parse::<f64>().ok())
    })?;
    let quote_qty_usd = parts.iter().find_map(|p| {
        p.strip_prefix("quote_usd=")
            .and_then(|v| v.parse::<f64>().ok())
    });
    let order_type = parts
        .iter()
        .find_map(|p| {
            p.strip_prefix("order_type=")
                .map(|v| v.to_ascii_lowercase())
        })
        .unwrap_or_else(|| rules.trade_preview_default_order_type.clone());
    let price = parts
        .iter()
        .find_map(|p| p.strip_prefix("price=").and_then(|v| v.parse::<f64>().ok()));
    let stop_price = parts.iter().find_map(|p| {
        p.strip_prefix("stop_price=")
            .and_then(|v| v.parse::<f64>().ok())
    });
    let time_in_force = parts
        .iter()
        .find_map(|p| p.strip_prefix("tif=").map(|v| v.to_ascii_uppercase()));
    Some(TradePreviewContext {
        exchange: parts[1].trim().to_ascii_lowercase(),
        symbol: parts[2].trim().to_ascii_uppercase(),
        side: parts[3].trim().to_ascii_lowercase(),
        order_type,
        qty,
        quote_qty_usd,
        price,
        stop_price,
        time_in_force,
    })
}

fn extract_trade_preview_context_from_result_json(
    result_json: &str,
    rules: &MainFlowRules,
) -> Option<TradePreviewContext> {
    let v: Value = serde_json::from_str(result_json).ok()?;
    let mut candidates = Vec::new();
    if let Some(text) = v.get("text").and_then(|x| x.as_str()) {
        candidates.push(text.to_string());
    }
    if let Some(messages) = v.get("messages").and_then(|x| x.as_array()) {
        for msg in messages {
            if let Some(s) = msg.as_str() {
                candidates.push(s.to_string());
            }
        }
    }
    for text in candidates.into_iter().rev() {
        for line in text.lines().rev() {
            if let Some(ctx) = parse_trade_preview_line(line, rules) {
                return Some(ctx);
            }
        }
    }
    None
}

fn find_recent_trade_preview_context(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    window_secs: i64,
) -> Option<TradePreviewContext> {
    let rules = main_flow_rules(state);
    let now = now_ts_u64() as i64;
    let db = state.db.lock().ok()?;
    let mut stmt = db
        .prepare(
            "SELECT result_json, CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) AS ts
             FROM tasks
             WHERE user_id = ?1 AND chat_id = ?2 AND kind = 'ask' AND status = 'succeeded'
             ORDER BY ts DESC
             LIMIT ?3",
        )
        .ok()?;
    let rows = stmt
        .query_map(
            params![
                user_id,
                chat_id,
                rules.recent_trade_preview_scan_limit as i64
            ],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
        )
        .ok()?;
    for row in rows.flatten() {
        let (result_json_opt, ts) = row;
        if now.saturating_sub(ts) > window_secs {
            continue;
        }
        let Some(result_json) = result_json_opt else {
            // A newer successful ask exists but does not carry preview text,
            // so treat previous preview as no longer pending.
            return None;
        };
        if let Some(ctx) = extract_trade_preview_context_from_result_json(&result_json, rules) {
            return Some(ctx);
        }
        // The latest successful ask is not a trade preview; pending confirmation
        // should be considered cleared by subsequent conversation turns.
        return None;
    }
    None
}

fn find_recent_duplicate_affirmation_task(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    ask_text: &str,
    window_secs: i64,
) -> Option<Uuid> {
    let rules = main_flow_rules(state);
    if !is_affirmation_click_text(state, ask_text) {
        return None;
    }
    let normalized = normalize_affirmation_text(ask_text);
    let now = now_ts_u64() as i64;
    let db = state.db.lock().ok()?;
    let mut stmt = db
        .prepare(
            "SELECT task_id, payload_json, status, CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) AS ts
             FROM tasks
             WHERE user_id = ?1 AND chat_id = ?2 AND kind = 'ask'
             ORDER BY ts DESC
             LIMIT ?3",
        )
        .ok()?;
    let rows = stmt
        .query_map(
            params![
                user_id,
                chat_id,
                rules.duplicate_affirmation_scan_limit as i64
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .ok()?;
    for row in rows.flatten() {
        let (task_id, payload_json, status, ts) = row;
        let status_lc = status.to_ascii_lowercase();
        if !rules
            .duplicate_affirmation_statuses
            .iter()
            .any(|s| s == &status_lc)
        {
            continue;
        }
        if now.saturating_sub(ts) > window_secs {
            continue;
        }
        let Ok(payload) = serde_json::from_str::<Value>(&payload_json) else {
            continue;
        };
        let text = payload
            .get("text")
            .and_then(|v| v.as_str())
            .map(normalize_affirmation_text)
            .unwrap_or_default();
        if text == normalized {
            if let Ok(id) = Uuid::parse_str(&task_id) {
                return Some(id);
            }
        }
    }
    None
}

fn has_newer_successful_ask_after(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    ts: i64,
) -> bool {
    let Ok(db) = state.db.lock() else {
        return false;
    };
    let mut stmt = match db.prepare(
        "SELECT 1
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND kind = 'ask'
           AND status = 'succeeded'
           AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) > ?3
         LIMIT 1",
    ) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let mut rows = match stmt.query(params![user_id, chat_id, ts]) {
        Ok(v) => v,
        Err(_) => return false,
    };
    matches!(rows.next(), Ok(Some(_)))
}

fn find_recent_failed_resume_context(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
) -> Option<(Value, i64)> {
    let db = state.db.lock().ok()?;
    let mut stmt = db
        .prepare(
            "SELECT result_json,
                    CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER)
             FROM tasks
             WHERE user_id = ?1 AND chat_id = ?2 AND kind = 'ask' AND status = 'failed'
             ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) DESC
             LIMIT 24",
        )
        .ok()?;
    let rows = stmt
        .query_map(params![user_id, chat_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<i64>>(1)?.unwrap_or_default(),
            ))
        })
        .ok()?;
    for row in rows.flatten() {
        let (result_json, ts) = row;
        let Some(result_json) = result_json else {
            continue;
        };
        let Ok(result) = serde_json::from_str::<Value>(&result_json) else {
            continue;
        };
        let Some(resume_context) = result.get("resume_context").cloned() else {
            continue;
        };
        if !resume_context.is_null() {
            return Some((resume_context, ts));
        }
    }
    None
}

async fn submit_task(
    State(state): State<AppState>,
    Json(req): Json<SubmitTaskRequest>,
) -> (StatusCode, Json<ApiResponse<SubmitTaskResponse>>) {
    let resolved_identity = match req.user_key.as_deref() {
        Some(user_key) => match resolve_auth_identity_by_key(&state, user_key) {
            Ok(v) => v,
            Err(err) => {
                error!("resolve auth key failed: {}", err);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("Auth lookup failed".to_string()),
                    }),
                );
            }
        },
        None => None,
    };
    let effective_user_key = resolved_identity.as_ref().map(|v| v.user_key.clone());
    let effective_user_id = resolved_identity
        .as_ref()
        .map(|v| v.user_id)
        .unwrap_or(req.user_id);
    let channel = req.channel.unwrap_or(ChannelKind::Telegram);
    let normalized_external_user_id = normalize_external_id_opt(req.external_user_id.as_deref());
    let normalized_external_chat_id = normalize_external_id_opt(req.external_chat_id.as_deref());
    let effective_chat_id = if let Some(user_key) = effective_user_key.as_deref() {
        build_conversation_chat_id(
            channel_kind_name(channel),
            normalized_external_user_id.as_deref(),
            normalized_external_chat_id.as_deref(),
            user_key,
        )
    } else {
        req.chat_id
    };

    if resolved_identity.is_none() && !is_user_allowed(&state, req.user_id) {
        let unauthorized = "Unauthorized user".to_string();
        let _ = insert_audit_log(
            &state,
            Some(effective_user_id),
            "auth_fail",
            Some(
                &json!({
                    "chat_id": effective_chat_id,
                    "kind": format!("{:?}", req.kind),
                    "user_key": effective_user_key,
                })
                .to_string(),
            ),
            Some(&unauthorized),
        );
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(unauthorized),
            }),
        );
    }

    let limit_result = {
        let mut limiter = match state.rate_limiter.lock() {
            Ok(v) => v,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("Rate limiter lock poisoned".to_string()),
                    }),
                )
            }
        };
        limiter.check_and_record(effective_user_id)
    };

    if let Err(kind) = limit_result {
        let limit_exceeded = "Rate limit exceeded".to_string();
        let _ = insert_audit_log(
            &state,
            Some(effective_user_id),
            "limit_hit",
            Some(&json!({ "limit": kind, "chat_id": effective_chat_id }).to_string()),
            Some(&limit_exceeded),
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(limit_exceeded),
            }),
        );
    }

    let queued_count =
        match task_count_by_status(&state, &main_flow_rules(&state).task_status_queued) {
            Ok(v) => v,
            Err(err) => {
                error!("Count queued tasks failed: {}", err);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("Database error".to_string()),
                    }),
                );
            }
        };

    if queued_count >= state.queue_limit {
        let queue_full = "Task queue is full".to_string();
        let _ = insert_audit_log(
            &state,
            Some(effective_user_id),
            "limit_hit",
            Some(&json!({ "limit": "queue_limit", "chat_id": effective_chat_id }).to_string()),
            Some(&queue_full),
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(queue_full),
            }),
        );
    }

    let is_ask_task = matches!(&req.kind, claw_core::types::TaskKind::Ask);
    if is_ask_task {
        if let Some(text) = req.payload.get("text").and_then(|v| v.as_str()) {
            if let Some(existing_id) = find_recent_duplicate_affirmation_task(
                &state,
                effective_user_id,
                effective_chat_id,
                text,
                main_flow_rules(&state).duplicate_affirmation_window_secs,
            ) {
                info!(
                    "task_submit dedup: reused recent affirmative task_id={} user_id={} chat_id={} text={}",
                    existing_id,
                    effective_user_id,
                    effective_chat_id,
                    truncate_for_log(text)
                );
                return (
                    StatusCode::OK,
                    Json(ApiResponse {
                        ok: true,
                        data: Some(SubmitTaskResponse {
                            task_id: existing_id,
                        }),
                        error: None,
                    }),
                );
            }
        }
    }

    let task_id = Uuid::new_v4();
    let call_id = task_id.to_string();
    let mut payload = req.payload;
    if let Some(obj) = payload.as_object_mut() {
        let channel_str = channel_kind_name(channel);
        obj.insert(
            "channel".to_string(),
            Value::String(channel_str.to_string()),
        );
        if let Some(v) = normalized_external_user_id.as_deref() {
            obj.insert("external_user_id".to_string(), Value::String(v.to_string()));
        }
        if let Some(v) = normalized_external_chat_id.as_deref() {
            obj.insert("external_chat_id".to_string(), Value::String(v.to_string()));
        }
        if let Some(user_key) = effective_user_key.as_ref() {
            obj.insert("user_key".to_string(), Value::String(user_key.clone()));
        }
        obj.insert("call_id".to_string(), Value::String(call_id.clone()));
    }
    let payload_text = payload.to_string();
    let now = now_ts();
    let kind = match req.kind {
        claw_core::types::TaskKind::Ask => "ask",
        claw_core::types::TaskKind::RunSkill => "run_skill",
        claw_core::types::TaskKind::Admin => "admin",
    };

    let write_result = (|| -> anyhow::Result<()> {
        let db = state
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

        db.execute(
            "INSERT INTO tasks (task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, message_id, kind, payload_json, status, result_json, error_text, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, 'queued', NULL, NULL, ?10, ?10)",
            params![
                task_id.to_string(),
                effective_user_id,
                effective_chat_id,
                effective_user_key.as_deref(),
                channel_kind_name(channel),
                normalized_external_user_id.as_deref(),
                normalized_external_chat_id.as_deref(),
                kind,
                payload_text,
                now
            ],
        )?;
        Ok(())
    })();

    if let Err(err) = write_result {
        error!("Insert task failed: {}", err);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Database error".to_string()),
            }),
        );
    }

    let _ = insert_audit_log(
        &state,
        Some(effective_user_id),
        "submit_task",
        Some(
            &json!({
                "call_id": call_id,
                "task_id": task_id,
                "kind": kind,
                "chat_id": effective_chat_id,
                "user_key": effective_user_key,
            })
            .to_string(),
        ),
        None,
    );
    info!(
        "task_submit accepted call_id={} task_id={} kind={} user_id={} chat_id={}",
        task_id, task_id, kind, effective_user_id, effective_chat_id
    );

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(SubmitTaskResponse { task_id }),
            error: None,
        }),
    )
}

fn stable_i64_from_key(input: &str) -> i64 {
    use std::hash::{Hash, Hasher};

    let mut h = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut h);
    let v = h.finish() & (i64::MAX as u64);
    v as i64
}

pub(crate) fn normalize_user_key(raw: &str) -> String {
    raw.trim().to_string()
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

fn normalize_exchange_name(raw: &str) -> anyhow::Result<String> {
    let exchange = raw.trim().to_ascii_lowercase();
    match exchange.as_str() {
        "binance" | "okx" => Ok(exchange),
        _ => Err(anyhow::anyhow!("unsupported exchange: {exchange}")),
    }
}

pub(crate) fn exchange_credential_status_for_user_key(
    state: &AppState,
    user_key: &str,
) -> anyhow::Result<Vec<ExchangeCredentialStatus>> {
    let user_key = normalize_user_key(user_key);
    if user_key.is_empty() {
        return Ok(Vec::new());
    }
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let mut out = Vec::new();
    for exchange in ["binance", "okx"] {
        let row = db
            .query_row(
                "SELECT api_key, updated_at, enabled
                 FROM exchange_api_credentials
                 WHERE user_key = ?1 AND exchange = ?2
                 LIMIT 1",
                params![user_key, exchange],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let (configured, api_key_masked, updated_at) = match row {
            Some((api_key, updated_at, enabled)) if enabled == 1 => {
                (true, Some(api_key), Some(updated_at))
            }
            _ => (false, None, None),
        };
        out.push(ExchangeCredentialStatus {
            exchange: exchange.to_string(),
            configured,
            api_key_masked,
            updated_at,
        });
    }
    Ok(out)
}

pub(crate) fn upsert_exchange_credential_for_user_key(
    state: &AppState,
    user_key: &str,
    exchange_raw: &str,
    api_key: &str,
    api_secret: &str,
    passphrase: Option<&str>,
) -> anyhow::Result<ExchangeCredentialStatus> {
    let user_key = normalize_user_key(user_key);
    if user_key.is_empty() {
        return Err(anyhow::anyhow!("user_key is required"));
    }
    let exchange = normalize_exchange_name(exchange_raw)?;
    let api_key = api_key.trim();
    let api_secret = api_secret.trim();
    if api_key.is_empty() || api_secret.is_empty() {
        return Err(anyhow::anyhow!("api_key and api_secret are required"));
    }
    let passphrase = passphrase.map(str::trim).filter(|v| !v.is_empty());
    let now = now_ts();
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "INSERT INTO exchange_api_credentials (user_key, exchange, api_key, api_secret, passphrase, enabled, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
         ON CONFLICT(user_key, exchange)
         DO UPDATE SET api_key=excluded.api_key, api_secret=excluded.api_secret, passphrase=excluded.passphrase, enabled=1, updated_at=excluded.updated_at",
        params![user_key, exchange, api_key, api_secret, passphrase, now],
    )?;
    Ok(ExchangeCredentialStatus {
        exchange,
        configured: true,
        api_key_masked: Some(api_key.to_string()),
        updated_at: Some(now),
    })
}

fn exchange_credential_context_for_task(state: &AppState, task: &ClaimedTask) -> serde_json::Value {
    let Some(user_key) = task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return json!({});
    };
    let Ok(db) = state.db.lock() else {
        return json!({});
    };
    let mut stmt = match db.prepare(
        "SELECT exchange, api_key, api_secret, passphrase
         FROM exchange_api_credentials
         WHERE user_key = ?1 AND enabled = 1",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return json!({}),
    };
    let rows = match stmt.query_map(params![user_key], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return json!({}),
    };
    let mut exchanges = serde_json::Map::new();
    for row in rows.flatten() {
        let (exchange, api_key, api_secret, passphrase) = row;
        exchanges.insert(
            exchange,
            json!({
                "api_key": api_key,
                "api_secret": api_secret,
                "passphrase": passphrase,
            }),
        );
    }
    Value::Object(exchanges)
}

pub(crate) fn normalize_external_id_opt(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn channel_kind_name(channel: ChannelKind) -> &'static str {
    match channel {
        ChannelKind::Telegram => "telegram",
        ChannelKind::Whatsapp => "whatsapp",
        ChannelKind::Ui => "ui",
    }
}

fn build_conversation_chat_id(
    channel: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
    user_key: &str,
) -> i64 {
    let scope = external_chat_id
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or_else(|| external_user_id.map(str::trim).filter(|v| !v.is_empty()))
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("principal:{user_key}"));
    stable_i64_from_key(&format!("conv:{channel}:{scope}"))
}

fn build_auth_identity(
    user_key: &str,
    role: &str,
    channel: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
) -> AuthIdentity {
    let user_id = stable_i64_from_key(user_key);
    AuthIdentity {
        user_key: user_key.to_string(),
        role: role.to_string(),
        user_id,
        chat_id: build_conversation_chat_id(channel, external_user_id, external_chat_id, user_key),
    }
}

pub(crate) fn resolve_auth_identity_by_key(
    state: &AppState,
    raw_user_key: &str,
) -> anyhow::Result<Option<AuthIdentity>> {
    let user_key = normalize_user_key(raw_user_key);
    if user_key.is_empty() {
        return Ok(None);
    }
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let row = db
        .query_row(
            "SELECT role FROM auth_keys WHERE user_key = ?1 AND enabled = 1 LIMIT 1",
            params![user_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(row.map(|role| build_auth_identity(&user_key, &role, "ui", None, Some("console"))))
}

fn touch_auth_key_usage(db: &Connection, user_key: &str) -> anyhow::Result<()> {
    db.execute(
        "UPDATE auth_keys SET last_used_at = ?2 WHERE user_key = ?1",
        params![user_key, now_ts()],
    )?;
    Ok(())
}

pub(crate) fn resolve_channel_binding_identity(
    state: &AppState,
    channel: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
) -> anyhow::Result<Option<AuthIdentity>> {
    let external_user_id = normalize_external_id_opt(external_user_id);
    let external_chat_id =
        normalize_external_id_opt(external_chat_id).or_else(|| external_user_id.clone());
    if external_user_id.is_none() && external_chat_id.is_none() {
        return Ok(None);
    }
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let row = if external_user_id.is_some() && external_chat_id.is_some() {
        db.query_row(
            "SELECT k.user_key, k.role
             FROM channel_bindings b
             JOIN auth_keys k ON k.user_key = b.user_key
             WHERE b.channel = ?1
               AND k.enabled = 1
               AND b.external_user_id = ?2
               AND b.external_chat_id = ?3
             ORDER BY b.id DESC
             LIMIT 1",
            params![channel, external_user_id, external_chat_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    } else if external_chat_id.is_some() {
        db.query_row(
            "SELECT k.user_key, k.role
             FROM channel_bindings b
             JOIN auth_keys k ON k.user_key = b.user_key
             WHERE b.channel = ?1
               AND k.enabled = 1
               AND b.external_chat_id = ?2
             ORDER BY b.id DESC
             LIMIT 1",
            params![channel, external_chat_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    } else {
        db.query_row(
            "SELECT k.user_key, k.role
             FROM channel_bindings b
             JOIN auth_keys k ON k.user_key = b.user_key
             WHERE b.channel = ?1
               AND k.enabled = 1
               AND b.external_user_id = ?2
             ORDER BY b.id DESC
             LIMIT 1",
            params![channel, external_user_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    };
    if let Some((user_key, role)) = row {
        touch_auth_key_usage(&db, &user_key)?;
        return Ok(Some(build_auth_identity(
            &user_key,
            &role,
            channel,
            external_user_id.as_deref(),
            external_chat_id.as_deref(),
        )));
    }
    Ok(None)
}

pub(crate) fn bind_channel_identity(
    state: &AppState,
    channel: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
    raw_user_key: &str,
) -> anyhow::Result<Option<AuthIdentity>> {
    let Some(identity) = resolve_auth_identity_by_key(state, raw_user_key)? else {
        return Ok(None);
    };
    let external_user_id = normalize_external_id_opt(external_user_id);
    let external_chat_id =
        normalize_external_id_opt(external_chat_id).or_else(|| external_user_id.clone());
    if external_user_id.is_none() && external_chat_id.is_none() {
        return Ok(None);
    }
    let now = now_ts();
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "INSERT INTO channel_bindings (channel, external_user_id, external_chat_id, user_key, bound_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(channel, external_user_id, external_chat_id)
         DO UPDATE SET user_key=excluded.user_key, updated_at=excluded.updated_at",
        params![
            channel,
            external_user_id,
            external_chat_id,
            &identity.user_key,
            now
        ],
    )?;
    touch_auth_key_usage(&db, &identity.user_key)?;
    Ok(Some(build_auth_identity(
        &identity.user_key,
        &identity.role,
        channel,
        external_user_id.as_deref(),
        external_chat_id.as_deref(),
    )))
}

fn is_user_allowed(state: &AppState, user_id: i64) -> bool {
    let Ok(db) = state.db.lock() else {
        return false;
    };

    let query = db
        .query_row(
            "SELECT is_allowed FROM users WHERE user_id = ?1",
            params![user_id],
            |row| row.get::<_, i64>(0),
        )
        .optional();

    matches!(query, Ok(Some(v)) if v == 1)
}

async fn get_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<Uuid>,
) -> (StatusCode, Json<ApiResponse<TaskQueryResponse>>) {
    let read_result = (|| -> anyhow::Result<Option<(TaskQueryResponse, Option<String>)>> {
        let db = state
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

        let mut stmt = db.prepare(
            "SELECT status, result_json, error_text, user_key
             FROM tasks
             WHERE task_id = ?1
             LIMIT 1",
        )?;

        let row = stmt
            .query_row(params![task_id.to_string()], |row| {
                let status_str: String = row.get(0)?;
                let result_json_str: Option<String> = row.get(1)?;
                let error_text: Option<String> = row.get(2)?;
                let task_user_key: Option<String> = row.get(3)?;

                let status = parse_task_status_with_rules(main_flow_rules(&state), &status_str);

                let result_json = result_json_str
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());

                Ok((
                    TaskQueryResponse {
                        task_id,
                        status,
                        result_json,
                        error_text,
                    },
                    task_user_key,
                ))
            })
            .optional()?;

        Ok(row)
    })();

    match read_result {
        Ok(Some((task, task_user_key))) => {
            let expected_key = task_user_key
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty());
            if let Some(expected_key) = expected_key {
                let provided_key = headers
                    .get("x-rustclaw-key")
                    .and_then(|v| v.to_str().ok())
                    .map(normalize_user_key)
                    .filter(|v| !v.is_empty());
                if provided_key.as_deref() != Some(expected_key) {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some("Task owner mismatch".to_string()),
                        }),
                    );
                }
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(task),
                    error: None,
                }),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Task not found".to_string()),
            }),
        ),
        Err(err) => {
            error!("Read task failed: {}", err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("Database error".to_string()),
                }),
            )
        }
    }
}

#[derive(Debug, Deserialize)]
struct CancelTasksRequest {
    user_id: i64,
    chat_id: i64,
}

async fn cancel_tasks(
    State(state): State<AppState>,
    Json(req): Json<CancelTasksRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    if !is_user_allowed(&state, req.user_id) {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Unauthorized user".to_string()),
            }),
        );
    }

    let now = now_ts();
    let result = (|| -> anyhow::Result<i64> {
        let db = state
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

        let mut stmt = db.prepare(
            "UPDATE tasks
             SET status = 'canceled',
                 error_text = COALESCE(error_text, 'Canceled by user'),
                 updated_at = ?1
             WHERE user_id = ?2
               AND chat_id = ?3
               AND status IN ('queued', 'running')",
        )?;
        let affected = stmt.execute(params![now, req.user_id, req.chat_id])?;
        Ok(affected as i64)
    })();

    match result {
        Ok(count) => {
            info!(
                "cancel_tasks: user_id={} chat_id={} canceled={}",
                req.user_id, req.chat_id, count
            );
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({ "canceled": count })),
                    error: None,
                }),
            )
        }
        Err(err) => {
            error!("Cancel tasks failed: {}", err);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("Cancel tasks failed".to_string()),
                }),
            )
        }
    }
}
