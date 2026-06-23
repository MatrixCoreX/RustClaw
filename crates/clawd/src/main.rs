use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::routing::{delete, get, get_service, post};
use axum::{Json, Router};
use claw_core::config::AppConfig;
use claw_core::types::{
    ApiResponse, AuthIdentity, ChannelKind, DirectClassifyRequest, DirectClassifyResponse,
    HealthResponse, SubmitTaskRequest, SubmitTaskResponse, TaskQueryResponse,
};
use reqwest::Client;
use serde::Serialize;
use serde_json::json;
use tokio::sync::Semaphore;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tracing::{error, info, warn};
use uuid::Uuid;

mod agent_engine;
mod agent_hooks;
mod agent_runtime_contract;
mod answer_verifier;
mod app_helpers;
mod ask_flow;
mod async_job_contract;
mod bootstrap;
mod capability_map;
mod capability_resolver;
mod channel_send;
mod clarify_followup;
mod clarify_state;
mod contract_matrix;
mod conversation_state;
mod db_init;
mod delivery_utils;
mod execution_adapters;
mod execution_recipe;
mod executor;
mod fallback;
mod finalize;
#[cfg(test)]
mod fixture_replay_e2e;
mod followup_frame;
mod http;
mod intent;
mod intent_router;
mod language_policy;
mod llm_gateway;
mod log_utils;
mod media_artifact_paths;
mod memory;
mod observed_facts;
mod output_contract_verifier;
mod output_paths;
mod package_commands;
mod pipeline_types;
mod policy_decision;
mod post_route_policy;
mod prompt_utils;
mod providers;
mod repair_boundary_inventory;
mod repair_signal;
mod repo;
mod routing_context;
mod runtime;
mod schedule_service;
mod self_extension;
mod semantic_judge;
mod skill_availability;
mod skills;
mod system_health;
mod task_admin_routes;
mod task_context_builder;
mod task_contract;
mod task_journal;
mod task_lifecycle;
mod verifier;
mod virtual_tools;
mod visible_text;
mod worker;

pub(crate) use app_helpers::{
    bilingual_t_with_default_vars, ensure_column_exists, i18n_t_with_default,
    i18n_t_with_default_vars, is_affirmation_click_text, main_flow_rules, mask_secret,
    normalize_affirmation_text, normalize_exchange_name, normalize_external_id_opt, now_ts,
    now_ts_u64, parse_resume_context_error, parse_task_status, RESUME_CONTINUE_SOURCES,
    TASK_STATUS_QUEUED,
};
pub(crate) use ask_flow::{
    analyze_attached_images_for_ask, build_resume_continue_execute_prompt,
    build_resume_followup_discussion_prompt, execute_ask_routed,
};
use bootstrap::{
    active_prompt_vendor_name, load_command_intent_runtime, load_feishu_send_config,
    load_lark_send_config, load_memory_runtime_config, load_persona_prompt,
    load_prompt_template_for_state, load_schedule_runtime, load_wechat_send_config,
    resolve_prompt_rel_path_for_vendor, resolve_ui_dist_dir,
};
use db_init::{
    ensure_channel_schema, ensure_memory_schema, ensure_schedule_schema, init_db, seed_users,
};
pub(crate) use delivery_utils::{
    collect_recent_image_candidates, extract_delivery_file_tokens,
    intercept_response_payload_for_delivery, intercept_response_text_for_delivery,
};
pub(crate) use log_utils::{
    append_act_plan_log, append_subtask_result, highlight_tag, truncate_for_agent_trace,
    truncate_for_log,
};
pub(crate) use memory::dynamic_chat_memory_budget_chars;
pub(crate) use output_paths::ensure_default_file_path;
pub(crate) use pipeline_types::{
    plan_step_from_agent_action, IntentOutputContract, OutputDeliveryIntent, OutputListSelector,
    OutputLocatorKind, OutputResponseShape, OutputScalarCountFilter, OutputScalarCountTargetKind,
    OutputSemanticKind, PlanKind, PlanResult, PlanStep, ResumeBehavior, RiskCeiling, RouteResult,
    ScheduleKind, SelfExtensionContract, SelfExtensionMode, SelfExtensionTrigger,
};
pub(crate) use prompt_utils::{
    extract_first_json_value_any, log_prompt_render, log_prompt_render_with_version,
    parse_agent_action_json_with_repair, parse_llm_json_extract_or_any, parse_llm_json_raw_or_any,
    render_prompt_template,
};
use providers::{
    append_model_io_log, call_provider_with_retry, call_provider_with_retry_with_hints,
    log_color_enabled, maybe_sanitize_llm_text_output, truncate_text, utf8_safe_prefix,
    ChatRequestHints,
};
pub(crate) use repo::{
    attach_pending_channel_bind_session_install_flow, bind_channel_identity,
    build_conversation_chat_id, build_submit_task_payload, cancel_one_task_for_user_chat,
    cancel_task_by_id, cancel_tasks_for_user_chat, check_submit_task_access,
    check_submit_task_limits, check_task_view_access, create_auth_key,
    create_pending_channel_bind_session, delete_auth_key_by_id,
    exchange_credential_status_for_user_key, factory_reset_auth_state,
    finalize_pending_channel_bind_session, find_recent_failed_resume_context,
    get_auth_key_value_by_id, get_pending_channel_bind_session_by_id,
    get_pending_channel_bind_session_by_token, get_task_admin_target, get_task_query_record,
    has_channel_binding_for_user_key, insert_audit_log, insert_submitted_task, is_user_allowed,
    list_active_tasks_internal, list_auth_keys, mark_pending_channel_bind_session_detected,
    mark_pending_channel_bind_session_expired, mark_pending_channel_bind_session_failed,
    maybe_find_submit_task_dedup, normalize_user_key, reset_channel_binding_state_for_user_key,
    resolve_auth_identity_by_key, resolve_channel_binding_identity, resolve_submit_task_context,
    stable_i64_from_key, submit_task_audit_detail, task_count_by_status, task_kind_name,
    update_auth_key_by_id, update_task_timeout, upsert_exchange_credential_for_user_key,
    upsert_webd_login_account, verify_webd_password_login, FactoryResetDbResult,
    PendingChannelBindSession, SubmitTaskAccessError, SubmitTaskContextError, SubmitTaskLimitError,
    TaskAdminTarget, TaskViewerAccessError,
};
use repo::{ensure_bootstrap_admin_key, ensure_key_auth_schema, seed_channel_bindings};
use task_admin_routes::{
    cancel_one_task, cancel_task_by_id as cancel_task_by_id_handler, cancel_tasks,
    list_active_tasks, pause_task_by_id, resume_task_by_id,
};
pub(crate) use task_contract::TaskContract;
// Phase 3.2 Stage B：AskMode 已经被 RouteResult/PreparedAskRouting 消费；
// ChatEntryStrategy/ActFinalizeStyle 在 Stage C 切换 match 时才会被显式 import。
pub(crate) use runtime::{
    build_skill_views, llm_model_kind, llm_vendor_name, log_ask_transition, reload_skill_views,
    ActFinalizeStyle, AgentAction, AgentRuntimeConfig, AppState, AskMode, AskReply, AskState,
    AskStateRegistry, AskTransition, ChannelConfig, ChatEntryStrategy, ClaimedTask,
    CommandIntentRules, CommandIntentRuntime, CoreServices, FirstLayerDecision, LlmPromptBucket,
    LlmProviderRuntime, LocalInteractionContext, MemoryConfigFileWrapper, PolicyConfig,
    RateLimiter, ReloadContext, RouteGateKind, RuntimeChannel, ScheduleIntentOutput,
    ScheduleRuntime, ScheduledJobDue, SkillRuntime, SkillViewsSnapshot, TaskMetricsRegistry,
    ToolsPolicy, WhatsappDeliveryRoute, WorkerConfig,
};
pub(crate) use skills::{canonical_skill_name, is_builtin_skill_name};
use skills::{run_skill_with_runner, run_skill_with_runner_outcome};
pub(crate) use system_health::{
    channel_gateway_process_stats, current_rss_bytes, daemon_process_pids_by_name,
    feishud_process_stats, larkd_process_stats, oldest_running_task_age_seconds,
    telegramd_process_stats, wa_webd_process_stats, webd_process_stats, wechatd_process_stats,
    whatsappd_process_stats,
};
pub(crate) use worker::task_payload_value;
use worker::{
    recover_stale_running_tasks_on_startup, spawn_cleanup_worker, spawn_schedule_worker,
    spawn_worker, task_external_chat_id,
};

