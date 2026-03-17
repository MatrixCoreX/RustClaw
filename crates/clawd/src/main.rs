use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{create_dir_all, OpenOptions};
use std::io::IsTerminal;
use std::io::Write as IoWrite;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::routing::{get, get_service, post};
use axum::{Json, Router};
use chrono::{Local, TimeZone};
use claw_core::config::{
    AgentConfig, AppConfig, ChannelBindingConfig, CommandIntentConfig, LlmProviderConfig,
    MaintenanceConfig, MemoryConfig, PersonaConfig, RoutingConfig, ScheduleConfig, ToolsConfig,
};
use claw_core::hard_rules::main_flow::load_main_flow_rules;
use claw_core::hard_rules::trade as hard_trade;
use claw_core::hard_rules::trade::CompiledTradeRules;
use claw_core::hard_rules::types::MainFlowRules;
use claw_core::skill_registry::{SkillKind, SkillsRegistry};
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
use tokio::sync::{oneshot, Semaphore};
use toml::Value as TomlValue;
use tower_http::cors::{Any, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing::{debug, error, info, info_span, warn, Instrument};
use uuid::Uuid;

mod agent_engine;
mod channel_send;
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
const DEFAULT_AGENT_ID: &str = "main";

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

/// 统一错误响应，避免重复手写 (StatusCode, Json(ApiResponse)).
fn api_err<T: Serialize>(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(message.into()),
        }),
    )
}

/// 统一成功响应 (200 OK).
fn api_ok<T: Serialize>(data: T) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

/// Phase 4: 统一 skill 视图重建结果，供启动与 reload 复用。
struct SkillViews {
    registry: Option<Arc<SkillsRegistry>>,
    execution_skills: HashSet<String>,
    planner_visible: Vec<String>,
}

/// Phase 4 review: 原子快照，reload 时整体替换，避免混合视图。
pub(crate) struct SkillViewsSnapshot {
    pub registry: Option<Arc<SkillsRegistry>>,
    pub skills_list: Arc<HashSet<String>>,
}

/// Phase 4: 从 registry 路径 + skill_switches 重建 skill 视图。无 registry 时用 initial_skills_list（启动用）；reload 时 registry_path 必设。
fn build_skill_views(
    workspace_root: &Path,
    registry_path: Option<&str>,
    skill_switches: &HashMap<String, bool>,
    initial_skills_list: &[String],
) -> Result<SkillViews, String> {
    let registry: Option<Arc<SkillsRegistry>> = if let Some(p) = registry_path {
        let path = if Path::new(p).is_absolute() {
            PathBuf::from(p)
        } else {
            workspace_root.join(p)
        };
        match SkillsRegistry::load_from_path(&path) {
            Ok(reg) => Some(Arc::new(reg)),
            Err(e) => return Err(format!("registry load failed: {}: {}", path.display(), e)),
        }
    } else {
        None
    };

    // 显式 false 的 canonical 集合：用于覆盖 core-floor，使 skill_switches=false 真正生效
    let explicitly_disabled: HashSet<String> = skill_switches
        .iter()
        .filter(|(_, &on)| !on)
        .map(|(skill, _)| {
            registry
                .as_ref()
                .and_then(|r| r.resolve_canonical(skill).map(String::from))
                .unwrap_or_else(|| canonical_skill_name(skill).to_string())
        })
        .collect();

    let mut enabled: HashSet<String> = if let Some(ref reg) = registry {
        reg.enabled_names().into_iter().collect()
    } else {
        initial_skills_list
            .iter()
            .map(|s| canonical_skill_name(s).to_string())
            .collect()
    };
    for (skill, is_enabled) in skill_switches {
        let canonical = registry
            .as_ref()
            .and_then(|r| r.resolve_canonical(skill).map(String::from))
            .unwrap_or_else(|| canonical_skill_name(skill).to_string());
        if *is_enabled {
            enabled.insert(canonical);
        } else {
            enabled.remove(&canonical);
        }
    }
    // core-floor 仅作默认保底；显式 skill_switches=false 可覆盖，与 config 注释「false = 强制关闭」一致
    for s in claw_core::config::core_skills_always_enabled() {
        let c = canonical_skill_name(s).to_string();
        if !explicitly_disabled.contains(&c) {
            enabled.insert(c);
        }
    }
    // planner_visible 与 execution 一致：同一 enabled 集合（含 skill_switches + core floor），避免 planner 看到 execution 已关的技能
    let mut planner_visible: Vec<String> = enabled.iter().cloned().collect();
    planner_visible.sort_unstable();

    Ok(SkillViews {
        registry,
        execution_skills: enabled,
        planner_visible,
    })
}

/// Phase 4: 重载 skill 视图并更新 AppState。从 config_path_for_reload 重读 config，取最新 skills.registry_path / skill_switches / skills_list，再重建视图。失败不更新状态，返回 Err。
pub(crate) fn reload_skill_views(state: &AppState) -> Result<ReloadSkillViewsResult, String> {
    info!(
        "reload_skill_views: started config_path={}",
        state.config_path_for_reload
    );
    let config = AppConfig::load(&state.config_path_for_reload)
        .map_err(|e| format!("reload_skill_views: load config failed: {}", e))?;
    let registry_path = config.skills.registry_path.as_deref();
    let path_display = registry_path.unwrap_or("(none)");
    let views = build_skill_views(
        &state.workspace_root,
        registry_path,
        &config.skills.skill_switches,
        &config.skills.skills_list,
    )?;
    let registry_entries = views
        .registry
        .as_ref()
        .map(|r| r.all_names().len())
        .unwrap_or(0);
    let execution_count = views.execution_skills.len();
    let planner_count = views.planner_visible.len();

    let snapshot = SkillViewsSnapshot {
        registry: views.registry,
        skills_list: Arc::new(views.execution_skills),
    };
    *state.skill_views_snapshot.write().unwrap() = Arc::new(snapshot);

    info!(
        "reload_skill_views: success path={} registry_entries={} execution_skills_count={} planner_visible_count={}",
        path_display, registry_entries, execution_count, planner_count
    );
    Ok(ReloadSkillViewsResult {
        registry_entries,
        execution_skills_count: execution_count,
        planner_visible_count: planner_count,
    })
}

#[derive(Debug, Serialize)]
pub(crate) struct ReloadSkillViewsResult {
    pub registry_entries: usize,
    pub execution_skills_count: usize,
    pub planner_visible_count: usize,
}

#[derive(Clone)]
struct AppState {
    started_at: Instant,
    queue_limit: usize,
    db: Arc<Mutex<Connection>>,
    llm_providers: Vec<Arc<LlmProviderRuntime>>,
    agents_by_id: Arc<HashMap<String, AgentRuntimeConfig>>,
    skill_timeout_seconds: u64,
    skill_runner_path: PathBuf,
    /// 原子快照（可重载）。reload 时整体替换，读用 get_skill_views_snapshot()。
    skill_views_snapshot: Arc<RwLock<Arc<SkillViewsSnapshot>>>,
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
    worker_task_heartbeat_seconds: u64,
    worker_running_no_progress_timeout_seconds: u64,
    worker_running_recovery_check_interval_seconds: u64,
    last_running_recovery_check_ts: Arc<Mutex<u64>>,
    routing: RoutingConfig,
    persona_prompt: String,
    command_intent: CommandIntentRuntime,
    schedule: ScheduleRuntime,
    telegram_bot_token: String,
    telegram_configured_bot_names: Arc<Vec<String>>,
    telegram_crypto_confirm_ttl_seconds: i64,
    whatsapp_cloud_enabled: bool,
    whatsapp_api_base: String,
    whatsapp_access_token: String,
    whatsapp_phone_number_id: String,
    whatsapp_web_enabled: bool,
    whatsapp_web_bridge_base_url: String,
    future_adapters_enabled: Arc<Vec<String>>,
    /// 定时任务结果主动推送到 Feishu 时使用（从 configs/channels/feishu.toml 可选加载）
    feishu_send_config: Option<channel_send::FeishuSendConfig>,
    /// 定时任务结果主动推送到 Lark 时使用（从 configs/channels/lark.toml 可选加载）
    lark_send_config: Option<channel_send::LarkSendConfig>,
    http_client: Client,
    /// reload 时用：主配置路径，reload 时从此文件重读 skills.registry_path / skill_switches / skills_list
    config_path_for_reload: String,
    /// 兼容保留：仅启动时写入，reload 不再使用（改由重读 config 得到）
    #[allow(dead_code)]
    registry_path_for_reload: Option<String>,
    #[allow(dead_code)]
    skill_switches_for_reload: Arc<HashMap<String, bool>>,
    #[allow(dead_code)]
    initial_skills_list_for_reload: Vec<String>,
}

impl AppState {
    fn snapshot(&self) -> Arc<SkillViewsSnapshot> {
        self.skill_views_snapshot.read().unwrap().clone()
    }
    pub(crate) fn get_skills_registry(&self) -> Option<Arc<SkillsRegistry>> {
        self.snapshot().registry.clone()
    }
    pub(crate) fn get_skills_list(&self) -> Arc<HashSet<String>> {
        self.snapshot().skills_list.clone()
    }

    /// Planner 可见技能（按 task/agent 动态收敛）：
    /// 全局执行可用集（skills_list + skill_switches + core floor）∩ agent allowed_skills。
    /// 当 agent 未配置 allowed_skills 时，继承全量执行可用集。
    pub(crate) fn planner_visible_skills_for_task(&self, task: &ClaimedTask) -> Vec<String> {
        let execution_skills = self.get_skills_list();
        let agent = self.task_agent(task);
        let mut visible: Vec<String> = execution_skills
            .iter()
            .filter(|skill| agent.allows_skill(skill))
            .cloned()
            .collect();
        visible.sort_unstable();
        visible
    }

    fn normalize_known_agent_id(&self, agent_id: Option<&str>) -> Option<String> {
        agent_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .and_then(|id| self.agents_by_id.get(id).map(|_| id.to_string()))
    }

    pub(crate) fn task_agent_id(&self, task: &ClaimedTask) -> String {
        if let Some(payload) = task_payload_value(task) {
            if let Some(agent_id) =
                self.normalize_known_agent_id(payload.get("agent_id").and_then(|v| v.as_str()))
            {
                return agent_id;
            }
        }
        DEFAULT_AGENT_ID.to_string()
    }

    fn task_agent(&self, task: &ClaimedTask) -> AgentRuntimeConfig {
        let agent_id = self.task_agent_id(task);
        self.agents_by_id
            .get(&agent_id)
            .cloned()
            .or_else(|| self.agents_by_id.get(DEFAULT_AGENT_ID).cloned())
            .unwrap_or_else(|| AgentRuntimeConfig {
                persona_prompt: String::new(),
                restrict_skills: false,
                allowed_skills: Arc::new(HashSet::new()),
                llm_providers: Vec::new(),
            })
    }

    pub(crate) fn task_persona_prompt(&self, task: &ClaimedTask) -> String {
        let agent = self.task_agent(task);
        if !agent.persona_prompt.trim().is_empty() {
            agent.persona_prompt
        } else {
            self.persona_prompt.clone()
        }
    }

    pub(crate) fn task_allows_skill(&self, task: &ClaimedTask, canonical_skill: &str) -> bool {
        self.task_agent(task).allows_skill(canonical_skill)
    }

    fn task_llm_providers(&self, task: &ClaimedTask) -> Vec<Arc<LlmProviderRuntime>> {
        let agent = self.task_agent(task);
        if !agent.llm_providers.is_empty() {
            return agent.llm_providers;
        }
        self.llm_providers.clone()
    }

    /// 解析为 canonical 技能名：有 registry 时先查 registry，否则走代码内 canonical_skill_name。
    pub(crate) fn resolve_canonical_skill_name(&self, name: &str) -> String {
        if let Some(ref r) = self.get_skills_registry() {
            if let Some(c) = r.resolve_canonical(name) {
                return c.to_string();
            }
        }
        canonical_skill_name(name).to_string()
    }

    /// 是否 builtin 技能：有 registry 时按 kind，否则走静态白名单。
    pub(crate) fn is_builtin_skill(&self, name: &str) -> bool {
        let canonical = self.resolve_canonical_skill_name(name);
        if let Some(ref r) = self.get_skills_registry() {
            return r.is_builtin(&canonical);
        }
        is_builtin_skill_name(&canonical)
    }

    /// 该技能在 registry 中的 prompt 文件路径；无 registry 或未配置则返回 None（Phase 2 可由此处接动态加载）。
    pub(crate) fn skill_prompt_file(&self, canonical_name: &str) -> Option<String> {
        self.get_skills_registry()
            .as_ref()
            .and_then(|r| r.prompt_file(canonical_name).map(String::from))
    }

    /// Phase 3: 执行分发用。有 registry 时以 registry.kind 为准；无 registry 时兼容 fallback：builtin 白名单为 Builtin，否则 Runner。
    pub(crate) fn skill_kind_for_dispatch(&self, canonical_name: &str) -> SkillKind {
        if let Some(ref r) = self.get_skills_registry() {
            if let Some(entry) = r.get(canonical_name) {
                return entry.kind;
            }
        }
        if is_builtin_skill_name(canonical_name) {
            SkillKind::Builtin
        } else {
            SkillKind::Runner
        }
    }

