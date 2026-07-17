use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use claw_core::config::{
    AppConfig, MaintenanceConfig, MemoryConfig, RoutingConfig, SelfExtensionConfig,
};
use claw_core::skill_registry::{
    OutputKind, SkillKind, SkillManifest, SkillRiskLevel, SkillsRegistry,
};
use reqwest::Client;
use serde::Serialize;
use tokio::sync::Semaphore;

use super::policy::{RateLimiter, ToolsPolicy};
pub(crate) use super::provider_runtime::{AgentRuntimeConfig, LlmProviderRuntime};
use super::types::{CommandIntentRuntime, ScheduleRuntime};

pub(crate) struct SkillViews {
    pub(crate) registry: Option<Arc<SkillsRegistry>>,
    pub(crate) execution_skills: HashSet<String>,
    pub(crate) planner_visible: Vec<String>,
}

/// Phase 1.5: per-(task, prompt-label) LLM 调用统计桶。
///
/// `count` = 该 label 下 [`crate::llm_gateway::run_with_fallback_with_prompt_source`]
/// 入口被命中的次数（与全局预算的"逻辑调用次数"语义一致）。
/// `elapsed_ms` = 这些调用的累计 wall-clock 耗时（成功/失败都计入）。
///
/// 用于在 `task_journal_summary.task_metrics.by_prompt` 暴露细分维度，
/// 让"哪个 prompt 把单任务预算烧光了"一眼可见，作为后续 prompt-level
/// 优化与告警的诊断基础。
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct LlmPromptBucket {
    pub(crate) count: u64,
    pub(crate) elapsed_ms: u64,
    pub(crate) provider_attempt_count: u64,
    pub(crate) provider_retry_count: u64,
    pub(crate) provider_retryable_error_count: u64,
    pub(crate) provider_final_error_count: u64,
    pub(crate) provider_last_retry_error_kinds: BTreeMap<String, u64>,
    pub(crate) provider_final_error_kinds: BTreeMap<String, u64>,
    pub(crate) prompt_truncation_count: u64,
    pub(crate) prompt_bytes_before_max: Option<usize>,
    pub(crate) prompt_bytes_budget_min: Option<usize>,
    pub(crate) prompt_bytes_after_max: Option<usize>,
    pub(crate) prompt_truncated_bytes_total: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct LlmCallSequenceEntry {
    pub(crate) call_index: u64,
    pub(crate) prompt_label: String,
    pub(crate) prompt_bytes: usize,
}

pub(crate) struct SkillViewsSnapshot {
    pub(crate) registry: Option<Arc<SkillsRegistry>>,
    pub(crate) skills_list: Arc<HashSet<String>>,
}

/// P2.1 — 把"对外通道适配器配置"从 [`AppState`] 主体中剥出来，放进独立子 struct。
///
/// 设计决定：
/// - 这一组字段在主流程（dispatch / planner / skill / db）几乎用不到，只在
///   `channel_send.rs` / `worker/channels.rs` / `http/ui_routes.rs` 三个文件被读。
/// - 把它们隔离掉之后，AppState 主体从 55 字段降到 ~41，未来加新通道（Discord /
///   Slack / 企微）只动这一个子 struct，不再"加一个字段动 13 个 test fixture"。
#[derive(Clone, Default)]
pub(crate) struct ChannelConfig {
    pub(crate) telegram_bot_token: String,
    pub(crate) telegram_configured_bot_names: Arc<Vec<String>>,
    pub(crate) whatsapp_cloud_enabled: bool,
    pub(crate) whatsapp_api_base: String,
    pub(crate) whatsapp_access_token: String,
    pub(crate) whatsapp_phone_number_id: String,
    pub(crate) whatsapp_web_enabled: bool,
    pub(crate) whatsapp_web_bridge_base_url: String,
    pub(crate) future_adapters_enabled: Arc<Vec<String>>,
    pub(crate) wechat_send_config: Option<crate::channel_send::WechatSendConfig>,
    pub(crate) feishu_send_config: Option<crate::channel_send::FeishuSendConfig>,
    pub(crate) lark_send_config: Option<crate::channel_send::LarkSendConfig>,
}

/// P2.1 — 把"配置 / 注册表 reload 时需要查的元信息"从 [`AppState`] 主体中剥出来。
///
/// 这一组字段除了 `config_path_for_reload` 在 `reload_skill_views` 用到，其他字段
/// 只保留为 reload 兼容快照，避免重新解析配置失败时丢失上一轮运行态上下文。
#[derive(Clone, Default)]
pub(crate) struct ReloadContext {
    pub(crate) config_path_for_reload: String,
    pub(crate) _registry_path_for_reload: Option<String>,
    pub(crate) _skill_switches_for_reload: Arc<HashMap<String, bool>>,
    pub(crate) _initial_skills_list_for_reload: Vec<String>,
}

/// P2.1 Stage 2 — `CoreServices` 簇：所有模块都需要的核心运行时句柄
/// （DB/audit DB pool、LLM provider 列表、agent 字典、HTTP client、技能视图快照）。
///
/// 拆分动机：这些字段是"加一个新 pool / 新 provider 类型 / 新 agent runtime
/// 字段"时最容易动到的，把它们集中在一个子 struct 里之后：
///   * 12 个 test fixture 只需要 `CoreServices::test_default()` 一行；
///   * 未来 `LlmProvider trait` 抽象（P2.3）只动这个簇；
///   * 未来 memory pool 拆分（P2.2 Stage 2 memory）也只动这个簇。
#[derive(Clone)]
pub(crate) struct CoreServices {
    pub(crate) db: crate::db_init::DbPool,
    /// Phase 2.2 Stage 2: 独立 audit pool（独立 SQLite 文件）。
    /// audit_logs 走这个池，主 pool 只承载任务/调度/记忆等热路径。
    pub(crate) audit_db: crate::db_init::DbPool,
    pub(crate) llm_providers: Vec<Arc<LlmProviderRuntime>>,
    pub(crate) agents_by_id: Arc<HashMap<String, AgentRuntimeConfig>>,
    pub(crate) http_client: Client,
    pub(crate) skill_views_snapshot: Arc<RwLock<Arc<SkillViewsSnapshot>>>,
    pub(crate) active_provider_type: Option<String>,
    pub(crate) mcp_runtime: Arc<crate::mcp_runtime::McpRuntime>,
}

impl CoreServices {
    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        let agents_by_id = HashMap::from([(
            crate::DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&claw_core::config::AgentConfig::default(), Vec::new()),
        )]);
        Self {
            db: crate::db_init::test_pool(),
            audit_db: crate::db_init::test_audit_pool(),
            llm_providers: Vec::new(),
            agents_by_id: Arc::new(agents_by_id),
            http_client: Client::new(),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(HashSet::new()),
            }))),
            active_provider_type: None,
            mcp_runtime: Arc::new(crate::mcp_runtime::McpRuntime::disabled()),
        }
    }

    /// §7.5 Step 4.b.1：与 [`Self::test_default`] 等价，但 `llm_providers` 装一条
    /// `fixture_replay` runtime（`vendor-fixture-test`）、`active_provider_type`
    /// 设为 `"fixture_replay"`。
    ///
    /// 用途：未来覆盖 `process_ask_task` 的真 e2e harness 装 `AppState` 时直接
    /// 调 [`AppState::test_default_with_fixture_provider`]，省掉重复"new provider
    /// runtime + 塞进 Vec + 改 active_provider_type"三连。
    ///
    /// **必须配合** [`crate::fixture_replay_e2e::FixtureEnvGuard`] 才能让 provider
    /// 真的命中 fixture：guard 负责 set `RUSTCLAW_FIXTURE_LLM_ROOT` /
    /// `RUSTCLAW_FIXTURE_CASE`，本 helper 只负责"把 provider 装进 AppState"。
    #[cfg(test)]
    pub(crate) fn test_default_with_fixture_provider() -> Self {
        let mut base = Self::test_default();
        base.llm_providers = vec![
            crate::providers::fixture_replay::build_fixture_replay_runtime("vendor-fixture-test"),
        ];
        base.active_provider_type =
            Some(crate::providers::fixture_replay::FIXTURE_REPLAY_PROVIDER_TYPE.to_string());
        base
    }
}