pub(crate) const INIT_SQL: &str = include_str!("../../../migrations/001_init.sql");
pub(crate) const MEMORY_UPGRADE_SQL: &str =
    include_str!("../../../migrations/002_memory_upgrade.sql");
pub(crate) const CHANNEL_UPGRADE_SQL: &str =
    include_str!("../../../migrations/003_channels_upgrade.sql");
const KEY_AUTH_UPGRADE_SQL: &str = include_str!("../../../migrations/004_key_auth.sql");
pub(crate) const WEBD_LOGIN_SQL: &str = include_str!("../../../migrations/005_webd_login.sql");
const LLM_RETRY_TIMES: usize = 2;
pub(crate) const AGENT_MAX_STEPS: usize = 32;
pub(crate) const RESUME_CONTEXT_ERROR_PREFIX: &str = "__RESUME_CTX__";
pub(crate) const MAX_READ_FILE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_WRITE_FILE_BYTES: usize = 128 * 1024;
/// Per-line truncation ceiling for [`crate::log_utils::truncate_for_log`].
///
/// 历史值是 16_000：早期既要防"single fat line wrecks IDE / journalctl
/// follow"，又要防"model_io.log 总量爆磁盘"；后者已被
/// [`crate::providers::output::rotate_model_io_log_daily`] + 7 天过期机制覆盖
/// （[`crate::providers::output::MODEL_IO_LOG_KEEP_DAYS`]）。
///
/// §7.5 把上限抬到 128_000：覆盖 99% 真实 normalizer / planner prompt
/// （模板 + skill catalog + few-shots + 历史上下文 通常 15~30 KB；极端可达
/// 60~100 KB），同时给 fixture 录制留足 response 长度 —— 之前 16K 截断会让
/// `convert_model_io_log_to_fixture` 拒掉长 response 的 case。
///
/// 仍不去掉天花板的两个理由：
///   1. stdout / tracing 行（被 25+ 处复用此函数）一行 1MB 时 IDE / `journalctl
///      -f` / `docker logs` 会有可见卡顿；128K 是"留够空间但不让单行失控"的
///      折衷点。
///   2. 防御性编程：万一未来某条 prompt 被错误拼接到 GB 级（bug），日志
///      也不至于把磁盘灌满整 行。
///
/// 仅当真的撞到 128K 上限时 [`crate::providers::fixture_replay::convert_model_io_log_to_fixture`]
/// 仍会 fail-loud（截断后 response 喂回 LLM-output parser 会在结尾静默炸）。
const MODEL_IO_LOG_MAX_CHARS: usize = 128_000;
const AGENT_TRACE_LOG_MAX_CHARS: usize = 4000;
const LOG_CALL_WRAP: &str = "---- task-call ----";
const DEFAULT_AGENT_ID: &str = "main";

