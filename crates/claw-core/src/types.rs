use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Ask,
    RunSkill,
    Admin,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Telegram,
    Whatsapp,
    Ui,
    Feishu,
    /// 国际版 Lark（与 Feishu 中国站分开）
    Lark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTaskRequest {
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub chat_id: Option<i64>,
    #[serde(default)]
    pub user_key: Option<String>,
    #[serde(default)]
    pub channel: Option<ChannelKind>,
    #[serde(default)]
    pub external_user_id: Option<String>,
    #[serde(default)]
    pub external_chat_id: Option<String>,
    pub kind: TaskKind,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTaskResponse {
    pub task_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthIdentity {
    pub user_key: String,
    pub role: String,
    pub user_id: i64,
    pub chat_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiKeyVerifyRequest {
    pub user_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveChannelBindingRequest {
    pub channel: ChannelKind,
    #[serde(default)]
    pub external_user_id: Option<String>,
    #[serde(default)]
    pub external_chat_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveChannelBindingResponse {
    pub bound: bool,
    #[serde(default)]
    pub identity: Option<AuthIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindChannelKeyRequest {
    pub channel: ChannelKind,
    #[serde(default)]
    pub external_user_id: Option<String>,
    #[serde(default)]
    pub external_chat_id: Option<String>,
    pub user_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertExchangeCredentialRequest {
    pub exchange: String,
    pub api_key: String,
    pub api_secret: String,
    #[serde(default)]
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeCredentialStatus {
    pub exchange: String,
    pub configured: bool,
    #[serde(default)]
    pub api_key_masked: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskQueryResponse {
    pub task_id: Uuid,
    pub status: TaskStatus,
    pub result_json: Option<Value>,
    pub error_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub version: String,
    pub queue_length: usize,
    pub worker_state: String,
    pub uptime_seconds: u64,
    pub memory_rss_bytes: Option<u64>,
    /// 当前处于 running 状态的任务数量
    pub running_length: usize,
    /// worker 级别的任务超时时间（秒）
    pub task_timeout_seconds: u64,
    /// 最久运行中的任务已运行时长（秒），没有 running 任务时为 0
    pub running_oldest_age_seconds: u64,
    /// telegramd 进程健康状态（None 表示无法检测）
    pub telegramd_healthy: Option<bool>,
    /// 检测到的 telegramd 进程数量（None 表示无法检测）
    pub telegramd_process_count: Option<usize>,
    /// telegramd 进程 RSS 内存总和（字节，None 表示无法检测）
    pub telegramd_memory_rss_bytes: Option<u64>,
    /// whatsappd 进程健康状态（None 表示无法检测）
    pub whatsappd_healthy: Option<bool>,
    /// 检测到的 whatsappd 进程数量（None 表示无法检测）
    pub whatsappd_process_count: Option<usize>,
    /// whatsappd 进程 RSS 内存总和（字节，None 表示无法检测）
    pub whatsappd_memory_rss_bytes: Option<u64>,
    /// telegram bot adapter 健康状态（None 表示无法检测）
    pub telegram_bot_healthy: Option<bool>,
    /// telegram bot adapter 进程数量（None 表示无法检测）
    pub telegram_bot_process_count: Option<usize>,
    /// telegram bot adapter RSS 内存总和（字节，None 表示无法检测）
    pub telegram_bot_memory_rss_bytes: Option<u64>,
    /// whatsapp cloud adapter 健康状态（None 表示无法检测）
    pub whatsapp_cloud_healthy: Option<bool>,
    /// whatsapp cloud adapter 进程数量（None 表示无法检测）
    pub whatsapp_cloud_process_count: Option<usize>,
    /// whatsapp cloud adapter RSS 内存总和（字节，None 表示无法检测）
    pub whatsapp_cloud_memory_rss_bytes: Option<u64>,
    /// whatsapp web adapter 健康状态（None 表示无法检测）
    pub whatsapp_web_healthy: Option<bool>,
    /// whatsapp web adapter 进程数量（None 表示无法检测）
    pub whatsapp_web_process_count: Option<usize>,
    /// whatsapp web adapter RSS 内存总和（字节，None 表示无法检测）
    pub whatsapp_web_memory_rss_bytes: Option<u64>,
    /// feishud 进程健康状态（None 表示无法检测）
    pub feishud_healthy: Option<bool>,
    /// 检测到的 feishud 进程数量（None 表示无法检测）
    pub feishud_process_count: Option<usize>,
    /// feishud 进程 RSS 内存总和（字节，None 表示无法检测）
    pub feishud_memory_rss_bytes: Option<u64>,
    /// larkd 进程健康状态（None 表示无法检测，国际版 Lark）
    pub larkd_healthy: Option<bool>,
    /// 检测到的 larkd 进程数量（None 表示无法检测）
    pub larkd_process_count: Option<usize>,
    /// larkd 进程 RSS 内存总和（字节，None 表示无法检测）
    pub larkd_memory_rss_bytes: Option<u64>,
    /// 当前启用中的用户 key 数量
    pub user_count: usize,
    /// 当前已绑定的通信端数量
    pub bound_channel_count: usize,
    /// 配置中启用但尚未实现的 future adapters
    pub future_adapters_enabled: Vec<String>,
}
