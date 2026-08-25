use std::collections::HashMap;

use serde::{Deserialize, Serialize};

mod defaults;
mod model_vendors;
mod runtime;

use defaults::*;
pub use model_vendors::llm_vendor_api_key_env_names;

pub const CLAWD_INTERNAL_LISTEN: &str = "127.0.0.1:8787";
pub const CLAWD_INTERNAL_BASE_URL: &str = "http://127.0.0.1:8787";
pub const AGENT_PERSONA_PROFILES: &[&str] = &[
    "inherit",
    "executor",
    "companion",
    "expert",
    "teacher",
    "advisor",
    "reviewer",
    "custom",
];

pub fn normalize_agent_persona_profile(profile: &str) -> (&'static str, bool) {
    let normalized = profile.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "inherit" => ("inherit", true),
        "executor" => ("executor", true),
        "companion" => ("companion", true),
        "expert" => ("expert", true),
        "teacher" => ("teacher", true),
        "advisor" => ("advisor", true),
        "reviewer" => ("reviewer", true),
        "custom" => ("custom", true),
        _ => ("inherit", false),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub telegram_bot: TelegramBotConfig,
    #[serde(default)]
    pub whatsapp: WhatsappConfig,
    #[serde(default)]
    pub whatsapp_cloud: WhatsappCloudConfig,
    #[serde(default)]
    pub whatsapp_web: WhatsappWebConfig,
    #[serde(default)]
    pub adapters: HashMap<String, AdapterPlaceholderConfig>,
    #[serde(default)]
    pub mcp: McpConfig,
    pub database: DatabaseConfig,
    pub worker: WorkerConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub skills: SkillsConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub maintenance: MaintenanceConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub workspace_instructions: WorkspaceInstructionsConfig,
    #[serde(default)]
    pub auto_review: AutoReviewConfig,
    #[serde(default)]
    pub image_vision: ImageSkillConfig,
    #[serde(default)]
    pub image_generation: ImageSkillConfig,
    #[serde(default)]
    pub image_edit: ImageSkillConfig,
    #[serde(default)]
    pub routing: RoutingConfig,
    #[serde(default)]
    pub command_intent: CommandIntentConfig,
    #[serde(default)]
    pub persona: PersonaConfig,
    #[serde(default)]
    pub agents: Vec<AgentConfig>,
    #[serde(default)]
    pub schedule: ScheduleConfig,
    #[serde(default)]
    pub webd: WebdConfig,
    #[serde(default)]
    pub prompts: PromptsConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub request_timeout_seconds: u64,
    /// 可选。仅供将通信守护进程显式连到另一个 clawd 实例。
    #[serde(default)]
    pub clawd_base_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramConfig {
    #[serde(default)]
    pub bot_token: String,
    #[serde(default = "default_agent_id")]
    pub agent_id: String,
    #[serde(default)]
    pub allowlist: Vec<i64>,
    #[serde(default = "default_telegram_access_mode")]
    pub access_mode: String,
    #[serde(default)]
    pub allowed_usernames: Vec<String>,
    #[serde(default)]
    pub bots: Vec<TelegramRuntimeBotConfig>,
    #[serde(default)]
    pub bindings: Vec<ChannelBindingConfig>,
    #[serde(default = "default_telegram_language")]
    pub language: String,
    #[serde(default = "default_telegram_i18n_path")]
    pub i18n_path: String,
    #[serde(default = "default_telegram_quick_result_wait_seconds")]
    pub quick_result_wait_seconds: u64,
    #[serde(default = "default_telegram_task_delivery_timeout_seconds")]
    pub task_delivery_timeout_seconds: u64,
    #[serde(default = "default_telegram_image_inbox_dir")]
    pub image_inbox_dir: String,
    #[serde(default = "default_telegram_video_inbox_dir")]
    pub video_inbox_dir: String,
    #[serde(default = "default_telegram_file_inbox_dir")]
    pub file_inbox_dir: String,
    #[serde(default = "default_telegram_audio_inbox_dir")]
    pub audio_inbox_dir: String,
    #[serde(default = "default_telegram_voice_reply_mode")]
    pub voice_reply_mode: String,
    #[serde(default)]
    pub voice_reply_mode_by_chat: HashMap<String, String>,
    #[serde(default = "default_telegram_max_audio_input_bytes")]
    pub max_audio_input_bytes: usize,
    #[serde(default = "default_telegram_ephemeral_image_saved_seconds")]
    pub ephemeral_image_saved_seconds: u64,
    #[serde(default = "default_telegram_update_mode")]
    pub update_mode: String,
    #[serde(default = "default_telegram_webhook_listen")]
    pub webhook_listen: String,
    #[serde(default)]
    pub webhook_public_url: String,
    #[serde(default = "default_telegram_webhook_secret_env")]
    pub webhook_secret_env: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramBotConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub allowlist: Vec<i64>,
    #[serde(default = "default_telegram_access_mode")]
    pub access_mode: String,
    #[serde(default)]
    pub allowed_usernames: Vec<String>,
    #[serde(default = "default_telegram_language")]
    pub language: String,
    #[serde(default = "default_telegram_i18n_path")]
    pub i18n_path: String,
    #[serde(default = "default_telegram_quick_result_wait_seconds")]
    pub quick_result_wait_seconds: u64,
    #[serde(default = "default_telegram_task_delivery_timeout_seconds")]
    pub task_delivery_timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct TelegramRuntimeBotConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub bot_token: String,
    #[serde(default = "default_agent_id")]
    pub agent_id: String,
    #[serde(default)]
    pub allowlist: Vec<i64>,
    #[serde(default = "default_telegram_access_mode")]
    pub access_mode: String,
    #[serde(default)]
    pub allowed_usernames: Vec<String>,
    #[serde(default = "default_telegram_language")]
    pub language: String,
    #[serde(default = "default_telegram_i18n_path")]
    pub i18n_path: String,
    #[serde(default = "default_telegram_quick_result_wait_seconds")]
    pub quick_result_wait_seconds: u64,
    #[serde(default = "default_telegram_task_delivery_timeout_seconds")]
    pub task_delivery_timeout_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct ResolvedTelegramBotConfig {
    pub name: String,
    pub bot_token: String,
    pub agent_id: String,
    pub allowlist: Vec<i64>,
    pub access_mode: String,
    pub allowed_usernames: Vec<String>,
    pub language: String,
    pub i18n_path: String,
    pub quick_result_wait_seconds: u64,
    pub task_delivery_timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    #[serde(default = "default_agent_id")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Canonical persona selection. `inherit` resolves through the global
    /// persona profile when a runtime snapshot is built.
    #[serde(default = "default_agent_persona_profile")]
    pub persona_profile: String,
    /// Canonical custom style fragment. It is metadata for the final
    /// presentation renderer and must never be injected into planning.
    #[serde(default)]
    pub persona_fragment: String,
    /// Legacy compatibility only. New writes use `persona_profile` and
    /// `persona_fragment` in configs/agents.toml.
    #[serde(default, skip_serializing)]
    pub persona_prompt: String,
    #[serde(default)]
    pub preferred_vendor: Option<String>,
    #[serde(default)]
    pub preferred_model: Option<String>,
    #[serde(default)]
    pub allowed_skills: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: default_agent_id(),
            name: "Main".to_string(),
            description: String::new(),
            persona_profile: default_agent_persona_profile(),
            persona_fragment: String::new(),
            persona_prompt: String::new(),
            preferred_vendor: None,
            preferred_model: None,
            allowed_skills: Vec::new(),
        }
    }
}