pub(crate) const CHAT_RESPONSE_PROMPT_LOGICAL_PATH: &str = "prompts/chat_response_prompt.md";
pub(crate) const RESUME_FOLLOWUP_DISCUSSION_PROMPT_LOGICAL_PATH: &str =
    "prompts/resume_followup_discussion_prompt.md";

/// 统一错误响应，避免重复手写 (StatusCode, Json(ApiResponse)).
fn api_err<T: Serialize>(
    status: StatusCode,
    message: impl Into<String>,
) -> (StatusCode, Json<ApiResponse<T>>) {
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

fn resolve_startup_config_path_from<I>(
    args: I,
    env_config_path: Option<String>,
) -> anyhow::Result<String>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let mut cli_config_path: Option<String> = None;
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--config=") {
            let value = value.trim();
            if value.is_empty() {
                anyhow::bail!("--config requires a non-empty path");
            }
            cli_config_path = Some(value.to_string());
            continue;
        }
        if arg == "--config" {
            let Some(value) = args.next() else {
                anyhow::bail!("--config requires a path");
            };
            let value = value.trim();
            if value.is_empty() {
                anyhow::bail!("--config requires a non-empty path");
            }
            cli_config_path = Some(value.to_string());
        }
    }
    Ok(cli_config_path
        .or_else(|| env_config_path.map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "configs/config.toml".to_string()))
}

fn resolve_startup_config_path() -> anyhow::Result<String> {
    resolve_startup_config_path_from(
        std::env::args().skip(1),
        std::env::var("RUSTCLAW_CONFIG_PATH").ok(),
    )
}

