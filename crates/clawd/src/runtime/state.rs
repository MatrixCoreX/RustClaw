use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use claw_core::config::{
    AgentConfig, AppConfig, MaintenanceConfig, MemoryConfig, RoutingConfig, SelfExtensionConfig,
};
use claw_core::skill_registry::{
    OutputKind, SkillKind, SkillManifest, SkillRiskLevel, SkillsRegistry,
};
use reqwest::Client;
use serde::Serialize;
use tokio::sync::Semaphore;

use super::policy::{RateLimiter, ToolsPolicy};
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
/// 这一组字段除了 `config_path_for_reload` 在 `reload_skill_views` 用到，其他
/// 三个目前实际只在 reload 时被读（`#[allow(dead_code)]` 在历史版本里就标着）。
#[derive(Clone, Default)]
pub(crate) struct ReloadContext {
    pub(crate) config_path_for_reload: String,
    #[allow(dead_code)]
    pub(crate) registry_path_for_reload: Option<String>,
    #[allow(dead_code)]
    pub(crate) skill_switches_for_reload: Arc<HashMap<String, bool>>,
    #[allow(dead_code)]
    pub(crate) initial_skills_list_for_reload: Vec<String>,
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
}

impl CoreServices {
    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        let agents_by_id = HashMap::from([(
            crate::DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
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
        }
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
            cmd_timeout_seconds: 30,
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
                all_result_suffixes: Vec::new(),
                default_locale: locale.to_string(),
                verify_enforce_enabled: false,
            },
            schedule: ScheduleRuntime {
                timezone: "Asia/Shanghai".to_string(),
                intent_prompt_template: Arc::new(RwLock::new(String::new())),
                intent_prompt_source: String::new(),
                intent_rules_template: Arc::new(RwLock::new(String::new())),
                locale: locale.to_string(),
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
    pub(crate) started_at: Instant,
    pub(crate) queue_limit: usize,
    pub(crate) worker_task_timeout_seconds: u64,
    pub(crate) worker_task_heartbeat_seconds: u64,
    pub(crate) worker_running_no_progress_timeout_seconds: u64,
    pub(crate) worker_running_recovery_check_interval_seconds: u64,
    pub(crate) last_running_recovery_check_ts: Arc<Mutex<u64>>,
    pub(crate) database_busy_timeout_ms: u64,
    pub(crate) database_sqlite_path: PathBuf,
}

impl WorkerConfig {
    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        Self {
            started_at: Instant::now(),
            queue_limit: 1,
            worker_task_timeout_seconds: 300,
            worker_task_heartbeat_seconds: 10,
            worker_running_no_progress_timeout_seconds: 300,
            worker_running_recovery_check_interval_seconds: 30,
            last_running_recovery_check_ts: Arc::new(Mutex::new(0)),
            database_busy_timeout_ms: 5_000,
            database_sqlite_path: PathBuf::new(),
        }
    }
}

/// P2.1 Stage 2 — `TaskMetricsRegistry` 簇：per-task LLM 计数 / 耗时 /
/// by-prompt 分桶 / schedule_intent 复用缓存。
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
    /// 入口处做一次预算检查，超过 `MAX_LLM_CALLS_PER_TASK` 或
    /// `MAX_LLM_TOTAL_MS_PER_TASK` 就直接短路返回错误，防止单个任务
    /// 无限扩张 LLM 预算（例如 plan_repair 抖动、fallback 雪崩）。
    pub(crate) llm_elapsed_per_task: Arc<Mutex<HashMap<String, u64>>>,
    /// Phase 1.5: per-task 的 LLM 调用按 prompt label 分桶累计（次数 + 耗时）。
    /// 与 `llm_calls_per_task` / `llm_elapsed_per_task` 是同一份数据的不同维度：
    /// 总量用前两个表，而"哪个 prompt 把额度烧光了"用这个表。诊断用。
    /// 外层 key = task_id；内层 key = label（如 `normalizer` / `plan` /
    /// `plan_repair` / `chat` / `classifier_direct` / `observed` / `nl2cmd`...）。
    /// 标签由 [`crate::llm_gateway::classify_prompt_source`] 从 `prompt_source` 抽出。
    pub(crate) llm_by_prompt_per_task:
        Arc<Mutex<HashMap<String, HashMap<String, LlmPromptBucket>>>>,
    /// Phase 0.4: 缓存 `run_intent_normalizer` 产出的 `schedule_intent`，
    /// 让后续 `schedule.compile` 技能在 `text` 与归一化后的原始输入一致时
    /// 直接复用，不再重跑一次 `schedule_intent_prompt` LLM 调用。
    /// Key = task_id；Value = (归一化后的原始 user_request, 解析结果)。
    pub(crate) task_schedule_intent_cache:
        Arc<Mutex<HashMap<String, (String, crate::ScheduleIntentOutput)>>>,
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
    let mut planner_visible: Vec<String> = enabled.iter().cloned().collect();
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

/// schedule.compile 技能的 text 归一化：trim + lowercase + 折叠内部空白。
/// 与缓存匹配时用同一函数，避免因"用户输入末尾有换行/多一个空格"导致误 miss。
pub(crate) fn normalize_schedule_compile_text(text: &str) -> String {
    let lowered = text.trim().to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut prev_whitespace = false;
    for ch in lowered.chars() {
        if ch.is_whitespace() {
            if !prev_whitespace && !out.is_empty() {
                out.push(' ');
            }
            prev_whitespace = true;
        } else {
            out.push(ch);
            prev_whitespace = false;
        }
    }
    out
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

impl AppState {
    fn snapshot(&self) -> Arc<SkillViewsSnapshot> {
        self.core.skill_views_snapshot.read().unwrap().clone()
    }

    /// 兼容入口：不带 label 的累计。新代码请优先调用 [`Self::note_task_llm_call_with_label`]
    /// 把 `label` 也传进来，否则 `by_prompt` 维度会缺失。
    #[allow(dead_code)]
    pub(crate) fn note_task_llm_call(&self, task_id: &str) {
        self.note_task_llm_call_with_label(task_id, "unspecified");
    }

    /// Phase 1.5: 累计一次 LLM 调用，并按 prompt label 分桶记录。
    /// `label` 由 [`crate::llm_gateway::classify_prompt_source`] 从 `prompt_source` 抽出。
    pub(crate) fn note_task_llm_call_with_label(&self, task_id: &str, label: &str) {
        {
            let mut guard = self.metrics.llm_calls_per_task.lock().unwrap();
            let counter = guard.entry(task_id.to_string()).or_insert(0);
            *counter = counter.saturating_add(1);
        }
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

    /// Phase 1.3: 追加一次 LLM 调用的耗时（成功/失败都记，保证预算真实反映压力）。
    /// 兼容入口：不带 label。新代码请优先调用 [`Self::note_task_llm_elapsed_with_label`]。
    #[allow(dead_code)]
    pub(crate) fn note_task_llm_elapsed(&self, task_id: &str, elapsed_ms: u64) {
        self.note_task_llm_elapsed_with_label(task_id, "unspecified", elapsed_ms);
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

    /// Phase 1.3: 在每次真正发起 LLM 调用前做预算检查。
    /// 超过任一阈值就返回 `Some(reason)`，调用方应立即短路。
    /// 阈值刻意给得宽松（40 次 / 180 秒），只用于兜底异常放大场景。
    pub(crate) fn task_llm_budget_exceeded(&self, task_id: &str) -> Option<String> {
        let calls = self.task_llm_call_count(task_id);
        if calls >= crate::llm_gateway::MAX_LLM_CALLS_PER_TASK {
            return Some(format!(
                "llm budget exceeded: calls={calls} limit={}",
                crate::llm_gateway::MAX_LLM_CALLS_PER_TASK
            ));
        }
        let elapsed = self.task_llm_elapsed_ms(task_id);
        if elapsed >= crate::llm_gateway::MAX_LLM_TOTAL_MS_PER_TASK {
            return Some(format!(
                "llm budget exceeded: elapsed_ms={elapsed} limit={}",
                crate::llm_gateway::MAX_LLM_TOTAL_MS_PER_TASK
            ));
        }
        None
    }

    pub(crate) fn clear_task_llm_call_count(&self, task_id: &str) {
        self.metrics.llm_calls_per_task.lock().unwrap().remove(task_id);
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
            .task_schedule_intent_cache
            .lock()
            .unwrap()
            .remove(task_id);
    }

    /// 把 normalizer 解析出的 `schedule_intent` 缓存起来，键入 `task_id` 与
    /// 对应的归一化原始文本。仅在 normalizer 实际返回了 schedule_intent 时调用。
    pub(crate) fn cache_task_schedule_intent(
        &self,
        task_id: &str,
        user_request: &str,
        intent: &crate::ScheduleIntentOutput,
    ) {
        let normalized = normalize_schedule_compile_text(user_request);
        if normalized.is_empty() {
            return;
        }
        self.metrics
            .task_schedule_intent_cache
            .lock()
            .unwrap()
            .insert(task_id.to_string(), (normalized, intent.clone()));
    }

    /// 若 `skill_text` 归一化后与缓存的 normalizer 输入一致则返回复用的解析结果，
    /// 否则返回 None，调用方应回退到 `parse_schedule_intent` 走完整的 LLM 链路。
    pub(crate) fn take_task_schedule_intent_if_matches(
        &self,
        task_id: &str,
        skill_text: &str,
    ) -> Option<crate::ScheduleIntentOutput> {
        let normalized = normalize_schedule_compile_text(skill_text);
        if normalized.is_empty() {
            return None;
        }
        let mut guard = self.metrics.task_schedule_intent_cache.lock().unwrap();
        let cached_text_matches = guard
            .get(task_id)
            .map(|(cached, _)| cached == &normalized)
            .unwrap_or(false);
        if cached_text_matches {
            guard.remove(task_id).map(|(_, intent)| intent)
        } else {
            None
        }
    }

    pub(crate) fn get_skills_registry(&self) -> Option<Arc<SkillsRegistry>> {
        self.snapshot().registry.clone()
    }

    pub(crate) fn get_skills_list(&self) -> Arc<HashSet<String>> {
        self.snapshot().skills_list.clone()
    }

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
                persona_prompt: String::new(),
                restrict_skills: false,
                allowed_skills: Arc::new(HashSet::new()),
                llm_providers: Vec::new(),
            })
    }

    pub(crate) fn task_persona_prompt(&self, task: &ClaimedTask) -> String {
        let agent = self.task_agent(task);
        let base_prompt = if !agent.persona_prompt.trim().is_empty() {
            agent.persona_prompt
        } else {
            self.policy.persona_prompt_string()
        };
        let auth_role = task
            .user_key
            .as_deref()
            .and_then(|user_key| {
                crate::resolve_auth_identity_by_key(self, user_key)
                    .ok()
                    .flatten()
            })
            .map(|identity| identity.role)
            .unwrap_or_else(|| "unknown".to_string());
        let permission_hint = if auth_role.eq_ignore_ascii_case("admin") {
            format!(
                "Current auth role for this task: {auth_role}. Only admin may modify files under configs/, and this task is admin-authenticated."
            )
        } else {
            format!(
                "Current auth role for this task: {auth_role}. Only admin may modify files under configs/. If the request is to change config files, reply that there is no permission and do not attempt the modification."
            )
        };
        if base_prompt.trim().is_empty() {
            permission_hint
        } else {
            format!("{base_prompt}\n\n{permission_hint}")
        }
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

    pub(crate) fn skill_requires_confirmation_policy(&self, canonical_name: &str) -> bool {
        let Some(manifest) = self.skill_manifest(canonical_name) else {
            return false;
        };
        manifest.requires_confirmation == Some(true)
            || matches!(manifest.risk_level, Some(SkillRiskLevel::High))
            || (manifest.side_effect == Some(true) && manifest.auto_invocable == Some(false))
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

#[derive(Debug, Clone)]
pub(crate) struct LlmProviderRuntime {
    pub(crate) config: claw_core::config::LlmProviderConfig,
    pub(crate) client: Client,
    pub(crate) semaphore: Arc<Semaphore>,
    /// Phase 2.1: 每 provider 一个 circuit breaker，避免坏 provider 在 fallback
    /// 链路里被反复重试 + 反复消耗 retry/timeout 预算。`Arc` 保证 `Clone` 后
    /// 多份引用共享同一份故障状态。
    pub(crate) breaker: Arc<crate::providers::CircuitBreaker>,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentRuntimeConfig {
    pub(crate) persona_prompt: String,
    pub(crate) restrict_skills: bool,
    pub(crate) allowed_skills: Arc<HashSet<String>>,
    pub(crate) llm_providers: Vec<Arc<LlmProviderRuntime>>,
}

impl AgentRuntimeConfig {
    pub(crate) fn from_config(
        config: &AgentConfig,
        llm_providers: Vec<Arc<LlmProviderRuntime>>,
    ) -> Self {
        let allowed_skills = config
            .allowed_skills
            .iter()
            .map(|skill| crate::canonical_skill_name(skill).to_string())
            .collect::<HashSet<_>>();
        Self {
            persona_prompt: config.persona_prompt.trim().to_string(),
            restrict_skills: !allowed_skills.is_empty(),
            allowed_skills: Arc::new(allowed_skills),
            llm_providers,
        }
    }

    pub(crate) fn allows_skill(&self, canonical_skill: &str) -> bool {
        !self.restrict_skills || self.allowed_skills.contains(canonical_skill)
    }
}

#[derive(Clone)]
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
