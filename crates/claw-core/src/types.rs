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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTaskRequest {
    pub user_id: i64,
    pub chat_id: i64,
    pub kind: TaskKind,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTaskResponse {
    pub task_id: Uuid,
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
}