#[cfg(test)]
#[path = "main_startup_config_path_tests.rs"]
mod startup_config_path_tests;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        // 默认用 info 级别，若设置 RUST_LOG 则以环境变量为准。
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .with_target(false)
        .with_ansi(log_color_enabled())
        .compact()
        .init();

    let config_path = resolve_startup_config_path()?;
    let config = AppConfig::load(&config_path)?;
    let workspace_root = std::env::current_dir()?;
    info!("startup config_path={}", config_path);
    let tools_policy = ToolsPolicy::from_config(&config.tools)
        .map_err(|err| anyhow::anyhow!("invalid tools config: {err}"))?;
    let db_pool = init_db(&config)?;
    let audit_db_pool = db_init::init_audit_db(&config)?;
    if let Err(e) = db_init::migrate_audit_logs_from_main_db(&db_pool, &audit_db_pool) {
        warn!(
            "phase2.2-stage2: audit_logs one-shot migration failed (non-fatal, audit_logs left in main db): {e}"
        );
    }
    {
        let db = db_pool
            .get()
            .map_err(|e| anyhow::anyhow!("get db conn for setup: {e}"))?;
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
            memory::indexing::rebuild_retrieval_index(&db, &config.memory, &workspace_root)?;
        }
    }
    let bootstrap_admin_key = {
        let db = db_pool
            .get()
            .map_err(|e| anyhow::anyhow!("get db conn: {e}"))?;
        let key = ensure_bootstrap_admin_key(&db)?;
        seed_channel_bindings(&db, &config)?;
        key
    };
    if let Some(user_key) = bootstrap_admin_key.as_deref() {
        warn!("============================================================");
        warn!("No auth key found in database. Generated initial admin key.");
        warn!("Initial admin key: {}", user_key);
        warn!("Default web login: username=rustclaw password=123456");
        warn!("Please save it now and use it to bind UI / Telegram / WhatsApp.");
        warn!("============================================================");
        eprintln!("============================================================");
        eprintln!("Initial admin key: {}", user_key);
        eprintln!("Default web login: username=rustclaw password=123456");
        eprintln!("Please save it now and use it to bind UI / Telegram / WhatsApp.");
        eprintln!("============================================================");
    }
    let recovered_task_ids = {
        let db = db_pool
            .get()
            .map_err(|e| anyhow::anyhow!("get db conn: {e}"))?;
        recover_stale_running_tasks_on_startup(
            &db,
            config.worker.running_no_progress_timeout_seconds.max(1),
        )?
    };
    if !recovered_task_ids.is_empty() {
        let recovery_detail = json!({
            "reason": "startup_stale_running_recovery",
            "no_progress_timeout_seconds": config.worker.running_no_progress_timeout_seconds.max(1),
            "recovered_count": recovered_task_ids.len(),
            "task_ids": recovered_task_ids,
        });
        let audit_res = {
            let db = db_pool
                .get()
                .map_err(|e| anyhow::anyhow!("get db conn: {e}"))?;
            repo::insert_audit_log_raw(
                &db,
                None,
                "startup_recover_running_timeout",
                Some(&recovery_detail.to_string()),
                None,
            )
        };
        if let Err(err) = audit_res {
            warn!("write startup recovery audit log failed: {err}");
        }
        warn!(
            "startup stale-running recovery applied: converted {} tasks to timeout (threshold={}s)",
            recovery_detail["recovered_count"]
                .as_u64()
                .unwrap_or_default(),
            config.worker.running_no_progress_timeout_seconds.max(1)
        );
    } else {
        info!(
            "startup stale-running recovery: no stale running tasks found (threshold={}s)",
            config.worker.running_no_progress_timeout_seconds.max(1)
        );
    }

    let memory_runtime = load_memory_runtime_config(&workspace_root, &config.memory);
    let command_intent = load_command_intent_runtime(&workspace_root, &config.command_intent);
    let schedule = load_schedule_runtime(
        &workspace_root,
        &config.schedule,
        config.llm.selected_vendor.as_deref(),
    )?;
    let routing = config.routing.clone();
    let persona_prompt = load_persona_prompt(
        &workspace_root,
        config.llm.selected_vendor.as_deref(),
        &config.persona,
    );
    {
        let prompt_validation = bootstrap::validate_core_prompts(
            &workspace_root,
            config.llm.selected_vendor.as_deref(),
        );
        bootstrap::log_prompt_validation_report(&prompt_validation);
        if config.prompts.strict_validation_at_startup {
            if let Some(message) = bootstrap::strict_prompt_validation_error(&prompt_validation) {
                anyhow::bail!(message);
            }
        }
    }
    let effective_skill_runner_path = workspace_root.join("target/release/skill-runner");
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
        "run_cmd config: timeout_seconds={}, idle_timeout_seconds={}, max_output_bytes={}, max_cmd_length={}, allow_outside_workspace={}, allow_sudo={}",
        config.tools.cmd_timeout_seconds.max(1),
        config.tools.cmd_idle_timeout_seconds.max(1),
        config.tools.cmd_max_output_bytes.max(128),
        config.tools.max_cmd_length.max(16),
        config.tools.allow_path_outside_workspace,
        config.tools.allow_sudo
    );
    info!(
        "schedule config: timezone={}, prompt_chars={}, rules_chars={}",
        schedule.timezone,
        schedule.intent_prompt_template_string().chars().count(),
        schedule.intent_rules_template_string().chars().count()
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
    let database_sqlite_path = {
        let raw = std::path::PathBuf::from(&config.database.sqlite_path);
        if raw.is_absolute() {
            raw
        } else {
            workspace_root.join(raw)
        }
    };
    let normalized_agents = config.normalized_agents();
    let mut agents_by_id = HashMap::new();
    for agent in &normalized_agents {
        let override_providers =
            if agent.preferred_vendor.is_some() || agent.preferred_model.is_some() {
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

    // §P4.1 收尾：registry 必须覆盖所有 REQUIRED_BUILTIN_SKILLS（且 kind=builtin），
    // 否则 chat / run_cmd / read_file 这些核心技能在 dispatch 时会被走 runner
    // 子进程，行为静默回退。这里在启动期一次性 bail，便于早发现别名漂移或
    // 误改 kind。
    if let Some(ref reg) = views.registry {
        let report = reg.integrity_report();
        if !report.is_clean() {
            let path_display = config.skills.registry_path.as_deref().unwrap_or("(none)");
            let detail = report.into_human_message().unwrap_or_default();
            let msg =
                format!("skills registry integrity check failed (path={path_display}): {detail}");
            error!("startup: {msg}");
            return Err(anyhow::anyhow!(msg));
        }
    } else {
        warn!(
            "startup: no skills registry loaded (path={}); falling back to hardcoded builtin set, future routing may drift",
            config.skills.registry_path.as_deref().unwrap_or("(none)")
        );
    }

    let feishu_send_config = load_feishu_send_config(&workspace_root);
    let lark_send_config = load_lark_send_config(&workspace_root);
    let wechat_send_config = load_wechat_send_config(&workspace_root);
    if wechat_send_config.is_some() {
        info!("wechat send config loaded for schedule push (configs/channels/wechat.toml)");
    }
    if feishu_send_config.is_some() {
        info!("feishu send config loaded for schedule push (configs/channels/feishu.toml)");
    }
    if lark_send_config.is_some() {
        info!("lark send config loaded for schedule push (configs/channels/lark.toml)");
    }
    let default_locator_search_dir = {
        let raw = config.routing.default_locator_search_dir.trim();
        if raw.is_empty() {
            workspace_root.clone()
        } else {
            let path = std::path::Path::new(raw);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                workspace_root.join(path)
            }
        }
    };
    let locator_scan_max_depth = config.routing.locator_scan_max_depth;
    let locator_scan_max_files = config.routing.locator_scan_max_files.max(1);
    info!(
        "routing default_locator_search_dir={} locator_scan_max_depth={} locator_scan_max_files={}",
        default_locator_search_dir.display(),
        locator_scan_max_depth,
        locator_scan_max_files,
    );

    let state = AppState {
        core: crate::CoreServices {
            db: db_pool,
            audit_db: audit_db_pool,
            llm_providers,
            agents_by_id: Arc::new(agents_by_id),
            http_client: Client::new(),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: views.registry,
                skills_list: Arc::new(views.execution_skills),
            }))),
            active_provider_type,
        },
        skill_rt: crate::SkillRuntime {
            skill_timeout_seconds: config.skills.skill_timeout_seconds,
            skill_runner_path: effective_skill_runner_path,
            skill_semaphore: Arc::new(Semaphore::new(config.skills.skill_max_concurrency.max(1))),
            tools_policy: Arc::new(tools_policy),
            cmd_timeout_seconds: config.tools.cmd_timeout_seconds.max(1),
            cmd_idle_timeout_seconds: config.tools.cmd_idle_timeout_seconds.max(1),
            cmd_max_output_bytes: config.tools.cmd_max_output_bytes.max(128),
            max_cmd_length: config.tools.max_cmd_length.max(16),
            workspace_root,
            default_locator_search_dir,
            locator_scan_max_depth,
            locator_scan_max_files,
        },
        policy: crate::PolicyConfig {
            maintenance: config.maintenance.clone(),
            memory: memory_runtime,
            routing,
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
        },
        worker: crate::WorkerConfig {
            started_at: Instant::now(),
            queue_limit: config.worker.queue_limit,
            worker_task_timeout_seconds: config.worker.task_timeout_seconds.max(1),
            llm_max_calls_per_task: if config.worker.llm_max_calls_per_task == 0 {
                crate::llm_gateway::DEFAULT_MAX_LLM_CALLS_PER_TASK
            } else {
                config.worker.llm_max_calls_per_task
            },
            llm_total_timeout_ms: if config.worker.llm_total_timeout_seconds == 0 {
                crate::llm_gateway::DEFAULT_MAX_LLM_TOTAL_MS_PER_TASK
            } else {
                config.worker.llm_total_timeout_seconds.saturating_mul(1000)
            },
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
            database_busy_timeout_ms: config.database.busy_timeout_ms,
            database_sqlite_path,
        },
        metrics: crate::TaskMetricsRegistry::default(),
        channels: ChannelConfig {
            telegram_bot_token,
            telegram_configured_bot_names,
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
            wechat_send_config,
            feishu_send_config,
            lark_send_config,
        },
        reload_ctx: ReloadContext {
            config_path_for_reload: config_path.clone(),
            _registry_path_for_reload: config.skills.registry_path.clone(),
            _skill_switches_for_reload: Arc::new(config.skills.skill_switches.clone()),
            _initial_skills_list_for_reload: config.skills.skills_list.clone(),
        },
        ask_states: AskStateRegistry::default(),
    };

    spawn_worker(
        state.clone(),
        config.worker.poll_interval_ms,
        config.worker.concurrency.max(1),
    );
    spawn_cleanup_worker(state.clone());
    spawn_schedule_worker(state.clone());
    http::ui_routes::spawn_nni_heartbeat_worker(state.clone());

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
        .route("/classifiers/direct", post(classify_direct))
        .route("/memory", get(get_memory_overview))
        .route("/memory/recent", get(list_memory_recent_handler))
        .route("/memory/preferences", get(list_memory_preferences_handler))
        .route("/memory/facts", get(list_memory_facts_handler))
        .route("/memory/:memory_id", delete(delete_memory_handler))
        .route("/memory/:memory_id/expire", post(expire_memory_handler))
        .route("/memory/clear", post(clear_memory_handler))
        .route("/memory/settings", post(update_memory_settings_handler))
        .route("/tasks/:task_id", get(get_task))
        .route("/tasks/active", post(list_active_tasks))
        .route("/tasks/cancel", post(cancel_tasks))
        .route("/tasks/cancel-one", post(cancel_one_task))
        .route("/tasks/cancel-by-task-id", post(cancel_task_by_id_handler))
        .route("/tasks/resume-by-task-id", post(resume_task_by_id))
        .route("/tasks/pause-by-task-id", post(pause_task_by_id))
        .route("/admin/reload-skills", post(reload_skills_handler))
        .with_state(state.clone());

    let ui_service =
        get_service(ServeDir::new(&ui_dist_dir).not_found_service(ServeFile::new(ui_index_path)))
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

    // §3.5d: prompts hot-reload via SIGHUP。
    // 行为见 [`crate::PromptsConfig`] / [`bootstrap::reload_runtime_prompts`]。
    // 仅在 unix + reload_on_sighup=true 时启用；其它 target / 显式禁用直接跳过。
    spawn_prompts_sighup_listener(state.clone(), config.prompts.clone());

    axum::serve(listener, app).await?;
    Ok(())
}