/// P2.1 Stage 2 — `SkillRuntime` 簇：技能链路 / 命令执行 / locator 相关参数。
///
/// 拆分动机：原 AppState 上这一簇有 10 个字段，其中 `workspace_root` 单独
/// 84 处引用，`default_locator_search_dir` / `locator_scan_*` 在 locator
/// 路径强相关——把它们集中在一个子 struct 之后，未来 sandbox/cap-std 改造
/// (Phase 5) 改的也是同一份。
#[derive(Clone)]
pub(crate) struct SkillRuntime {
    pub(crate) skill_timeout_seconds: u64,
    pub(crate) skill_runner_path: PathBuf,
    pub(crate) skill_semaphore: Arc<Semaphore>,
    pub(crate) tools_policy: Arc<ToolsPolicy>,
    pub(crate) cmd_timeout_seconds: u64,
    pub(crate) cmd_idle_timeout_seconds: u64,
    pub(crate) cmd_max_output_bytes: usize,
    pub(crate) max_cmd_length: usize,
    pub(crate) workspace_root: PathBuf,
    pub(crate) default_locator_search_dir: PathBuf,
    pub(crate) locator_scan_max_depth: usize,
    pub(crate) locator_scan_max_files: usize,
}

impl SkillRuntime {
    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        let tools_policy = ToolsPolicy::from_config(&claw_core::config::ToolsConfig::default())
            .expect("tools policy");
        Self {
            skill_timeout_seconds: 30,
            skill_runner_path: PathBuf::new(),
            skill_semaphore: Arc::new(Semaphore::new(1)),
            tools_policy: Arc::new(tools_policy),
            cmd_timeout_seconds: 60,
            cmd_idle_timeout_seconds: 60,
            cmd_max_output_bytes: 8000,
            max_cmd_length: 4096,
            workspace_root: std::env::temp_dir(),
            default_locator_search_dir: std::env::temp_dir(),
            locator_scan_max_depth: 2,
            locator_scan_max_files: 100,
        }
    }
}

/// P2.1 Stage 2 — `PolicyConfig` 簇：运维 / 安全 / 限速 / 路由 / persona /
/// 命令意图 / 调度运行时配置。
///
/// 拆分动机：这一簇是"启动时从 config.toml 装配出来、运行期只读"的策略
/// 集合，与 SkillRuntime 的"执行参数"区分开。`maintenance` / `memory` /
/// `routing` 是高频读字段，集中放可避免各模块为了读策略来回 import。
#[derive(Clone)]
pub(crate) struct PolicyConfig {
    pub(crate) maintenance: MaintenanceConfig,
    pub(crate) memory: MemoryConfig,
    pub(crate) routing: RoutingConfig,
    pub(crate) self_extension: SelfExtensionConfig,
    pub(crate) rate_limiter: Arc<Mutex<RateLimiter>>,
    pub(crate) allow_path_outside_workspace: bool,
    pub(crate) allow_sudo: bool,
    /// §3.5d: persona prompt 文本封装为 `Arc<RwLock<String>>`，使 SIGHUP 触发的
    /// hot reload 能 swap 内部内容；所有 `AppState` clone（axum router 分发）
    /// 共享同一份内部存储。读取请用 `persona_prompt_string()` helper。
    pub(crate) persona_prompt: Arc<RwLock<String>>,
    pub(crate) command_intent: CommandIntentRuntime,
    pub(crate) schedule: ScheduleRuntime,
}

impl PolicyConfig {
    pub(crate) fn persona_prompt_string(&self) -> String {
        self.persona_prompt
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// §3.5d: 用新串覆盖现有 persona prompt 内容（写锁；poison 时静默回退）。
    pub(crate) fn replace_persona_prompt(&self, new_persona: String) {
        if let Ok(mut guard) = self.persona_prompt.write() {
            *guard = new_persona;
        }
    }

    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        let locale = "zh-CN";
        Self {
            maintenance: MaintenanceConfig::default(),
            memory: MemoryConfig::default(),
            routing: RoutingConfig::default(),
            self_extension: SelfExtensionConfig::default(),
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(60, 30))),
            allow_path_outside_workspace: false,
            allow_sudo: false,
            persona_prompt: Arc::new(RwLock::new(String::new())),
            command_intent: CommandIntentRuntime {
                default_locale: locale.to_string(),
                verify_enforce_enabled: false,
            },
            schedule: ScheduleRuntime {
                timezone: "Asia/Shanghai".to_string(),
                intent_prompt_template: Arc::new(RwLock::new(String::new())),
                intent_prompt_source: String::new(),
                intent_rules_template: Arc::new(RwLock::new(String::new())),
                locale: locale.to_string(),
                i18n_dir: "configs/i18n".to_string(),
                i18n_dict: HashMap::new(),
            },
        }
    }
}

/// P2.1 Stage 2 — `WorkerConfig` 簇：worker / 调度 / DB busy_timeout 等
/// "进程级别参数"。
///
/// 拆分动机：字段不多（10 个）、读频也低（10 处引用），但每次新增 worker
/// 行为参数都要改 12 个 fixture。集中后 fixtures 只调一次 `test_default()`。
#[derive(Clone)]
pub(crate) struct WorkerConfig {
    pub(crate) worker_id: String,
    pub(crate) started_at: Instant,
    pub(crate) queue_limit: usize,
    pub(crate) worker_task_timeout_seconds: u64,
    pub(crate) llm_max_calls_per_task: u64,
    pub(crate) llm_total_timeout_ms: u64,
    pub(crate) worker_task_heartbeat_seconds: u64,
    pub(crate) worker_running_no_progress_timeout_seconds: u64,
    pub(crate) worker_running_recovery_check_interval_seconds: u64,
    pub(crate) last_running_recovery_check_ts: Arc<Mutex<u64>>,
    pub(crate) active_running_task_ids: Arc<Mutex<HashSet<String>>>,
    pub(crate) task_cancellation_tokens:
        Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
    pub(crate) database_busy_timeout_ms: u64,
    pub(crate) database_sqlite_path: PathBuf,
}

impl WorkerConfig {
    pub(crate) fn register_active_task(
        &self,
        task_id: &str,
    ) -> tokio_util::sync::CancellationToken {
        let token = tokio_util::sync::CancellationToken::new();
        if let Ok(mut active) = self.active_running_task_ids.lock() {
            active.insert(task_id.to_string());
        }
        if let Ok(mut tokens) = self.task_cancellation_tokens.lock() {
            tokens.insert(task_id.to_string(), token.clone());
        }
        token
    }

    pub(crate) fn unregister_active_task(&self, task_id: &str) {
        if let Ok(mut active) = self.active_running_task_ids.lock() {
            active.remove(task_id);
        }
        if let Ok(mut tokens) = self.task_cancellation_tokens.lock() {
            tokens.remove(task_id);
        }
    }

