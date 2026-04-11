use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::routing::{get, get_service, post};
use axum::{Json, Router};
use claw_core::config::AppConfig;
use claw_core::hard_rules::types::MainFlowRules;
use claw_core::types::{
    ApiResponse, AuthIdentity, ChannelKind, DirectClassifyRequest, DirectClassifyResponse,
    HealthResponse, SubmitTaskRequest, SubmitTaskResponse, TaskQueryResponse,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Semaphore;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tracing::{error, info, warn};
use uuid::Uuid;

mod agent_engine;
mod app_helpers;
mod ask_flow;
mod bootstrap;
mod capability_map;
mod channel_send;
mod db_init;
mod delivery_utils;
mod execution_adapters;
mod executor;
mod finalizer;
mod http;
mod intent_router;
mod llm_gateway;
mod log_utils;
mod memory;
mod output_paths;
mod pipeline_types;
mod post_route_policy;
mod prompt_utils;
mod providers;
mod repo;
mod routing_context;
mod runtime;
mod schedule_service;
mod semantic_judge;
mod skills;
mod system_health;
mod task_context_builder;
mod task_journal;
mod verifier;
mod worker;

pub(crate) use app_helpers::{
    ensure_column_exists, i18n_t_with_default, is_affirmation_click_text, main_flow_rules,
    mask_secret, normalize_affirmation_text, normalize_exchange_name, normalize_external_id_opt,
    now_ts, now_ts_u64, parse_resume_context_error, parse_task_status_with_rules,
};
pub(crate) use ask_flow::{
    analyze_attached_images_for_ask, build_resume_continue_execute_prompt,
    build_resume_followup_discussion_prompt, execute_ask_routed,
};
use bootstrap::{
    active_prompt_vendor_name, load_command_intent_runtime, load_feishu_send_config,
    load_lark_send_config, load_memory_runtime_config, load_persona_prompt,
    load_prompt_template_for_state, load_prompt_template_for_vendor, load_schedule_runtime,
    load_wechat_send_config, resolve_prompt_rel_path_for_vendor, resolve_ui_dist_dir,
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
    plan_step_from_agent_action, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind,
    OutputResponseShape, PlanKind, PlanResult, PlanStep, ResumeBehavior, RiskCeiling, RouteResult,
    ScheduleKind,
};
pub(crate) use prompt_utils::{
    extract_first_json_object_any, extract_first_json_value_any, log_prompt_render,
    parse_agent_action_json_with_repair, parse_llm_json_extract_or_any,
    parse_llm_json_extract_then_raw, parse_llm_json_raw_or_any, render_prompt_template,
};
use providers::{
    append_model_io_log, call_provider_with_retry, log_color_enabled,
    maybe_sanitize_llm_text_output, truncate_text, utf8_safe_prefix,
};
pub(crate) use repo::{
    attach_pending_channel_bind_session_install_flow, bind_channel_identity,
    build_conversation_chat_id, build_submit_task_payload, cancel_one_task_for_user_chat,
    cancel_tasks_for_user_chat, check_submit_task_access, check_submit_task_limits,
    check_task_view_access, create_auth_key, create_pending_channel_bind_session,
    delete_auth_key_by_id, exchange_credential_status_for_user_key,
    finalize_pending_channel_bind_session, find_recent_failed_resume_context,
    get_auth_key_value_by_id, get_pending_channel_bind_session_by_id,
    get_pending_channel_bind_session_by_token, get_task_query_record,
    has_channel_binding_for_user_key, insert_audit_log, insert_submitted_task, is_user_allowed,
    list_active_tasks_internal, list_auth_keys, mark_pending_channel_bind_session_detected,
    mark_pending_channel_bind_session_expired, mark_pending_channel_bind_session_failed,
    maybe_find_submit_task_dedup, normalize_user_key, reset_channel_binding_state_for_user_key,
    resolve_auth_identity_by_key, resolve_channel_binding_identity, resolve_submit_task_context,
    stable_i64_from_key, submit_task_audit_detail, task_count_by_status, task_kind_name,
    update_auth_key_by_id, update_task_timeout, upsert_exchange_credential_for_user_key,
    upsert_webd_login_account, verify_webd_password_login, PendingChannelBindSession,
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
const MODEL_IO_LOG_MAX_CHARS: usize = 16000;
const AGENT_TRACE_LOG_MAX_CHARS: usize = 4000;
const LOG_CALL_WRAP: &str = "---- task-call ----";
const DEFAULT_AGENT_ID: &str = "main";

pub(crate) const CHAT_RESPONSE_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/chat_response_prompt.md");
pub(crate) const CHAT_RESPONSE_PROMPT_LOGICAL_PATH: &str = "prompts/chat_response_prompt.md";
pub(crate) const RESUME_CONTINUE_EXECUTE_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/resume_continue_execute_prompt.md");
pub(crate) const RESUME_FOLLOWUP_DISCUSSION_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/resume_followup_discussion_prompt.md");
pub(crate) const RESUME_FOLLOWUP_DISCUSSION_PROMPT_LOGICAL_PATH: &str =
    "prompts/resume_followup_discussion_prompt.md";
const LONG_TERM_SUMMARY_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/long_term_summary_prompt.md");
const SCHEDULE_INTENT_PROMPT_TEMPLATE_DEFAULT: &str =
    include_str!("../../../prompts/layers/overlays/schedule_intent_prompt.md");
const SCHEDULE_INTENT_RULES_TEMPLATE_DEFAULT: &str =
    include_str!("../../../prompts/layers/overlays/schedule_intent_rules.md");

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
    let workspace_root = std::env::current_dir()?;
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
        memory::indexing::rebuild_retrieval_index(&db, &config.memory, &workspace_root)?;
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
        llm_calls_per_task: Arc::new(Mutex::new(HashMap::new())),
        maintenance: config.maintenance.clone(),
        memory: memory_runtime,
        workspace_root,
        default_locator_search_dir,
        locator_scan_max_depth,
        locator_scan_max_files,
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
        database_sqlite_path,
        database_busy_timeout_ms: config.database.busy_timeout_ms,
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
        .route("/classifiers/direct", post(classify_direct))
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

fn classifier_source_allowed(rules: &MainFlowRules, source: &str) -> bool {
    let normalized = source.trim().to_ascii_lowercase();
    !normalized.is_empty()
        && rules
            .classifier_direct_sources
            .iter()
            .any(|v| v == &normalized)
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
    if !classifier_source_allowed(main_flow_rules(&state), &source) {
        return api_err::<DirectClassifyResponse>(
            StatusCode::BAD_REQUEST,
            "source is not enabled for direct classifier",
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
    let result = worker::run_classifier_direct_reply(&state, &task, text).await;
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