/// §3.5d: 启动后台 SIGHUP listener。该任务与 `axum::serve` 同 runtime 共存；
/// clawd 进程退出时随之终止（无须显式 join）。
///
/// - **非 unix 平台**：直接 no-op（windows / wasm 等无 SIGHUP 概念）。
/// - **`reload_on_sighup = false`**：明确不订阅 signal，让 SIGHUP 走 default
///   tokio 行为（即终止进程，与未启用本特性时一致），避免改变运维语义。
#[cfg(unix)]
fn spawn_prompts_sighup_listener(state: AppState, cfg: claw_core::config::PromptsConfig) {
    if !cfg.reload_on_sighup {
        info!("prompt_hot_reload: SIGHUP listener disabled (prompts.reload_on_sighup=false)");
        return;
    }
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut stream = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(err) => {
                warn!(
                    "prompt_hot_reload: failed to install SIGHUP listener: err={}",
                    err
                );
                return;
            }
        };
        info!(
            "prompt_hot_reload: SIGHUP listener active (config_path={}); send `kill -HUP <pid>` to swap persona/schedule prompts in-place",
            cfg.config_path
        );
        while stream.recv().await.is_some() {
            info!("prompt_hot_reload: SIGHUP received, reloading runtime prompts");
            let report = bootstrap::reload_runtime_prompts(&state, &cfg.config_path);
            info!("prompt_hot_reload: report {}", report.trace_summary());
        }
        info!("prompt_hot_reload: SIGHUP listener exiting");
    });
}