    /// Phase 5: Runner 执行名；有 registry 时用 registry.runner_name，否则 canonical。
    pub(crate) fn runner_name_for_skill(&self, canonical_name: &str) -> String {
        self.get_skills_registry()
            .as_ref()
            .map(|r| r.runner_name(canonical_name))
            .unwrap_or_else(|| canonical_skill_name(canonical_name).to_string())
    }
}

#[derive(Debug, Clone)]
struct LlmProviderRuntime {
    config: LlmProviderConfig,
    client: Client,
    semaphore: Arc<Semaphore>,
}

#[derive(Debug, Clone)]
struct AgentRuntimeConfig {
    persona_prompt: String,
    restrict_skills: bool,
    allowed_skills: Arc<HashSet<String>>,
    llm_providers: Vec<Arc<LlmProviderRuntime>>,
}

impl AgentRuntimeConfig {
    fn from_config(config: &AgentConfig, llm_providers: Vec<Arc<LlmProviderRuntime>>) -> Self {
        let allowed_skills = config
            .allowed_skills
            .iter()
            .map(|skill| canonical_skill_name(skill).to_string())
            .collect::<HashSet<_>>();
        Self {
            persona_prompt: config.persona_prompt.trim().to_string(),
            restrict_skills: !allowed_skills.is_empty(),
            allowed_skills: Arc::new(allowed_skills),
            llm_providers,
        }
    }

    fn allows_skill(&self, canonical_skill: &str) -> bool {
        !self.restrict_skills || self.allowed_skills.contains(canonical_skill)
    }
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
    Feishu,
    Lark,
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

        let (limit_ok, user_q_empty_after_pop) = {
            let user_q = self.per_user.entry(user_id).or_default();
            while user_q.front().is_some_and(|v| *v < min_ts) {
                user_q.pop_front();
            }
            let empty = user_q.is_empty();
            if self.global.len() >= self.global_rpm {
                (Err("global_rpm"), empty)
            } else if user_q.len() >= self.user_rpm {
                (Err("user_rpm"), false)
            } else {
                (Ok(()), empty)
            }
        };

        if let Err("global_rpm") = limit_ok {
            if user_q_empty_after_pop {
                self.per_user.remove(&user_id);
            }
            return Err("global_rpm");
        }
        if limit_ok.is_err() {
            return limit_ok;
        }

        self.global.push_back(now);
        self.per_user.entry(user_id).or_default().push_back(now);
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
    memory::indexing::ensure_retrieval_schema(&db)?;
    if config.memory.hybrid_recall_enabled
        && (config.memory.reindex_on_startup
            || memory::indexing::retrieval_index_is_empty(&db).unwrap_or(true))
    {
        memory::indexing::rebuild_retrieval_index(&db, &config.memory)?;
    }
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
    info!(
        "Loaded LLM providers count={} (config selected_vendor={:?}, selected_model={:?})",
        llm_providers.len(),
        config.llm.selected_vendor,
        config.llm.selected_model
    );
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
    let normalized_agents = config.normalized_agents();
    let mut agents_by_id = HashMap::new();
    for agent in &normalized_agents {
        let override_providers = if agent.preferred_vendor.is_some() || agent.preferred_model.is_some()
        {
            llm_gateway::build_providers_for_selection(
                &config,
                agent.preferred_vendor.as_deref(),
                agent.preferred_model.as_deref(),
            )
        } else {
            Vec::new()
        };
        agents_by_id.insert(
            agent.id.clone(),
            AgentRuntimeConfig::from_config(agent, override_providers),
        );
    }

    let ui_dist_dir = resolve_ui_dist_dir(&workspace_root);
    let telegram_runtime_bots = config.telegram_runtime_bots();
    let telegram_bot_token = telegram_runtime_bots
        .iter()
        .map(|bot| bot.bot_token.trim())
        .find(|token| !token.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| config.telegram.bot_token.clone());
    let telegram_configured_bot_names = Arc::new(
        telegram_runtime_bots
            .iter()
            .map(|bot| bot.name.clone())
            .collect::<Vec<_>>(),
    );
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

    // Phase 4: 统一 skill 视图重建（启动与 reload 复用）
    let views = build_skill_views(
        &workspace_root,
        config.skills.registry_path.as_deref(),
        &config.skills.skill_switches,
        &config.skills.skills_list,
    )
    .map_err(|e| {
        error!("startup: build_skill_views failed: {}", e);
        anyhow::anyhow!(e)
    })?;
    let registry_entries = views
        .registry
        .as_ref()
        .map(|r| r.all_names().len())
        .unwrap_or(0);
    info!(
        "skills registry path={} entries={} execution_count={} planner_visible_count={}",
        config.skills.registry_path.as_deref().unwrap_or("(none)"),
        registry_entries,
        views.execution_skills.len(),
        views.planner_visible.len()
    );

    let feishu_send_config = load_feishu_send_config(&workspace_root);
    let lark_send_config = load_lark_send_config(&workspace_root);
    if feishu_send_config.is_some() {
        info!("feishu send config loaded for schedule push (configs/channels/feishu.toml)");
    }
    if lark_send_config.is_some() {
        info!("lark send config loaded for schedule push (configs/channels/lark.toml)");
    }

    let state = AppState {
        started_at: Instant::now(),
        queue_limit: config.worker.queue_limit,
        db: Arc::new(Mutex::new(db)),
        llm_providers,
        agents_by_id: Arc::new(agents_by_id),
        skill_timeout_seconds: config.skills.skill_timeout_seconds,
        skill_runner_path: effective_skill_runner_path,
        skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
            registry: views.registry,
            skills_list: Arc::new(views.execution_skills),
        }))),
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
        worker_task_heartbeat_seconds: config.worker.task_heartbeat_seconds.max(5),
        worker_running_no_progress_timeout_seconds: config
            .worker
            .running_no_progress_timeout_seconds
            .max(60),
        worker_running_recovery_check_interval_seconds: config
            .worker
            .running_recovery_check_interval_seconds
            .max(10),
        last_running_recovery_check_ts: Arc::new(Mutex::new(0)),
        routing,
        persona_prompt,
        command_intent,
        schedule,
        telegram_bot_token,
        telegram_configured_bot_names,
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
        feishu_send_config,
        lark_send_config,
        http_client: Client::new(),
        config_path_for_reload: "configs/config.toml".to_string(),
        registry_path_for_reload: config.skills.registry_path.clone(),
        skill_switches_for_reload: Arc::new(config.skills.skill_switches.clone()),
        initial_skills_list_for_reload: config.skills.skills_list.clone(),
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
        .route("/tasks/active", post(list_active_tasks))
        .route("/tasks/cancel", post(cancel_tasks))
        .route("/tasks/cancel-one", post(cancel_one_task))
        .route("/admin/reload-skills", post(reload_skills_handler))
        .with_state(state.clone());

    let ui_service = get_service(
        ServeDir::new(&ui_dist_dir).not_found_service(ServeFile::new(ui_index_path)),
    )
    .layer(SetResponseHeaderLayer::if_not_present(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    ));

    let app = Router::new()
        .nest("/v1", api)
        .fallback_service(ui_service)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::PUT,
                    axum::http::Method::DELETE,
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

/// 从 configs/channels/feishu.toml 加载发送配置（定时任务主动推送用）。缺文件或缺 app_id/app_secret 则返回 None。
fn load_feishu_send_config(workspace_root: &Path) -> Option<channel_send::FeishuSendConfig> {
    let path = workspace_root.join("configs/channels/feishu.toml");
    let content = std::fs::read_to_string(&path).ok()?;
    let table: TomlValue = toml::from_str(&content).ok()?;
    let feishu = table.get("feishu")?.as_table()?;
    let app_id = feishu.get("app_id")?.as_str()?.trim().to_string();
    let app_secret = feishu.get("app_secret")?.as_str()?.trim().to_string();
    if app_id.is_empty() || app_secret.is_empty() {
        return None;
    }
    let api_base_url = feishu
        .get("api_base_url")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://open.feishu.cn".to_string());
    Some(channel_send::FeishuSendConfig {
        app_id,
        app_secret,
        api_base_url,
    })
}

/// 从 configs/channels/lark.toml 加载发送配置（定时任务主动推送用）。缺文件或缺 app_id/app_secret 则返回 None。
fn load_lark_send_config(workspace_root: &Path) -> Option<channel_send::LarkSendConfig> {
    let path = workspace_root.join("configs/channels/lark.toml");
    let content = std::fs::read_to_string(&path).ok()?;
    let table: TomlValue = toml::from_str(&content).ok()?;
    let lark = table.get("lark")?.as_table()?;
    let app_id = lark.get("app_id")?.as_str()?.trim().to_string();
    let app_secret = lark.get("app_secret")?.as_str()?.trim().to_string();
    if app_id.is_empty() || app_secret.is_empty() {
        return None;
    }
    let api_base_url = lark
        .get("api_base_url")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://open.larksuite.com".to_string());
    Some(channel_send::LarkSendConfig {
        app_id,
        app_secret,
        api_base_url,
    })
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

fn recover_stale_running_tasks_by_no_progress(state: &AppState) -> anyhow::Result<Vec<String>> {
    let timeout_secs = state.worker_running_no_progress_timeout_seconds.max(60);
    let now = now_ts_u64() as i64;
    let stale_before = now.saturating_sub(timeout_secs as i64);
    let stale_note = format!(
        "auto failed: no progress heartbeat for {}s while status=running",
        timeout_secs
    );
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

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

    let changed = db.execute(
        "UPDATE tasks
         SET status = 'failed',
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
            "runtime stale-running recovery count mismatch: selected={} updated={}",
            task_ids.len(),
            changed
        );
    }
    Ok(task_ids)
}

fn maybe_recover_stale_running_tasks_runtime(state: &AppState) -> anyhow::Result<()> {
    let now = now_ts_u64();
    let interval = state.worker_running_recovery_check_interval_seconds.max(10);
    {
        let mut guard = state
            .last_running_recovery_check_ts
            .lock()
            .map_err(|_| anyhow::anyhow!("running recovery lock poisoned"))?;
        if now.saturating_sub(*guard) < interval {
            return Ok(());
        }
        *guard = now;
    }
    let recovered = recover_stale_running_tasks_by_no_progress(state)?;
    if !recovered.is_empty() {
        warn!(
            "runtime stale-running recovery applied: converted {} running tasks to failed (no_progress_timeout={}s)",
            recovered.len(),
            state.worker_running_no_progress_timeout_seconds
        );
    }
    Ok(())
}