    pub(crate) fn cancel_active_task(&self, task_id: &str) -> bool {
        self.task_cancellation_tokens
            .lock()
            .ok()
            .and_then(|tokens| tokens.get(task_id).cloned())
            .is_some_and(|token| {
                token.cancel();
                true
            })
    }

    pub(crate) fn task_cancellation_token(
        &self,
        task_id: &str,
    ) -> Option<tokio_util::sync::CancellationToken> {
        self.task_cancellation_tokens
            .lock()
            .ok()
            .and_then(|tokens| tokens.get(task_id).cloned())
    }

    pub(crate) fn is_task_active(&self, task_id: &str) -> bool {
        self.active_running_task_ids
            .lock()
            .is_ok_and(|active| active.contains(task_id))
    }

    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        Self {
            worker_id: "worker:test-default".to_string(),
            started_at: Instant::now(),
            queue_limit: 1,
            worker_task_timeout_seconds: 300,
            llm_max_calls_per_task: crate::llm_gateway::DEFAULT_MAX_LLM_CALLS_PER_TASK,
            llm_total_timeout_ms: crate::llm_gateway::DEFAULT_MAX_LLM_TOTAL_MS_PER_TASK,
            worker_task_heartbeat_seconds: 10,
            worker_running_no_progress_timeout_seconds: 300,
            worker_running_recovery_check_interval_seconds: 30,
            last_running_recovery_check_ts: Arc::new(Mutex::new(0)),
            active_running_task_ids: Arc::new(Mutex::new(HashSet::new())),
            task_cancellation_tokens: Arc::new(Mutex::new(HashMap::new())),
            database_busy_timeout_ms: 5_000,
            database_sqlite_path: PathBuf::new(),
        }
    }
}

/// P2.1 Stage 2 — `TaskMetricsRegistry` 簇：per-task LLM 计数 / 耗时 /
/// by-prompt 分桶 / ordered call sequence。
///
/// 拆分动机：Phase 1.5 加 `llm_by_prompt_per_task` 一个字段就要改 12
/// 处 fixture——这正是 P2.1 想根治的痛点。集中后 `TaskMetricsRegistry`
/// 全 4 字段都是 `Arc<Mutex<HashMap::default()>>` 形式，`#[derive(Default)]`
/// 直接生效，fixture 写 `TaskMetricsRegistry::default()` 即可。
#[derive(Clone, Default)]
pub(crate) struct TaskMetricsRegistry {
    pub(crate) llm_calls_per_task: Arc<Mutex<HashMap<String, u64>>>,
    /// Phase 1.3: 单任务 LLM 累计耗时（ms），与 `llm_calls_per_task` 一起构成
    /// "单任务 LLM 预算"。在 `llm_gateway::run_with_fallback_with_prompt_source`
    /// 入口处做一次预算检查，超过 `worker.llm_max_calls_per_task` 或
    /// `worker.llm_total_timeout_ms` 就直接短路返回错误，防止单个任务
    /// 无限扩张 LLM 预算（例如 plan_repair 抖动、fallback 雪崩）。
    pub(crate) llm_elapsed_per_task: Arc<Mutex<HashMap<String, u64>>>,
    /// Phase 1.5: per-task 的 LLM 调用按 prompt label 分桶累计（次数 + 耗时）。
    /// 与 `llm_calls_per_task` / `llm_elapsed_per_task` 是同一份数据的不同维度：
    /// 总量用前两个表，而"哪个 prompt 把额度烧光了"用这个表。诊断用。
    /// 外层 key = task_id；内层 key = label（如 `plan` /
    /// `plan_repair` / `chat` / `delivery_classifier` / `observed` / `nl2cmd`...）。
    /// 标签由 [`crate::llm_gateway::classify_prompt_source`] 从 `prompt_source` 抽出。
    pub(crate) llm_by_prompt_per_task:
        Arc<Mutex<HashMap<String, HashMap<String, LlmPromptBucket>>>>,
    /// Ordered, machine-only metadata for measuring calls before the planner.
    /// Prompt and response text are intentionally not retained here.
    pub(crate) llm_call_sequence_per_task: Arc<Mutex<HashMap<String, Vec<LlmCallSequenceEntry>>>>,
    /// Live task-event wakeups. Event payloads and replay state live in SQLite; this registry only
    /// wakes connected consumers, so the default test fixture remains lightweight.
    pub(crate) task_event_notifier: crate::task_event_transport::TaskEventNotifier,
}

pub(crate) fn build_skill_views(
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

    let explicitly_disabled: HashSet<String> = skill_switches
        .iter()
        .filter(|(_, &on)| !on)
        .map(|(skill, _)| {
            registry
                .as_ref()
                .and_then(|r| r.resolve_canonical(skill).map(String::from))
                .unwrap_or_else(|| crate::canonical_skill_name(skill).to_string())
        })
        .collect();

    let mut enabled: HashSet<String> = if let Some(ref reg) = registry {
        reg.enabled_names().into_iter().collect()
    } else {
        initial_skills_list
            .iter()
            .map(|s| crate::canonical_skill_name(s).to_string())
            .collect()
    };
    for (skill, is_enabled) in skill_switches {
        let canonical = registry
            .as_ref()
            .and_then(|r| r.resolve_canonical(skill).map(String::from))
            .unwrap_or_else(|| crate::canonical_skill_name(skill).to_string());
        if *is_enabled {
            enabled.insert(canonical);
        } else {
            enabled.remove(&canonical);
        }
    }
    for s in claw_core::config::core_skills_always_enabled() {
        let c = crate::canonical_skill_name(s).to_string();
        if !explicitly_disabled.contains(&c) {
            enabled.insert(c);
        }
    }
    let mut planner_visible: Vec<String> = enabled
        .iter()
        .filter(|skill| {
            registry
                .as_ref()
                .map(|reg| reg.is_planner_visible(skill))
                .unwrap_or(true)
        })
        .cloned()
        .collect();
    planner_visible.sort_unstable();

    Ok(SkillViews {
        registry,
        execution_skills: enabled,
        planner_visible,
    })
}