#[cfg(not(unix))]
fn spawn_prompts_sighup_listener(_state: AppState, _cfg: claw_core::config::PromptsConfig) {
    // No-op on non-unix targets.
}

async fn submit_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut req): Json<SubmitTaskRequest>,
) -> (StatusCode, Json<ApiResponse<SubmitTaskResponse>>) {
    if req.user_key.is_none() {
        req.user_key = headers
            .get("x-rustclaw-key")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
    }
    let submit_ctx = match resolve_submit_task_context(&state, &req, DEFAULT_AGENT_ID) {
        Ok(ctx) => ctx,
        Err(SubmitTaskContextError::AuthLookup(err)) => {
            error!("resolve auth key failed: {}", err);
            return api_err::<SubmitTaskResponse>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Auth lookup failed",
            );
        }
        Err(SubmitTaskContextError::InvalidUserKey) => {
            return api_err::<SubmitTaskResponse>(StatusCode::UNAUTHORIZED, "Invalid user_key");
        }
        Err(SubmitTaskContextError::UnknownAgentId(agent_id)) => {
            return api_err::<SubmitTaskResponse>(
                StatusCode::BAD_REQUEST,
                format!("unknown agent_id={agent_id}"),
            );
        }
        Err(SubmitTaskContextError::MissingChatId) => {
            return api_err::<SubmitTaskResponse>(
                StatusCode::BAD_REQUEST,
                "chat_id is required when user_key is absent",
            );
        }
    };
    let effective_user_key = submit_ctx.effective_user_key.clone();
    let effective_user_id = submit_ctx.effective_user_id;
    let channel = submit_ctx.channel;
    let effective_agent_id = submit_ctx.effective_agent_id.clone();
    let normalized_external_user_id = submit_ctx.normalized_external_user_id.clone();
    let normalized_external_chat_id = submit_ctx.normalized_external_chat_id.clone();
    let effective_chat_id = submit_ctx.effective_chat_id;

    match check_submit_task_access(&state, &submit_ctx) {
        Ok(()) => {}
        Err(SubmitTaskAccessError::MissingUserId) => {
            return api_err::<SubmitTaskResponse>(
                StatusCode::BAD_REQUEST,
                "user_id is required when user_key is absent",
            );
        }
        Err(SubmitTaskAccessError::Database(err)) => {
            error!("upsert public channel user failed: {}", err);
            return api_err::<SubmitTaskResponse>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error",
            );
        }
        Err(SubmitTaskAccessError::UnauthorizedUser) => {
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

    match check_submit_task_limits(&state, effective_user_id) {
        Ok(()) => {}
        Err(SubmitTaskLimitError::RateLimiterPoisoned) => {
            return api_err::<SubmitTaskResponse>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Rate limiter lock poisoned",
            );
        }
        Err(SubmitTaskLimitError::RateLimited(kind)) => {
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
        Err(SubmitTaskLimitError::QueueCount(err)) => {
            error!("Count queued tasks failed: {}", err);
            return api_err::<SubmitTaskResponse>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error",
            );
        }
        Err(SubmitTaskLimitError::QueueFull) => {
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
    }

    if let Some((existing_id, text)) = maybe_find_submit_task_dedup(
        &state,
        &req.kind,
        &req.payload,
        effective_user_id,
        effective_chat_id,
    ) {
        info!(
            "task_submit dedup: reused recent affirmative task_id={} user_id={} chat_id={} text={}",
            existing_id,
            effective_user_id,
            effective_chat_id,
            truncate_for_log(&text)
        );
        return api_ok(SubmitTaskResponse {
            task_id: existing_id,
        });
    }

    let task_id = Uuid::new_v4();
    let call_id = task_id.to_string();
    let kind = task_kind_name(&req.kind);
    let payload = build_submit_task_payload(
        req.payload,
        channel,
        normalized_external_user_id.as_deref(),
        normalized_external_chat_id.as_deref(),
        effective_user_key.as_deref(),
        &effective_agent_id,
        &call_id,
    );
    let payload_text = payload.to_string();

    let write_result = insert_submitted_task(
        &state,
        &task_id,
        effective_user_id,
        effective_chat_id,
        effective_user_key.as_deref(),
        channel,
        normalized_external_user_id.as_deref(),
        normalized_external_chat_id.as_deref(),
        kind,
        &payload_text,
    );

    if let Err(err) = write_result {
        error!("Insert task failed: {}", err);
        return api_err::<SubmitTaskResponse>(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
    }

    let _ = insert_audit_log(
        &state,
        Some(effective_user_id),
        "submit_task",
        Some(&submit_task_audit_detail(
            &call_id,
            &task_id,
            kind,
            effective_chat_id,
            effective_user_key.as_deref(),
        )),
        None,
    );
    info!(
        "task_submit accepted call_id={} task_id={} kind={} user_id={} chat_id={}",
        task_id, task_id, kind, effective_user_id, effective_chat_id
    );

    api_ok(SubmitTaskResponse { task_id })
}

async fn get_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(task_id): AxumPath<Uuid>,
) -> (StatusCode, Json<ApiResponse<TaskQueryResponse>>) {
    let read_result = get_task_query_record(&state, task_id);

    match read_result {
        Ok(Some((task, task_user_key, channel))) => {
            let provided_key = headers
                .get("x-rustclaw-key")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            match check_task_view_access(
                &state,
                task_user_key.as_deref(),
                &channel,
                provided_key.as_deref(),
            ) {
                Ok(()) => {}
                Err(TaskViewerAccessError::AuthLookup(err)) => {
                    error!("Resolve task viewer failed: {}", err);
                    return api_err::<TaskQueryResponse>(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Auth lookup failed",
                    );
                }
                Err(TaskViewerAccessError::TaskOwnerMismatch) => {
                    return api_err::<TaskQueryResponse>(
                        StatusCode::UNAUTHORIZED,
                        "Task owner mismatch",
                    );
                }
                Err(TaskViewerAccessError::InvalidUserKey) => {
                    return api_err::<TaskQueryResponse>(
                        StatusCode::UNAUTHORIZED,
                        "Invalid user_key",
                    );
                }
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

fn classifier_source_allowed(source: &str) -> bool {
    let normalized = source.trim().to_ascii_lowercase();
    !normalized.is_empty()
}

fn channel_kind_label(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Telegram => "telegram",
        ChannelKind::Whatsapp => "whatsapp",
        ChannelKind::Ui => "ui",
        ChannelKind::Wechat => "wechat",
        ChannelKind::Feishu => "feishu",
        ChannelKind::Lark => "lark",
    }
}

fn require_auth_identity_for_api<T: Serialize>(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthIdentity, (StatusCode, Json<ApiResponse<T>>)> {
    let Some(raw_key) = headers
        .get("x-rustclaw-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Err(api_err::<T>(
            StatusCode::UNAUTHORIZED,
            "Missing X-RustClaw-Key header",
        ));
    };
    match resolve_auth_identity_by_key(state, raw_key) {
        Ok(Some(identity)) => Ok(identity),
        Ok(None) => Err(api_err::<T>(StatusCode::UNAUTHORIZED, "Invalid user_key")),
        Err(err) => {
            error!("resolve auth identity failed: {}", err);
            Err(api_err::<T>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Auth lookup failed",
            ))
        }
    }
}

async fn get_memory_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<memory::api::MemoryOverview>>) {
    let identity =
        match require_auth_identity_for_api::<memory::api::MemoryOverview>(&state, &headers) {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("get memory overview db failed: {}", err);
            return api_err(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
        }
    };
    match memory::api::memory_overview(
        &db,
        identity.user_id,
        identity.chat_id,
        &identity.user_key,
        state.policy.memory.long_term_enabled,
        state.policy.memory.hybrid_recall_enabled,
    ) {
        Ok(overview) => api_ok(overview),
        Err(err) => {
            error!("get memory overview failed: {}", err);
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "Memory lookup failed")
        }
    }
}