fn start_task_heartbeat(state: AppState, task_id: String) -> oneshot::Sender<()> {
    let interval_secs = state.worker_task_heartbeat_seconds.max(5);
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {
                    if let Err(err) = repo::touch_running_task(&state, &task_id) {
                        warn!(
                            "task heartbeat update failed: task_id={} interval_secs={} err={}",
                            task_id, interval_secs, err
                        );
                    }
                }
                _ = &mut stop_rx => {
                    break;
                }
            }
        }
    });
    stop_tx
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
    if state.memory.hybrid_recall_enabled {
        let index_max_rows = state.memory.max_rows.saturating_mul(3).max(2000);
        memory::indexing::cleanup_retrieval_index(&db, memory_cutoff, index_max_rows)?;
    }

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
    maybe_recover_stale_running_tasks_runtime(state)?;

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
        let heartbeat_stop = start_task_heartbeat(state.clone(), task.task_id.clone());
        let task_result = tokio::time::timeout(Duration::from_secs(worker_timeout_secs), async {
        match task.kind.as_str() {
        "ask" => {
            let original_prompt = payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let _ = maybe_bind_recent_failed_resume_context(
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
            // Unified intent normalizer: one LLM call for resume + context resolution + schedule classification (replaces resume_followup_intent -> context_resolver -> schedule_intent).
            let (now_iso, timezone_str, schedule_rules) =
                schedule_service::schedule_context_for_normalizer(state);
            let resume_context_opt = if is_resume_continue {
                payload.get("resume_context").map(|v| v.clone())
            } else {
                None
            };
            let binding_context_json = json!({
                "source": "resume_continue_source",
                "failed_resume_context_ts": Value::Null,
                "has_newer_successful_ask_after_failed_task": false,
            });
            let normalizer_out = intent_router::run_intent_normalizer(
                state,
                &task,
                prompt,
                resume_context_opt.as_ref(),
                Some(&binding_context_json),
                &now_iso,
                &timezone_str,
                &schedule_rules,
            )
            .await;
            let resume_should_apply_context = is_resume_continue
                && normalizer_out.resume_behavior == intent_router::ResumeBehavior::ResumeExecute;
            let resume_should_discuss_context = is_resume_continue
                && normalizer_out.resume_behavior == intent_router::ResumeBehavior::ResumeDiscuss;
            // 原始用户输入，供 Pi App 等解析“用户发送”用（与 received_message 的 runtime_prompt 区分）
            info!(
                "worker_once: ask raw_message task_id={} user_id={} chat_id={} text={}",
                task.task_id,
                task.user_id,
                task.chat_id,
                truncate_for_log(prompt)
            );
            let runtime_prompt = if resume_should_apply_context {
                build_resume_continue_execute_prompt(state, &payload, prompt)
            } else if resume_should_discuss_context {
                build_resume_followup_discussion_prompt(state, &payload, prompt)
            } else {
                normalizer_out.resolved_user_intent.clone()
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
            // context_resolution from normalizer output (no second LLM for context_resolver).
            let context_resolution = intent_router::ContextResolution {
                resolved_user_intent: runtime_prompt.clone(),
                needs_clarify: normalizer_out.needs_clarify,
                confidence: Some(normalizer_out.confidence),
                reason: normalizer_out.reason.clone(),
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
            let chat_memory_budget_chars =
                dynamic_chat_memory_budget_chars(state, &task, &resolved_prompt);
            let memory_ctx = memory::service::prepare_prompt_with_memory(
                state,
                &task,
                &resolved_prompt,
                chat_memory_budget_chars,
            );
            let long_term_summary = memory_ctx.long_term_summary;
            let preferences = memory_ctx.preferences;
            let recalled = memory_ctx.recalled;
            let similar_triggers = memory_ctx.similar_triggers;
            let relevant_facts = memory_ctx.relevant_facts;
            let recent_related_events = memory_ctx.recent_related_events;
            let prompt_with_memory = memory_ctx.prompt_with_memory;
            let chat_prompt_context = memory_ctx.chat_prompt_context;
            let mut resolved_prompt_for_execution = resolved_prompt.clone();
            let mut prompt_with_memory_for_execution = prompt_with_memory.clone();
            if let Some(image_context) =
                analyze_attached_images_for_ask(state, &task, &payload, &resolved_prompt).await?
            {
                let trimmed_image_context = image_context.trim();
                if !trimmed_image_context.is_empty() {
                    let image_context_block = format!(
                        "\n\nAttached image analysis context:\n{}",
                        trimmed_image_context
                    );
                    resolved_prompt_for_execution.push_str(&image_context_block);
                    prompt_with_memory_for_execution.push_str(&image_context_block);
                }
            }
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
            let trigger_log = if similar_triggers.is_empty() {
                "<none>".to_string()
            } else {
                truncate_for_log(
                    &similar_triggers
                        .iter()
                        .map(|v| v.text.clone())
                        .collect::<Vec<_>>()
                        .join(" | "),
                )
            };
            let fact_log = if relevant_facts.is_empty() {
                "<none>".to_string()
            } else {
                truncate_for_log(
                    &relevant_facts
                        .iter()
                        .map(|v| v.text.clone())
                        .collect::<Vec<_>>()
                        .join(" | "),
                )
            };
            let related_log = if recent_related_events.is_empty() {
                "<none>".to_string()
            } else {
                truncate_for_log(
                    &recent_related_events
                        .iter()
                        .map(|v| v.text.clone())
                        .collect::<Vec<_>>()
                        .join(" | "),
                )
            };
            info!(
                "worker_once: ask memory task_id={} memory.long_term_summary={} memory.preferences={} memory.similar_triggers={} memory.relevant_facts={} memory.related_events={} memory.recalled_recent_count={} memory.recalled_recent={}",
                task.task_id,
                long_term_log,
                preferences_log,
                trigger_log,
                fact_log,
                related_log,
                recalled.len(),
                recalled_log,
            );

            // Source-id based classifier bypass list is hard_rules-driven.
            let classifier_direct_mode = main_flow_rules(state)
                .classifier_direct_sources
                .iter()
                .any(|s| s == &source.to_ascii_lowercase());

            // needs_clarify is the main signal: if normalizer says clarify, we clarify. confidence is for logging only.
            let force_clarify = context_resolution.needs_clarify;
            let has_schedule_intent =
                normalizer_out.schedule_kind != intent_router::ScheduleKind::None;
            // Schedule intent should be honored even when payload source was auto-marked as
            // resume_continue_execute by failed-task context binding, as long as normalizer
            // did not request resume execution/discussion.
            let should_route_schedule_direct =
                has_schedule_intent && !direct_resume_execution && !direct_resume_discussion;

            let result = if force_clarify {
                let clarify = intent_router::generate_clarify_question(
                    state,
                    &task,
                    &resolved_prompt_for_execution,
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
                    &resolved_prompt_for_execution,
                    &resume_prompt_file,
                )
                .await
                .map(|s| AskReply::llm(s.trim().to_string()))
            } else if direct_resume_execution {
                agent_engine::run_agent_with_tools(
                    state,
                    &task,
                    &prompt_with_memory_for_execution,
                    &resolved_prompt_for_execution,
                )
                    .await
            } else if should_route_schedule_direct {
                if let Ok(Some(schedule_reply)) =
                    intent_router::try_handle_schedule_request(
                        state,
                        &task,
                        &resolved_prompt_for_execution,
                    )
                    .await
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
                    log_prompt_render(
                        &task.task_id,
                        "classifier_direct",
                        "prompts/classifier_direct.md",
                        None,
                    );
                    llm_gateway::run_with_fallback_with_prompt_file(
                        state,
                        &task,
                        &resolved_prompt_for_execution,
                        "prompts/classifier_direct.md",
                    )
                    .await
                    .map(|s| AskReply::llm(s.trim().to_string()))
                    .map_err(|e| e.to_string())
                } else {
                    execute_ask_routed(
                        state,
                        &task,
                        &chat_prompt_context,
                        &prompt_with_memory_for_execution,
                        &resolved_prompt_for_execution,
                        agent_mode,
                        resume_should_discuss_context,
                        Some(normalizer_out.routed_mode),
                    )
                    .await
                }
            } else if classifier_direct_mode {
                log_prompt_render(
                    &task.task_id,
                    "classifier_direct",
                    "prompts/classifier_direct.md",
                    None,
                );
                llm_gateway::run_with_fallback_with_prompt_file(
                    state,
                    &task,
                    &resolved_prompt_for_execution,
                    "prompts/classifier_direct.md",
                )
                .await
                .map(|s| AskReply::llm(s.trim().to_string()))
                .map_err(|e| e.to_string())
            } else {
                execute_ask_routed(
                    state,
                    &task,
                    &chat_prompt_context,
                    &prompt_with_memory_for_execution,
                    &resolved_prompt_for_execution,
                    agent_mode,
                    false,
                    Some(normalizer_out.routed_mode),
                )
                .await
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
                    let result = if answer_messages.is_empty() {
                        json!({ "text": answer_text.clone() })
                    } else {
                        json!({ "text": answer_text.clone(), "messages": answer_messages })
                    };
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
                    let result = json!({
                        "text": clean_text,
                        "delivery_meta": {
                            "mode": "single_step_skill",
                            "label": "step",
                            "skill_name": skill_name
                        }
                    });
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
        let _ = heartbeat_stop.send(());

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
    let runtime_ch = runtime_channel_from_payload(state, payload);
    let channel_str = task.channel.trim();
    info!(
        "schedule notify push: task_id={} channel={} runtime_channel={:?}",
        task.task_id, channel_str, runtime_ch
    );
    match send_task_channel_message(state, task, payload, &message).await {
        Ok(()) => {
            info!(
                "schedule notify success: task_id={} channel={} runtime_channel={:?}",
                task.task_id, channel_str, runtime_ch
            );
        }
        Err(err) => {
            warn!(
                "schedule notify failed: task_id={} channel={} runtime_channel={:?} err={}",
                task.task_id, channel_str, runtime_ch, err
            );
        }
    }
}

fn runtime_channel_from_payload(state: &AppState, payload: &Value) -> RuntimeChannel {
    let ch = payload
        .get("channel")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if is_whatsapp_channel_value(main_flow_rules(state), &ch) {
        return RuntimeChannel::Whatsapp;
    }
    if ch == "feishu" {
        return RuntimeChannel::Feishu;
    }
    if ch == "lark" {
        return RuntimeChannel::Lark;
    }
    RuntimeChannel::Telegram
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

/// Injects recent failed resume context into payload when present. No LLM call: the single
/// intent normalizer in the ask path will later decide resume_behavior from this context.
async fn maybe_bind_recent_failed_resume_context(
    state: &AppState,
    task: &ClaimedTask,
    payload: &mut Value,
    user_text: &str,
) -> Option<()> {
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
    let (resume_context, _resume_context_ts) =
        find_recent_failed_resume_context(state, task.user_id, task.chat_id)?;
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
    Some(())
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
    if state.resolve_canonical_skill_name(skill_name) != "crypto" {
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
    let ch = task.channel.trim().to_ascii_lowercase();
    if is_whatsapp_channel_value(main_flow_rules(state), &ch) {
        return RuntimeChannel::Whatsapp;
    }
    if ch == "feishu" {
        return RuntimeChannel::Feishu;
    }
    if ch == "lark" {
        return RuntimeChannel::Lark;
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
        RuntimeChannel::Telegram => {
            channel_send::send_telegram_message(state, task.chat_id, text).await
        }
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
                    channel_send::send_whatsapp_web_bridge_text_message(state, &to, text).await
                }
                WhatsappDeliveryRoute::Cloud => {
                    channel_send::send_whatsapp_cloud_text_message(state, &to, text).await
                }
            }
        }
        RuntimeChannel::Feishu => {
            let receive_id = task_external_chat_id(task)
                .or_else(|| {
                    payload
                        .get("external_chat_id")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                })
                .ok_or_else(|| "missing external_chat_id for feishu task".to_string())?;
            channel_send::send_feishu_text_message(state, &receive_id, text).await
        }
        RuntimeChannel::Lark => {
            let receive_id = task_external_chat_id(task)
                .or_else(|| {
                    payload
                        .get("external_chat_id")
                        .and_then(|v| v.as_str())
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                })
                .ok_or_else(|| "missing external_chat_id for lark task".to_string())?;
            channel_send::send_lark_text_message(state, &receive_id, text).await
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

/// Phase 3: 统一 skill 执行入口。按 registry.kind 分发：builtin -> 进程内；runner -> skill-runner；external -> external_kind。
async fn run_skill_with_runner(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: serde_json::Value,
) -> Result<String, String> {
    let skill_name = state.resolve_canonical_skill_name(skill_name);
    if skill_name.is_empty() {
        return Err("skill_name is empty".to_string());
    }

    let policy_token = format!("skill:{skill_name}");
    if !state
        .tools_policy
        .is_allowed(&policy_token, state.active_provider_type.as_deref())
    {
        return Err(format!("blocked by policy: {policy_token}"));
    }

    if !state.get_skills_list().contains(&skill_name) {
        let mut allowed: Vec<String> = state.get_skills_list().iter().cloned().collect();
        allowed.sort();
        let enabled = allowed.join(", ");
        let err_text = i18n_t_with_default(
            state,
            "clawd.msg.skill_disabled_with_enabled_list",
            "Skill is not enabled: {skill}. Please enable it in config and try again. (Currently enabled: {enabled_skills})",
        )
        .replace("{skill}", &skill_name)
        .replace("{enabled_skills}", &enabled);
        return Err(err_text);
    }
    if !state.task_allows_skill(task, &skill_name) {
        return Err(format!(
            "Skill is not enabled for agent {}: {}",
            state.task_agent_id(task),
            skill_name
        ));
    }

    let kind = state.skill_kind_for_dispatch(&skill_name);
    let kind_str = match kind {
        SkillKind::Builtin => "builtin",
        SkillKind::Runner => "runner",
        SkillKind::External => "external",
    };
    info!(
        "skill_dispatch skill={} kind={} branch={}",
        skill_name, kind_str, kind_str
    );

    match kind {
        SkillKind::Builtin => {
            return execute_builtin_skill(state, &skill_name, &args).await;
        }
        SkillKind::External | SkillKind::Runner => {}
    }

    let skill_timeout_secs = state
        .get_skills_registry()
        .as_ref()
        .and_then(|r| {
            let s = r.timeout_seconds(&skill_name);
            if s > 0 {
                Some(state.skill_timeout_seconds.max(s))
            } else {
                None
            }
        })
        .unwrap_or_else(|| match skill_name.as_str() {
            "image_generate" | "image_edit" => state.skill_timeout_seconds.max(180),
            "image_vision" => state.skill_timeout_seconds.max(90),
            "audio_transcribe" => state.skill_timeout_seconds.max(120),
            "audio_synthesize" => state.skill_timeout_seconds.max(90),
            "crypto" => state.skill_timeout_seconds.max(60),
            _ => state.skill_timeout_seconds,
        });

    let _permit = state
        .skill_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| format!("skill semaphore closed: {err}"))?;

    let args = enrich_skill_args_with_memory(state, task, &skill_name, args).await;
    let args = inject_skill_memory_context(state, task, &skill_name, args);
    let args = ensure_default_output_dir_for_skill_args(&state.workspace_root, &skill_name, args);
    let source = match task_runtime_channel(state, task) {
        RuntimeChannel::Whatsapp => "whatsapp",
        RuntimeChannel::Telegram => "telegram",
        RuntimeChannel::Feishu => "feishu",
        RuntimeChannel::Lark => "lark",
    };

    let mut value = match kind {
        SkillKind::External => {
            execute_external_skill(state, task, &skill_name, &args, &source).await?
        }
        SkillKind::Runner => {
            let runner_name = state.runner_name_for_skill(&skill_name);
            info!(
                "skill_dispatch skill={} runner_name={} kind=runner",
                skill_name, runner_name
            );
            run_skill_with_runner_once(
                state,
                task,
                &skill_name,
                &runner_name,
                &args,
                &source,
                skill_timeout_secs,
            )
            .await?
        }
        SkillKind::Builtin => unreachable!(),
    };
    let mut status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("error")
        .to_string();

    if status != "ok" && canonical_skill_name(&skill_name) == "crypto" {
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
                let runner_name = state.runner_name_for_skill(&skill_name);
                let retry_value = run_skill_with_runner_once(
                    state,
                    task,
                    &skill_name,
                    &runner_name,
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
    if canonical_skill_name(&skill_name) == "image_vision" {
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

async fn execute_external_skill(
    state: &AppState,
    task: &ClaimedTask,
    canonical_skill_name: &str,
    args: &Value,
    source: &str,
) -> Result<Value, String> {
    let reg = state
        .get_skills_registry()
        .ok_or_else(|| "external skill requires registry".to_string())?;
    let config = reg
        .external_config(canonical_skill_name)
        .ok_or_else(|| "external skill missing external_kind in registry".to_string())?;
    match config.kind {
        "http_json" => {
            execute_external_http_json(state, task, canonical_skill_name, args, source).await
        }
        "local_shell_recipe" => {
            execute_external_local_shell_recipe(state, task, canonical_skill_name, args, source)
                .await
        }
        "local_script" => {
            execute_external_local_script(state, task, canonical_skill_name, args, source).await
        }
        "prompt_bundle" => Ok(json!({
            "request_id": task.task_id,
            "status": "error",
            "text": "",
            "error_text": format!(
                "Imported external skill preview is registered, but runtime execution for external_kind={} is not enabled yet.",
                config.kind
            )
        })),
        other => Err(format!("external_kind not supported: {other}")),
    }
}

fn external_reserved_arg_key(key: &str) -> bool {
    if key.starts_with('_') {
        return true;
    }
    matches!(
        key,
        "action"
            | "output_dir"
            | "response_language"
            | "language"
            | "confirm"
            | "dry_run"
            | "timeout_seconds"
            | "source"
            | "skill_name"
    )
}

fn value_to_cli_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn build_external_cli_args(args: &Value) -> Vec<String> {
    if let Some(cli_args) = args.get("cli_args").and_then(|v| v.as_array()) {
        let collected: Vec<String> = cli_args
            .iter()
            .filter_map(|value| value.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect();
        if !collected.is_empty() {
            return collected;
        }
    }

    for key in ["command", "script", "recipe"] {
        if let Some(raw) = args.get(key).and_then(|v| v.as_str()) {
            let collected = raw
                .split_whitespace()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            if !collected.is_empty() {
                return collected;
            }
        }
    }

    let Some(map) = args.as_object() else {
        return Vec::new();
    };

    let mut cli_args = Vec::new();
    for (key, value) in map {
        if external_reserved_arg_key(key) || key == "cli_args" {
            continue;
        }
        let flag = format!("--{}", key.replace('_', "-"));
        match value {
            Value::Bool(true) => cli_args.push(flag),
            Value::Bool(false) | Value::Null => {}
            Value::Array(items) => {
                for item in items {
                    cli_args.push(flag.clone());
                    cli_args.push(value_to_cli_string(item));
                }
            }
            other => {
                cli_args.push(flag);
                cli_args.push(value_to_cli_string(other));
            }
        }
    }
    cli_args
}

fn resolve_external_bundle_dir(state: &AppState, bundle_rel: &str) -> Result<PathBuf, String> {
    if bundle_rel.trim().is_empty() {
        return Err("external skill missing external_bundle_dir".to_string());
    }
    let joined = state.workspace_root.join(bundle_rel);
    let canonical = joined
        .canonicalize()
        .map_err(|err| format!("external bundle directory not found: {err}"))?;
    if !canonical.starts_with(&state.workspace_root) {
        return Err("external bundle directory must stay inside workspace_root".to_string());
    }
    Ok(canonical)
}

fn resolve_external_entry_path(bundle_dir: &Path, entry_rel: &str) -> Result<PathBuf, String> {
    if entry_rel.trim().is_empty() {
        return Err("external skill missing external_entry_file".to_string());
    }
    let entry_path = bundle_dir.join(entry_rel);
    let canonical = entry_path
        .canonicalize()
        .map_err(|err| format!("external entry file not found: {err}"))?;
    if !canonical.starts_with(bundle_dir) {
        return Err("external entry file must stay inside the imported bundle".to_string());
    }
    Ok(canonical)
}

fn is_bin_available(bin: &str) -> bool {
    let bin = bin.trim();
    if bin.is_empty() {
        return false;
    }
    if bin.contains('/') {
        return Path::new(bin).is_file();
    }
    let Some(path_env) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_env).any(|dir| dir.join(bin).is_file())
}

async fn verify_external_python_modules(
    runtime: &str,
    modules: &[String],
    bundle_dir: &Path,
) -> Result<(), String> {
    if modules.is_empty() {
        return Ok(());
    }
    let imports = modules.join(",");
    let mut cmd = Command::new(runtime);
    cmd.arg("-c")
        .arg(format!("import {imports}"))
        .current_dir(bundle_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .map_err(|_| "checking Python dependencies timed out".to_string())?
        .map_err(|err| format!("checking Python dependencies failed: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    Err(format!(
        "missing Python dependencies for imported skill: {}{}",
        modules.join(", "),
        if detail.is_empty() {
            String::new()
        } else {
            format!(" ({detail})")
        }
    ))
}

async fn execute_external_local_script(
    state: &AppState,
    task: &ClaimedTask,
    canonical_skill_name: &str,
    args: &Value,
    source: &str,
) -> Result<Value, String> {
    let reg = state
        .get_skills_registry()
        .ok_or_else(|| "external skill requires registry".to_string())?;
    let config = reg
        .external_config(canonical_skill_name)
        .ok_or_else(|| "external skill missing execution config".to_string())?;
    if config.kind != "local_script" {
        return Err(format!(
            "external_kind not supported by local_script executor: {}",
            config.kind
        ));
    }

    for bin in config.require_bins {
        if !is_bin_available(bin) {
            return Err(format!(
                "missing required local command for imported skill: {}",
                bin
            ));
        }
    }

    let bundle_dir =
        resolve_external_bundle_dir(state, config.bundle_dir.unwrap_or_default())?;
    let entry_rel = config
        .entry_file
        .ok_or_else(|| "external skill missing external_entry_file".to_string())?;
    let entry_path = resolve_external_entry_path(&bundle_dir, entry_rel)?;
    let runtime = config
        .runtime
        .map(str::to_string)
        .or_else(|| {
            if entry_rel.ends_with(".py") {
                Some("python3".to_string())
            } else if entry_rel.ends_with(".js")
                || entry_rel.ends_with(".mjs")
                || entry_rel.ends_with(".cjs")
            {
                Some("node".to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| "external skill missing external_runtime".to_string())?;

    if runtime.starts_with("python") {
        verify_external_python_modules(&runtime, config.require_py_modules, &bundle_dir).await?;
    }

    let cli_args = build_external_cli_args(args);
    info!(
        "skill_dispatch external skill={} external_kind=local_script runtime={} entry={} cli_args={:?} source={}",
        canonical_skill_name,
        runtime,
        entry_rel,
        cli_args,
        source
    );

    let timeout_secs = config
        .timeout_seconds
        .unwrap_or(state.skill_timeout_seconds)
        .max(1);
    let entry_arg = entry_path
        .strip_prefix(&bundle_dir)
        .unwrap_or(&entry_path)
        .to_string_lossy()
        .to_string();

    let mut cmd = Command::new(&runtime);
    cmd.arg(&entry_arg);
    for arg in &cli_args {
        cmd.arg(arg);
    }
    cmd.current_dir(&bundle_dir)
        .env("WORKSPACE_ROOT", state.workspace_root.display().to_string())
        .env("RUSTCLAW_IMPORTED_SKILL", canonical_skill_name)
        .env("RUSTCLAW_TASK_ID", task.task_id.clone())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output())
        .await
        .map_err(|_| {
            format!(
                "imported external skill timed out after {}s",
                timeout_secs
            )
        })?
        .map_err(|err| format!("run imported external skill failed: {err}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if output.status.success() {
        let text = if !stdout.is_empty() && !stderr.is_empty() {
            format!("{stdout}\n\n{stderr}")
        } else if !stdout.is_empty() {
            stdout
        } else if !stderr.is_empty() {
            stderr
        } else {
            "Imported external skill completed with no output.".to_string()
        };
        return Ok(json!({
            "request_id": task.task_id,
            "status": "ok",
            "text": text,
            "error_text": Value::Null,
            "extra": {
                "external_kind": config.kind,
                "runtime": runtime,
                "entry_file": entry_rel,
                "cli_args": cli_args,
            }
        }));
    }

    let exit_code = output.status.code().unwrap_or(-1);
    let mut detail = String::new();
    if !stderr.is_empty() {
        detail.push_str(&stderr);
    }
    if !stdout.is_empty() {
        if !detail.is_empty() {
            detail.push_str("\n\n");
        }
        detail.push_str(&stdout);
    }
    if detail.is_empty() {
        detail = format!("process exited with code {}", exit_code);
    }

    Ok(json!({
        "request_id": task.task_id,
        "status": "error",
        "text": "",
        "error_text": format!("Imported external skill failed (exit={}): {}", exit_code, detail),
        "extra": {
            "external_kind": config.kind,
            "runtime": runtime,
            "entry_file": entry_rel,
            "cli_args": cli_args,
        }
    }))
}

fn extract_external_shell_command(args: &Value) -> Result<String, String> {
    if let Some(command) = args.get("command").and_then(|v| v.as_str()) {
        let trimmed = command.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(command) = args.get("script").and_then(|v| v.as_str()) {
        let trimmed = command.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(command) = args.get("recipe").and_then(|v| v.as_str()) {
        let trimmed = command.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    Err(
        "Imported shell skill needs a command string in args.command (or args.script / args.recipe)."
            .to_string(),
    )
}

async fn execute_external_local_shell_recipe(
    state: &AppState,
    task: &ClaimedTask,
    canonical_skill_name: &str,
    args: &Value,
    source: &str,
) -> Result<Value, String> {
    let reg = state
        .get_skills_registry()
        .ok_or_else(|| "external skill requires registry".to_string())?;
    let config = reg
        .external_config(canonical_skill_name)
        .ok_or_else(|| "external skill missing execution config".to_string())?;
    if config.kind != "local_shell_recipe" {
        return Err(format!(
            "external_kind not supported by local_shell_recipe executor: {}",
            config.kind
        ));
    }

    for bin in config.require_bins {
        if !is_bin_available(bin) {
            return Err(format!(
                "missing required local command for imported skill: {}",
                bin
            ));
        }
    }

    let bundle_dir =
        resolve_external_bundle_dir(state, config.bundle_dir.unwrap_or_default())?;
    let command = extract_external_shell_command(args)?;
    let timeout_secs = config
        .timeout_seconds
        .unwrap_or(state.cmd_timeout_seconds)
        .max(1);

    info!(
        "skill_dispatch external skill={} external_kind=local_shell_recipe command={} source={}",
        canonical_skill_name,
        truncate_for_log(&command),
        source
    );

    match run_safe_command(
        &bundle_dir,
        &command,
        state.max_cmd_length,
        timeout_secs,
        state.allow_sudo,
    )
    .await
    {
        Ok(text) => Ok(json!({
            "request_id": task.task_id,
            "status": "ok",
            "text": text,
            "error_text": Value::Null,
            "extra": {
                "external_kind": config.kind,
                "command": command,
            }
        })),
        Err(err) => Ok(json!({
            "request_id": task.task_id,
            "status": "error",
            "text": "",
            "error_text": format!("Imported shell skill failed: {err}"),
            "extra": {
                "external_kind": config.kind,
                "command": command,
            }
        })),
    }
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

/// Phase 6: 解析并解析 external_auth_ref（从环境变量等取 secret），不打印 secret。
/// 约定：`env:VAR` → 从环境变量 VAR 取值，注入 `Authorization: Bearer <value>`；
///       `env:VAR:header:HeaderName` → 从环境变量 VAR 取值，注入 `HeaderName: <value>`（不加重 Bearer 前缀）。
/// 返回 (header_name, header_value)；取不到 secret 时返回 Err。
fn resolve_external_auth(auth_ref: Option<&str>) -> Result<Option<(String, String)>, String> {
    let s = match auth_ref {
        Some(x) => x.trim(),
        None => return Ok(None),
    };
    if s.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = s.splitn(4, ':').collect();
    let auth_type = parts.get(0).map(|x| x.trim()).unwrap_or("");
    if auth_type != "env" {
        return Err(format!(
            "external_auth_ref unsupported type: {:?}, only env is supported",
            auth_type
        ));
    }
    let var_name = parts.get(1).map(|x| x.trim()).filter(|x| !x.is_empty());
    let Some(var_name) = var_name else {
        return Err("external_auth_ref env: missing variable name".to_string());
    };
    let (header_name, use_bearer) = if parts.get(2) == Some(&"header") {
        let h = parts.get(3).map(|x| x.trim()).filter(|x| !x.is_empty());
        let Some(h) = h else {
            return Err("external_auth_ref env:var:header: missing header name".to_string());
        };
        (h.to_string(), false)
    } else {
        ("Authorization".to_string(), true)
    };
    let value = std::env::var(var_name).map_err(|_| {
        format!(
            "external_auth_ref env:{} not set or empty (set the environment variable)",
            var_name
        )
    })?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!(
            "external_auth_ref env:{} is empty (set the environment variable)",
            var_name
        ));
    }
    let header_value = if use_bearer {
        format!("Bearer {}", value)
    } else {
        value.to_string()
    };
    Ok(Some((header_name, header_value)))
}

/// Phase 5: 脱敏 endpoint 用于日志（仅保留 scheme+host，路径用 ...）
fn mask_endpoint_for_log(endpoint: &str) -> String {
    let s = endpoint.trim();
    if s.is_empty() {
        return "<empty>".to_string();
    }
    if let Some((scheme, rest)) = s.split_once("://") {
        if let Some(after) = rest.find('/') {
            return format!("{}://{}...", scheme, rest.split_at(after).0);
        }
        return format!("{}://...", scheme);
    }
    if s.len() > 32 {
        return format!("{}...", &s[..32.min(s.len())]);
    }
    s.to_string()
}

/// Phase 5: External 技能 http_json 执行。请求体含 skill/args/task_id/source；响应含 ok/text/messages/file/image_file/error，转为与 runner 一致的 Value 形状。
async fn execute_external_http_json(
    state: &AppState,
    task: &ClaimedTask,
    canonical_skill_name: &str,
    args: &Value,
    source: &str,
) -> Result<Value, String> {
    use claw_core::skill_registry::ExternalSkillConfig;

    let reg = state
        .get_skills_registry()
        .ok_or_else(|| "external skill requires registry".to_string())?;
    let config: ExternalSkillConfig<'_> =
        reg.external_config(canonical_skill_name).ok_or_else(|| {
            "external skill missing external_kind or external_endpoint in registry".to_string()
        })?;
    if config.kind != "http_json" {
        return Err(format!(
            "external_kind not supported: {}, only http_json is supported",
            config.kind
        ));
    }
    let timeout_secs = config
        .timeout_seconds
        .unwrap_or(state.skill_timeout_seconds)
        .max(1);
    let endpoint = config
        .endpoint
        .ok_or_else(|| "external http_json skill missing external_endpoint".to_string())?;
    let endpoint_masked = mask_endpoint_for_log(endpoint);

    let auth_header = match resolve_external_auth(config.auth_ref) {
        Ok(Some((name, value))) => {
            info!(
                "skill_dispatch external skill={} external_kind={} external_endpoint={} auth_ref_type=env auth_header={} auth_resolved=ok",
                canonical_skill_name, config.kind, endpoint_masked, name
            );
            Some((name, value))
        }
        Ok(None) => {
            info!(
                "skill_dispatch external skill={} external_kind={} external_endpoint={} auth_ref=none",
                canonical_skill_name, config.kind, endpoint_masked
            );
            None
        }
        Err(e) => {
            warn!(
                "skill_dispatch external skill={} external_endpoint={} auth_ref_type=env auth_resolved=fail err={}",
                canonical_skill_name, endpoint_masked, e
            );
            return Err(e);
        }
    };

    let body = json!({
        "skill": canonical_skill_name,
        "args": args,
        "task_id": task.task_id,
        "source": source,
    });

    let timeout = Duration::from_secs(timeout_secs);
    let mut req = state
        .http_client
        .post(endpoint)
        .json(&body)
        .timeout(timeout);
    if let Some((name, value)) = auth_header {
        req = req.header(name.as_str(), value);
    }
    let res = req.send().await.map_err(|e| {
        let msg = format!("external http_json request failed: {}", e);
        warn!(
            "skill_dispatch external request failed skill={} endpoint={} err={}",
            canonical_skill_name, endpoint_masked, e
        );
        msg
    })?;

    let status_code = res.status();
    let resp_body = res.text().await.map_err(|e| {
        let msg = format!("external http_json read body failed: {}", e);
        warn!(
            "skill_dispatch external read_body failed skill={} err={}",
            canonical_skill_name, e
        );
        msg
    })?;

    if !status_code.is_success() {
        warn!(
            "skill_dispatch external response non-2xx skill={} endpoint={} status={} body_len={}",
            canonical_skill_name,
            endpoint_masked,
            status_code,
            resp_body.len()
        );
        return Err(format!(
            "external endpoint returned {}: {}",
            status_code,
            resp_body.chars().take(200).collect::<String>()
        ));
    }

    let parsed: Value = serde_json::from_str(&resp_body).map_err(|e| {
        let msg = format!("external http_json response parse failed: {}", e);
        warn!(
            "skill_dispatch external response parse failed skill={} err={} raw_len={}",
            canonical_skill_name,
            e,
            resp_body.len()
        );
        msg
    })?;

    let ok = parsed.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let status = if ok { "ok" } else { "error" };
    let error_str = parsed
        .get("error")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let text_str = parsed
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or_default();
    let messages: Vec<&str> = parsed
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let file_path = parsed
        .get("file")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let image_file_path = parsed
        .get("image_file")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let mut text = text_str.to_string();
    for m in messages {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(m);
    }
    if let Some(p) = file_path {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("FILE: ");
        text.push_str(p);
    }
    if let Some(p) = image_file_path {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("IMAGE_FILE: ");
        text.push_str(p);
    }

    info!(
        "skill_dispatch external response_parse_ok skill={} status={} text_len={}",
        canonical_skill_name,
        status,
        text.len()
    );

    let error_text = if ok {
        ""
    } else {
        error_str.unwrap_or("external returned ok=false")
    };
    let value = json!({
        "status": status,
        "text": text,
        "error_text": error_text,
    });
    Ok(value)
}

async fn run_skill_with_runner_once(
    state: &AppState,
    task: &ClaimedTask,
    canonical_skill_name: &str,
    runner_name: &str,
    args: &serde_json::Value,
    source: &str,
    skill_timeout_secs: u64,
) -> Result<serde_json::Value, String> {
    let credential_context = if canonical_skill_name == "crypto" {
        exchange_credential_context_for_task(state, task)
    } else {
        json!({})
    };
    let llm_skill = canonical_skill_name == "chat";
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
        "skill_name": runner_name,
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

    let selected_openai_model = llm_gateway::selected_openai_model(state, Some(task));
    let mut child = Command::new(&state.skill_runner_path)
        .env("SKILL_TIMEOUT_SECONDS", skill_timeout_secs.to_string())
        .env(
            "OPENAI_API_KEY",
            llm_gateway::selected_openai_api_key(state, Some(task)),
        )
        .env(
            "OPENAI_BASE_URL",
            llm_gateway::selected_openai_base_url(state, Some(task)),
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
    let structured = memory::service::recall_structured_memory_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        &anchor,
        state.memory.recall_limit.max(1),
        true,
        true,
    );
    let memory_context = memory::service::structured_memory_context_block(
        &structured,
        memory::retrieval::MemoryContextMode::Skill,
        state.memory.skill_memory_max_chars.max(384),
    );
    let mut pref_map = serde_json::Map::new();
    for (k, v) in &structured.preferences {
        pref_map.insert(k.clone(), Value::String(v.clone()));
    }
    let lang_hint =
        memory::service::preferred_response_language(&structured.preferences).unwrap_or_default();
    obj.insert(
        "_memory".to_string(),
        json!({
            "context": memory_context,
            "long_term_summary": structured.long_term_summary.clone().unwrap_or_default(),
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
    let structured = memory::service::recall_structured_memory_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        "infer language preference",
        state.memory.recall_limit.max(1),
        state.memory.image_memory_include_long_term,
        state.memory.image_memory_include_preferences,
    );
    let memory_context = memory::service::structured_memory_context_block(
        &structured,
        memory::retrieval::MemoryContextMode::Image,
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
        structured.recalled_recent.len()
            + structured.similar_triggers.len()
            + structured.relevant_facts.len()
            + structured.recent_related_events.len(),
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

fn selected_openai_api_key_for_task(state: &AppState, task: Option<&ClaimedTask>) -> String {
    let providers = task
        .map(|task| state.task_llm_providers(task))
        .unwrap_or_else(|| state.llm_providers.clone());
    if let Some(p) = providers
        .iter()
        .find(|p| p.config.provider_type == "openai_compat")
    {
        return p.config.api_key.clone();
    }
    String::new()
}

fn selected_openai_base_url_for_task(state: &AppState, task: Option<&ClaimedTask>) -> String {
    let providers = task
        .map(|task| state.task_llm_providers(task))
        .unwrap_or_else(|| state.llm_providers.clone());
    if let Some(p) = providers
        .iter()
        .find(|p| p.config.provider_type == "openai_compat")
    {
        return p.config.base_url.clone();
    }
    "https://api.openai.com/v1".to_string()
}

fn selected_openai_model_for_task(state: &AppState, task: Option<&ClaimedTask>) -> String {
    let providers = task
        .map(|task| state.task_llm_providers(task))
        .unwrap_or_else(|| state.llm_providers.clone());
    providers
        .iter()
        .find(|p| p.config.provider_type == "openai_compat")
        .map(|p| p.config.model.clone())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "gpt-4o-mini".to_string())
}

fn dynamic_chat_memory_budget_chars(state: &AppState, task: &ClaimedTask, request_text: &str) -> usize {
    let configured_budget = state.memory.chat_memory_budget_chars.max(384);
    let prompt_budget_cap = state.memory.prompt_max_chars.max(384);
    let providers = state.task_llm_providers(task);
    if providers.is_empty() {
        return configured_budget.min(prompt_budget_cap);
    }
    let min_context_tokens = providers
        .iter()
        .map(|p| estimate_context_window_tokens(p))
        .min()
        .unwrap_or(32_000)
        .max(8_000);
    // Reserve output and control prompt overhead to keep headroom for provider formatting.
    let output_reserve_tokens = 4_096usize.min(min_context_tokens / 3).max(768);
    let fixed_overhead_tokens = 1_200usize;
    let request_tokens = estimate_text_tokens(request_text);
    let available_tokens = min_context_tokens
        .saturating_sub(output_reserve_tokens)
        .saturating_sub(fixed_overhead_tokens)
        .saturating_sub(request_tokens);
    // Keep memory context as a bounded fraction of remaining context.
    let dynamic_tokens = (available_tokens / 4).clamp(192, 8_000);
    let dynamic_chars = dynamic_tokens.saturating_mul(2);
    let dynamic_budget = dynamic_chars.clamp(384, prompt_budget_cap);
    info!(
        "{} dynamic_chat_memory_budget task_id={} configured={} computed={} cap={} min_ctx_tokens={} request_tokens={}",
        highlight_tag("memory"),
        task.task_id,
        configured_budget,
        dynamic_budget,
        prompt_budget_cap,
        min_context_tokens,
        request_tokens
    );
    dynamic_budget
}

fn estimate_context_window_tokens(provider: &LlmProviderRuntime) -> usize {
    let model = provider.config.model.trim().to_ascii_lowercase();
    if let Some(explicit) = extract_model_k_or_m_capacity_tokens(&model) {
        return explicit.max(8_000);
    }
    match provider.config.provider_type.as_str() {
        "anthropic_claude" => 200_000,
        "google_gemini" => 256_000,
        "openai_compat" => {
            if model.contains("gpt-4.1")
                || model.contains("gpt-4o")
                || model.contains("o3")
                || model.contains("o4")
            {
                128_000
            } else if model.contains("gpt-3.5") {
                16_000
            } else if model.contains("deepseek") {
                64_000
            } else if model.contains("qwen") {
                32_000
            } else {
                64_000
            }
        }
        _ => 64_000,
    }
}

fn extract_model_k_or_m_capacity_tokens(model_lower: &str) -> Option<usize> {
    let bytes = model_lower.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if !bytes[idx].is_ascii_digit() {
            idx += 1;
            continue;
        }
        let start = idx;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx >= bytes.len() {
            break;
        }
        let number = model_lower[start..idx].parse::<usize>().ok()?;
        let unit = bytes[idx];
        if unit == b'k' {
            return Some(number.saturating_mul(1_000));
        }
        if unit == b'm' {
            return Some(number.saturating_mul(1_000_000));
        }
        idx += 1;
    }
    None
}

fn estimate_text_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    let mut cjk_count = 0usize;
    for ch in text.chars() {
        if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
            cjk_count += 1;
        }
    }
    if cjk_count * 2 >= chars.max(1) {
        chars.max(1)
    } else {
        chars.div_ceil(3).max(1)
    }
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
    let task_providers = state.task_llm_providers(task);
    if task_providers.is_empty() {
        return Err("No available LLM provider configured".to_string());
    }

    let mut last_error = "unknown llm error".to_string();

    for provider in &task_providers {
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
            Ok(output) => {
                let (cleaned_text, sanitized) =
                    maybe_sanitize_llm_text_output(vendor, &output.text);
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
                    prompt_file,
                    prompt,
                    &output.request_payload,
                    Some(&output.raw_response),
                    Some(&cleaned_text),
                    output.usage.as_ref(),
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
                    prompt_file,
                    prompt,
                    &err.request_payload,
                    err.raw_response.as_deref(),
                    None,
                    err.usage.as_ref(),
                    false,
                    Some(&err.message),
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
        let ts = value.get("ts").and_then(|item| item.as_i64()).filter(|v| *v > 0);
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
    let detail_trimmed = detail.trim();
    if detail_trimmed.is_empty() {
        subtask_results.push(format!("subtask#{index} {action_label}: {status}"));
    } else {
        let header = format!("subtask#{index} {action_label}: {status}");
        subtask_results.push(format!("{}\n{}", header, detail_trimmed));
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

/// Secondary mode only: goal suffix when user explicitly asked for execute + summary (see intent_normalizer_prompt).
fn chat_act_goal_from_prompt(prompt_with_memory: &str) -> String {
    format!(
        "{}\n\nMode hint: chat_act. Complete required actions first, then return a concise user-facing reply that confirms results naturally.",
        prompt_with_memory
    )
}

/// Single implementation for chat / act / chat_act / ask_clarify.
/// Ask main path always passes Some(normalizer_out.routed_mode). When normalizer_mode is Some(...),
/// we use it so that explicit-execute intents (e.g. "执行ls") get Act regardless of payload agent_mode.
/// When normalizer_mode is None (parse failure or legacy path), we fall back to route_request_mode only if agent_mode.
async fn execute_ask_routed(
    state: &AppState,
    task: &ClaimedTask,
    chat_prompt_context: &str,
    prompt_with_memory: &str,
    resolved_prompt: &str,
    agent_mode: bool,
    resume_force_chat: bool,
    normalizer_mode: Option<RoutedMode>,
) -> Result<AskReply, String> {
    let (routed_mode, used_fallback_router, override_reason) = if resume_force_chat {
        (RoutedMode::Chat, false, Some("resume_force_chat"))
    } else if let Some(m) = normalizer_mode {
        // Normalizer already decided; respect it so Feishu/any client explicit-execute (e.g. "执行ls") gets Act.
        (m, false, None)
    } else if agent_mode {
        let mode = intent_router::route_request_mode(state, task, resolved_prompt).await;
        (mode, true, None)
    } else {
        (
            RoutedMode::Chat,
            false,
            Some("normalizer_mode=None and agent_mode=false"),
        )
    };
    info!(
        "{} worker_once: ask task_id={} normalizer_mode={:?} routed_mode={:?} agent_mode={} used_fallback_router={} override={}",
        highlight_tag("routing"),
        task.task_id,
        normalizer_mode,
        routed_mode,
        agent_mode,
        used_fallback_router,
        override_reason.unwrap_or("")
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
            let task_persona_prompt = state.task_persona_prompt(task);
            let chat_prompt = render_prompt_template(
                &chat_prompt_template,
                &[
                    ("__PERSONA_PROMPT__", &task_persona_prompt),
                    ("__CONTEXT__", chat_prompt_context),
                    ("__REQUEST__", resolved_prompt),
                ],
            );
            llm_gateway::run_with_fallback_with_prompt_file(
                state,
                task,
                &chat_prompt,
                &chat_prompt_file,
            )
            .await
            .map(|s| AskReply::llm(s))
            .map_err(|e| e.to_string())
        }
        RoutedMode::Act => {
            agent_engine::run_agent_with_tools(state, task, prompt_with_memory, resolved_prompt)
                .await
        }
        RoutedMode::ChatAct => {
            agent_engine::run_agent_with_tools(
                state,
                task,
                &chat_act_goal_from_prompt(prompt_with_memory),
                resolved_prompt,
            )
            .await
        }
        RoutedMode::AskClarify => {
            let clarify = intent_router::generate_clarify_question(
                state,
                task,
                resolved_prompt,
                "router_selected_ask_clarify",
            )
            .await;
            Ok(AskReply::non_llm(clarify))
        }
    }
}

async fn analyze_attached_images_for_ask(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    resolved_prompt: &str,
) -> anyhow::Result<Option<String>> {
    let Some(images) = payload.get("images").and_then(|v| v.as_array()) else {
        return Ok(None);
    };
    if images.is_empty() {
        return Ok(None);
    }
    let mut args = json!({
        "action": "describe",
        "images": images,
    });
    let instruction = resolved_prompt.trim();
    if let Some(obj) = args.as_object_mut() {
        if !instruction.is_empty() {
            obj.insert(
                "instruction".to_string(),
                Value::String(instruction.to_string()),
            );
        }
        if let Some(language) = payload
            .get("response_language")
            .or_else(|| payload.get("language"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            obj.insert(
                "response_language".to_string(),
                Value::String(language.to_string()),
            );
        }
    }
    run_skill_with_runner(state, task, "image_vision", args)
        .await
        .map_err(anyhow::Error::msg)
        .map(Some)
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
    let structured = memory::service::recall_structured_memory_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        goal,
        state.memory.recall_limit.max(1),
        state.memory.image_memory_include_long_term,
        state.memory.image_memory_include_preferences,
    );
    let memory_text = memory::service::structured_memory_context_block(
        &structured,
        memory::retrieval::MemoryContextMode::Image,
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

pub(crate) fn parse_agent_action_json_with_repair(
    raw: &str,
    state: &AppState,
) -> Result<Value, String> {
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
    Ok(normalize_agent_action_shape(parsed, state))
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

fn normalize_agent_action_shape(value: Value, state: &AppState) -> Value {
    let Some(obj) = value.as_object() else {
        return value;
    };
    let Some(raw_type) = obj.get("type").and_then(|v| v.as_str()) else {
        if let Some(skill) = obj.get("skill").and_then(|v| v.as_str()) {
            let normalized_skill = state.resolve_canonical_skill_name(skill.trim());
            if state.is_builtin_skill(&normalized_skill) {
                let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
                return json!({
                    "type": "call_skill",
                    "skill": normalized_skill,
                    "args": args,
                });
            }
        }
        if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
            let normalized_tool = state.resolve_canonical_skill_name(tool.trim());
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
                let normalized_tool = state.resolve_canonical_skill_name(tool.trim());
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
    if state.is_builtin_skill(&step_type) {
        return json!({
            "type": "call_skill",
            "skill": step_type,
            "args": args,
        });
    }

    let normalized_skill = state.resolve_canonical_skill_name(&step_type);
    if state.is_builtin_skill(&normalized_skill) {
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

/// Fallback 仅当无 registry 时使用：只包含 execute_builtin_skill 真正支持的进程内技能，避免把 runner skill 错判成 builtin。
pub(crate) fn is_builtin_skill_name(name: &str) -> bool {
    matches!(
        name,
        "run_cmd" | "read_file" | "write_file" | "list_dir" | "make_dir" | "remove_file"
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
        return format!("{default_dir}/artifact-{}.txt", now_ts_u64());
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
                return Err(
                    "remove_file only supports files; use run_cmd for directory removal"
                        .to_string(),
                );
            }
            std::fs::remove_file(&real_path).map_err(|err| format!("remove_file failed: {err}"))?;
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
) -> Result<LlmProviderResponse, ProviderError> {
    let mut attempts = 0usize;

    loop {
        attempts += 1;
        match call_provider(provider.clone(), prompt).await {
            Ok(output) => return Ok(output),
            Err(err) if err.retryable => {
                if attempts > LLM_RETRY_TIMES {
                    return Err(err);
                }
                tokio::time::sleep(Duration::from_millis(250 * attempts as u64)).await;
            }
            Err(err) => return Err(err),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct LlmUsageSnapshot {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    cached_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
struct LlmProviderResponse {
    text: String,
    request_payload: Value,
    raw_response: String,
    usage: Option<LlmUsageSnapshot>,
}

#[derive(Debug, Clone)]
struct ProviderError {
    retryable: bool,
    message: String,
    request_payload: Value,
    raw_response: Option<String>,
    usage: Option<LlmUsageSnapshot>,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl ProviderError {
    fn retryable(message: String, request_payload: Value) -> Self {
        Self {
            retryable: true,
            message,
            request_payload,
            raw_response: None,
            usage: None,
        }
    }

    fn retryable_with_response(
        message: String,
        request_payload: Value,
        raw_response: String,
        usage: Option<LlmUsageSnapshot>,
    ) -> Self {
        Self {
            retryable: true,
            message,
            request_payload,
            raw_response: Some(raw_response),
            usage,
        }
    }

    fn non_retryable(message: String, request_payload: Value) -> Self {
        Self {
            retryable: false,
            message,
            request_payload,
            raw_response: None,
            usage: None,
        }
    }

    fn non_retryable_with_response(
        message: String,
        request_payload: Value,
        raw_response: String,
        usage: Option<LlmUsageSnapshot>,
    ) -> Self {
        Self {
            retryable: false,
            message,
            request_payload,
            raw_response: Some(raw_response),
            usage,
        }
    }
}

fn value_as_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
    })
}

fn sum_u64(parts: &[Option<u64>]) -> Option<u64> {
    let mut total = 0u64;
    let mut seen = false;
    for part in parts {
        if let Some(value) = part {
            total = total.saturating_add(*value);
            seen = true;
        }
    }
    seen.then_some(total)
}

fn openai_usage_snapshot(value: &Value) -> Option<LlmUsageSnapshot> {
    let usage = value.get("usage")?;
    let prompt_tokens = value_as_u64(usage.get("prompt_tokens"));
    let completion_tokens = value_as_u64(usage.get("completion_tokens"));
    let total_tokens = value_as_u64(usage.get("total_tokens"))
        .or_else(|| sum_u64(&[prompt_tokens, completion_tokens]));
    let reasoning_tokens = value_as_u64(
        usage.get("completion_tokens_details")
            .and_then(|details| details.get("reasoning_tokens")),
    );
    let cached_tokens = value_as_u64(
        usage.get("prompt_tokens_details")
            .and_then(|details| details.get("cached_tokens")),
    );
    if prompt_tokens.is_none()
        && completion_tokens.is_none()
        && total_tokens.is_none()
        && reasoning_tokens.is_none()
        && cached_tokens.is_none()
    {
        return None;
    }
    Some(LlmUsageSnapshot {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        input_tokens: None,
        output_tokens: None,
        reasoning_tokens,
        cached_tokens,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    })
}

fn gemini_usage_snapshot(value: &Value) -> Option<LlmUsageSnapshot> {
    let usage = value.get("usageMetadata")?;
    let prompt_tokens = value_as_u64(usage.get("promptTokenCount"));
    let completion_tokens = value_as_u64(usage.get("candidatesTokenCount"));
    let total_tokens = value_as_u64(usage.get("totalTokenCount"))
        .or_else(|| sum_u64(&[prompt_tokens, completion_tokens]));
    let reasoning_tokens = value_as_u64(usage.get("thoughtsTokenCount"));
    let cached_tokens = value_as_u64(usage.get("cachedContentTokenCount"));
    if prompt_tokens.is_none()
        && completion_tokens.is_none()
        && total_tokens.is_none()
        && reasoning_tokens.is_none()
        && cached_tokens.is_none()
    {
        return None;
    }
    Some(LlmUsageSnapshot {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        input_tokens: None,
        output_tokens: None,
        reasoning_tokens,
        cached_tokens,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    })
}

fn anthropic_usage_snapshot(value: &Value) -> Option<LlmUsageSnapshot> {
    let usage = value.get("usage")?;
    let input_tokens = value_as_u64(usage.get("input_tokens"));
    let output_tokens = value_as_u64(usage.get("output_tokens"));
    let cache_creation_input_tokens = value_as_u64(usage.get("cache_creation_input_tokens"));
    let cache_read_input_tokens = value_as_u64(usage.get("cache_read_input_tokens"));
    let total_tokens = sum_u64(&[input_tokens, output_tokens]);
    if input_tokens.is_none()
        && output_tokens.is_none()
        && cache_creation_input_tokens.is_none()
        && cache_read_input_tokens.is_none()
    {
        return None;
    }
    Some(LlmUsageSnapshot {
        prompt_tokens: input_tokens,
        completion_tokens: output_tokens,
        total_tokens,
        input_tokens,
        output_tokens,
        reasoning_tokens: None,
        cached_tokens: None,
        cache_creation_input_tokens,
        cache_read_input_tokens,
    })
}

async fn call_provider(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<LlmProviderResponse, ProviderError> {
    match provider.config.provider_type.as_str() {
        "openai_compat" => call_openai_compat(provider, prompt).await,
        "google_gemini" => call_google_gemini(provider, prompt).await,
        "anthropic_claude" => call_anthropic_claude(provider, prompt).await,
        other => Err(ProviderError::non_retryable(
            format!("unsupported provider type: {other}"),
            Value::Null,
        )),
    }
}

async fn call_openai_compat(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<LlmProviderResponse, ProviderError> {
    let _permit = provider
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| {
            ProviderError::non_retryable(format!("semaphore closed: {err}"), Value::Null)
        })?;

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
                ProviderError::retryable(format!("timeout: {err}"), req_body.clone())
            } else {
                ProviderError::retryable(format!("request failed: {err}"), req_body.clone())
            }
        })?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|err| {
            ProviderError::retryable(format!("read response failed: {err}"), req_body.clone())
        })?;

    if status.as_u16() == 429 || status.is_server_error() {
        return Err(ProviderError::retryable_with_response(
            format!("http {}: {}", status.as_u16(), body_text),
            req_body.clone(),
            body_text,
            None,
        ));
    }

    if !status.is_success() {
        return Err(ProviderError::non_retryable_with_response(
            format!("http {}: {}", status.as_u16(), body_text),
            req_body.clone(),
            body_text,
            None,
        ));
    }

    let value: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|err| {
            ProviderError::non_retryable_with_response(
                format!("parse response failed: {err}"),
                req_body.clone(),
                body_text.clone(),
                None,
            )
        })?;
    let usage = openai_usage_snapshot(&value);

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
            ProviderError::non_retryable_with_response(
                "missing choices[0].message.content".to_string(),
                req_body.clone(),
                body_text.clone(),
                usage.clone(),
            )
        })?;

    Ok(LlmProviderResponse {
        text,
        request_payload: req_body,
        raw_response: body_text,
        usage,
    })
}

async fn call_google_gemini(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<LlmProviderResponse, ProviderError> {
    let _permit = provider
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| {
            ProviderError::non_retryable(format!("semaphore closed: {err}"), Value::Null)
        })?;

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
                ProviderError::retryable(format!("timeout: {err}"), req_body.clone())
            } else {
                ProviderError::retryable(format!("request failed: {err}"), req_body.clone())
            }
        })?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|err| {
            ProviderError::retryable(format!("read response failed: {err}"), req_body.clone())
        })?;

    if status.as_u16() == 429 || status.is_server_error() {
        return Err(ProviderError::retryable_with_response(
            format!("http {}: {}", status.as_u16(), body_text),
            req_body.clone(),
            body_text,
            None,
        ));
    }

    if !status.is_success() {
        return Err(ProviderError::non_retryable_with_response(
            format!("http {}: {}", status.as_u16(), body_text),
            req_body.clone(),
            body_text,
            None,
        ));
    }

    let value: Value = serde_json::from_str(&body_text)
        .map_err(|err| {
            ProviderError::non_retryable_with_response(
                format!("parse response failed: {err}"),
                req_body.clone(),
                body_text.clone(),
                None,
            )
        })?;
    let usage = gemini_usage_snapshot(&value);

    if let Some(block_reason) = value
        .get("promptFeedback")
        .and_then(|v| v.get("blockReason"))
        .and_then(|v| v.as_str())
    {
        return Err(ProviderError::non_retryable_with_response(
            format!("gemini prompt blocked: blockReason={block_reason}"),
            req_body.clone(),
            body_text.clone(),
            usage.clone(),
        ));
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
                return Err(ProviderError::non_retryable_with_response(
                    format!(
                        "gemini response blocked by safety filter: finishReason=SAFETY model={}",
                        provider.config.model
                    ),
                    req_body.clone(),
                    body_text.clone(),
                    usage.clone(),
                ));
            }
            "RECITATION" => {
                return Err(ProviderError::non_retryable_with_response(
                    format!(
                        "gemini response blocked: finishReason=RECITATION model={}",
                        provider.config.model
                    ),
                    req_body.clone(),
                    body_text.clone(),
                    usage.clone(),
                ));
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
            ProviderError::non_retryable_with_response(
                "missing candidates[0].content.parts[*].text".to_string(),
                req_body.clone(),
                body_text.clone(),
                usage.clone(),
            )
        })?;

    Ok(LlmProviderResponse {
        text,
        request_payload: req_body,
        raw_response: body_text,
        usage,
    })
}

async fn call_anthropic_claude(
    provider: Arc<LlmProviderRuntime>,
    prompt: &str,
) -> Result<LlmProviderResponse, ProviderError> {
    let _permit = provider
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|err| {
            ProviderError::non_retryable(format!("semaphore closed: {err}"), Value::Null)
        })?;

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
                ProviderError::retryable(format!("timeout: {err}"), req_body.clone())
            } else {
                ProviderError::retryable(format!("request failed: {err}"), req_body.clone())
            }
        })?;

    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|err| {
            ProviderError::retryable(format!("read response failed: {err}"), req_body.clone())
        })?;

    if status.as_u16() == 429 || status.is_server_error() {
        return Err(ProviderError::retryable_with_response(
            format!("http {}: {}", status.as_u16(), body_text),
            req_body.clone(),
            body_text,
            None,
        ));
    }

    if !status.is_success() {
        return Err(ProviderError::non_retryable_with_response(
            format!("http {}: {}", status.as_u16(), body_text),
            req_body.clone(),
            body_text,
            None,
        ));
    }

    let value: Value = serde_json::from_str(&body_text)
        .map_err(|err| {
            ProviderError::non_retryable_with_response(
                format!("parse response failed: {err}"),
                req_body.clone(),
                body_text.clone(),
                None,
            )
        })?;
    let usage = anthropic_usage_snapshot(&value);

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
        .ok_or_else(|| {
            ProviderError::non_retryable_with_response(
                "missing content[*].text".to_string(),
                req_body.clone(),
                body_text.clone(),
                usage.clone(),
            )
        })?;

    Ok(LlmProviderResponse {
        text,
        request_payload: req_body,
        raw_response: body_text,
        usage,
    })
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

fn touch_running_task(state: &AppState, task_id: &str) -> anyhow::Result<bool> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let changed = db.execute(
        "UPDATE tasks SET updated_at = ?2 WHERE task_id = ?1 AND status = 'running'",
        params![task_id, now_ts()],
    )?;
    Ok(changed > 0)
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

    // 已改为靠 key 绑定用户，admin 由 auth_keys.role 决定，不再从 config.telegram.admins 写入 users
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
            channel           TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark')),
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
        "ALTER TABLE tasks ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark'))",
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
        "ALTER TABLE scheduled_jobs ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark'))",
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
        "ALTER TABLE memories ADD COLUMN channel TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark'))",
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
    if tasks_sql.contains("'lark'") {
        return Ok(());
    }
    info!("channel schema: rebuilding tasks/scheduled_jobs/memories to allow channel=lark (and feishu)");
    db.execute_batch(
        "BEGIN IMMEDIATE;
         ALTER TABLE tasks RENAME TO tasks_old;
         CREATE TABLE tasks (
             task_id       TEXT PRIMARY KEY,
             user_id       INTEGER NOT NULL,
             chat_id       INTEGER NOT NULL,
             channel       TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark')),
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
             channel           TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark')),
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
             channel          TEXT NOT NULL DEFAULT 'telegram' CHECK (channel IN ('telegram', 'whatsapp', 'ui', 'feishu', 'lark')),
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

/// 列出所有 auth key（脱敏 + rowid），仅 admin 调用。
pub(crate) fn list_auth_keys(
    state: &AppState,
) -> anyhow::Result<Vec<(i64, String, String, i64, String, Option<String>)>> {
    let db = state.db.lock().map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let mut stmt = db.prepare(
        "SELECT rowid, user_key, role, enabled, created_at, last_used_at FROM auth_keys ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (key_id, user_key, role, enabled, created_at, last_used_at) = row?;
        out.push((
            key_id,
            mask_secret(&user_key),
            role,
            enabled,
            created_at,
            last_used_at,
        ));
    }
    Ok(out)
}

/// 生成新的 auth key，仅 admin 调用。返回明文 key（仅此一次，需展示给用户保存）。
pub(crate) fn create_auth_key(state: &AppState, role: &str) -> anyhow::Result<String> {
    let role = match role {
        "admin" => "admin",
        _ => "user",
    };
    let user_key = generate_user_key();
    let db = state.db.lock().map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, ?2, 1, ?3, NULL)",
        params![user_key, role, now_ts()],
    )?;
    Ok(user_key)
}

/// 更新 auth key 的角色或启用状态。返回是否命中记录。
pub(crate) fn update_auth_key_by_id(
    state: &AppState,
    key_id: i64,
    role: Option<&str>,
    enabled: Option<bool>,
    actor_user_key: &str,
) -> anyhow::Result<bool> {
    if role.is_none() && enabled.is_none() {
        return Err(anyhow::anyhow!("nothing to update"));
    }
    let normalized_role = role.map(|v| if v.eq_ignore_ascii_case("admin") { "admin" } else { "user" });
    let enabled_i64 = enabled.map(|v| if v { 1_i64 } else { 0_i64 });

    let db = state.db.lock().map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let target = db.query_row(
        "SELECT user_key FROM auth_keys WHERE rowid = ?1",
        params![key_id],
        |row| row.get::<_, String>(0),
    );
    let target_user_key = match target {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let actor_user_key = normalize_user_key(actor_user_key);
    if !actor_user_key.is_empty() && target_user_key == actor_user_key {
        if enabled == Some(false) {
            return Err(anyhow::anyhow!("cannot disable current key"));
        }
        if normalized_role == Some("user") {
            return Err(anyhow::anyhow!("cannot demote current admin key"));
        }
    }

    let changed = db.execute(
        "UPDATE auth_keys
         SET role = COALESCE(?2, role),
             enabled = COALESCE(?3, enabled)
         WHERE rowid = ?1",
        params![key_id, normalized_role, enabled_i64],
    )?;
    Ok(changed > 0)
}

/// 删除 auth key 及其关联绑定。返回是否命中记录。
pub(crate) fn delete_auth_key_by_id(
    state: &AppState,
    key_id: i64,
    actor_user_key: &str,
) -> anyhow::Result<bool> {
    let mut db = state.db.lock().map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let target = db.query_row(
        "SELECT user_key, role FROM auth_keys WHERE rowid = ?1",
        params![key_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    );
    let (target_user_key, target_role) = match target {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
        Err(err) => return Err(err.into()),
    };

    let actor_user_key = normalize_user_key(actor_user_key);
    if !actor_user_key.is_empty() && target_user_key == actor_user_key {
        return Err(anyhow::anyhow!("cannot delete current key"));
    }

    if target_role.eq_ignore_ascii_case("admin") {
        let admin_count: i64 = db.query_row(
            "SELECT COUNT(*) FROM auth_keys WHERE role = 'admin' AND enabled = 1",
            [],
            |row| row.get(0),
        )?;
        if admin_count <= 1 {
            return Err(anyhow::anyhow!("cannot delete the last enabled admin key"));
        }
    }

    let tx = db.transaction()?;
    tx.execute(
        "DELETE FROM channel_bindings WHERE user_key = ?1",
        params![target_user_key],
    )?;
    tx.execute(
        "DELETE FROM exchange_api_credentials WHERE user_key = ?1",
        params![target_user_key],
    )?;
    let changed = tx.execute("DELETE FROM auth_keys WHERE rowid = ?1", params![key_id])?;
    tx.commit()?;
    Ok(changed > 0)
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

fn channel_gateway_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("channel-gateway")
}

fn whatsappd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("whatsappd")
}

fn wa_webd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("whatsapp_webd")
}

fn feishud_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("feishud")
}

fn larkd_process_stats() -> Option<(usize, u64)> {
    daemon_process_stats("larkd")
}

/// 仅当“可执行文件名/进程名”与目标 daemon 名**完全一致**时计数，避免 substring 误判。
/// 使用 cmdline.contains(process_name) 会把 grep/rg/bash -c ... whatsappd、启动脚本参数里带
/// whatsappd 的临时进程都算进去，导致未运行 whatsappd 时仍误报 healthy。
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
        if !process_name_matches(&pid, process_name) {
            continue;
        }
        count += 1;
        let status_path = format!("/proc/{pid}/status");
        if let Some(rss_bytes) = current_rss_bytes_from_status(&status_path) {
            total_rss_bytes = total_rss_bytes.saturating_add(rss_bytes);
        }
    }

    Some((count, total_rss_bytes))
}

/// 判断该 pid 对应的进程是否就是目标 daemon：用 exe basename / comm / argv[0] basename 做**精确匹配**，
/// 避免把包含 daemon 名字的 grep/rg/bash 等临时进程算进去。
fn process_name_matches(pid: &str, process_name: &str) -> bool {
    // 1) 优先：/proc/<pid>/exe 的 basename（真实可执行文件）
    let exe_path = format!("/proc/{pid}/exe");
    if let Ok(target) = std::fs::read_link(&exe_path) {
        let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let name = name.strip_suffix(" (deleted)").unwrap_or(name);
        if name == process_name {
            return true;
        }
    }

    // 2) /proc/<pid>/comm（内核/进程设置的 16 字符内名称，通常与可执行文件名一致）
    let comm_path = format!("/proc/{pid}/comm");
    if let Ok(s) = std::fs::read_to_string(&comm_path) {
        let comm = s.trim();
        if comm == process_name {
            return true;
        }
    }

    // 3) 退回：argv[0] 的 basename（可能被进程改写，仅作补充）
    let cmdline_path = format!("/proc/{pid}/cmdline");
    if let Ok(bytes) = std::fs::read(&cmdline_path) {
        if let Some(first_arg) = bytes.split(|&b| b == 0).next() {
            let argv0 = std::str::from_utf8(first_arg).unwrap_or("");
            let base = argv0.rsplit('/').next().unwrap_or(argv0);
            if base == process_name {
                return true;
            }
        }
    }

    false
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
                return api_err::<SubmitTaskResponse>(StatusCode::INTERNAL_SERVER_ERROR, "Auth lookup failed");
            }
        },
        None => None,
    };
    // Key-only: if client sent user_key but it is invalid, reject immediately (no fallback to user_id/chat_id)
    if req.user_key.is_some() && resolved_identity.is_none() {
        return api_err::<SubmitTaskResponse>(StatusCode::UNAUTHORIZED, "Invalid user_key");
    }
    let effective_user_key = resolved_identity.as_ref().map(|v| v.user_key.clone());
    let requested_user_id = req.user_id;
    let requested_chat_id = req.chat_id;
    let effective_user_id = resolved_identity
        .as_ref()
        .map(|v| v.user_id)
        .or(requested_user_id)
        .unwrap_or_default();
    let channel = req.channel.unwrap_or(ChannelKind::Telegram);
    let requested_agent_id = req
        .payload
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let effective_agent_id = if let Some(agent_id) = requested_agent_id.as_deref() {
        if let Some(normalized) = state.normalize_known_agent_id(Some(agent_id)) {
            normalized
        } else {
            return api_err::<SubmitTaskResponse>(StatusCode::BAD_REQUEST, format!("unknown agent_id={agent_id}"));
        }
    } else {
        DEFAULT_AGENT_ID.to_string()
    };
    let normalized_external_user_id = normalize_external_id_opt(req.external_user_id.as_deref());
    let normalized_external_chat_id = normalize_external_id_opt(req.external_chat_id.as_deref());
    let public_conversation_seed = format!(
        "public:{}:{}",
        requested_user_id
            .map(|v| v.to_string())
            .unwrap_or_else(|| "anon".to_string()),
        requested_chat_id
            .map(|v| v.to_string())
            .unwrap_or_else(|| "chat".to_string())
    );
    let effective_chat_id = if let Some(user_key) = effective_user_key.as_deref() {
        build_conversation_chat_id(
            channel_kind_name(channel),
            normalized_external_user_id.as_deref(),
            normalized_external_chat_id.as_deref(),
            user_key,
        )
    } else if channel_allows_public_access(channel)
        && (normalized_external_user_id.is_some() || normalized_external_chat_id.is_some())
    {
        build_conversation_chat_id(
            channel_kind_name(channel),
            normalized_external_user_id.as_deref(),
            normalized_external_chat_id.as_deref(),
            &public_conversation_seed,
        )
    } else if let Some(chat_id) = requested_chat_id {
        chat_id
    } else {
        return api_err::<SubmitTaskResponse>(StatusCode::BAD_REQUEST, "chat_id is required when user_key is absent");
    };

    if resolved_identity.is_none() {
        let Some(request_user_id) = requested_user_id else {
            return api_err::<SubmitTaskResponse>(StatusCode::BAD_REQUEST, "user_id is required when user_key is absent");
        };
        if channel_allows_public_access(channel) {
            if let Err(err) = upsert_public_channel_user(&state, request_user_id) {
                error!("upsert public channel user failed: {}", err);
                return api_err::<SubmitTaskResponse>(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
            }
        } else if !is_user_allowed(&state, request_user_id) {
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
            return api_err::<SubmitTaskResponse>(StatusCode::FORBIDDEN, unauthorized);
        }
    }

    let limit_result = {
        let mut limiter = match state.rate_limiter.lock() {
            Ok(v) => v,
            Err(_) => return api_err::<SubmitTaskResponse>(StatusCode::INTERNAL_SERVER_ERROR, "Rate limiter lock poisoned"),
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
        return api_err::<SubmitTaskResponse>(StatusCode::TOO_MANY_REQUESTS, limit_exceeded);
    }

    let queued_count =
        match task_count_by_status(&state, &main_flow_rules(&state).task_status_queued) {
            Ok(v) => v,
            Err(err) => {
                error!("Count queued tasks failed: {}", err);
                return api_err::<SubmitTaskResponse>(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
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
        return api_err::<SubmitTaskResponse>(StatusCode::TOO_MANY_REQUESTS, queue_full);
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
                return api_ok(SubmitTaskResponse {
                    task_id: existing_id,
                });
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
        obj.insert(
            "agent_id".to_string(),
            Value::String(effective_agent_id.clone()),
        );
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
        return api_err::<SubmitTaskResponse>(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
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
        ChannelKind::Feishu => "feishu",
        ChannelKind::Lark => "lark",
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

fn channel_allows_shared_ui_task_access(channel: &str) -> bool {
    matches!(channel, "telegram" | "whatsapp" | "feishu" | "lark")
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

fn channel_allows_public_access(channel: ChannelKind) -> bool {
    matches!(
        channel,
        ChannelKind::Telegram | ChannelKind::Whatsapp | ChannelKind::Feishu | ChannelKind::Lark
    )
}

fn upsert_public_channel_user(state: &AppState, user_id: i64) -> anyhow::Result<()> {
    let now = now_ts();
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "INSERT INTO users (user_id, role, is_allowed, created_at, last_seen)
         VALUES (?1, 'user', 1, ?2, ?2)
         ON CONFLICT(user_id) DO UPDATE SET is_allowed=1, last_seen=excluded.last_seen",
        params![user_id, now],
    )?;
    Ok(())
}

async fn get_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<Uuid>,
) -> (StatusCode, Json<ApiResponse<TaskQueryResponse>>) {
    let read_result =
        (|| -> anyhow::Result<Option<(TaskQueryResponse, Option<String>, String)>> {
        let db = state
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

        let mut stmt = db.prepare(
            "SELECT status, result_json, error_text, user_key, channel
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
                let channel: String = row.get(4)?;

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
                    channel,
                ))
            })
            .optional()?;

        Ok(row)
    })();

    match read_result {
        Ok(Some((task, task_user_key, channel))) => {
            let expected_key = task_user_key
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty());
            let provided_key = headers
                .get("x-rustclaw-key")
                .and_then(|v| v.to_str().ok())
                .map(normalize_user_key)
                .filter(|v| !v.is_empty());
            let viewer_identity = match provided_key.as_deref() {
                Some(key) => match resolve_auth_identity_by_key(&state, key) {
                    Ok(identity) => identity,
                    Err(err) => {
                        error!("Resolve task viewer failed: {}", err);
                        return api_err::<TaskQueryResponse>(StatusCode::INTERNAL_SERVER_ERROR, "Auth lookup failed");
                    }
                },
                None => None,
            };
            if !channel_allows_shared_ui_task_access(&channel) {
                if let Some(expected_key) = expected_key {
                    if provided_key.as_deref() != Some(expected_key) {
                        return api_err::<TaskQueryResponse>(StatusCode::UNAUTHORIZED, "Task owner mismatch");
                    }
                }
            } else if provided_key.is_some() && viewer_identity.is_none() {
                    return api_err::<TaskQueryResponse>(StatusCode::UNAUTHORIZED, "Invalid user_key");
            }
            api_ok(task)
        }
        Ok(None) => api_err::<TaskQueryResponse>(StatusCode::NOT_FOUND, "Task not found"),
        Err(err) => {
            error!("Read task failed: {}", err);
            api_err::<TaskQueryResponse>(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        }
    }
}

#[derive(Debug, Deserialize)]
struct CancelTasksRequest {
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ActiveTasksRequest {
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct ActiveTaskItem {
    index: usize,
    task_id: String,
    kind: String,
    status: String,
    summary: String,
    age_seconds: i64,
}

#[derive(Debug, Deserialize)]
struct CancelOneTaskRequest {
    user_id: i64,
    chat_id: i64,
    index: usize,
    exclude_task_id: Option<String>,
}

fn normalized_optional_task_id(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn summarize_active_task_payload(kind: &str, payload_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(payload_json) else {
        return truncate_for_log(payload_json);
    };
    let summary = match kind {
        "ask" => v
            .get("text")
            .and_then(|x| x.as_str())
            .unwrap_or(payload_json)
            .to_string(),
        "run_skill" => {
            let skill = v
                .get("skill_name")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown");
            let action = v
                .get("args")
                .and_then(|x| x.get("action"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim();
            if action.is_empty() {
                format!("run_skill:{skill}")
            } else {
                format!("run_skill:{skill} action={action}")
            }
        }
        _ => payload_json.to_string(),
    };
    truncate_for_log(summary.trim())
}

fn list_active_tasks_internal(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<&str>,
) -> anyhow::Result<Vec<ActiveTaskItem>> {
    let exclude_task_id = normalized_optional_task_id(exclude_task_id);
    let now = now_ts().parse::<i64>().unwrap_or_default();
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, kind, payload_json, status,
                CAST(COALESCE(NULLIF(created_at, ''), '0') AS INTEGER) AS created_ts,
                CAST(COALESCE(NULLIF(updated_at, ''), created_at, '0') AS INTEGER) AS updated_ts
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND status IN ('running', 'queued')
           AND (?3 IS NULL OR task_id <> ?3)
         ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END,
                  created_ts ASC,
                  task_id ASC",
    )?;
    let rows = stmt.query_map(
        params![user_id, chat_id, exclude_task_id.as_deref()],
        |row| {
            let task_id: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let payload_json: String = row.get(2)?;
            let status: String = row.get(3)?;
            let created_ts: i64 = row.get(4)?;
            let updated_ts: i64 = row.get(5)?;
            Ok((task_id, kind, payload_json, status, created_ts, updated_ts))
        },
    )?;
    let mut out = Vec::new();
    for (idx, row) in rows.enumerate() {
        let (task_id, kind, payload_json, status, created_ts, updated_ts) = row?;
        let ref_ts = if updated_ts > 0 { updated_ts } else { created_ts };
        let age_seconds = if ref_ts > 0 { (now - ref_ts).max(0) } else { 0 };
        let summary = summarize_active_task_payload(&kind, &payload_json);
        out.push(ActiveTaskItem {
            index: idx + 1,
            task_id,
            kind,
            status,
            summary,
            age_seconds,
        });
    }
    Ok(out)
}

async fn list_active_tasks(
    State(state): State<AppState>,
    Json(req): Json<ActiveTasksRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    if !is_user_allowed(&state, req.user_id) {
        return api_err::<serde_json::Value>(StatusCode::FORBIDDEN, "Unauthorized user");
    }
    match list_active_tasks_internal(
        &state,
        req.user_id,
        req.chat_id,
        req.exclude_task_id.as_deref(),
    ) {
        Ok(tasks) => api_ok(json!({
            "count": tasks.len(),
            "tasks": tasks,
        })),
        Err(err) => {
            error!("List active tasks failed: {}", err);
            api_err::<serde_json::Value>(StatusCode::INTERNAL_SERVER_ERROR, "List active tasks failed")
        }
    }
}

async fn cancel_tasks(
    State(state): State<AppState>,
    Json(req): Json<CancelTasksRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    if !is_user_allowed(&state, req.user_id) {
        return api_err::<serde_json::Value>(StatusCode::FORBIDDEN, "Unauthorized user");
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
               AND status IN ('queued', 'running')
               AND (?4 IS NULL OR task_id <> ?4)",
        )?;
        let exclude_task_id = normalized_optional_task_id(req.exclude_task_id.as_deref());
        let affected =
            stmt.execute(params![now, req.user_id, req.chat_id, exclude_task_id.as_deref()])?;
        Ok(affected as i64)
    })();

    match result {
        Ok(count) => {
            info!(
                "cancel_tasks: user_id={} chat_id={} canceled={}",
                req.user_id, req.chat_id, count
            );
            api_ok(json!({ "canceled": count }))
        }
        Err(err) => {
            error!("Cancel tasks failed: {}", err);
            api_err::<serde_json::Value>(StatusCode::INTERNAL_SERVER_ERROR, "Cancel tasks failed")
        }
    }
}

async fn cancel_one_task(
    State(state): State<AppState>,
    Json(req): Json<CancelOneTaskRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    if !is_user_allowed(&state, req.user_id) {
        return api_err::<serde_json::Value>(StatusCode::FORBIDDEN, "Unauthorized user");
    }
    if req.index == 0 {
        return api_err::<serde_json::Value>(StatusCode::BAD_REQUEST, "index must be >= 1");
    }
    let tasks = match list_active_tasks_internal(
        &state,
        req.user_id,
        req.chat_id,
        req.exclude_task_id.as_deref(),
    ) {
        Ok(tasks) => tasks,
        Err(err) => {
            error!("Cancel one task list failed: {}", err);
            return api_err::<serde_json::Value>(StatusCode::INTERNAL_SERVER_ERROR, "Cancel one task failed");
        }
    };
    let Some(target) = tasks.into_iter().find(|t| t.index == req.index) else {
        return api_err::<serde_json::Value>(
            StatusCode::NOT_FOUND,
            format!("Active task index {} not found", req.index),
        );
    };
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
               AND task_id = ?4
               AND status IN ('queued', 'running')",
        )?;
        let affected = stmt.execute(params![now, req.user_id, req.chat_id, &target.task_id])?;
        Ok(affected as i64)
    })();
    match result {
        Ok(count) if count > 0 => api_ok(json!({
            "canceled": count,
            "task": target,
        })),
        Ok(_) => api_err::<serde_json::Value>(StatusCode::NOT_FOUND, "Target task is no longer active"),
        Err(err) => {
            error!("Cancel one task failed: {}", err);
            api_err::<serde_json::Value>(StatusCode::INTERNAL_SERVER_ERROR, "Cancel one task failed")
        }
    }
}

/// Phase 4: 重载 skill 视图。POST /v1/admin/reload-skills。与现有管理接口一致：需 x-rustclaw-key 鉴权。
async fn reload_skills_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    if let Err((status, json)) = http::ui_routes::require_ui_identity(&state, &headers) {
        return (status, json);
    }
    match reload_skill_views(&state) {
        Ok(result) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(serde_json::to_value(&result).unwrap_or_default()),
                error: None,
            }),
        ),
        Err(e) => {
            warn!("reload_skill_views failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("reload failed: {}", e)),
                }),
            )
        }
    }
}
