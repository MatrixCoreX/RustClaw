use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Instant;

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::routing::{get, get_service, post};
use axum::{Json, Router};
use claw_core::config::AppConfig;
use claw_core::hard_rules::main_flow::load_main_flow_rules;
use claw_core::hard_rules::trade as hard_trade;
use claw_core::hard_rules::trade::CompiledTradeRules;
use claw_core::hard_rules::types::MainFlowRules;
use claw_core::types::{
    ApiResponse, HealthResponse, SubmitTaskRequest, SubmitTaskResponse, TaskQueryResponse,
    TaskStatus,
};
use reqwest::Client;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tracing::{error, info, warn};
use uuid::Uuid;

mod agent_engine;
mod ask_flow;
mod bootstrap;
mod capability_map;
mod channel_send;
mod db_init;
mod delivery_utils;
mod execution_adapters;
mod http;
mod intent_router;
mod llm_gateway;
mod log_utils;
mod memory;
mod output_paths;
mod prompt_utils;
mod providers;
mod repo;
mod routing_context;
mod runtime;
mod schedule_service;
mod skills;
mod system_health;
mod worker;

pub(crate) use ask_flow::{
    analyze_attached_images_for_ask, build_resume_continue_execute_prompt,
    build_resume_followup_discussion_prompt, execute_ask_routed,
};
use bootstrap::{
    active_prompt_vendor_name, load_command_intent_runtime, load_feishu_send_config,
    load_lark_send_config, load_memory_runtime_config, load_persona_prompt,
    load_prompt_template_for_state, load_prompt_template_for_vendor, load_schedule_runtime,
    resolve_prompt_rel_path_for_vendor, resolve_ui_dist_dir, load_wechat_send_config,
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
pub(crate) use output_paths::ensure_default_file_path;
pub(crate) use prompt_utils::{
    extract_first_json_object_any, log_prompt_render, parse_agent_action_json_with_repair,
    parse_llm_json_extract_or_any, parse_llm_json_extract_then_raw, parse_llm_json_raw_or_any,
    render_prompt_template,
};
use providers::{
    append_model_io_log, call_provider_with_retry, log_color_enabled,
    maybe_sanitize_llm_text_output, truncate_text, utf8_safe_prefix,
};
pub(crate) use repo::{
    bind_channel_identity, build_conversation_chat_id, build_submit_task_payload,
    cancel_one_task_for_user_chat, cancel_tasks_for_user_chat, check_submit_task_access,
    check_submit_task_limits, check_task_view_access, create_auth_key, delete_auth_key_by_id,
    exchange_credential_status_for_user_key, find_recent_failed_resume_context,
    get_task_query_record, insert_audit_log, insert_submitted_task, is_user_allowed,
    list_active_tasks_internal, list_auth_keys, maybe_find_submit_task_dedup, normalize_user_key,
    resolve_auth_identity_by_key, resolve_channel_binding_identity, resolve_submit_task_context,
    stable_i64_from_key, submit_task_audit_detail, task_count_by_status, task_kind_name,
    update_auth_key_by_id, update_task_timeout, upsert_exchange_credential_for_user_key,
    SubmitTaskAccessError, SubmitTaskContextError, SubmitTaskLimitError, TaskViewerAccessError,
};
use repo::{ensure_bootstrap_admin_key, ensure_key_auth_schema, seed_channel_bindings};
pub(crate) use runtime::{
    build_skill_views, llm_model_kind, llm_vendor_name, reload_skill_views, AgentAction,
    AgentRuntimeConfig, AppState, AskReply, ClaimedTask, CommandIntentRules, CommandIntentRuntime,
    LlmProviderRuntime, LocalInteractionContext, MemoryConfigFileWrapper, RateLimiter, RoutedMode,
    RuntimeChannel, ScheduleIntentOutput, ScheduleRuntime, ScheduledJobDue, SkillViewsSnapshot,
    ToolsPolicy, WhatsappDeliveryRoute,
};
pub(crate) use skills::{canonical_skill_name, is_builtin_skill_name};
use skills::{run_skill_with_runner, run_skill_with_runner_outcome};
pub(crate) use system_health::{
    channel_gateway_process_stats, current_rss_bytes, feishud_process_stats, larkd_process_stats,
    oldest_running_task_age_seconds, telegramd_process_stats, wa_webd_process_stats,
    wechatd_process_stats, whatsappd_process_stats,
};
pub(crate) use worker::task_payload_value;
use worker::{spawn_cleanup_worker, spawn_schedule_worker, spawn_worker, task_external_chat_id};

pub(crate) const INIT_SQL: &str = include_str!("../../../migrations/001_init.sql");
pub(crate) const MEMORY_UPGRADE_SQL: &str =
    include_str!("../../../migrations/002_memory_upgrade.sql");
pub(crate) const CHANNEL_UPGRADE_SQL: &str =
    include_str!("../../../migrations/003_channels_upgrade.sql");
const KEY_AUTH_UPGRADE_SQL: &str = include_str!("../../../migrations/004_key_auth.sql");
const LLM_RETRY_TIMES: usize = 2;
pub(crate) const AGENT_MAX_STEPS: usize = 32;
pub(crate) const RESUME_CONTEXT_ERROR_PREFIX: &str = "__RESUME_CTX__";
pub(crate) const MAX_READ_FILE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_WRITE_FILE_BYTES: usize = 128 * 1024;
const MODEL_IO_LOG_MAX_CHARS: usize = 16000;
const AGENT_TRACE_LOG_MAX_CHARS: usize = 4000;
const LOG_CALL_WRAP: &str = "---- task-call ----";
const DEFAULT_AGENT_ID: &str = "main";

pub(crate) const CHAT_RESPONSE_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/chat_response_prompt.md");
pub(crate) const CHAT_RESPONSE_PROMPT_PATH: &str = "prompts/chat_response_prompt.md";
pub(crate) const RESUME_CONTINUE_EXECUTE_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/resume_continue_execute_prompt.md");
pub(crate) const RESUME_FOLLOWUP_DISCUSSION_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/resume_followup_discussion_prompt.md");
pub(crate) const RESUME_FOLLOWUP_DISCUSSION_PROMPT_PATH: &str =
    "prompts/resume_followup_discussion_prompt.md";
const LONG_TERM_SUMMARY_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/vendors/default/long_term_summary_prompt.md");
const SCHEDULE_INTENT_PROMPT_TEMPLATE_DEFAULT: &str =
    include_str!("../../../prompts/vendors/default/schedule_intent_prompt.md");
const SCHEDULE_INTENT_RULES_TEMPLATE_DEFAULT: &str =
    include_str!("../../../prompts/vendors/default/schedule_intent_rules.md");

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
    let recovered_task_ids = recover_stale_running_tasks_on_startup(
        &db,
        config.worker.running_no_progress_timeout_seconds.max(1),
    )?;
    if !recovered_task_ids.is_empty() {
        let recovery_detail = json!({
            "reason": "startup_stale_running_recovery",
            "no_progress_timeout_seconds": config.worker.running_no_progress_timeout_seconds.max(1),
            "recovered_count": recovered_task_ids.len(),
            "task_ids": recovered_task_ids,
        });
        if let Err(err) = repo::insert_audit_log_raw(
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
            config.worker.running_no_progress_timeout_seconds.max(1)
        );
    } else {
        info!(
            "startup stale-running recovery: no stale running tasks found (threshold={}s)",
            config.worker.running_no_progress_timeout_seconds.max(1)
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
    axum::serve(listener, app).await?;
    Ok(())
}

fn recover_stale_running_tasks_on_startup(
    db: &Connection,
    no_progress_timeout_seconds: u64,
) -> anyhow::Result<Vec<String>> {
    let now = now_ts_u64() as i64;
    let timeout = no_progress_timeout_seconds.max(1) as i64;
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
        "auto timeout on startup: no progress heartbeat for {}s while status=running",
        no_progress_timeout_seconds.max(1)
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

fn dynamic_chat_memory_budget_chars(
    state: &AppState,
    task: &ClaimedTask,
    request_text: &str,
) -> usize {
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

pub(crate) fn i18n_t_with_default(state: &AppState, key: &str, default_text: &str) -> String {
    state
        .schedule
        .i18n_dict
        .get(key)
        .cloned()
        .unwrap_or_else(|| default_text.to_string())
}

/// Base (builtin) skills: run_cmd, read_file, write_file, list_dir, make_dir, remove_file; executed in-process. Policy uses skill:* token.
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
    let source_id = memory::read_long_term_source_memory_id(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
    )
    .map_err(|err| format!("read long-term source id failed: {err}"))?;
    let fetch_limit = state.memory.long_term_source_rounds.max(1) * 2;
    let entries = memory::recall_memories_since_id(
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

    let previous_summary = memory::recall_long_term_summary(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
    )
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

    let summary = llm_gateway::run_with_fallback_with_prompt_file(
        state,
        task,
        &summary_prompt,
        &summary_prompt_file,
    )
    .await?;
    let trimmed = truncate_text(&summary, state.memory.long_term_summary_max_chars.max(512));
    memory::upsert_long_term_summary(
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

pub(crate) fn ensure_column_exists(
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

async fn submit_task(
    State(state): State<AppState>,
    Json(req): Json<SubmitTaskRequest>,
) -> (StatusCode, Json<ApiResponse<SubmitTaskResponse>>) {
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

pub(crate) fn normalize_external_id_opt(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
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

fn authorize_task_admin_request(
    state: &AppState,
    headers: &HeaderMap,
    requested_user_id: i64,
) -> Result<i64, (StatusCode, Json<ApiResponse<serde_json::Value>>)> {
    let provided_key = headers
        .get("x-rustclaw-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if let Some(raw_key) = provided_key {
        match resolve_auth_identity_by_key(state, raw_key) {
            Ok(Some(identity)) => return Ok(identity.user_id),
            Ok(None) => {
                return Err(api_err::<serde_json::Value>(
                    StatusCode::UNAUTHORIZED,
                    "Invalid user_key",
                ));
            }
            Err(err) => {
                error!("Resolve task admin actor failed: {}", err);
                return Err(api_err::<serde_json::Value>(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Auth lookup failed",
                ));
            }
        }
    }
    if is_user_allowed(state, requested_user_id) {
        Ok(requested_user_id)
    } else {
        Err(api_err::<serde_json::Value>(
            StatusCode::FORBIDDEN,
            "Unauthorized user",
        ))
    }
}

async fn list_active_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ActiveTasksRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let effective_user_id = match authorize_task_admin_request(&state, &headers, req.user_id) {
        Ok(user_id) => user_id,
        Err(resp) => return resp,
    };
    match list_active_tasks_internal(
        &state,
        effective_user_id,
        req.chat_id,
        req.exclude_task_id.as_deref(),
    ) {
        Ok(tasks) => api_ok(json!({
            "count": tasks.len(),
            "tasks": tasks,
        })),
        Err(err) => {
            error!("List active tasks failed: {}", err);
            api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "List active tasks failed",
            )
        }
    }
}

async fn cancel_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CancelTasksRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let effective_user_id = match authorize_task_admin_request(&state, &headers, req.user_id) {
        Ok(user_id) => user_id,
        Err(resp) => return resp,
    };

    let result = cancel_tasks_for_user_chat(
        &state,
        effective_user_id,
        req.chat_id,
        req.exclude_task_id.as_deref(),
    );

    match result {
        Ok(count) => {
            info!(
                "cancel_tasks: user_id={} chat_id={} canceled={}",
                effective_user_id, req.chat_id, count
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
    headers: HeaderMap,
    Json(req): Json<CancelOneTaskRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    let effective_user_id = match authorize_task_admin_request(&state, &headers, req.user_id) {
        Ok(user_id) => user_id,
        Err(resp) => return resp,
    };
    if req.index == 0 {
        return api_err::<serde_json::Value>(StatusCode::BAD_REQUEST, "index must be >= 1");
    }
    let tasks = match list_active_tasks_internal(
        &state,
        effective_user_id,
        req.chat_id,
        req.exclude_task_id.as_deref(),
    ) {
        Ok(tasks) => tasks,
        Err(err) => {
            error!("Cancel one task list failed: {}", err);
            return api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cancel one task failed",
            );
        }
    };
    let Some(target) = tasks.into_iter().find(|t| t.index == req.index) else {
        return api_err::<serde_json::Value>(
            StatusCode::NOT_FOUND,
            format!("Active task index {} not found", req.index),
        );
    };
    let result =
        cancel_one_task_for_user_chat(&state, effective_user_id, req.chat_id, &target.task_id);
    match result {
        Ok(count) if count > 0 => api_ok(json!({
            "canceled": count,
            "task": target,
        })),
        Ok(_) => {
            api_err::<serde_json::Value>(StatusCode::NOT_FOUND, "Target task is no longer active")
        }
        Err(err) => {
            error!("Cancel one task failed: {}", err);
            api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Cancel one task failed",
            )
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
        Ok(result) => api_ok(serde_json::to_value(&result).unwrap_or_default()),
        Err(e) => {
            warn!("reload_skill_views failed: {}", e);
            api_err::<serde_json::Value>(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("reload failed: {}", e),
            )
        }
    }
}