async fn list_memory_preferences_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (
    StatusCode,
    Json<ApiResponse<Vec<memory::api::MemoryPreferenceItem>>>,
) {
    let identity = match require_auth_identity_for_api::<Vec<memory::api::MemoryPreferenceItem>>(
        &state, &headers,
    ) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("list memory preferences db failed: {}", err);
            return api_err(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
        }
    };
    match memory::api::list_preferences(&db, identity.user_id, identity.chat_id, &identity.user_key)
    {
        Ok(items) => api_ok(items),
        Err(err) => {
            error!("list memory preferences failed: {}", err);
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "Memory lookup failed")
        }
    }
}

async fn list_memory_facts_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (
    StatusCode,
    Json<ApiResponse<Vec<memory::api::MemoryFactItem>>>,
) {
    let identity =
        match require_auth_identity_for_api::<Vec<memory::api::MemoryFactItem>>(&state, &headers) {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("list memory facts db failed: {}", err);
            return api_err(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
        }
    };
    match memory::api::list_facts(&db, identity.user_id, &identity.user_key) {
        Ok(items) => api_ok(items),
        Err(err) => {
            error!("list memory facts failed: {}", err);
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "Memory lookup failed")
        }
    }
}

async fn list_memory_recent_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (
    StatusCode,
    Json<ApiResponse<Vec<memory::api::MemoryRecentItem>>>,
) {
    let identity =
        match require_auth_identity_for_api::<Vec<memory::api::MemoryRecentItem>>(&state, &headers)
        {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("list recent memories db failed: {}", err);
            return api_err(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
        }
    };
    match memory::api::list_recent(
        &db,
        identity.user_id,
        identity.chat_id,
        &identity.user_key,
        50,
    ) {
        Ok(items) => api_ok(items),
        Err(err) => {
            error!("list recent memories failed: {}", err);
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "Memory lookup failed")
        }
    }
}

