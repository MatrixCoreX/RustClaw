use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use claw_core::config::{AgentConfig, AppConfig, MaintenanceConfig, MemoryConfig, RoutingConfig};
use claw_core::skill_registry::{SkillKind, SkillsRegistry};
use reqwest::Client;
use rusqlite::Connection;
use serde::Serialize;
use tokio::sync::Semaphore;

use super::policy::{RateLimiter, ToolsPolicy};
use super::types::{CommandIntentRuntime, ScheduleRuntime};

pub(crate) struct SkillViews {
    pub(crate) registry: Option<Arc<SkillsRegistry>>,
    pub(crate) execution_skills: HashSet<String>,
    pub(crate) planner_visible: Vec<String>,
}

pub(crate) struct SkillViewsSnapshot {
    pub(crate) registry: Option<Arc<SkillsRegistry>>,
    pub(crate) skills_list: Arc<HashSet<String>>,
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

    tracing::info!(
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
    pub(crate) registry_entries: usize,
    pub(crate) execution_skills_count: usize,
    pub(crate) planner_visible_count: usize,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) started_at: Instant,
    pub(crate) queue_limit: usize,
    pub(crate) db: Arc<Mutex<Connection>>,
    pub(crate) llm_providers: Vec<Arc<LlmProviderRuntime>>,
    pub(crate) agents_by_id: Arc<HashMap<String, AgentRuntimeConfig>>,
    pub(crate) skill_timeout_seconds: u64,
    pub(crate) skill_runner_path: PathBuf,
    pub(crate) skill_views_snapshot: Arc<RwLock<Arc<SkillViewsSnapshot>>>,
    pub(crate) skill_semaphore: Arc<Semaphore>,
    pub(crate) rate_limiter: Arc<Mutex<RateLimiter>>,
    pub(crate) maintenance: MaintenanceConfig,
    pub(crate) memory: MemoryConfig,
    pub(crate) workspace_root: PathBuf,
    pub(crate) tools_policy: Arc<ToolsPolicy>,
    pub(crate) active_provider_type: Option<String>,
    pub(crate) cmd_timeout_seconds: u64,
    pub(crate) max_cmd_length: usize,
    pub(crate) allow_path_outside_workspace: bool,
    pub(crate) allow_sudo: bool,
    pub(crate) worker_task_timeout_seconds: u64,
    pub(crate) worker_task_heartbeat_seconds: u64,
    pub(crate) worker_running_no_progress_timeout_seconds: u64,
    pub(crate) worker_running_recovery_check_interval_seconds: u64,
    pub(crate) last_running_recovery_check_ts: Arc<Mutex<u64>>,
    pub(crate) routing: RoutingConfig,
    pub(crate) persona_prompt: String,
    pub(crate) command_intent: CommandIntentRuntime,
    pub(crate) schedule: ScheduleRuntime,
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
    pub(crate) http_client: Client,
    pub(crate) config_path_for_reload: String,
    #[allow(dead_code)]
    pub(crate) registry_path_for_reload: Option<String>,
    #[allow(dead_code)]
    pub(crate) skill_switches_for_reload: Arc<HashMap<String, bool>>,
    #[allow(dead_code)]
    pub(crate) initial_skills_list_for_reload: Vec<String>,
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
            .and_then(|id| self.agents_by_id.get(id).map(|_| id.to_string()))
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
        self.agents_by_id
            .get(&agent_id)
            .cloned()
            .or_else(|| self.agents_by_id.get(crate::DEFAULT_AGENT_ID).cloned())
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

    pub(crate) fn task_llm_providers(&self, task: &ClaimedTask) -> Vec<Arc<LlmProviderRuntime>> {
        let agent = self.task_agent(task);
        if !agent.llm_providers.is_empty() {
            return agent.llm_providers;
        }
        self.llm_providers.clone()
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

    pub(crate) fn skill_prompt_file(&self, canonical_name: &str) -> Option<String> {
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
}

#[derive(Debug, Clone)]
pub(crate) struct LlmProviderRuntime {
    pub(crate) config: claw_core::config::LlmProviderConfig,
    pub(crate) client: Client,
    pub(crate) semaphore: Arc<Semaphore>,
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
