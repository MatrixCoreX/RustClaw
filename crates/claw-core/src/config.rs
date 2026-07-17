use std::collections::HashMap;

use serde::Deserialize;

mod defaults;
mod runtime;

use defaults::*;
pub use defaults::{base_skill_names, core_skills_always_enabled};

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
    pub self_extension: SelfExtensionConfig,
    #[serde(default)]
    pub prompts: PromptsConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub listen: String,
    pub request_timeout_seconds: u64,
    /// 可选。telegramd 等连 clawd 的地址；未设则用 http://{listen}（listen 为 0.0.0.0 时自动改为 127.0.0.1）。
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
    #[serde(default = "default_telegram_auto_vision_on_image_only")]
    pub auto_vision_on_image_only: bool,
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
    #[serde(default = "default_telegram_voice_mode_nl_intent_enabled")]
    pub voice_mode_nl_intent_enabled: bool,
    #[serde(default)]
    pub voice_reply_mode_by_chat: HashMap<String, String>,
    #[serde(default = "default_telegram_max_audio_input_bytes")]
    pub max_audio_input_bytes: usize,
    #[serde(default = "default_telegram_ephemeral_image_saved_seconds")]
    pub ephemeral_image_saved_seconds: u64,
    #[serde(default)]
    pub sendfile: SendfileConfig,
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

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_agent_id")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
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
            persona_prompt: String::new(),
            preferred_vendor: None,
            preferred_model: None,
            allowed_skills: Vec::new(),
        }
    }
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
            auto_vision_on_image_only: default_telegram_auto_vision_on_image_only(),
            image_inbox_dir: default_telegram_image_inbox_dir(),
            video_inbox_dir: default_telegram_video_inbox_dir(),
            file_inbox_dir: default_telegram_file_inbox_dir(),
            audio_inbox_dir: default_telegram_audio_inbox_dir(),
            voice_reply_mode: default_telegram_voice_reply_mode(),
            voice_mode_nl_intent_enabled: default_telegram_voice_mode_nl_intent_enabled(),
            voice_reply_mode_by_chat: HashMap::new(),
            max_audio_input_bytes: default_telegram_max_audio_input_bytes(),
            ephemeral_image_saved_seconds: default_telegram_ephemeral_image_saved_seconds(),
            sendfile: SendfileConfig::default(),
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
        }
    }
}

fn default_webd_session_cookie_name() -> String {
    "webd_sid".to_string()
}