pub(crate) fn reload_skill_views(state: &AppState) -> Result<ReloadSkillViewsResult, String> {
    tracing::info!(
        "reload_skill_views: started config_path={}",
        state.reload_ctx.config_path_for_reload
    );
    let config = AppConfig::load(&state.reload_ctx.config_path_for_reload)
        .map_err(|e| format!("reload_skill_views: load config failed: {}", e))?;
    let registry_path = config.skills.registry_path.as_deref();
    let path_display = registry_path.unwrap_or("(none)");
    let views = build_skill_views(
        &state.skill_rt.workspace_root,
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
    *state.core.skill_views_snapshot.write().unwrap() = Arc::new(snapshot);

    tracing::info!(
        "reload_skill_views: success path={} registry_entries={} execution_skills_count={} planner_visible_count={}",
        path_display,
        registry_entries,
        execution_count,
        planner_count
    );
    Ok(ReloadSkillViewsResult {
        registry_entries,
        execution_skills_count: execution_count,
        planner_visible_count: planner_count,
    })
}

#[derive(Debug, Serialize)]
pub(crate) struct ReloadSkillViewsResult {
    pub(crate) registry_entries: usize,
    pub(crate) execution_skills_count: usize,
    pub(crate) planner_visible_count: usize,
}

/// P2.1 Stage 2 完成后：AppState 主体只剩 7 个子 struct 字段（CoreServices /
/// SkillRuntime / PolicyConfig / WorkerConfig / TaskMetricsRegistry / ChannelConfig /
/// ReloadContext），fixture 不再需要为新增字段同步 12 处。新增字段时只动一个
/// 子 struct 的定义 + `test_default()`。
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) core: CoreServices,
    pub(crate) skill_rt: SkillRuntime,
    pub(crate) policy: PolicyConfig,
    pub(crate) worker: WorkerConfig,
    pub(crate) metrics: TaskMetricsRegistry,
    /// P2.1 — 通道配置子 struct（telegram / whatsapp / wechat / feishu / lark /
    /// future_adapters）。详见 [`ChannelConfig`] 头部 doc。
    pub(crate) channels: ChannelConfig,
    /// P2.1 — reload 元信息子 struct（config 路径、registry 路径、skill_switches、
    /// 初始 skills_list）。详见 [`ReloadContext`] 头部 doc。
    pub(crate) reload_ctx: ReloadContext,
    /// Phase 3.3 Stage 3.2 — per-task 当前 ask_state 注册表。
    /// 由 [`crate::log_ask_transition`] 同步更新；finalize 子层可通过
    /// [`Self::current_ask_state`] 查询，配合 `debug_assert` 保证 invariant。
    /// 终态（Completed/Failed）的 entry 会被立即清理，避免长跑泄漏。
    pub(crate) ask_states: AskStateRegistry,
}

const DEFAULT_AGENT_RUNTIME_IDENTITY: &str = "RustClaw";

/// Phase 3.3 Stage 3.2 — per-task ask_state 注册表。
///
/// 简单的 `Arc<Mutex<HashMap>>` 实现，与 `TaskMetricsRegistry` 形态一致；
/// 写入路径仅在 [`crate::log_ask_transition`]，读取路径仅在 finalize 子层
/// invariant `debug_assert`，并发竞争极低。
///
/// 终态（Completed/Failed）写入后会立刻 remove，避免长跑残留。
#[derive(Clone, Default)]
pub(crate) struct AskStateRegistry {
    inner: Arc<Mutex<HashMap<String, crate::AskState>>>,
}

impl AskStateRegistry {
    pub(crate) fn set(&self, task_id: &str, state: crate::AskState) {
        let mut guard = self.inner.lock().unwrap();
        if state.is_terminal() {
            guard.remove(task_id);
        } else {
            guard.insert(task_id.to_string(), state);
        }
    }

    pub(crate) fn get(&self, task_id: &str) -> Option<crate::AskState> {
        self.inner.lock().unwrap().get(task_id).copied()
    }
}

fn confirmation_exempt_matcher_matches(
    args: &serde_json::Value,
    matcher: &BTreeMap<String, serde_json::Value>,
) -> bool {
    let Some(args) = args.as_object() else {
        return false;
    };
    matcher.iter().all(|(key, expected)| {
        args.get(key)
            .is_some_and(|actual| json_value_matches(actual, expected))
    })
}

fn json_value_matches(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    if let Some(options) = expected.as_array() {
        return options
            .iter()
            .any(|candidate| json_value_matches(actual, candidate));
    }
    match (actual, expected) {
        (serde_json::Value::String(actual), serde_json::Value::String(expected)) => {
            normalize_arg_token(actual) == normalize_arg_token(expected)
        }
        (serde_json::Value::Bool(actual), serde_json::Value::Bool(expected)) => actual == expected,
        (serde_json::Value::Number(actual), serde_json::Value::Number(expected)) => {
            actual == expected
        }
        _ => actual == expected,
    }
}

fn normalize_arg_token(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if matches!(ch, '-' | ' ' | '.') {
                '_'
            } else {
                ch
            }
        })
        .collect()
}

impl AppState {
    /// §7.5 Step 4.b.1：装一份完整 minimal `AppState`，所有子 struct 走 `test_default()`，
    /// `core` 走 [`CoreServices::test_default_with_fixture_provider`] —— 即 LLM
    /// provider 链路上有且只有一条 fixture replay。
    ///
    /// 不复用现有 `crates/clawd/src/skills.rs` 里那个 file-private `test_state`
    /// helper：那个是 skills 模块自查时造的，留在原地与 skills 测试耦合；这里
    /// 给 fixture-replay e2e harness 一份纯净版，避免互相牵动。
    ///
    /// **不**初始化任何 schema / 不写任何表 / 不预 install SkillsRegistry —— 它
    /// 只保证：
    ///   * `state.task_llm_providers(&task)` 能拿到 fixture provider；
    ///   * `state.policy.persona_prompt` 是空串 +
    ///     `state.policy.routing` / `state.policy.maintenance` 走默认值；
    ///   * 所有 metrics 桶可写不可坏。
    ///
    /// 真正跑 [`crate::worker::process_ask_task`] 还要补 SkillsRegistry / prompts
    /// / channel mock 等 —— 见 [`crate::fixture_replay_e2e`] 模块顶部 doc 列出的
    /// "Step 4.b 前置清单"。
    #[cfg(test)]
    pub(crate) fn test_default_with_fixture_provider() -> Self {
        Self {
            core: CoreServices::test_default_with_fixture_provider(),
            skill_rt: SkillRuntime::test_default(),
            policy: PolicyConfig::test_default(),
            worker: WorkerConfig::test_default(),
            metrics: TaskMetricsRegistry::default(),
            channels: ChannelConfig::default(),
            reload_ctx: ReloadContext::default(),
            ask_states: AskStateRegistry::default(),
        }
    }