fn default_agent_persona_profile() -> String {
    "inherit".to_string()
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            agent_id: default_agent_id(),
            allowlist: Vec::new(),
            access_mode: default_telegram_access_mode(),
            allowed_usernames: Vec::new(),
            bots: Vec::new(),
            bindings: Vec::new(),
            language: default_telegram_language(),
            i18n_path: default_telegram_i18n_path(),
            quick_result_wait_seconds: default_telegram_quick_result_wait_seconds(),
            task_delivery_timeout_seconds: default_telegram_task_delivery_timeout_seconds(),
            image_inbox_dir: default_telegram_image_inbox_dir(),
            video_inbox_dir: default_telegram_video_inbox_dir(),
            file_inbox_dir: default_telegram_file_inbox_dir(),
            audio_inbox_dir: default_telegram_audio_inbox_dir(),
            voice_reply_mode: default_telegram_voice_reply_mode(),
            voice_reply_mode_by_chat: HashMap::new(),
            max_audio_input_bytes: default_telegram_max_audio_input_bytes(),
            ephemeral_image_saved_seconds: default_telegram_ephemeral_image_saved_seconds(),
            update_mode: default_telegram_update_mode(),
            webhook_listen: default_telegram_webhook_listen(),
            webhook_public_url: String::new(),
            webhook_secret_env: default_telegram_webhook_secret_env(),
        }
    }
}

impl Default for TelegramBotConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bot_token: String::new(),
            allowlist: Vec::new(),
            access_mode: default_telegram_access_mode(),
            allowed_usernames: Vec::new(),
            language: default_telegram_language(),
            i18n_path: default_telegram_i18n_path(),
            quick_result_wait_seconds: default_telegram_quick_result_wait_seconds(),
            task_delivery_timeout_seconds: default_telegram_task_delivery_timeout_seconds(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WhatsappConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_whatsapp_api_base")]
    pub api_base: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub app_secret: String,
    #[serde(default)]
    pub verify_token: String,
    #[serde(default)]
    pub phone_number_id: String,
    #[serde(default)]
    pub out_of_window_template_name: String,
    #[serde(default = "default_whatsapp_template_language")]
    pub out_of_window_template_language: String,
    #[serde(default = "default_whatsapp_webhook_listen")]
    pub webhook_listen: String,
    #[serde(default = "default_whatsapp_webhook_path")]
    pub webhook_path: String,
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub bindings: Vec<ChannelBindingConfig>,
    #[serde(default = "default_whatsapp_language")]
    pub language: String,
    #[serde(default = "default_whatsapp_i18n_path")]
    pub i18n_path: String,
    #[serde(default = "default_whatsapp_quick_result_wait_seconds")]
    pub quick_result_wait_seconds: u64,
    #[serde(default = "default_whatsapp_task_delivery_timeout_seconds")]
    pub task_delivery_timeout_seconds: u64,
    #[serde(default = "default_whatsapp_image_inbox_dir")]
    pub image_inbox_dir: String,
    #[serde(default = "default_whatsapp_audio_inbox_dir")]
    pub audio_inbox_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WhatsappCloudConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_whatsapp_api_base")]
    pub api_base: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub app_secret: String,
    #[serde(default)]
    pub verify_token: String,
    #[serde(default)]
    pub phone_number_id: String,
    #[serde(default)]
    pub out_of_window_template_name: String,
    #[serde(default = "default_whatsapp_template_language")]
    pub out_of_window_template_language: String,
    #[serde(default = "default_whatsapp_webhook_listen")]
    pub webhook_listen: String,
    #[serde(default = "default_whatsapp_webhook_path")]
    pub webhook_path: String,
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub bindings: Vec<ChannelBindingConfig>,
    #[serde(default = "default_whatsapp_quick_result_wait_seconds")]
    pub quick_result_wait_seconds: u64,
    #[serde(default = "default_whatsapp_task_delivery_timeout_seconds")]
    pub task_delivery_timeout_seconds: u64,
    #[serde(default = "default_whatsapp_image_inbox_dir")]
    pub image_inbox_dir: String,
    #[serde(default = "default_whatsapp_audio_inbox_dir")]
    pub audio_inbox_dir: String,
}

impl Default for WhatsappCloudConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_base: default_whatsapp_api_base(),
            access_token: String::new(),
            app_secret: String::new(),
            verify_token: String::new(),
            phone_number_id: String::new(),
            out_of_window_template_name: String::new(),
            out_of_window_template_language: default_whatsapp_template_language(),
            webhook_listen: default_whatsapp_webhook_listen(),
            webhook_path: default_whatsapp_webhook_path(),
            admins: Vec::new(),
            allowlist: Vec::new(),
            bindings: Vec::new(),
            quick_result_wait_seconds: default_whatsapp_quick_result_wait_seconds(),
            task_delivery_timeout_seconds: default_whatsapp_task_delivery_timeout_seconds(),
            image_inbox_dir: default_whatsapp_image_inbox_dir(),
            audio_inbox_dir: default_whatsapp_audio_inbox_dir(),
        }
    }
}

impl Default for WhatsappConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_base: default_whatsapp_api_base(),
            access_token: String::new(),
            app_secret: String::new(),
            verify_token: String::new(),
            phone_number_id: String::new(),
            out_of_window_template_name: String::new(),
            out_of_window_template_language: default_whatsapp_template_language(),
            webhook_listen: default_whatsapp_webhook_listen(),
            webhook_path: default_whatsapp_webhook_path(),
            admins: Vec::new(),
            allowlist: Vec::new(),
            bindings: Vec::new(),
            language: default_whatsapp_language(),
            i18n_path: default_whatsapp_i18n_path(),
            quick_result_wait_seconds: default_whatsapp_quick_result_wait_seconds(),
            task_delivery_timeout_seconds: default_whatsapp_task_delivery_timeout_seconds(),
            image_inbox_dir: default_whatsapp_image_inbox_dir(),
            audio_inbox_dir: default_whatsapp_audio_inbox_dir(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WhatsappWebConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_whatsapp_web_bridge_listen")]
    pub bridge_listen: String,
    #[serde(default = "default_whatsapp_web_wrapper_listen")]
    pub wrapper_listen: String,
    #[serde(default = "default_whatsapp_web_bridge_base_url")]
    pub bridge_base_url: String,
    #[serde(default = "default_whatsapp_web_auth_dir")]
    pub auth_dir: String,
    #[serde(default = "default_whatsapp_web_quick_result_wait_seconds")]
    pub quick_result_wait_seconds: u64,
    /// Local adapter policy. This is not a provider-advertised WhatsApp limit.
    #[serde(default = "default_whatsapp_web_max_outbound_image_bytes")]
    pub max_outbound_image_bytes: u64,
    /// Local adapter policy. This is not a provider-advertised WhatsApp limit.
    #[serde(default = "default_whatsapp_web_max_outbound_video_bytes")]
    pub max_outbound_video_bytes: u64,
    /// Local adapter policy. This is not a provider-advertised WhatsApp limit.
    #[serde(default = "default_whatsapp_web_max_outbound_audio_bytes")]
    pub max_outbound_audio_bytes: u64,
    /// Local adapter policy. This is not a provider-advertised WhatsApp limit.
    #[serde(default = "default_whatsapp_web_max_outbound_file_bytes")]
    pub max_outbound_file_bytes: u64,
    /// Scheduled/proactive delivery is opt-in for this experimental adapter.
    #[serde(default)]
    pub allow_proactive_send: bool,
    #[serde(default = "default_whatsapp_web_language")]
    pub language: String,
    #[serde(default = "default_whatsapp_web_i18n_path")]
    pub i18n_path: String,
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub bindings: Vec<ChannelBindingConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ChannelBindingConfig {
    #[serde(default)]
    pub external_user_id: String,
    #[serde(default)]
    pub external_chat_id: String,
    #[serde(default)]
    pub telegram_bot_name: String,
    pub user_key: String,
}

impl Default for WhatsappWebConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bridge_listen: default_whatsapp_web_bridge_listen(),
            wrapper_listen: default_whatsapp_web_wrapper_listen(),
            bridge_base_url: default_whatsapp_web_bridge_base_url(),
            auth_dir: default_whatsapp_web_auth_dir(),
            quick_result_wait_seconds: default_whatsapp_web_quick_result_wait_seconds(),
            max_outbound_image_bytes: default_whatsapp_web_max_outbound_image_bytes(),
            max_outbound_video_bytes: default_whatsapp_web_max_outbound_video_bytes(),
            max_outbound_audio_bytes: default_whatsapp_web_max_outbound_audio_bytes(),
            max_outbound_file_bytes: default_whatsapp_web_max_outbound_file_bytes(),
            allow_proactive_send: false,
            language: default_whatsapp_web_language(),
            i18n_path: default_whatsapp_web_i18n_path(),
            admins: Vec::new(),
            allowlist: Vec::new(),
            bindings: Vec::new(),
        }
    }
}

