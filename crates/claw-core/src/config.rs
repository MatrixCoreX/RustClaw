use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub telegram: TelegramConfig,
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
    pub schedule: ScheduleConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub listen: String,
    pub request_timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub admins: Vec<i64>,
    #[serde(default)]
    pub allowlist: Vec<i64>,
    #[serde(default = "default_telegram_quick_result_wait_seconds")]
    pub quick_result_wait_seconds: u64,
    #[serde(default = "default_telegram_auto_vision_on_image_only")]
    pub auto_vision_on_image_only: bool,
    #[serde(default = "default_telegram_audio_inbox_dir")]
    pub audio_inbox_dir: String,
    #[serde(default = "default_telegram_voice_reply_mode")]
    pub voice_reply_mode: String,
    #[serde(default)]
    pub voice_reply_mode_by_chat: HashMap<String, String>,
    #[serde(default = "default_telegram_max_audio_input_bytes")]
    pub max_audio_input_bytes: usize,
    #[serde(default)]
    pub sendfile: SendfileConfig,
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    pub concurrency: usize,
    pub task_timeout_seconds: u64,
    pub poll_interval_ms: u64,
    pub queue_limit: usize,
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
    // Legacy flat provider list, kept for backward compatibility.
    #[serde(default)]
    pub providers: Vec<LlmProviderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmVendorConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default = "default_llm_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_llm_max_concurrency")]
    pub max_concurrency: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmProviderConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub provider_type: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub priority: i32,
    pub timeout_seconds: u64,
    pub max_concurrency: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SkillsConfig {
    #[serde(default = "default_skill_timeout_seconds")]
    pub skill_timeout_seconds: u64,
    #[serde(default = "default_skill_max_concurrency")]
    pub skill_max_concurrency: usize,
    #[serde(default = "default_skill_runner_path")]
    pub skill_runner_path: String,
    #[serde(default = "default_skills_list")]
    pub skills_list: Vec<String>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skill_timeout_seconds: default_skill_timeout_seconds(),
            skill_max_concurrency: default_skill_max_concurrency(),
            skill_runner_path: default_skill_runner_path(),
            skills_list: default_skills_list(),
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
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
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
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolsConfig {
    #[serde(default = "default_tools_profile")]
    pub profile: String,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default = "default_tool_cmd_timeout_seconds")]
    pub cmd_timeout_seconds: u64,
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
            profile: default_tools_profile(),
            allow: Vec::new(),
            deny: Vec::new(),
            cmd_timeout_seconds: default_tool_cmd_timeout_seconds(),
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
    #[serde(default = "default_image_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default = "default_image_max_concurrency")]
    pub max_concurrency: usize,
    #[serde(default = "default_image_max_images")]
    pub max_images: usize,
    #[serde(default = "default_image_max_input_bytes")]
    pub max_input_bytes: usize,
}