    /// §7.5 Step 4.b.2.1：在已有 `AppState` 上"原地装一份 minimal builtin
    /// `SkillsRegistry`"链式 helper。
    ///
    /// 用途：`process_ask_task` e2e harness 启动期会跑
    /// [`SkillsRegistry::integrity_report`]，缺任何一条
    /// [`claw_core::skill_registry::REQUIRED_BUILTIN_SKILLS`] builtin 就 bail。
    /// 真生产 registry（`configs/skills_registry.toml`）有 30+ 条，其中 20+ 条
    /// 是 runner / external，要求 prompt 文件 / runner 二进制都在 —— 对
    /// fixture-replay 测试是不必要的依赖。这里只装 [`REQUIRED_BUILTIN_SKILLS`]
    /// 全集，最小、`integrity-clean` 且独立于 workspace 文件系统。
    ///
    /// 概念辨析：`normalize / delivery_classifier / nl2cmd` 等是 **prompt label**
    /// （走 `crates/clawd/configs/prompts/...`），不是 skill；它们的接入由后续
    /// `bootstrap::prompts::install_prompt_layers_to_workspace` 配套 helper
    /// 处理，本 helper 不涉及。
    ///
    /// 实现：把 minimal toml 写到一个 uuid 临时文件 → 调
    /// [`SkillsRegistry::load_from_path`]（加载完即把内容拷进 HashMap，
    /// 之后不再读 file）→ 立刻 unlink → 把 `Arc<SkillsRegistry>` +
    /// `enabled_names()` 集合写入 `core.skill_views_snapshot`。
    #[cfg(test)]
    pub(crate) fn with_minimal_builtin_registry(self) -> Self {
        use claw_core::skill_registry::REQUIRED_BUILTIN_SKILLS;
        let mut toml_buf = String::new();
        for name in REQUIRED_BUILTIN_SKILLS {
            toml_buf.push_str(&format!(
                "[[skills]]\nname = \"{name}\"\nenabled = true\nkind = \"builtin\"\n\n",
            ));
        }
        let path = std::env::temp_dir().join(format!(
            "rustclaw_test_minimal_registry_{}.toml",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, &toml_buf).expect("write minimal skills_registry.toml for test");
        let registry = SkillsRegistry::load_from_path(&path)
            .expect("load minimal skills_registry.toml for test");
        let _ = std::fs::remove_file(&path);
        let report = registry.integrity_report();
        assert!(
            report.is_clean(),
            "minimal builtin registry must satisfy integrity check, got: {:?}",
            report,
        );
        let enabled: HashSet<String> = registry.enabled_names().into_iter().collect();
        let snapshot = SkillViewsSnapshot {
            registry: Some(Arc::new(registry)),
            skills_list: Arc::new(enabled),
        };
        *self.core.skill_views_snapshot.write().unwrap() = Arc::new(snapshot);
        self
    }

    /// §7.5 Step 4.b.2.2：链式 helper，把 [`SkillRuntime::workspace_root`] 指向
    /// 真仓库根，让 [`crate::bootstrap::prompts::load_prompt_template_for_state`]
    /// 经 [`claw_core::prompt_layers`] manifest 命中磁盘 layered prompt，而不是
    /// 各 callsite 的 `include_str!` 兜底。
    ///
    /// 为什么需要：
    ///   * 录制 fixture 时，LLM 拿到的 prompt 文本来自真生产 workspace 的
    ///     `prompts/layers/{base,overlays}` 拼层（含 `prompts/layers/manifest.toml`
    ///     描述的 base / overlay / vendor_patch 三段）—— 比每个 callsite 自带的
    ///     `include_str!` 兜底文本通常要长且带 version 注释。
    ///   * 回放在 [`SkillRuntime::test_default`] 默认的 `std::env::temp_dir()`
    ///     workspace 下根本没有 `prompts/` 目录 → 加载落到兜底 → prompt 文本
    ///     与录制版不一致 → fnv1a 输入字符串不同 → fixture miss。
    ///   * 把 workspace_root 指到真仓库根后，prompt 加载读到的就是 git 里那份
    ///     "录制时的同一份"，hash 自洽。
    ///
    /// **安全约束**：调用本 helper 后，测试**不应**触发任何写 `workspace_root`
    /// 子树的代码路径（fs.write / make_dir / locator 落盘等）。fixture-replay
    /// 的 `process_ask_task` 本身不写 `prompts/` / `crates/` 等 git-tracked 路径，
    /// 但要配合 4.b.2.3 channel mock + 4.b.2.4 DB schema seed 一起约束"测试
    /// 整体不会 mutate 仓库目录"。`SkillRuntime::default_locator_search_dir`
    /// **不**改写，仍指 `std::env::temp_dir()`，避免 locator 把真仓库根全树扫
    /// 一遍。
    #[cfg(test)]
    pub(crate) fn with_prompt_layers_installed(mut self) -> Self {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // CARGO_MANIFEST_DIR 是 `<repo>/crates/clawd`，向上两层 = workspace 根
        let workspace_root = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .map(std::path::Path::to_path_buf)
            .expect("workspace root must exist (CARGO_MANIFEST_DIR is crates/clawd)");
        debug_assert!(
            workspace_root
                .join("prompts/layers/manifest.toml")
                .is_file(),
            "expected layered prompt manifest at workspace root: {}",
            workspace_root.display(),
        );
        self.skill_rt.workspace_root = workspace_root;
        self
    }

    /// §7.5 replay harness helper：按真实 workspace 的 `configs/config.toml` +
    /// `configs/skills_registry.toml` 装载完整 skill views，保证 capability map /
    /// planner-visible skills 与真录 case 对齐。
    #[cfg(test)]
    pub(crate) fn with_real_skill_registry(mut self) -> Self {
        let config_path = self.skill_rt.workspace_root.join("configs/config.toml");
        let config_path_str = config_path.to_string_lossy().to_string();
        let config = AppConfig::load(&config_path_str)
            .expect("load real configs/config.toml for fixture-replay test");
        let views = build_skill_views(
            &self.skill_rt.workspace_root,
            config.skills.registry_path.as_deref(),
            &config.skills.skill_switches,
            &config.skills.skills_list,
        )
        .expect("build real skill views for fixture-replay test");
        let snapshot = SkillViewsSnapshot {
            registry: views.registry,
            skills_list: Arc::new(views.execution_skills),
        };
        *self.core.skill_views_snapshot.write().unwrap() = Arc::new(snapshot);
        self.reload_ctx.config_path_for_reload = config_path_str;
        self.reload_ctx._registry_path_for_reload = config.skills.registry_path.clone();
        self.reload_ctx._skill_switches_for_reload = Arc::new(config.skills.skill_switches.clone());
        self.reload_ctx._initial_skills_list_for_reload = config.skills.skills_list.clone();
        self
    }

    /// §7.5 replay harness helper：把测试态 `AppState` 的 policy / tools /
    /// locator / prompt vendor 对齐到真实 `configs/config.toml`。
    ///
    /// 为什么要单独做这一步：
    ///   * `PolicyConfig::test_default()` 里的 persona / schedule / command_intent /
    ///     memory 都是极简空壳，正常会让 normalizer prompt 少掉一大块 runtime
    ///     片段，导致回放 hash 全 miss。
    ///   * fixture provider 的 `config.name` 也需要伪装成真实 vendor（例如
    ///     `vendor-minimax`），否则 layered prompt 会走错 vendor patch。
    #[cfg(test)]
    pub(crate) fn with_real_runtime_policy(mut self) -> Self {
        let config_path = self.skill_rt.workspace_root.join("configs/config.toml");
        let config_path_str = config_path.to_string_lossy().to_string();
        let config = AppConfig::load(&config_path_str)
            .expect("load real configs/config.toml for fixture-replay test policy");
        let workspace_root = self.skill_rt.workspace_root.clone();
        let tools_policy = ToolsPolicy::from_config(&config.tools)
            .expect("build tools policy from real config for fixture-replay test");
        let memory_runtime =
            crate::bootstrap::load_memory_runtime_config(&workspace_root, &config.memory);
        let command_intent = crate::bootstrap::load_command_intent_runtime(&config.command_intent);
        let schedule = crate::bootstrap::load_schedule_runtime(
            &workspace_root,
            &config.schedule,
            config.llm.selected_vendor.as_deref(),
        )
        .expect("load real schedule prompts for fixture-replay test policy");
        let persona_prompt = crate::bootstrap::load_persona_prompt(
            &workspace_root,
            config.llm.selected_vendor.as_deref(),
            &config.persona,
        );
        let default_locator_search_dir = {
            let raw = config.routing.default_locator_search_dir.trim();
            if raw.is_empty() {
                workspace_root.clone()
            } else {
                let path = Path::new(raw);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    workspace_root.join(path)
                }
            }
        };
        let fixture_vendor = config
            .llm
            .selected_vendor
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("default");

        self.core.llm_providers = vec![
            crate::providers::fixture_replay::build_fixture_replay_runtime(&format!(
                "vendor-{fixture_vendor}"
            )),
        ];
        self.core.active_provider_type =
            Some(crate::providers::fixture_replay::FIXTURE_REPLAY_PROVIDER_TYPE.to_string());
        self.skill_rt.skill_timeout_seconds = config.skills.skill_timeout_seconds;
        self.skill_rt.skill_runner_path = workspace_root.join("target/release/skill-runner");
        self.skill_rt.tools_policy = Arc::new(tools_policy);
        self.skill_rt.cmd_timeout_seconds = config.tools.cmd_timeout_seconds.max(1);
        self.skill_rt.cmd_idle_timeout_seconds = config.tools.cmd_idle_timeout_seconds.max(1);
        self.skill_rt.cmd_max_output_bytes = config.tools.cmd_max_output_bytes.max(128);
        self.skill_rt.max_cmd_length = config.tools.max_cmd_length.max(16);
        self.skill_rt.default_locator_search_dir = default_locator_search_dir;
        self.skill_rt.locator_scan_max_depth = config.routing.locator_scan_max_depth;
        self.skill_rt.locator_scan_max_files = config.routing.locator_scan_max_files.max(1);
        self.policy = PolicyConfig {
            maintenance: config.maintenance.clone(),
            memory: memory_runtime,
            routing: config.routing.clone(),
            self_extension: config.self_extension.clone(),
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(
                config.limits.global_rpm,
                config.limits.user_rpm,
            ))),
            allow_path_outside_workspace: config.tools.allow_path_outside_workspace,
            allow_sudo: config.tools.allow_sudo,
            persona_prompt: Arc::new(RwLock::new(persona_prompt)),
            command_intent,
            schedule,
        };
        self.reload_ctx.config_path_for_reload = config_path_str;
        self
    }

    /// §7.5 Step 4.b.2.4：链式 helper，把 `process_ask_task` 端到端跑通需要的
    /// 主库 schema 一次性建齐。
    ///
    /// 默认 [`CoreServices::test_default_with_fixture_provider`] 拿到的
    /// `core.db` 是空 in-memory pool（`db_init::test_pool()`）—— 没有 `tasks`
    /// / `users` / `memories` / `scheduled_jobs` 任何表；
    /// `core.audit_db` 已被 [`db_init::test_audit_pool`] 预 init `audit_logs`，
    /// 这里**不动**它。
    ///
    /// 本 helper 在 `core.db` 上按生产顺序执行：
    ///   1. `INIT_SQL`（`migrations/001_init.sql`）—— 建 users/tasks/audit_logs/
    ///      memories/long_term_memories/scheduled_jobs + 必要 index；
    ///   2. [`crate::ensure_memory_schema`] —— memory 链路 ALTER 升级（幂等）；
    ///   3. [`crate::repo::ensure_key_auth_schema`] —— user_key / auth_keys /
    ///      channel_bindings 等 auth 相关列与表（幂等）；
    ///   4. [`crate::ensure_channel_schema`] —— tasks/scheduled_jobs/memories
    ///      上 `channel` / `external_*_id` 列（幂等：列存在即 skip）。
    ///
    /// 注意 `migrations/001_init.sql` **不**含 FK 约束：
    /// [`crate::repo::tasks`] 里所有写都是 `UPDATE tasks ... WHERE task_id = ?`，
    /// 行不在不会报错也不会更新，导致 `process_ask_task` 后续读 `tasks.status`
    /// 时拿到旧/空记录而下游断言失败 —— 因此本 helper 只负责"建表"，行级
    /// seed 走 [`Self::seed_ask_task_row`]。
    #[cfg(test)]
    pub(crate) fn with_seeded_db_schema(self) -> Self {
        let conn = self
            .core
            .db
            .get()
            .expect("acquire test main-db connection for schema seed");
        conn.execute_batch(crate::INIT_SQL)
            .expect("apply migrations/001_init.sql to test main db");
        crate::ensure_memory_schema(&conn).expect("ensure_memory_schema for test main db");
        crate::repo::ensure_key_auth_schema(&conn)
            .expect("ensure_key_auth_schema for test main db");
        crate::ensure_channel_schema(&conn).expect("ensure_channel_schema for test main db");
        drop(conn);
        self
    }

    /// §7.5 Step 4.b.2.4：在已 [`Self::with_seeded_db_schema`] 过的 `AppState`
    /// 上 INSERT 一条 `tasks` 行，使后续 `UPDATE tasks ... WHERE task_id = ?`
    /// 真能命中。
    ///
    /// 不写成 chain helper（`fn(self) -> Self`），因为 `task_id` / `user_id` /
    /// `chat_id` / `payload_json` 都是 per-case 输入，链式风格会让 caller 眼花;
    /// 改成 `&self` 普通方法，调用方先 `let state = …; state.seed_ask_task_row(…);`
    /// 再传 `&state` 给 `process_ask_task`。
    ///
    /// `channel` 默认 `"ui"`：与 fixture-replay e2e 默认走 UI 入口一致；走
    /// `"telegram"` 这条路 finalize 末尾会走 `notify_user`，遇到空 token 会
    /// short-circuit 但日志噪音多。`status` 写 `"queued"` —— 与生产
    /// [`crate::repo::submit::insert_submitted_task`] 完全一致，让
    /// `crate::repo::tasks::mark_running` 那条 `UPDATE ... WHERE status =
    /// 'queued'` 能命中。
    #[cfg(test)]
    pub(crate) fn seed_ask_task_row(
        &self,
        task_id: &str,
        user_id: i64,
        chat_id: i64,
        payload_json: &str,
    ) {
        let conn = self
            .core
            .db
            .get()
            .expect("acquire test main-db connection for tasks seed");
        let now = crate::now_ts();
        let user_key = format!("anon:{user_id}:{chat_id}");
        conn.execute(
            "INSERT INTO tasks (task_id, user_id, chat_id, user_key, channel, kind, payload_json, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, 'ui', 'ask', ?5, 'queued', ?6, ?6)",
            rusqlite::params![task_id, user_id, chat_id, user_key, payload_json, now],
        )
        .expect("INSERT INTO tasks for fixture-replay e2e seed");
    }

    fn snapshot(&self) -> Arc<SkillViewsSnapshot> {
        self.core.skill_views_snapshot.read().unwrap().clone()
    }

    /// Phase 1.5: 累计一次 LLM 调用，并按 prompt label 分桶记录。
    /// `label` 由 [`crate::llm_gateway::classify_prompt_source`] 从 `prompt_source` 抽出。
    pub(crate) fn note_task_llm_call_with_label_and_prompt_size(
        &self,
        task_id: &str,
        label: &str,
        prompt_bytes: usize,
    ) {
        let call_index = {
            let mut guard = self.metrics.llm_calls_per_task.lock().unwrap();
            let counter = guard.entry(task_id.to_string()).or_insert(0);
            *counter = counter.saturating_add(1);
            *counter
        };
        self.metrics
            .llm_call_sequence_per_task
            .lock()
            .unwrap()
            .entry(task_id.to_string())
            .or_default()
            .push(LlmCallSequenceEntry {
                call_index,
                prompt_label: label.to_string(),
                prompt_bytes,
            });
        let mut guard = self.metrics.llm_by_prompt_per_task.lock().unwrap();
        let bucket = guard
            .entry(task_id.to_string())
            .or_default()
            .entry(label.to_string())
            .or_default();
        bucket.count = bucket.count.saturating_add(1);
    }

    /// Phase 3.3 Stage 3.2 — 查询某任务当前 ask_state。
    /// 终态进入后 entry 已 remove，因此返回 `None` 表示要么任务未启动，
    /// 要么已完成；finalize 子层 invariant `debug_assert` 须区分这两种情况，
    /// 通常只允许"任务还在 Executing/Finalizing"——None 视为"测试环境或资源
    /// 已回收"，不触发 panic（仅 warn）。
    pub(crate) fn current_ask_state(&self, task_id: &str) -> Option<crate::AskState> {
        self.ask_states.get(task_id)
    }

    pub(crate) fn task_llm_call_count(&self, task_id: &str) -> u64 {
        self.metrics
            .llm_calls_per_task
            .lock()
            .unwrap()
            .get(task_id)
            .copied()
            .unwrap_or(0)
    }

    /// Phase 1.5: 按 prompt label 分桶记录耗时。会同时累加全局耗时表。
    pub(crate) fn note_task_llm_elapsed_with_label(
        &self,
        task_id: &str,
        label: &str,
        elapsed_ms: u64,
    ) {
        {
            let mut guard = self.metrics.llm_elapsed_per_task.lock().unwrap();
            let counter = guard.entry(task_id.to_string()).or_insert(0);
            *counter = counter.saturating_add(elapsed_ms);
        }
        let mut guard = self.metrics.llm_by_prompt_per_task.lock().unwrap();
        let bucket = guard
            .entry(task_id.to_string())
            .or_default()
            .entry(label.to_string())
            .or_default();
        bucket.elapsed_ms = bucket.elapsed_ms.saturating_add(elapsed_ms);
    }

    pub(crate) fn note_task_prompt_size_with_label(
        &self,
        task_id: &str,
        label: &str,
        prompt_bytes: usize,
    ) {
        let mut guard = self.metrics.llm_by_prompt_per_task.lock().unwrap();
        let bucket = guard
            .entry(task_id.to_string())
            .or_default()
            .entry(label.to_string())
            .or_default();
        bucket.prompt_bytes_before_max = Some(
            bucket
                .prompt_bytes_before_max
                .map_or(prompt_bytes, |current| current.max(prompt_bytes)),
        );
        bucket.prompt_bytes_after_max = Some(
            bucket
                .prompt_bytes_after_max
                .map_or(prompt_bytes, |current| current.max(prompt_bytes)),
        );
    }

    pub(crate) fn note_task_provider_attempts_with_label(
        &self,
        task_id: &str,
        label: &str,
        attempts: usize,
        retryable_error_count: usize,
        last_retry_error_kind: Option<&str>,
        final_error_kind: Option<&str>,
    ) {
        let attempts = attempts.max(1) as u64;
        let mut guard = self.metrics.llm_by_prompt_per_task.lock().unwrap();
        let bucket = guard
            .entry(task_id.to_string())
            .or_default()
            .entry(label.to_string())
            .or_default();
        bucket.provider_attempt_count = bucket.provider_attempt_count.saturating_add(attempts);
        bucket.provider_retry_count = bucket
            .provider_retry_count
            .saturating_add(attempts.saturating_sub(1));
        bucket.provider_retryable_error_count = bucket
            .provider_retryable_error_count
            .saturating_add(retryable_error_count as u64);
        if let Some(kind) = last_retry_error_kind
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
        {
            let counter = bucket
                .provider_last_retry_error_kinds
                .entry(kind.to_string())
                .or_insert(0);
            *counter = counter.saturating_add(1);
        }
        if let Some(kind) = final_error_kind
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
        {
            bucket.provider_final_error_count = bucket.provider_final_error_count.saturating_add(1);
            let counter = bucket
                .provider_final_error_kinds
                .entry(kind.to_string())
                .or_insert(0);
            *counter = counter.saturating_add(1);
        }
    }

    pub(crate) fn note_task_prompt_truncation_with_label(
        &self,
        task_id: &str,
        label: &str,
        bytes_before: usize,
        bytes_budget: usize,
        bytes_after: usize,
    ) {
        let mut guard = self.metrics.llm_by_prompt_per_task.lock().unwrap();
        let bucket = guard
            .entry(task_id.to_string())
            .or_default()
            .entry(label.to_string())
            .or_default();
        bucket.prompt_truncation_count = bucket.prompt_truncation_count.saturating_add(1);
        bucket.prompt_bytes_before_max = Some(
            bucket
                .prompt_bytes_before_max
                .map_or(bytes_before, |current| current.max(bytes_before)),
        );
        bucket.prompt_bytes_budget_min = Some(
            bucket
                .prompt_bytes_budget_min
                .map_or(bytes_budget, |current| current.min(bytes_budget)),
        );
        bucket.prompt_bytes_after_max = Some(
            bucket
                .prompt_bytes_after_max
                .map_or(bytes_after, |current| current.max(bytes_after)),
        );
        let truncated_bytes = bytes_before.saturating_sub(bytes_after) as u64;
        bucket.prompt_truncated_bytes_total = bucket
            .prompt_truncated_bytes_total
            .saturating_add(truncated_bytes);
    }

    pub(crate) fn task_llm_elapsed_ms(&self, task_id: &str) -> u64 {
        self.metrics
            .llm_elapsed_per_task
            .lock()
            .unwrap()
            .get(task_id)
            .copied()
            .unwrap_or(0)
    }

    /// Phase 1.5: 取出 per-task 的 by-prompt 分桶快照。返回 owned map 避免锁外延。
    /// 用于在 task journal 收口时调用 `record_llm_by_prompt` 写入 metrics。
    pub(crate) fn task_llm_by_prompt(&self, task_id: &str) -> HashMap<String, LlmPromptBucket> {
        self.metrics
            .llm_by_prompt_per_task
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn task_llm_call_sequence(&self, task_id: &str) -> Vec<LlmCallSequenceEntry> {
        self.metrics
            .llm_call_sequence_per_task
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Phase 1.3: 在每次真正发起 LLM 调用前做预算检查。
    /// 超过任一阈值就返回 `Some(reason)`，调用方应立即短路。
    /// 阈值刻意给得宽松（40 次 / 180 秒），只用于兜底异常放大场景。
    pub(crate) fn task_llm_budget_exceeded(&self, task_id: &str) -> Option<String> {
        let calls = self.task_llm_call_count(task_id);
        let max_calls = self.worker.llm_max_calls_per_task.max(1);
        if calls >= max_calls {
            return Some(format!(
                "llm budget exceeded: calls={calls} limit={}",
                max_calls
            ));
        }
        let elapsed = self.task_llm_elapsed_ms(task_id);
        let max_elapsed = self.worker.llm_total_timeout_ms.max(1_000);
        if elapsed >= max_elapsed {
            return Some(format!(
                "llm budget exceeded: elapsed_ms={elapsed} limit={}",
                max_elapsed
            ));
        }
        None
    }

    pub(crate) fn clear_task_llm_call_count(&self, task_id: &str) {
        self.metrics
            .llm_calls_per_task
            .lock()
            .unwrap()
            .remove(task_id);
        self.metrics
            .llm_elapsed_per_task
            .lock()
            .unwrap()
            .remove(task_id);
        self.metrics
            .llm_by_prompt_per_task
            .lock()
            .unwrap()
            .remove(task_id);
        self.metrics
            .llm_call_sequence_per_task
            .lock()
            .unwrap()
            .remove(task_id);
    }

    pub(crate) fn get_skills_registry(&self) -> Option<Arc<SkillsRegistry>> {
        self.snapshot().registry.clone()
    }

    pub(crate) fn get_skills_list(&self) -> Arc<HashSet<String>> {
        self.snapshot().skills_list.clone()
    }

    pub(crate) fn planner_visible_skills_for_task(&self, task: &ClaimedTask) -> Vec<String> {
        let snapshot = self.snapshot();
        let execution_skills = snapshot.skills_list.clone();
        let registry = snapshot.registry.clone();
        let agent = self.task_agent(task);
        let mut visible: Vec<String> = execution_skills
            .iter()
            .filter(|skill| {
                registry
                    .as_ref()
                    .map(|reg| reg.is_planner_visible(skill))
                    .unwrap_or(true)
            })
            .filter(|skill| agent.allows_skill(skill))
            .cloned()
            .collect();
        visible.sort_unstable();
        visible
    }

    pub(crate) fn planner_available_skills_for_task(&self, task: &ClaimedTask) -> Vec<String> {
        self.planner_visible_skills_for_task(task)
            .into_iter()
            .filter(|skill| {
                self.skill_manifest(skill)
                    .map(|manifest| {
                        crate::skill_availability::evaluate_manifest_availability(&manifest)
                            .is_available()
                    })
                    .unwrap_or(true)
            })
            .collect()
    }

    pub(crate) fn normalize_known_agent_id(&self, agent_id: Option<&str>) -> Option<String> {
        agent_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .and_then(|id| self.core.agents_by_id.get(id).map(|_| id.to_string()))
    }

    pub(crate) fn task_agent_id(&self, task: &ClaimedTask) -> String {
        if let Some(payload) = crate::task_payload_value(task) {
            if let Some(agent_id) =
                self.normalize_known_agent_id(payload.get("agent_id").and_then(|v| v.as_str()))
            {
                return agent_id;
            }
        }
        crate::DEFAULT_AGENT_ID.to_string()
    }

    fn task_agent(&self, task: &ClaimedTask) -> AgentRuntimeConfig {
        let agent_id = self.task_agent_id(task);
        self.core
            .agents_by_id
            .get(&agent_id)
            .cloned()
            .or_else(|| self.core.agents_by_id.get(crate::DEFAULT_AGENT_ID).cloned())
            .unwrap_or_else(|| AgentRuntimeConfig {
                restrict_skills: false,
                allowed_skills: Arc::new(HashSet::new()),
                llm_providers: Vec::new(),
            })
    }

    pub(crate) fn agent_runtime_identity_label(&self) -> &'static str {
        DEFAULT_AGENT_RUNTIME_IDENTITY
    }

    pub(crate) fn task_allows_skill(&self, task: &ClaimedTask, canonical_skill: &str) -> bool {
        self.task_agent(task).allows_skill(canonical_skill)
    }

    pub(crate) fn task_llm_providers(&self, task: &ClaimedTask) -> Vec<Arc<LlmProviderRuntime>> {
        let agent = self.task_agent(task);
        if !agent.llm_providers.is_empty() {
            return agent.llm_providers;
        }
        self.core.llm_providers.clone()
    }

    pub(crate) fn resolve_canonical_skill_name(&self, name: &str) -> String {
        if let Some(ref r) = self.get_skills_registry() {
            if let Some(c) = r.resolve_canonical(name) {
                return c.to_string();
            }
        }
        crate::canonical_skill_name(name).to_string()
    }

    pub(crate) fn is_builtin_skill(&self, name: &str) -> bool {
        let canonical = self.resolve_canonical_skill_name(name);
        if let Some(ref r) = self.get_skills_registry() {
            return r.is_builtin(&canonical);
        }
        crate::is_builtin_skill_name(&canonical)
    }

    pub(crate) fn skill_registry_prompt_rel_path(&self, canonical_name: &str) -> Option<String> {
        self.get_skills_registry()
            .as_ref()
            .and_then(|r| r.prompt_file(canonical_name).map(String::from))
    }

    pub(crate) fn skill_kind_for_dispatch(&self, canonical_name: &str) -> SkillKind {
        if let Some(ref r) = self.get_skills_registry() {
            if let Some(entry) = r.get(canonical_name) {
                return entry.kind;
            }
        }
        if crate::is_builtin_skill_name(canonical_name) {
            SkillKind::Builtin
        } else {
            SkillKind::Runner
        }
    }

    pub(crate) fn runner_name_for_skill(&self, canonical_name: &str) -> String {
        self.get_skills_registry()
            .as_ref()
            .map(|r| r.runner_name(canonical_name))
            .unwrap_or_else(|| crate::canonical_skill_name(canonical_name).to_string())
    }

    pub(crate) fn skill_manifest(&self, canonical_name: &str) -> Option<SkillManifest> {
        self.get_skills_registry()
            .as_ref()
            .and_then(|r| r.manifest(canonical_name))
    }

    pub(crate) fn mcp_tool(
        &self,
        capability: &str,
    ) -> Option<crate::mcp_runtime::McpToolDescriptor> {
        self.core.mcp_runtime.tool(capability)
    }

    pub(crate) fn mcp_tools(&self) -> Vec<crate::mcp_runtime::McpToolDescriptor> {
        self.core.mcp_runtime.tools()
    }

    pub(crate) fn mcp_lifecycle_snapshots(&self) -> Vec<crate::mcp_runtime::McpLifecycleSnapshot> {
        self.core.mcp_runtime.lifecycle_snapshots()
    }

    pub(crate) async fn probe_mcp_server(
        &self,
        server_id: &str,
    ) -> Result<crate::mcp_runtime::McpProbeOutcome, &'static str> {
        self.core
            .mcp_runtime
            .probe(server_id)
            .await
            .map_err(|error| error.code())
    }

    pub(crate) fn skill_invocation_requires_confirmation_policy(
        &self,
        canonical_name: &str,
        args: Option<&serde_json::Value>,
    ) -> bool {
        if let Some(tool) = self.mcp_tool(canonical_name) {
            return !matches!(tool.policy.effect.as_str(), "observe" | "validate")
                || !matches!(tool.policy.risk_level.as_str(), "low");
        }
        let Some(manifest) = self.skill_manifest(canonical_name) else {
            return false;
        };
        let requires_confirmation = manifest.requires_confirmation == Some(true)
            || matches!(manifest.risk_level, Some(SkillRiskLevel::High))
            || (manifest.side_effect == Some(true) && manifest.auto_invocable == Some(false));
        if !requires_confirmation {
            return false;
        }
        if let Some(args) = args {
            if manifest
                .confirmation_exempt_when
                .iter()
                .any(|matcher| confirmation_exempt_matcher_matches(args, matcher))
            {
                return false;
            }
        }
        true
    }

    pub(crate) fn skill_is_retryable(&self, canonical_name: &str) -> bool {
        self.skill_manifest(canonical_name)
            .map(|manifest| manifest.retryable == Some(true))
            .unwrap_or(false)
    }

    pub(crate) fn skill_is_read_only(&self, canonical_name: &str) -> bool {
        self.skill_manifest(canonical_name)
            .map(|manifest| manifest.side_effect == Some(false))
            .unwrap_or(false)
    }

    pub(crate) fn skill_output_contract(
        &self,
        canonical_name: &str,
    ) -> Option<(OutputKind, serde_json::Value)> {
        let manifest = self.skill_manifest(canonical_name)?;
        let output_schema = manifest.output_schema?;
        Some((manifest.output_kind, output_schema))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaimedTask {
    pub(crate) task_id: String,
    pub(crate) user_id: i64,
    pub(crate) chat_id: i64,
    pub(crate) user_key: Option<String>,
    pub(crate) channel: String,
    pub(crate) external_user_id: Option<String>,
    pub(crate) external_chat_id: Option<String>,
    pub(crate) kind: String,
    pub(crate) payload_json: String,
}