/// 面向公网的 HTTP 反向代理（转发至本机 `clawd`），见 `webd` 二进制。
#[derive(Debug, Clone, Deserialize)]
pub struct WebdConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_webd_listen")]
    pub listen: String,
    #[serde(default = "default_webd_upstream")]
    pub upstream: String,
    #[serde(default = "default_webd_connect_timeout_seconds")]
    pub connect_timeout_seconds: u64,
    /// 0 表示使用 `[server].request_timeout_seconds`。
    #[serde(default)]
    pub request_timeout_seconds: u64,
    #[serde(default = "default_webd_forward_x_forwarded")]
    pub forward_x_forwarded: bool,
    /// 入站请求体最大字节数（缓冲后转发给 clawd）；过大返回 413。
    #[serde(default = "default_webd_max_incoming_body_bytes")]
    pub max_incoming_body_bytes: usize,
    /// HttpOnly 会话 Cookie 名。
    #[serde(default = "default_webd_session_cookie_name")]
    pub session_cookie_name: String,
    /// 会话有效期（秒）。
    #[serde(default = "default_webd_session_ttl_seconds")]
    pub session_ttl_seconds: u64,
    /// 服务端会话索引；用于 webd 重启后继续验证 HttpOnly Cookie。
    #[serde(default = "default_webd_session_store_path")]
    pub session_store_path: String,
    /// 同一客户端 IP 与用户名组合连续登录失败多少次后临时锁定。
    #[serde(default = "default_webd_login_failure_limit")]
    pub login_failure_limit: u32,
    /// 登录失败达到阈值后的锁定秒数。
    #[serde(default = "default_webd_login_lockout_seconds")]
    pub login_lockout_seconds: u64,
}

impl Default for WebdConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: default_webd_listen(),
            upstream: default_webd_upstream(),
            connect_timeout_seconds: default_webd_connect_timeout_seconds(),
            request_timeout_seconds: 0,
            forward_x_forwarded: default_webd_forward_x_forwarded(),
            max_incoming_body_bytes: default_webd_max_incoming_body_bytes(),
            session_cookie_name: default_webd_session_cookie_name(),
            session_ttl_seconds: default_webd_session_ttl_seconds(),
            session_store_path: default_webd_session_store_path(),
            login_failure_limit: default_webd_login_failure_limit(),
            login_lockout_seconds: default_webd_login_lockout_seconds(),
        }
    }
}

fn default_webd_session_cookie_name() -> String {
    "webd_sid".to_string()
}

fn default_webd_session_ttl_seconds() -> u64 {
    86400
}

fn default_webd_session_store_path() -> String {
    "data/webd_sessions.json".to_string()
}

fn default_webd_login_failure_limit() -> u32 {
    6
}

fn default_webd_login_lockout_seconds() -> u64 {
    15 * 60
}

fn default_webd_max_incoming_body_bytes() -> usize {
    100 * 1024 * 1024
}

fn default_webd_listen() -> String {
    "0.0.0.0:8788".to_string()
}

fn default_webd_upstream() -> String {
    "http://127.0.0.1:8787".to_string()
}

fn default_webd_connect_timeout_seconds() -> u64 {
    10
}

fn default_webd_forward_x_forwarded() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdapterPlaceholderConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub note: String,
}

impl Default for AdapterPlaceholderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            note: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportConfig {
    #[default]
    Stdio,
    Sse,
    StreamableHttp,
}

impl McpTransportConfig {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Sse => "sse",
            Self::StreamableHttp => "streamable_http",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_mcp_planner_visible_tools")]
    pub planner_visible_tools: usize,
    #[serde(default = "default_mcp_catalog_search_max_results")]
    pub catalog_search_max_results: usize,
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            planner_visible_tools: default_mcp_planner_visible_tools(),
            catalog_search_max_results: default_mcp_catalog_search_max_results(),
            servers: HashMap::new(),
        }
    }
}

impl McpConfig {
    pub fn enabled_server_names(&self) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        let mut names: Vec<String> = self
            .servers
            .iter()
            .filter(|(_, server)| server.enabled)
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub transport: McpTransportConfig,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub env_refs: HashMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub auth_token_env: Option<String>,
    #[serde(default)]
    pub oauth_client_id_env: Option<String>,
    #[serde(default)]
    pub oauth_client_secret_env: Option<String>,
    #[serde(default)]
    pub oauth_scopes: Vec<String>,
    #[serde(default)]
    pub oauth_resource: Option<String>,
    #[serde(default = "default_mcp_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_mcp_max_concurrency")]
    pub max_concurrency: usize,
    #[serde(default = "default_mcp_max_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default = "default_mcp_max_schema_bytes")]
    pub max_schema_bytes: usize,
    #[serde(default = "default_mcp_max_tools")]
    pub max_tools: usize,
    #[serde(default = "default_mcp_health_check_seconds")]
    pub health_check_seconds: u64,
    #[serde(default = "default_mcp_reconnect_base_seconds")]
    pub reconnect_base_seconds: u64,
    #[serde(default = "default_mcp_reconnect_max_seconds")]
    pub reconnect_max_seconds: u64,
    #[serde(default)]
    pub trusted: bool,
    #[serde(default)]
    pub capability_prefix: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub tool_policies: HashMap<String, McpToolPolicyConfig>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: McpTransportConfig::default(),
            command: None,
            args: Vec::new(),
            env: HashMap::new(),
            env_refs: HashMap::new(),
            url: None,
            auth_token_env: None,
            oauth_client_id_env: None,
            oauth_client_secret_env: None,
            oauth_scopes: Vec::new(),
            oauth_resource: None,
            timeout_seconds: default_mcp_timeout_seconds(),
            max_concurrency: default_mcp_max_concurrency(),
            max_output_bytes: default_mcp_max_output_bytes(),
            max_schema_bytes: default_mcp_max_schema_bytes(),
            max_tools: default_mcp_max_tools(),
            health_check_seconds: default_mcp_health_check_seconds(),
            reconnect_base_seconds: default_mcp_reconnect_base_seconds(),
            reconnect_max_seconds: default_mcp_reconnect_max_seconds(),
            trusted: false,
            capability_prefix: None,
            allowed_tools: Vec::new(),
            tool_policies: HashMap::new(),
        }
    }
}

impl McpServerConfig {
    pub fn uses_oauth_client_credentials(&self) -> bool {
        self.oauth_client_id_env.is_some() || self.oauth_client_secret_env.is_some()
    }