fn default_webd_session_ttl_seconds() -> u64 {
    86400
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
pub struct SendfileConfig {
    #[serde(default = "default_sendfile_admin_only")]
    pub admin_only: bool,
    #[serde(default = "default_sendfile_full_access")]
    pub full_access: bool,
    #[serde(default = "default_sendfile_allowed_dirs")]
    pub allowed_dirs: Vec<String>,
}

impl Default for SendfileConfig {
    fn default() -> Self {
        Self {
            admin_only: default_sendfile_admin_only(),
            full_access: default_sendfile_full_access(),
            allowed_dirs: default_sendfile_allowed_dirs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub sqlite_path: String,
    pub busy_timeout_ms: u64,
    /// SQLite 连接池最大连接数。≥ 2，默认 8（与 worker 并发*2 + http 路径预留）。
    /// 配合 WAL 模式：reader 不阻塞 writer，多 reader 并发；writer 串行（SQLite 限制）。
    #[serde(default = "default_db_pool_max_size")]
    pub pool_max_size: u32,
    /// Phase 2.2 Stage 2: 把 audit_logs 拆到独立 SQLite 文件 +
    /// 独立连接池，让任务流水线（tasks/scheduled_jobs/...）的 writer 锁
    /// 不再被 audit append 抢占。默认 `data/rustclaw_audit.db`，
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

fn default_audit_sqlite_path() -> String {
    "data/rustclaw_audit.db".to_string()
}

fn default_audit_pool_max_size() -> u32 {
    2
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    // Phase 0.1 / L4: 缺省值收敛为安全基线。
    // 历史上这四个字段在 struct 上没有 `#[serde(default)]`，部署模板里硬编码
    // `task_timeout_seconds = 86400`（24h），任何继承该模板的环境都会把单任务
    // 硬超时拉到一天。补上 serde default 后，未在 toml 中显式声明就走安全值：
    // - concurrency=1（单 worker，避免抢资源）
    // - poll_interval_ms=500
    // - queue_limit=64
    // - task_timeout_seconds=3600（所有任务类别的管理员硬上限，远小于原 24h；
    //   结构化 budget_profile 可选择更短预算，但不能突破该上限）
    // 现存 demo 模板会显式设大值，行为不变；新部署默认即安全。
    #[serde(default = "default_worker_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_worker_task_timeout_seconds")]
    pub task_timeout_seconds: u64,
    #[serde(default = "default_worker_llm_max_calls_per_task")]
    pub llm_max_calls_per_task: u64,
    #[serde(default = "default_worker_llm_total_timeout_seconds")]
    pub llm_total_timeout_seconds: u64,
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
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            concurrency: default_worker_concurrency(),
            task_timeout_seconds: default_worker_task_timeout_seconds(),
            llm_max_calls_per_task: default_worker_llm_max_calls_per_task(),
            llm_total_timeout_seconds: default_worker_llm_total_timeout_seconds(),
            poll_interval_ms: default_worker_poll_interval_ms(),
            queue_limit: default_worker_queue_limit(),
            task_heartbeat_seconds: default_worker_task_heartbeat_seconds(),
            running_no_progress_timeout_seconds: default_worker_running_no_progress_timeout_seconds(
            ),
            running_recovery_check_interval_seconds:
                default_worker_running_recovery_check_interval_seconds(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LlmConfig {
    #[serde(default)]
    pub selected_vendor: Option<String>,
    #[serde(default)]
    pub selected_model: Option<String>,
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
    #[serde(default = "default_skills_list")]
    pub skills_list: Vec<String>,
    #[serde(default)]
    pub skill_switches: HashMap<String, bool>,
    /// 技能注册表文件路径（相对 workspace 或绝对）。设则启用 registry 驱动发现/启用/别名/超时。
    #[serde(default)]
    pub registry_path: Option<String>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skill_timeout_seconds: default_skill_timeout_seconds(),
            skill_max_concurrency: default_skill_max_concurrency(),
            skills_list: default_skills_list(),
            skill_switches: HashMap::new(),
            registry_path: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SelfExtensionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub auto_on_capability_gap: bool,
    #[serde(default)]
    pub allow_execute: bool,
    #[serde(default)]
    pub allow_package_install: bool,
    #[serde(default)]
    pub allow_permanent_extension: bool,
    #[serde(default)]
    pub allow_runtime_enable: bool,
}

impl Default for SelfExtensionConfig {
    fn default() -> Self {
        // Phase 0.1: 默认安全收敛。self_extension 会允许 agent 临时生成/安装/
        // 执行脚本，属高权力能力。默认一律关闭，只有当用户在 config.toml 里
        // 显式设为 true 时才启用，避免"未配置即默认全开"的权限面过大问题。
        Self {
            enabled: false,
            auto_on_capability_gap: false,
            allow_execute: false,
            allow_package_install: false,
            allow_permanent_extension: false,
            allow_runtime_enable: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LimitsConfig {
    #[serde(default = "default_global_rpm")]
    pub global_rpm: usize,
    #[serde(default = "default_user_rpm")]
    pub user_rpm: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            global_rpm: default_global_rpm(),
            user_rpm: default_user_rpm(),
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
    #[serde(default = "default_memory_reindex_on_startup")]
    pub reindex_on_startup: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            config_path: default_memory_config_path(),
            mark_llm_reply_in_short_term: default_memory_mark_llm_reply_in_short_term(),
            prefer_llm_assistant_memory: default_memory_prefer_llm_assistant_memory(),
            prompt_recall_limit: default_memory_prompt_recall_limit(),
            recall_limit: default_memory_recall_limit(),
            item_max_chars: default_memory_item_max_chars(),
            prompt_max_chars: default_memory_prompt_max_chars(),
            retention_days: default_memory_retention_days(),
            max_rows: default_memory_max_rows(),
            long_term_enabled: default_memory_long_term_enabled(),
            long_term_every_rounds: default_memory_long_term_every_rounds(),
            long_term_source_rounds: default_memory_long_term_source_rounds(),
            long_term_summary_max_chars: default_memory_long_term_summary_max_chars(),
            long_term_recall_max_chars: default_memory_long_term_recall_max_chars(),
            long_term_retention_days: default_memory_long_term_retention_days(),
            long_term_max_rows: default_memory_long_term_max_rows(),
            write_filter_enabled: default_memory_write_filter_enabled(),
            write_min_chars: default_memory_write_min_chars(),
            enable_preference_extraction: default_memory_enable_preference_extraction(),
            llm_preference_fallback_enabled: default_memory_llm_preference_fallback_enabled(),
            llm_preference_min_confidence: default_memory_llm_preference_min_confidence(),
            llm_preference_max_chars: default_memory_llm_preference_max_chars(),
            preference_recall_limit: default_memory_preference_recall_limit(),
            recent_relevance_enabled: default_memory_recent_relevance_enabled(),
            recent_relevance_min_score: default_memory_recent_relevance_min_score(),
            safety_filter_enabled: default_memory_safety_filter_enabled(),
            long_term_refresh_min_new_chars: default_memory_long_term_refresh_min_new_chars(),
            long_term_refresh_max_repeat_ratio: default_memory_long_term_refresh_max_repeat_ratio(),
            route_memory_enabled: default_memory_route_memory_enabled(),
            route_memory_max_chars: default_memory_route_memory_max_chars(),
            skill_memory_enabled: default_memory_skill_memory_enabled(),
            skill_memory_max_chars: default_memory_skill_memory_max_chars(),
            schedule_memory_include_long_term: default_memory_schedule_memory_include_long_term(),
            schedule_memory_include_preferences: default_memory_schedule_memory_include_preferences(
            ),
            schedule_memory_max_chars: default_memory_schedule_memory_max_chars(),
            image_memory_include_long_term: default_memory_image_memory_include_long_term(),
            image_memory_include_preferences: default_memory_image_memory_include_preferences(),
            image_memory_max_chars: default_memory_image_memory_max_chars(),
            hybrid_recall_enabled: default_memory_hybrid_recall_enabled(),
            fts_candidate_limit: default_memory_fts_candidate_limit(),
            vector_candidate_limit: default_memory_vector_candidate_limit(),
            trigger_anchor_limit: default_memory_trigger_anchor_limit(),
            fact_card_limit: default_memory_fact_card_limit(),
            chat_memory_budget_chars: default_memory_chat_memory_budget_chars(),
            agent_memory_budget_chars: default_memory_agent_memory_budget_chars(),
            route_trigger_budget_chars: default_memory_route_trigger_budget_chars(),
            embedding_model: default_memory_embedding_model(),
            embedding_dims: default_memory_embedding_dims(),
            embedding_version: default_memory_embedding_version(),
            embedding_batch_size: default_memory_embedding_batch_size(),
            reindex_on_startup: default_memory_reindex_on_startup(),
        }
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
    pub approval_policy: ToolApprovalPolicy,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default = "default_tool_cmd_timeout_seconds")]
    pub cmd_timeout_seconds: u64,
    #[serde(default = "default_tool_cmd_idle_timeout_seconds")]
    pub cmd_idle_timeout_seconds: u64,
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

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            access_profile: default_tools_profile(),
            sandbox_mode: ToolSandboxMode::default(),
            approval_policy: ToolApprovalPolicy::default(),
            allow: Vec::new(),
            deny: Vec::new(),
            cmd_timeout_seconds: default_tool_cmd_timeout_seconds(),
            cmd_idle_timeout_seconds: default_tool_cmd_idle_timeout_seconds(),
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
    #[serde(default = "default_routing_locator_scan_max_depth")]
    pub locator_scan_max_depth: usize,
    #[serde(default = "default_routing_locator_scan_max_files")]
    pub locator_scan_max_files: usize,
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
            locator_scan_max_depth: default_routing_locator_scan_max_depth(),
            locator_scan_max_files: default_routing_locator_scan_max_files(),
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
    #[serde(default = "default_command_intent_verify_enforce_enabled")]
    pub verify_enforce_enabled: bool,
}

impl Default for CommandIntentConfig {
    fn default() -> Self {
        Self {
            default_locale: default_command_intent_default_locale(),
            verify_enforce_enabled: default_command_intent_verify_enforce_enabled(),
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