async fn delete_memory_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(memory_id): AxumPath<String>,
) -> (
    StatusCode,
    Json<ApiResponse<memory::api::MemoryDeleteResult>>,
) {
    let identity =
        match require_auth_identity_for_api::<memory::api::MemoryDeleteResult>(&state, &headers) {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("delete memory db failed: {}", err);
            return api_err(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
        }
    };
    match memory::api::delete_memory_object(
        &db,
        identity.user_id,
        identity.chat_id,
        &identity.user_key,
        &memory_id,
        now_ts_u64() as i64,
    ) {
        Ok(Some(result)) => api_ok(result),
        Ok(None) => api_err(StatusCode::NOT_FOUND, "Memory item not found"),
        Err(err) => {
            warn!("delete memory failed: {}", err);
            api_err(StatusCode::BAD_REQUEST, "Invalid memory id")
        }
    }
}

async fn expire_memory_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(memory_id): AxumPath<String>,
) -> (
    StatusCode,
    Json<ApiResponse<memory::api::MemoryExpireResult>>,
) {
    let identity =
        match require_auth_identity_for_api::<memory::api::MemoryExpireResult>(&state, &headers) {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("expire memory db failed: {}", err);
            return api_err(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
        }
    };
    match memory::api::expire_memory_object(
        &db,
        identity.user_id,
        identity.chat_id,
        &identity.user_key,
        &memory_id,
        now_ts_u64() as i64,
    ) {
        Ok(Some(result)) => api_ok(result),
        Ok(None) => api_err(StatusCode::NOT_FOUND, "Memory item not found"),
        Err(err) => {
            warn!("expire memory failed: {}", err);
            api_err(StatusCode::BAD_REQUEST, "Invalid memory id")
        }
    }
}

async fn clear_memory_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<memory::api::MemoryClearRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<memory::api::MemoryClearResult>>,
) {
    let identity =
        match require_auth_identity_for_api::<memory::api::MemoryClearResult>(&state, &headers) {
            Ok(identity) => identity,
            Err(resp) => return resp,
        };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            error!("clear memory db failed: {}", err);
            return api_err(StatusCode::INTERNAL_SERVER_ERROR, "Database error");
        }
    };
    match memory::api::clear_memory_scope(
        &db,
        identity.user_id,
        identity.chat_id,
        &identity.user_key,
        req.scope,
        now_ts_u64() as i64,
    ) {
        Ok(result) => api_ok(result),
        Err(err) => {
            error!("clear memory failed: {}", err);
            api_err(StatusCode::INTERNAL_SERVER_ERROR, "Memory clear failed")
        }
    }
}

async fn update_memory_settings_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<memory::api::MemorySettingsRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<memory::api::MemorySettingsResult>>,
) {
    let _identity = match require_auth_identity_for_api::<memory::api::MemorySettingsResult>(
        &state, &headers,
    ) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    match memory::api::update_memory_settings_file(&state.skill_rt.workspace_root, &req) {
        Ok(result) => api_ok(result),
        Err(err) => {
            error!("update memory settings failed: {}", err);
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Memory settings update failed",
            )
        }
    }
}

async fn classify_direct(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DirectClassifyRequest>,
) -> (StatusCode, Json<ApiResponse<DirectClassifyResponse>>) {
    let identity = match require_auth_identity_for_api(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    let source = req.source.trim().to_ascii_lowercase();
    if !classifier_source_allowed(&source) {
        return api_err::<DirectClassifyResponse>(
            StatusCode::BAD_REQUEST,
            "source is required for direct classifier",
        );
    }
    let text = req.text.trim();
    if text.is_empty() {
        return api_err::<DirectClassifyResponse>(StatusCode::BAD_REQUEST, "text is required");
    }
    let channel_kind = req.channel.unwrap_or(ChannelKind::Ui);
    let task = ClaimedTask {
        task_id: format!("direct-classify-{}", Uuid::new_v4()),
        user_id: identity.user_id,
        chat_id: req.chat_id.unwrap_or(identity.chat_id),
        user_key: Some(identity.user_key.clone()),
        channel: channel_kind_label(channel_kind).to_string(),
        external_user_id: normalize_external_id_opt(req.external_user_id.as_deref()),
        external_chat_id: normalize_external_id_opt(req.external_chat_id.as_deref()),
        kind: "ask".to_string(),
        payload_json: json!({
            "text": text,
            "source": source,
            "agent_mode": false
        })
        .to_string(),
    };
    info!(
        "direct_classifier_request task_id={} source={} user_id={} chat_id={}",
        task.task_id, source, task.user_id, task.chat_id
    );
    let result = finalize::run_direct_classifier_reply(&state, &task, text).await;
    state.clear_task_llm_call_count(&task.task_id);
    match result {
        Ok(reply) => api_ok(DirectClassifyResponse {
            text: reply.text.trim().to_string(),
        }),
        Err(err) => {
            warn!(
                "direct classifier failed: task_id={} source={} err={}",
                task.task_id, source, err
            );
            api_err::<DirectClassifyResponse>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Direct classifier failed",
            )
        }
    }
}

#[derive(Debug, Serialize)]
struct ActiveTaskItem {
    index: usize,
    task_id: String,
    kind: String,
    status: String,
    summary: String,
    age_seconds: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    lifecycle: Option<serde_json::Value>,
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
        Ok(result) => api_ok(serde_json::to_value(&result).unwrap_or_default()),
        Err(e) => {
            warn!("reload_skill_views failed: {}", e);
            api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                i18n_t_with_default_vars(
                    &state,
                    "clawd.msg.reload_failed",
                    "reload failed: {err}",
                    &[("err", &e.to_string())],
                ),
            )
        }
    }
}