    pub fn auth_mode_token(&self) -> &'static str {
        if self.uses_oauth_client_credentials() {
            "oauth_client_credentials"
        } else if self.auth_token_env.is_some() {
            "bearer_env"
        } else {
            "none"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpToolEffectConfig {
    Observe,
    #[default]
    Mutate,
    Validate,
    External,
}

impl McpToolEffectConfig {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Mutate => "mutate",
            Self::Validate => "validate",
            Self::External => "external",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpToolRiskConfig {
    #[default]
    Unknown,
    Low,
    Medium,
    High,
}

impl McpToolRiskConfig {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpToolPolicyConfig {
    #[serde(default)]
    pub effect: McpToolEffectConfig,
    #[serde(default)]
    pub risk_level: McpToolRiskConfig,
    #[serde(default)]
    pub idempotent: bool,
    #[serde(default)]
    pub isolation_profile: Option<crate::skill_registry::CapabilityIsolationProfile>,
    #[serde(default)]
    pub network_access: bool,
    #[serde(default)]
    pub filesystem_write: bool,
    #[serde(default)]
    pub external_publish: bool,
    #[serde(default)]
    pub credential_access: bool,
    #[serde(default)]
    pub subprocess: bool,
    #[serde(default)]
    pub package_install: bool,
    #[serde(default)]
    pub privilege_escalation: bool,
}

impl Default for McpToolPolicyConfig {
    fn default() -> Self {
        Self {
            effect: McpToolEffectConfig::Mutate,
            risk_level: McpToolRiskConfig::Unknown,
            idempotent: false,
            isolation_profile: None,
            network_access: false,
            filesystem_write: false,
            external_publish: false,
            credential_access: false,
            subprocess: false,
            package_install: false,
            privilege_escalation: false,
        }
    }
}

fn default_mcp_timeout_seconds() -> u64 {
    30
}

fn default_mcp_max_concurrency() -> usize {
    2
}

fn default_mcp_max_output_bytes() -> usize {
    256 * 1024
}

fn default_mcp_max_schema_bytes() -> usize {
    64 * 1024
}

fn default_mcp_max_tools() -> usize {
    128
}

fn default_mcp_planner_visible_tools() -> usize {
    32
}

fn default_mcp_catalog_search_max_results() -> usize {
    20
}

fn default_mcp_health_check_seconds() -> u64 {
    30
}

fn default_mcp_reconnect_base_seconds() -> u64 {
    2
}

fn default_mcp_reconnect_max_seconds() -> u64 {
    60
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub sqlite_path: String,
    pub busy_timeout_ms: u64,
    /// Root for storage owned by individual skills. Each persisted skill gets
    /// a traversal-safe child directory and its own `state.db`.
    #[serde(default = "default_skill_data_root")]
    pub skill_data_root: String,
    /// SQLite 连接池最大连接数。≥ 2，默认 8（与 worker 并发*2 + http 路径预留）。
    /// 配合 WAL 模式：reader 不阻塞 writer，多 reader 并发；writer 串行（SQLite 限制）。
    #[serde(default = "default_db_pool_max_size")]
    pub pool_max_size: u32,
    /// Phase 2.2 Stage 2: 把 audit_logs 拆到独立 SQLite 文件 +
    /// 独立连接池，让任务流水线（tasks/scheduled_jobs/...）的 writer 锁
    /// 不再被 audit append 抢占。默认 `data/agent-runtime-audit.db`，
    /// 启动时若主库存在 audit_logs 行会一次性迁移过去。
    #[serde(default = "default_audit_sqlite_path")]
    pub audit_sqlite_path: String,
    /// audit pool 比主 pool 小：append-only 路径只需要 1 个 writer + 1 个
    /// reader（清理任务 + 偶尔后台查询），默认 2。
    #[serde(default = "default_audit_pool_max_size")]
    pub audit_pool_max_size: u32,
}

fn default_db_pool_max_size() -> u32 {
    8
}

fn default_skill_data_root() -> String {
    "data/skills".to_string()
}

fn default_audit_sqlite_path() -> String {
    "data/agent-runtime-audit.db".to_string()
}

fn default_audit_pool_max_size() -> u32 {
    2
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    // Worker lifecycle settings are deliberately independent from task runtime
    // deadlines. Durable/background work has no implicit global wall-clock kill;
    // explicit operation deadlines, cancellation, adapter timeouts, resource
    // policy and stale-lease recovery remain separate machine contracts.
    // Missing fields use the safe worker scheduling defaults below:
    // - concurrency=1（单 worker，避免抢资源）
    // - poll_interval_ms=500
    // - queue_limit=64
    #[serde(default = "default_worker_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_worker_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_worker_queue_limit")]
    pub queue_limit: usize,
    #[serde(default = "default_worker_task_heartbeat_seconds")]
    pub task_heartbeat_seconds: u64,
    #[serde(default = "default_worker_running_no_progress_timeout_seconds")]
    pub running_no_progress_timeout_seconds: u64,
    #[serde(default = "default_worker_running_recovery_check_interval_seconds")]
    pub running_recovery_check_interval_seconds: u64,
    /// Optional delegation to a separately authenticated worker. Remote API
    /// calls are not remote-executor work and must not use this switch.
    #[serde(default)]
    pub remote_executor: RemoteExecutorConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RemoteExecutorConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub trusted_attestation_digests: Vec<String>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            concurrency: default_worker_concurrency(),
            poll_interval_ms: default_worker_poll_interval_ms(),
            queue_limit: default_worker_queue_limit(),
            task_heartbeat_seconds: default_worker_task_heartbeat_seconds(),
            running_no_progress_timeout_seconds: default_worker_running_no_progress_timeout_seconds(
            ),
            running_recovery_check_interval_seconds:
                default_worker_running_recovery_check_interval_seconds(),
            remote_executor: RemoteExecutorConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LlmConfig {
    #[serde(default)]
    pub selected_vendor: Option<String>,
    #[serde(default)]
    pub selected_model: Option<String>,
    /// Trusted release mapping from a closed subagent model class to one
    /// provider/model pair. Runtime policy selects only the class; credentials
    /// continue to come from the configured provider and credential broker.
    #[serde(default)]
    pub model_classes: HashMap<String, LlmModelClassConfig>,
    /// Optional machine-readable pricing catalog. Entries are matched by exact
    /// provider/model identifiers; missing entries remain explicitly unknown.
    #[serde(default)]
    pub pricing: Vec<LlmModelPricingConfig>,
    #[serde(default)]
    pub cost_governance: LlmCostGovernanceConfig,
    #[serde(default)]
    pub hosted_relay: Option<LlmHostedRelayConfig>,
    #[serde(default)]
    pub openai: Option<LlmVendorConfig>,
    #[serde(default)]
    pub google: Option<LlmVendorConfig>,
    #[serde(default)]
    pub anthropic: Option<LlmVendorConfig>,
    #[serde(default)]
    pub grok: Option<LlmVendorConfig>,
    #[serde(default)]
    pub deepseek: Option<LlmVendorConfig>,
    #[serde(default)]
    pub qwen: Option<LlmVendorConfig>,
    #[serde(default)]
    pub minimax: Option<LlmVendorConfig>,
    #[serde(default)]
    pub mimo: Option<LlmVendorConfig>,
    #[serde(default)]
    pub custom: Option<LlmVendorConfig>,
    // Legacy flat provider list, kept for backward compatibility.
    #[serde(default)]
    pub providers: Vec<LlmProviderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmHostedRelayConfig {
    #[serde(default)]
    pub enabled: bool,
    pub vendor: String,
    pub model: String,
    pub base_url: String,
    #[serde(default)]
    pub daily_request_limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LlmModelClassConfig {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmCostGovernanceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub soft_task_usd: Option<f64>,
    #[serde(default)]
    pub soft_user_24h_usd: Option<f64>,
    #[serde(default)]
    pub soft_provider_24h_usd: Option<f64>,
    #[serde(default)]
    pub hard_task_usd: Option<f64>,
    #[serde(default = "default_llm_cost_checkpoint_retry_after_seconds")]
    pub checkpoint_retry_after_seconds: u64,
}

impl Default for LlmCostGovernanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            soft_task_usd: None,
            soft_user_24h_usd: None,
            soft_provider_24h_usd: None,
            hard_task_usd: None,
            checkpoint_retry_after_seconds: default_llm_cost_checkpoint_retry_after_seconds(),
        }
    }
}

fn default_llm_cost_checkpoint_retry_after_seconds() -> u64 {
    3_600
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmModelPricingConfig {
    pub provider: String,
    pub model: String,
    pub effective_from: String,
    #[serde(default = "default_llm_pricing_currency")]
    pub currency: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub input_usd_per_million: Option<f64>,
    #[serde(default)]
    pub output_usd_per_million: Option<f64>,
    #[serde(default)]
    pub cache_read_usd_per_million: Option<f64>,
    #[serde(default)]
    pub cache_write_usd_per_million: Option<f64>,
    #[serde(default)]
    pub reasoning_usd_per_million: Option<f64>,
    #[serde(default)]
    pub long_context_threshold_tokens: Option<u64>,
    #[serde(default)]
    pub long_context_input_usd_per_million: Option<f64>,
    #[serde(default)]
    pub long_context_output_usd_per_million: Option<f64>,
    #[serde(default)]
    pub long_context_cache_read_usd_per_million: Option<f64>,
}

fn default_llm_pricing_currency() -> String {
    "USD".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmVendorConfig {
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    pub model: String,
    /// Optional model/provider context window override. Use this for providers
    /// whose compatible API does not expose a reliable model capacity.
    #[serde(default)]
    pub context_window_tokens: Option<usize>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default = "default_llm_input_modalities")]
    pub input_modalities: Vec<String>,
    #[serde(default = "default_llm_supports_tools")]
    pub supports_tools: bool,
    #[serde(default)]
    pub expected_latency_ms: Option<u64>,
    #[serde(default = "default_llm_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_llm_max_concurrency")]
    pub max_concurrency: usize,
    /// 支持双协议的厂商使用：clawd 合成 `vendor-*` 时的协议。未填或空字符串默认
    /// `openai_compat`；`anthropic_claude`（及别名）走 Anthropic Messages。其它厂商忽略。
    #[serde(default)]
    pub api_format: Option<String>,
    /// Phase 2.5: per-vendor 默认参数，从 toml 子表 `[llm.<vendor>.params]` 读取，
    /// 在 [`LlmGateway::build_providers`] 合成 `LlmProviderConfig` 时透传到
    /// [`LlmProviderConfig::params`]。全字段可选，空表 = 沿用 vendor 默认行为。
    /// 例：
    /// ```toml
    /// [llm.qwen.params]
    /// default_temperature = 0.4
    /// default_max_tokens  = 2048
    /// top_p               = 0.9
    /// ```
    #[serde(default)]
    pub params: LlmProviderParams,
}

fn default_llm_input_modalities() -> Vec<String> {
    vec!["text".to_string()]
}

fn default_llm_supports_tools() -> bool {
    false
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmProviderConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub provider_type: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Optional resolved context window for prompt budgeting.
    #[serde(default)]
    pub context_window_tokens: Option<usize>,
    #[serde(default = "default_llm_input_modalities")]
    pub input_modalities: Vec<String>,
    #[serde(default = "default_llm_supports_tools")]
    pub supports_tools: bool,
    #[serde(default)]
    pub expected_latency_ms: Option<u64>,
    pub priority: i32,
    pub timeout_seconds: u64,
    pub max_concurrency: usize,
    /// Phase 2.5: per-provider 默认参数。toml 里写在 `[llm_providers.params]` 子表，
    /// 例如：
    /// ```toml
    /// [[llm_providers]]
    /// name = "vendor-qwen"
    /// type = "openai_compat"
    /// ...
    /// [llm_providers.params]
    /// default_temperature = 0.4
    /// default_max_tokens  = 2048
    /// top_p               = 0.9
    /// ```
    /// chat 调用如果通过 `ChatRequestHints` 显式传了 temperature/max_tokens，
    /// 优先用 hints；否则 fallback 到这里的 default 值；都没写则不向 provider
    /// 显式发字段，由 vendor 走自己的默认（与 Phase 2.5 之前行为一致）。
    /// 全部 `Option`，缺省即"不主动设置"，**完全向后兼容**。
    #[serde(default)]
    pub params: LlmProviderParams,
}

/// Phase 2.5: per-provider 默认参数（来自 `[llm_providers.params]` 子表）。
/// 全部字段都是 `Option`，没在 toml 里写就保持 `None`，对外行为与不带本字段时
/// 完全一致——目的是把以前散落在 provider 实现里的"硬编码默认值"（OpenAI compat
/// 的 `stream=false`、Anthropic 的 `max_tokens=4096` 等）显式化为可观测、可改的
/// 配置入口，但不强制每个 provider 都填。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LlmProviderParams {
    /// The managed relay may bootstrap its private bearer key after a one-time
    /// Slot 0 challenge. This is derived from `[llm.hosted_relay]`; ordinary
    /// provider configuration must not set it as an authorization bypass.
    #[serde(default)]
    pub device_key_enrollment: bool,
    /// chat-class 调用没传 hints.temperature 时使用；不设走 vendor 默认。
    #[serde(default)]
    pub default_temperature: Option<f64>,
    /// chat-class 调用没传 hints.max_tokens 时使用；anthropic_claude 协议
    /// 因协议要求必须传 max_tokens，没在 hints/params 里写时仍 fallback 到 4096。
    #[serde(default)]
    pub default_max_tokens: Option<u64>,
    /// 透传给 OpenAI compat / Gemini / Anthropic 的 `top_p`（核采样）。
    #[serde(default)]
    pub top_p: Option<f64>,
    /// 是否走 SSE 流式响应。默认 false（clawd 当前不消费 stream，留作未来用）。
    /// 仅 OpenAI compat 协议下生效；Gemini/Anthropic 路由暂忽略此字段。
    #[serde(default)]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SkillsConfig {
    #[serde(default = "default_skill_timeout_seconds")]
    pub skill_timeout_seconds: u64,
    #[serde(default = "default_skill_max_concurrency")]
    pub skill_max_concurrency: usize,
    /// Reuse only host-verified stateless/read-only outer runner processes.
    #[serde(default)]
    pub runner_warm_pool_enabled: bool,
    #[serde(default = "default_runner_warm_pool_max_idle_per_skill")]
    pub runner_warm_pool_max_idle_per_skill: usize,
    #[serde(default = "default_runner_warm_pool_min_available_memory_mib")]
    pub runner_warm_pool_min_available_memory_mib: u64,
    #[serde(default = "default_runner_warm_pool_idle_timeout_seconds")]
    pub runner_warm_pool_idle_timeout_seconds: u64,
    #[serde(default = "default_skills_list")]
    pub skills_list: Vec<String>,
    #[serde(default)]
    pub skill_switches: HashMap<String, bool>,
    /// 已从活动运行时移除、但仍可从 Skill Store 重新安装的技能。
    #[serde(default = "default_uninstalled_skills")]
    pub uninstalled_skills: Vec<String>,
    /// 技能注册表文件路径（相对 workspace 或绝对）。生产启动要求该文件存在且有效。
    #[serde(default = "default_skill_registry_path")]
    pub registry_path: Option<String>,
}

fn default_skill_registry_path() -> Option<String> {
    Some("configs/skills_registry.toml".to_string())
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skill_timeout_seconds: default_skill_timeout_seconds(),
            skill_max_concurrency: default_skill_max_concurrency(),
            runner_warm_pool_enabled: false,
            runner_warm_pool_max_idle_per_skill: default_runner_warm_pool_max_idle_per_skill(),
            runner_warm_pool_min_available_memory_mib:
                default_runner_warm_pool_min_available_memory_mib(),
            runner_warm_pool_idle_timeout_seconds: default_runner_warm_pool_idle_timeout_seconds(),
            skills_list: default_skills_list(),
            skill_switches: HashMap::new(),
            uninstalled_skills: default_uninstalled_skills(),
            registry_path: default_skill_registry_path(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LimitsConfig {
    #[serde(default = "default_global_rpm")]
    pub global_rpm: usize,
    #[serde(default = "default_user_rpm")]
    pub user_rpm: usize,
    #[serde(default = "default_context_tool_observation_reserve_tokens")]
    pub context_tool_observation_reserve_tokens: usize,
    #[serde(default = "default_context_estimator_safety_margin_tokens")]
    pub context_estimator_safety_margin_tokens: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            global_rpm: default_global_rpm(),
            user_rpm: default_user_rpm(),
            context_tool_observation_reserve_tokens:
                default_context_tool_observation_reserve_tokens(),
            context_estimator_safety_margin_tokens: default_context_estimator_safety_margin_tokens(
            ),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MaintenanceConfig {
    #[serde(default = "default_cleanup_interval_seconds")]
    pub cleanup_interval_seconds: u64,
    #[serde(default = "default_tasks_retention_days")]
    pub tasks_retention_days: u64,
    #[serde(default = "default_tasks_max_rows")]
    pub tasks_max_rows: usize,
    #[serde(default = "default_audit_retention_days")]
    pub audit_retention_days: u64,
    #[serde(default = "default_audit_max_rows")]
    pub audit_max_rows: usize,
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            cleanup_interval_seconds: default_cleanup_interval_seconds(),
            tasks_retention_days: default_tasks_retention_days(),
            tasks_max_rows: default_tasks_max_rows(),
            audit_retention_days: default_audit_retention_days(),
            audit_max_rows: default_audit_max_rows(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_memory_config_path")]
    pub config_path: String,
    #[serde(default = "default_memory_mark_llm_reply_in_short_term")]
    pub mark_llm_reply_in_short_term: bool,
    #[serde(default = "default_memory_prefer_llm_assistant_memory")]
    pub prefer_llm_assistant_memory: bool,
    #[serde(default = "default_memory_prompt_recall_limit")]
    pub prompt_recall_limit: usize,
    #[serde(default = "default_memory_recall_limit")]
    pub recall_limit: usize,
    #[serde(default = "default_memory_item_max_chars")]
    pub item_max_chars: usize,
    #[serde(default = "default_memory_prompt_max_chars")]
    pub prompt_max_chars: usize,
    #[serde(default = "default_memory_retention_days")]
    pub retention_days: u64,
    #[serde(default = "default_memory_max_rows")]
    pub max_rows: usize,
    #[serde(default = "default_memory_long_term_enabled")]
    pub long_term_enabled: bool,
    #[serde(default = "default_memory_long_term_every_rounds")]
    pub long_term_every_rounds: usize,
    #[serde(default = "default_memory_long_term_source_rounds")]
    pub long_term_source_rounds: usize,
    #[serde(default = "default_memory_long_term_summary_max_chars")]
    pub long_term_summary_max_chars: usize,
    #[serde(default = "default_memory_long_term_recall_max_chars")]
    pub long_term_recall_max_chars: usize,
    #[serde(default = "default_memory_long_term_retention_days")]
    pub long_term_retention_days: u64,
    #[serde(default = "default_memory_long_term_max_rows")]
    pub long_term_max_rows: usize,
    #[serde(default = "default_memory_write_filter_enabled")]
    pub write_filter_enabled: bool,
    #[serde(default = "default_memory_write_min_chars")]
    pub write_min_chars: usize,
    #[serde(default = "default_memory_enable_preference_extraction")]
    pub enable_preference_extraction: bool,
    #[serde(default = "default_memory_llm_preference_fallback_enabled")]
    pub llm_preference_fallback_enabled: bool,
    #[serde(default = "default_memory_llm_preference_min_confidence")]
    pub llm_preference_min_confidence: f32,
    #[serde(default = "default_memory_llm_preference_max_chars")]
    pub llm_preference_max_chars: usize,
    #[serde(default = "default_memory_preference_recall_limit")]
    pub preference_recall_limit: usize,
    #[serde(default = "default_memory_recent_relevance_enabled")]
    pub recent_relevance_enabled: bool,
    #[serde(default = "default_memory_recent_relevance_min_score")]
    pub recent_relevance_min_score: f32,
    #[serde(default = "default_memory_safety_filter_enabled")]
    pub safety_filter_enabled: bool,
    #[serde(default = "default_memory_long_term_refresh_min_new_chars")]
    pub long_term_refresh_min_new_chars: usize,
    #[serde(default = "default_memory_long_term_refresh_max_repeat_ratio")]
    pub long_term_refresh_max_repeat_ratio: f32,
    #[serde(default = "default_memory_route_memory_enabled")]
    pub route_memory_enabled: bool,
    #[serde(default = "default_memory_route_memory_max_chars")]
    pub route_memory_max_chars: usize,
    #[serde(default = "default_memory_skill_memory_enabled")]
    pub skill_memory_enabled: bool,
    #[serde(default = "default_memory_skill_memory_max_chars")]
    pub skill_memory_max_chars: usize,
    #[serde(default = "default_memory_schedule_memory_include_long_term")]
    pub schedule_memory_include_long_term: bool,
    #[serde(default = "default_memory_schedule_memory_include_preferences")]
    pub schedule_memory_include_preferences: bool,
    #[serde(default = "default_memory_schedule_memory_max_chars")]
    pub schedule_memory_max_chars: usize,
    #[serde(default = "default_memory_image_memory_include_long_term")]
    pub image_memory_include_long_term: bool,
    #[serde(default = "default_memory_image_memory_include_preferences")]
    pub image_memory_include_preferences: bool,
    #[serde(default = "default_memory_image_memory_max_chars")]
    pub image_memory_max_chars: usize,
    #[serde(default = "default_memory_hybrid_recall_enabled")]
    pub hybrid_recall_enabled: bool,
    #[serde(default = "default_memory_fts_candidate_limit")]
    pub fts_candidate_limit: usize,
    #[serde(default = "default_memory_vector_candidate_limit")]
    pub vector_candidate_limit: usize,
    #[serde(default = "default_memory_trigger_anchor_limit")]
    pub trigger_anchor_limit: usize,
    #[serde(default = "default_memory_fact_card_limit")]
    pub fact_card_limit: usize,
    #[serde(default = "default_memory_chat_memory_budget_chars")]
    pub chat_memory_budget_chars: usize,
    #[serde(default = "default_memory_agent_memory_budget_chars")]
    pub agent_memory_budget_chars: usize,
    #[serde(default = "default_memory_route_trigger_budget_chars")]
    pub route_trigger_budget_chars: usize,
    #[serde(default = "default_memory_embedding_model")]
    pub embedding_model: String,
    #[serde(default = "default_memory_embedding_dims")]
    pub embedding_dims: usize,
    #[serde(default = "default_memory_embedding_version")]
    pub embedding_version: String,
    #[serde(default = "default_memory_embedding_batch_size")]
    pub embedding_batch_size: usize,
    #[serde(default = "default_memory_embedding_provider_kind")]
    pub embedding_provider_kind: String,
    #[serde(default)]
    pub embedding_endpoint_ref: String,
    #[serde(default)]
    pub embedding_credential_ref: String,
    #[serde(default = "default_memory_embedding_normalization")]
    pub embedding_normalization: String,
    #[serde(default = "default_memory_embedding_metric")]
    pub embedding_metric: String,
    #[serde(default = "default_memory_embedding_query_timeout_ms")]
    pub embedding_query_timeout_ms: u64,
    #[serde(default = "default_memory_embedding_connect_timeout_ms")]
    pub embedding_connect_timeout_ms: u64,
    #[serde(default = "default_memory_embedding_idle_timeout_ms")]
    pub embedding_idle_timeout_ms: u64,
    #[serde(default = "default_memory_embedding_retry_max_attempts")]
    pub embedding_retry_max_attempts: usize,
    #[serde(default = "default_memory_embedding_circuit_failure_threshold")]
    pub embedding_circuit_failure_threshold: usize,
    #[serde(default = "default_memory_embedding_circuit_reset_seconds")]
    pub embedding_circuit_reset_seconds: u64,
    #[serde(default = "default_memory_embedding_query_cache_ttl_seconds")]
    pub embedding_query_cache_ttl_seconds: u64,
    #[serde(default = "default_memory_embedding_query_cache_max_bytes")]
    pub embedding_query_cache_max_bytes: usize,
    #[serde(default = "default_memory_embedding_max_request_bytes")]
    pub embedding_max_request_bytes: usize,
    #[serde(default = "default_memory_embedding_remote_opt_in_required")]
    pub embedding_remote_opt_in_required: bool,
    #[serde(default = "default_memory_embedding_reindex_batch_delay_ms")]
    pub embedding_reindex_batch_delay_ms: u64,
    #[serde(default = "default_memory_reindex_on_startup")]
    pub reindex_on_startup: bool,
    #[serde(default = "default_memory_background_job_concurrency")]
    pub background_job_concurrency: usize,
    #[serde(default = "default_memory_background_idle_seconds")]
    pub background_idle_seconds: u64,
    #[serde(default = "default_memory_background_lease_seconds")]
    pub background_lease_seconds: u64,
    #[serde(default = "default_memory_background_max_attempts")]
    pub background_max_attempts: usize,
    #[serde(default = "default_memory_raw_candidate_retention_days")]
    pub raw_candidate_retention_days: u64,
    #[serde(default = "default_memory_raw_candidate_max_rows_per_principal")]
    pub raw_candidate_max_rows_per_principal: usize,
    #[serde(default = "default_memory_storage_soft_limit_bytes")]
    pub storage_soft_limit_bytes: u64,
    #[serde(default = "default_memory_principal_max_bytes")]
    pub principal_max_bytes: u64,
    #[serde(default = "default_memory_principal_background_cost_microunits")]
    pub principal_background_cost_microunits: u64,
    #[serde(default)]
    pub extract_provider: String,
    #[serde(default)]
    pub extract_model: String,
    #[serde(default)]
    pub consolidation_provider: String,
    #[serde(default)]
    pub consolidation_model: String,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        toml::from_str(include_str!("../../../configs/memory.toml"))
            .expect("tracked configs/memory.toml must satisfy MemoryConfig")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolSandboxMode {
    ReadOnly,
    #[default]
    WorkspaceWrite,
    IsolatedWorktree,
    DangerFull,
}

impl ToolSandboxMode {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::WorkspaceWrite => "workspace_write",
            Self::IsolatedWorktree => "isolated_worktree",
            Self::DangerFull => "danger_full",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolSandboxBackend {
    #[default]
    Auto,
    Bubblewrap,
    MacosSeatbelt,
    RemoteContainer,
}

impl ToolSandboxBackend {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Bubblewrap => "bubblewrap",
            Self::MacosSeatbelt => "macos_seatbelt",
            Self::RemoteContainer => "remote_container",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalPolicy {
    Never,
    #[default]
    OnRisk,
    OnRequest,
    Always,
}

impl ToolApprovalPolicy {
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::OnRisk => "on_risk",
            Self::OnRequest => "on_request",
            Self::Always => "always",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolsConfig {
    #[serde(default = "default_tools_profile")]
    pub access_profile: String,
    #[serde(default)]
    pub sandbox_mode: ToolSandboxMode,
    #[serde(default)]
    pub sandbox_backend: ToolSandboxBackend,
    #[serde(default)]
    pub approval_policy: ToolApprovalPolicy,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default = "default_tool_access_profiles")]
    pub profiles: HashMap<String, Vec<String>>,
    #[serde(default = "default_tool_cmd_timeout_seconds")]
    pub cmd_timeout_seconds: u64,
    #[serde(default = "default_tool_cmd_idle_timeout_seconds")]
    pub cmd_idle_timeout_seconds: u64,
    #[serde(default = "default_tool_cmd_async_retention_seconds")]
    pub cmd_async_retention_seconds: u64,
    #[serde(default = "default_tool_cmd_terminate_grace_seconds")]
    pub cmd_terminate_grace_seconds: u64,
    #[serde(default = "default_tool_cmd_max_output_bytes")]
    pub cmd_max_output_bytes: usize,
    #[serde(default = "default_tool_max_cmd_length")]
    pub max_cmd_length: usize,
    #[serde(default)]
    pub allow_path_outside_workspace: bool,
    #[serde(default)]
    pub allow_sudo: bool,
    #[serde(default)]
    pub by_provider: HashMap<String, ProviderToolsConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkspaceInstructionsConfig {
    #[serde(default)]
    pub enabled_for_coding: bool,
    #[serde(default)]
    pub enabled_for_non_coding: bool,
    #[serde(default)]
    pub filenames: Vec<String>,
    /// Lower-precedence user instruction sources. Paths are trusted release
    /// templates and may use `<config_home>` and `<product_namespace>`.
    #[serde(default)]
    pub user_instruction_paths: Vec<String>,
    #[serde(default)]
    pub max_total_bytes: usize,
    #[serde(default)]
    pub max_file_bytes: usize,
    #[serde(default)]
    pub max_files: usize,
}

impl WorkspaceInstructionsConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled_for_coding && !self.enabled_for_non_coding {
            return Ok(());
        }
        if self.filenames.is_empty() || self.filenames.len() > 8 {
            return Err("workspace_instruction_filenames_invalid".to_string());
        }
        if self.filenames.iter().any(|name| {
            let path = std::path::Path::new(name);
            name.trim().is_empty()
                || name.len() > 128
                || path.components().count() != 1
                || !matches!(
                    path.components().next(),
                    Some(std::path::Component::Normal(_))
                )
        }) {
            return Err("workspace_instruction_filename_invalid".to_string());
        }
        let unique_filenames = self
            .filenames
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        if unique_filenames.len() != self.filenames.len() {
            return Err("workspace_instruction_filenames_duplicate".to_string());
        }
        if self.user_instruction_paths.len() > 8
            || self.user_instruction_paths.iter().any(|path| {
                let path = path.trim();
                path.is_empty()
                    || path.len() > 512
                    || path.contains('\0')
                    || path.contains("..")
                    || path
                        .replace("<config_home>", "")
                        .replace("<product_namespace>", "")
                        .contains(|ch| matches!(ch, '<' | '>'))
            })
        {
            return Err("workspace_user_instruction_paths_invalid".to_string());
        }
        if !(1_024..=262_144).contains(&self.max_total_bytes)
            || !(1_024..=1_048_576).contains(&self.max_file_bytes)
            || !(1..=64).contains(&self.max_files)
        {
            return Err("workspace_instruction_budget_invalid".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AutoReviewConfig {
    pub enabled: bool,
    pub review_role: String,
    pub blocking: bool,
}

impl Default for AutoReviewConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            review_role: "review".to_string(),
            blocking: false,
        }
    }
}

impl AutoReviewConfig {
    pub fn validate(&self) -> Result<(), String> {
        let valid_role = !self.review_role.is_empty()
            && self.review_role.len() <= 80
            && self
                .review_role
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_');
        valid_role
            .then_some(())
            .ok_or_else(|| "auto_review_role_invalid".to_string())
    }
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            access_profile: default_tools_profile(),
            sandbox_mode: ToolSandboxMode::default(),
            sandbox_backend: ToolSandboxBackend::default(),
            approval_policy: ToolApprovalPolicy::default(),
            allow: Vec::new(),
            deny: Vec::new(),
            profiles: default_tool_access_profiles(),
            cmd_timeout_seconds: default_tool_cmd_timeout_seconds(),
            cmd_idle_timeout_seconds: default_tool_cmd_idle_timeout_seconds(),
            cmd_async_retention_seconds: default_tool_cmd_async_retention_seconds(),
            cmd_terminate_grace_seconds: default_tool_cmd_terminate_grace_seconds(),
            cmd_max_output_bytes: default_tool_cmd_max_output_bytes(),
            max_cmd_length: default_tool_max_cmd_length(),
            allow_path_outside_workspace: false,
            allow_sudo: false,
            by_provider: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProviderToolsConfig {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageSkillConfig {
    #[serde(default = "default_image_default_output_dir")]
    pub default_output_dir: String,
    #[serde(default)]
    pub default_vendor: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub openai_models: Vec<String>,
    #[serde(default)]
    pub google_models: Vec<String>,
    #[serde(default)]
    pub anthropic_models: Vec<String>,
    #[serde(default)]
    pub grok_models: Vec<String>,
    #[serde(default)]
    pub deepseek_models: Vec<String>,
    #[serde(default)]
    pub qwen_models: Vec<String>,
    #[serde(default)]
    pub native_models: Vec<String>,
    #[serde(default)]
    pub custom_models: Vec<String>,
    #[serde(default = "default_image_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_image_max_concurrency")]
    pub max_concurrency: usize,
    #[serde(default = "default_image_max_images")]
    pub max_images: usize,
    #[serde(default = "default_image_max_input_bytes")]
    pub max_input_bytes: usize,
    #[serde(default)]
    pub local_auto_upload_enabled: bool,
    #[serde(default)]
    pub oss_access_key_id: Option<String>,
    #[serde(default)]
    pub oss_access_key_secret: Option<String>,
    #[serde(default)]
    pub oss_bucket: Option<String>,
    #[serde(default)]
    pub oss_endpoint: Option<String>,
    #[serde(default)]
    pub oss_object_prefix: Option<String>,
    #[serde(default)]
    pub oss_url_ttl_seconds: Option<u64>,
}

impl Default for ImageSkillConfig {
    fn default() -> Self {
        Self {
            default_output_dir: default_image_default_output_dir(),
            default_vendor: None,
            default_model: None,
            models: Vec::new(),
            openai_models: Vec::new(),
            google_models: Vec::new(),
            anthropic_models: Vec::new(),
            grok_models: Vec::new(),
            deepseek_models: Vec::new(),
            qwen_models: Vec::new(),
            native_models: Vec::new(),
            custom_models: Vec::new(),
            timeout_seconds: default_image_timeout_seconds(),
            max_concurrency: default_image_max_concurrency(),
            max_images: default_image_max_images(),
            max_input_bytes: default_image_max_input_bytes(),
            local_auto_upload_enabled: false,
            oss_access_key_id: None,
            oss_access_key_secret: None,
            oss_bucket: None,
            oss_endpoint: None,
            oss_object_prefix: None,
            oss_url_ttl_seconds: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoutingConfig {
    #[serde(default)]
    pub debug_log_prompt: bool,
    /// §3.1: 是否打开 ask 状态机 transition 日志（`[ask_state]`）。
    /// 默认开（生命周期可观测性建议常驻）；如果嫌噪音可关。
    #[serde(default = "default_routing_debug_log_ask_state")]
    pub debug_log_ask_state: bool,
    #[serde(default = "default_routing_default_locator_search_dir")]
    pub default_locator_search_dir: String,
}

fn default_routing_debug_log_ask_state() -> bool {
    true
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            debug_log_prompt: false,
            debug_log_ask_state: default_routing_debug_log_ask_state(),
            default_locator_search_dir: default_routing_default_locator_search_dir(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PersonaConfig {
    #[serde(default = "default_persona_profile")]
    pub profile: String,
    #[serde(default = "default_persona_dir")]
    pub dir: String,
}

impl Default for PersonaConfig {
    fn default() -> Self {
        Self {
            profile: default_persona_profile(),
            dir: default_persona_dir(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandIntentConfig {
    #[serde(default = "default_command_intent_default_locale")]
    pub default_locale: String,
}

impl Default for CommandIntentConfig {
    fn default() -> Self {
        Self {
            default_locale: default_command_intent_default_locale(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleConfig {
    #[serde(default = "default_schedule_timezone")]
    pub timezone: String,
    #[serde(default = "default_schedule_intent_prompt_path")]
    pub intent_prompt_path: String,
    #[serde(default = "default_schedule_intent_rules_path")]
    pub intent_rules_path: String,
    #[serde(default = "default_schedule_locale")]
    pub locale: String,
    #[serde(default = "default_schedule_i18n_dir")]
    pub i18n_dir: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct SplitImageConfig {
    #[serde(default)]
    image_vision: ImageSkillConfig,
    #[serde(default)]
    image_generation: ImageSkillConfig,
    #[serde(default)]
    image_edit: ImageSkillConfig,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            timezone: default_schedule_timezone(),
            intent_prompt_path: default_schedule_intent_prompt_path(),
            intent_rules_path: default_schedule_intent_rules_path(),
            locale: default_schedule_locale(),
            i18n_dir: default_schedule_i18n_dir(),
        }
    }
}

/// §3.5d: prompts hot-reload 运维开关。
///
/// - `reload_on_sighup`：进程收到 SIGHUP 信号时，是否触发
///   [`crate::bootstrap::prompts::reload_runtime_prompts`] 把 persona /
///   schedule.intent_prompt / schedule.intent_rules 的内存快照与磁盘对齐，并
///   复跑核心 prompt 校验。
///   - 默认 true：本地开发体验佳（编辑 → kill -HUP → 下一次 LLM 调用即生效）。
///   - 生产环境若希望显式禁用 SIGHUP 行为（例如 systemd 用 SIGHUP 做 reload
///     其它资源），可显式设为 false。
/// - `strict_validation_at_startup`：启动时若核心 prompt 只能退回到 embedded
///   `include_str!` 常量，是否直接拒绝启动。
///   - 默认 false：兼容当前 warn-only 行为。
///   - 生产环境建议显式打开，避免部署漏带 `prompts/` 树时静默跑旧模板。
/// - `config_path`：reload 时重读的 config 文件路径。默认与 clawd 启动相同：
///   `configs/config.toml`。允许覆盖以适配多套 config 共存的部署。
#[derive(Debug, Clone, Deserialize)]
pub struct PromptsConfig {
    #[serde(default = "default_prompts_reload_on_sighup")]
    pub reload_on_sighup: bool,
    #[serde(default)]
    pub strict_validation_at_startup: bool,
    #[serde(default = "default_prompts_config_path")]
    pub config_path: String,
}

impl Default for PromptsConfig {
    fn default() -> Self {
        Self {
            reload_on_sighup: default_prompts_reload_on_sighup(),
            strict_validation_at_startup: false,
            config_path: default_prompts_config_path(),
        }
    }
}

fn default_prompts_reload_on_sighup() -> bool {
    true
}

fn default_prompts_config_path() -> String {
    "configs/config.toml".to_string()
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