impl Default for ImageSkillConfig {
    fn default() -> Self {
        Self {
            default_output_dir: default_image_default_output_dir(),
            default_vendor: None,
            default_model: None,
            timeout_seconds: default_image_timeout_seconds(),
            max_concurrency: default_image_max_concurrency(),
            max_images: default_image_max_images(),
            max_input_bytes: default_image_max_input_bytes(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoutingConfig {
    #[serde(default)]
    pub debug_log_prompt: bool,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self { debug_log_prompt: false }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandIntentConfig {
    #[serde(default = "default_command_intent_default_locale")]
    pub default_locale: String,
    #[serde(default = "default_command_intent_rules_dir")]
    pub rules_dir: String,
    #[serde(default = "default_command_intent_llm_fallback_enabled")]
    pub llm_fallback_enabled: bool,
}

impl Default for CommandIntentConfig {
    fn default() -> Self {
        Self {
            default_locale: default_command_intent_default_locale(),
            rules_dir: default_command_intent_rules_dir(),
            llm_fallback_enabled: default_command_intent_llm_fallback_enabled(),
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
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            timezone: default_schedule_timezone(),
            intent_prompt_path: default_schedule_intent_prompt_path(),
            intent_rules_path: default_schedule_intent_rules_path(),
        }
    }
}

fn default_skill_timeout_seconds() -> u64 {
    30
}

fn default_skill_max_concurrency() -> usize {
    1
}

fn default_skill_runner_path() -> String {
    "target/debug/skill-runner".to_string()
}

fn default_skills_list() -> Vec<String> {
    vec![
        "install_module".to_string(),
        "system_basic".to_string(),
        "http_basic".to_string(),
        "git_basic".to_string(),
        "process_basic".to_string(),
        "package_manager".to_string(),
        "archive_basic".to_string(),
        "db_basic".to_string(),
        "docker_basic".to_string(),
        "fs_search".to_string(),
        "rss_fetch".to_string(),
        "image_vision".to_string(),
        "image_generate".to_string(),
        "image_edit".to_string(),
        "audio_transcribe".to_string(),
        "audio_synthesize".to_string(),
        "health_check".to_string(),
        "log_analyze".to_string(),
        "service_control".to_string(),
        "config_guard".to_string(),
    ]
}

fn default_global_rpm() -> usize {
    60
}

fn default_user_rpm() -> usize {
    20
}

fn default_cleanup_interval_seconds() -> u64 {
    300
}

fn default_tasks_retention_days() -> u64 {
    7
}

fn default_tasks_max_rows() -> usize {
    2000
}

fn default_audit_retention_days() -> u64 {
    14
}

fn default_audit_max_rows() -> usize {
    10000
}

fn default_memory_mark_llm_reply_in_short_term() -> bool {
    true
}

fn default_memory_prefer_llm_assistant_memory() -> bool {
    false
}

fn default_memory_prompt_recall_limit() -> usize {
    3
}

fn default_memory_recall_limit() -> usize {
    8
}

fn default_memory_item_max_chars() -> usize {
    2000
}

fn default_memory_prompt_max_chars() -> usize {
    8000
}

fn default_memory_retention_days() -> u64 {
    30
}

fn default_memory_max_rows() -> usize {
    50000
}

fn default_memory_long_term_enabled() -> bool {
    true
}

fn default_memory_long_term_every_rounds() -> usize {
    6
}

fn default_memory_long_term_source_rounds() -> usize {
    20
}

fn default_memory_long_term_summary_max_chars() -> usize {
    3000
}

fn default_memory_long_term_recall_max_chars() -> usize {
    1200
}

fn default_memory_long_term_retention_days() -> u64 {
    180
}

fn default_memory_long_term_max_rows() -> usize {
    10000
}

fn default_tools_profile() -> String {
    "full".to_string()
}

fn default_telegram_quick_result_wait_seconds() -> u64 {
    3
}

fn default_telegram_auto_vision_on_image_only() -> bool {
    true
}

fn default_telegram_audio_inbox_dir() -> String {
    "audio/upload".to_string()
}

fn default_telegram_voice_reply_mode() -> String {
    "voice".to_string()
}

fn default_telegram_max_audio_input_bytes() -> usize {
    25 * 1024 * 1024
}

fn default_sendfile_admin_only() -> bool {
    false
}

fn default_sendfile_full_access() -> bool {
    true
}

fn default_sendfile_allowed_dirs() -> Vec<String> {
    vec!["image/download".to_string(), "document".to_string()]
}

fn default_tool_cmd_timeout_seconds() -> u64 {
    10
}

fn default_tool_max_cmd_length() -> usize {
    240
}

fn default_llm_timeout_seconds() -> u64 {
    30
}

fn default_llm_max_concurrency() -> usize {
    1
}

fn default_image_default_output_dir() -> String {
    "image".to_string()
}

fn default_image_timeout_seconds() -> u64 {
    90
}

fn default_image_max_concurrency() -> usize {
    1
}

fn default_image_max_images() -> usize {
    6
}

fn default_image_max_input_bytes() -> usize {
    10 * 1024 * 1024
}

fn default_command_intent_default_locale() -> String {
    "zh-CN".to_string()
}

fn default_command_intent_rules_dir() -> String {
    "configs/command_intent".to_string()
}

fn default_command_intent_llm_fallback_enabled() -> bool {
    true
}

fn default_schedule_timezone() -> String {
    "Asia/Shanghai".to_string()
}

fn default_schedule_intent_prompt_path() -> String {
    "prompts/schedule_intent_prompt.md".to_string()
}

fn default_schedule_intent_rules_path() -> String {
    "prompts/schedule_intent_rules.md".to_string()
}

impl AppConfig {
    pub fn load(path: &str) -> Result<Self, config::ConfigError> {
        let cfg = config::Config::builder()
            .add_source(config::File::with_name(path))
            .build()?;
        cfg.try_deserialize()
    }
}
