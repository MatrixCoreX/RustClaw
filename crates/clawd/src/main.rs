use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post, put};
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
use tracing::{error, info, warn};
use uuid::Uuid;

mod agent_engine;
mod agent_hooks;
mod agent_runtime_contract;
mod answer_verifier;
mod app_helpers;
mod approval_grant;
mod ask_flow;
mod assistant_delivery_policy;
mod assistant_presentation;
mod assistant_presentation_stream;
mod async_job_contract;
mod bootstrap;
mod browser_session_service;
mod capability_map;
mod capability_resolver;
mod capability_result;
mod channel_send;
mod child_task_contract;
mod clarify_state;
mod communication_preferences;
mod contract_matrix;
mod conversation_state;
#[cfg(test)]
#[path = "http/cors_tests.rs"]
mod cors_tests;
mod db_init;
mod delivery_service;
mod delivery_utils;
mod evidence_policy;
mod execution_adapters;
mod execution_isolation;
mod execution_recipe;
mod executor;
mod fallback;
mod finalize;
#[cfg(test)]
mod fixture_replay_e2e;
mod followup_frame;
mod hook_admin_routes;
mod http;
mod intent;
mod language_policy;
mod llm_gateway;
mod local_process_job;
mod log_utils;
mod long_task_progress;
mod machine_selector;
mod mcp_admin_routes;
mod mcp_runtime;
mod media_artifact_paths;
mod memory;
mod observed_facts;
mod output_contract_verifier;
mod output_paths;
#[cfg(test)]
mod package_commands;
mod persona_style;
mod pipeline_types;
mod policy_decision;
mod process_sandbox;
mod prompt_budget;
mod prompt_utils;
mod providers;
mod read_range_utils;
mod remote_executor_admission;
mod remote_executor_contract;
mod repair_boundary_inventory;
mod repair_signal;
mod repo;
mod resource_scheduler;
mod routing_context;
mod runtime;
mod schedule_service;
mod scheduled_run_contract;
mod schema_contract;
mod semantic_judge;
mod skill_admission;
mod skill_availability;
mod skill_output_artifact;
mod skill_storage;
mod skills;
mod sqlite_busy_retry;
mod system_health;
mod task_admin_routes;
mod task_artifacts;
mod task_budget_contract;
mod task_context_builder;
mod task_contract;
mod task_event_archive;
mod task_event_transport;
mod task_execution_policy;
mod task_journal;
mod task_lifecycle;
mod task_model_selection;
mod token_estimator;
mod turn_boundary_envelope;
mod turn_context;
mod ui_attachments;
mod verifier;
mod virtual_tools;
mod visible_text;
mod whatsapp_cloud_events;
mod worker;

pub(crate) use app_helpers::{
    ensure_column_exists, i18n_t_for_language_hint_with_default_vars, i18n_t_with_default,
    i18n_t_with_default_vars, is_affirmation_click_text, main_flow_rules, mask_secret,
    normalize_affirmation_text, normalize_exchange_name, normalize_external_id_opt, now_ts,
    now_ts_u64, parse_resume_context_error, parse_task_status, TASK_STATUS_QUEUED,
};
pub(crate) use ask_flow::{analyze_attached_images_for_ask, transcribe_attached_audio_for_ask};
#[cfg(test)]
pub(crate) use bootstrap::active_prompt_vendor_name;
use bootstrap::{
    load_command_intent_runtime, load_feishu_send_config, load_lark_send_config,
    load_memory_runtime_config, load_persona_prompt, load_prompt_template_for_state,
    load_schedule_runtime, load_wechat_send_config,
};
use db_init::{
    ensure_channel_schema, ensure_memory_schema, ensure_schedule_schema, ensure_task_lease_schema,
    init_db, seed_users,
};
pub(crate) use delivery_utils::{
    collect_recent_image_candidates, extract_delivery_file_tokens,
    intercept_response_payload_for_delivery, intercept_response_text_for_delivery,
};
use hook_admin_routes::get_hook_status;
pub(crate) use log_utils::{
    append_act_plan_log, append_subtask_result, highlight_tag, truncate_for_agent_trace,
    truncate_for_log,
};
use mcp_admin_routes::{
    get_mcp_config, list_mcp_servers, list_mcp_tools, test_mcp_server, update_mcp_config,
};
pub(crate) use memory::dynamic_chat_memory_budget_chars;
pub(crate) use output_paths::ensure_default_file_path;
#[cfg(test)]
pub(crate) use pipeline_types::OutputSelectionContract;
pub(crate) use pipeline_types::{
    plan_step_from_agent_action, IntentOutputContract, MachineTokenMarkers, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape, OutputScalarCountTargetKind, PlanKind, PlanResult,
    PlanStep,
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
    build_channel_ingress_snapshot, build_conversation_chat_id, build_submit_task_payload,
    cancel_one_task_for_user_chat, cancel_task_by_id, cancel_tasks_for_user_chat,
    check_submit_task_access, check_submit_task_limits, check_task_view_access, create_auth_key,
    create_pending_channel_bind_session, delete_auth_key_by_id,
    exchange_credential_status_for_user_key, factory_reset_auth_state,
    finalize_pending_channel_bind_session, find_task_by_idempotency_key,
    finish_pending_channel_resume, get_auth_key_value_by_id,
    get_pending_channel_bind_session_by_id, get_pending_channel_bind_session_by_token,
    get_task_admin_target, get_task_query_record, has_channel_binding_for_user_key,
    hydrate_submit_task_from_ingress, insert_audit_log, insert_submitted_task, is_user_allowed,
    list_active_tasks_for_user_internal, list_active_tasks_internal,
    list_all_active_tasks_internal, list_auth_keys, mark_pending_channel_bind_session_detected,
    mark_pending_channel_bind_session_expired, mark_pending_channel_bind_session_failed,
    maybe_find_submit_task_dedup, normalize_user_key, pending_channel_resume_candidate,
    request_attachment_paths, reset_channel_binding_state_for_user_key,
    resolve_auth_identity_by_key, resolve_channel_binding_identity, resolve_submit_task_context,
    stable_i64_from_key, store_pending_channel_request, submit_task_audit_detail,
    task_count_by_status, task_count_by_status_for_user, task_kind_name, update_auth_key_by_id,
    upsert_exchange_credential_for_user_key, upsert_webd_login_account, verify_webd_password_login,
    FactoryResetDbResult, PendingChannelBindSession, SubmitTaskAccessError, SubmitTaskContextError,
    SubmitTaskLimitError, TaskAdminTarget, TaskViewerAccessError,
};
use repo::{ensure_bootstrap_admin_key, ensure_key_auth_schema, seed_channel_bindings};
#[cfg(test)]
pub(crate) use runtime::AgentRuntimeConfig;
pub(crate) use runtime::{
    assemble_skill_views_snapshot, build_skill_views_with_overlay, llm_model_kind, llm_vendor_name,
    load_skill_admission_snapshot, log_ask_transition, reload_skill_views, AgentAction, AppState,
    AskReply, AskState, AskStateRegistry, AskTransition, ChannelConfig, ClaimedTask,
    CommandIntentRuntime, CoreServices, LlmCallSequenceEntry, LlmPromptBucket, LlmProviderRuntime,
    LocalInteractionContext, MemoryConfigFileWrapper, PolicyConfig, RateLimiter, ReloadContext,
    RuntimeChannel, ScheduleIntentOutput, ScheduleRuntime, ScheduledJobDue, SkillRuntime,
    SkillViewsSnapshot, TaskCostBlocker, TaskMetricsRegistry, TaskProviderBlocker, ToolsPolicy,
    WhatsappDeliveryRoute, WorkerConfig,
};
pub(crate) use skills::canonical_skill_name;
use skills::{run_skill_with_runner, run_skill_with_runner_outcome};
pub(crate) use system_health::{
    active_running_task_count, active_running_task_count_for_user, channel_gateway_process_stats,
    current_rss_bytes, daemon_process_pids_by_name, feishud_process_stats, larkd_process_stats,
    oldest_running_task_age_seconds, oldest_running_task_age_seconds_for_user,
    telegramd_process_stats, wa_webd_process_stats, webd_process_stats, wechatd_process_stats,
    whatsappd_process_stats,
};
use task_admin_routes::{
    cancel_one_task, cancel_task_by_id as cancel_task_by_id_handler, cancel_tasks,
    close_child_task_by_id, goal_by_task_id, list_active_tasks, list_approval_scope_grants,
    list_automation_runs, pause_task_by_id, resume_task_by_id, retry_child_task_by_id,
    revoke_approval_scope_grant, steer_task_by_id, stop_child_tasks_by_parent,
};
pub(crate) use worker::task_payload_value;
use worker::{
    adopt_recoverable_resume_executions_on_startup, recover_stale_running_tasks_on_startup,
    spawn_channel_terminal_delivery_worker, spawn_cleanup_worker, spawn_schedule_worker,
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
pub(crate) const AGENT_MAX_ACTIONS_PER_TURN: usize = 32;
pub(crate) const RESUME_CONTEXT_ERROR_PREFIX: &str = "__RESUME_CTX__";
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
const ISOLATION_STARTUP_CLEANUP_MIN_SECONDS: u64 = 6 * 60 * 60;
const DEFAULT_TOKIO_WORKER_STACK_BYTES: usize = 4 * 1024 * 1024;
const MIN_TOKIO_WORKER_STACK_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOKIO_WORKER_STACK_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_AGENT_ID: &str = "main";

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

pub(crate) fn auth_key_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(claw_core::product_identity::AUTH_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
}

fn api_cors_layer() -> CorsLayer {
    let allowed_headers = vec![
        axum::http::header::CONTENT_TYPE,
        axum::http::header::IF_NONE_MATCH,
        axum::http::HeaderName::from_static("last-event-id"),
        axum::http::HeaderName::from_static(claw_core::product_identity::AUTH_KEY_HEADER),
        axum::http::HeaderName::from_static(claw_core::product_identity::CLIENT_ORIGIN_HEADER),
    ];
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers(allowed_headers)
        .expose_headers([axum::http::header::ETAG])
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
        claw_core::product_identity::env_string("CONFIG_PATH").ok(),
    )
}

fn resolve_offline_bundled_repair_skill_from<I>(args: I) -> anyhow::Result<Option<String>>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let mut skill_name = None;
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--repair-bundled-skill=") {
            let value = value.trim();
            if value.is_empty() {
                anyhow::bail!("--repair-bundled-skill requires a non-empty skill name");
            }
            skill_name = Some(value.to_string());
            continue;
        }
        if arg == "--repair-bundled-skill" {
            let Some(value) = args.next() else {
                anyhow::bail!("--repair-bundled-skill requires a skill name");
            };
            let value = value.trim();
            if value.is_empty() {
                anyhow::bail!("--repair-bundled-skill requires a non-empty skill name");
            }
            skill_name = Some(value.to_string());
        }
    }
    Ok(skill_name)
}

fn resolve_offline_bundled_repair_skill() -> anyhow::Result<Option<String>> {
    resolve_offline_bundled_repair_skill_from(std::env::args().skip(1))
}

#[cfg(test)]
#[path = "main_startup_config_path_tests.rs"]
mod startup_config_path_tests;

fn startup_isolation_cleanup_age_seconds(running_no_progress_timeout_seconds: u64) -> u64 {
    running_no_progress_timeout_seconds
        .saturating_mul(4)
        .max(ISOLATION_STARTUP_CLEANUP_MIN_SECONDS)
}

fn tokio_worker_stack_bytes(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(MIN_TOKIO_WORKER_STACK_BYTES, MAX_TOKIO_WORKER_STACK_BYTES))
        .unwrap_or(DEFAULT_TOKIO_WORKER_STACK_BYTES)
}

fn main() -> anyhow::Result<()> {
    let worker_stack_bytes = tokio_worker_stack_bytes(
        claw_core::product_identity::env_string("TOKIO_WORKER_STACK_BYTES")
            .ok()
            .as_deref(),
    );
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(worker_stack_bytes)
        .build()?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
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
    let credential_store_path =
        claw_core::git_remote_config::git_credential_store_path(&workspace_root);
    claw_core::secrets::install_global(Arc::new(claw_core::secrets::EnvFileSecretsBroker::new(
        credential_store_path,
    )))
    .map_err(|_| anyhow::anyhow!("credential_broker_already_installed"))?;
    info!("startup config_path={}", config_path);
    if let Some(skill_name) = resolve_offline_bundled_repair_skill()? {
        let snapshot = http::ui_routes::repair_bundled_skill_admission_offline(
            &workspace_root,
            &config,
            &skill_name,
        )
        .map_err(anyhow::Error::msg)?;
        println!(
            "{}",
            json!({
                "command": "repair-bundled-skill",
                "skill_name": skill_name,
                "registry_generation": snapshot.generation,
                "registry_generation_digest": snapshot.generation_digest,
                "status": "ok"
            })
        );
        return Ok(());
    }
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
        repo::ensure_channel_delivery_receipt_schema(&db)?;
        repo::ensure_channel_delivery_outbox_schema(&db)?;
        ensure_task_lease_schema(&db)?;
        ensure_key_auth_schema(&db)?;
        repo::child_task_graph::ensure_child_task_graph_schema(&db)?;
        repo::task_plan::ensure_task_plan_schema(&db)?;
        memory::indexing::ensure_retrieval_schema(&db)?;
        repo::ensure_principal_ownership_schema(&db)?;
        memory::scope::ensure_memory_scope_schema(&db)?;
        memory::jobs::ensure_memory_job_schema(&db)?;
        memory::ux::ensure_memory_ux_schema(&db)?;
        memory::embedding_jobs::initialize_embedding_runtime(&db, &config.memory)?;
        task_context_builder::context_compaction_lifecycle::ensure_context_compaction_lifecycle_schema(&db)?;
        if config.memory.hybrid_recall_enabled
            && (config.memory.reindex_on_startup
                || memory::indexing::retrieval_index_is_empty(&db).unwrap_or(true))
        {
            memory::indexing::rebuild_retrieval_index(&db, &config.memory)?;
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
    let skill_storage = Arc::new(skill_storage::SkillStorageRuntime::initialize(
        &workspace_root,
        &config.database,
        &db_pool,
    )?);
    if let Some(user_key) = bootstrap_admin_key.as_deref() {
        warn!("============================================================");
        warn!("No auth key found in database. Generated initial admin key.");
        warn!("Initial admin key: {}", user_key);
        warn!("Default web login: username=admin password=123456");
        warn!("Please save it now and use it to bind UI / Telegram / WhatsApp.");
        warn!("============================================================");
        eprintln!("============================================================");
        eprintln!("Initial admin key: {}", user_key);
        eprintln!("Default web login: username=admin password=123456");
        eprintln!("Please save it now and use it to bind UI / Telegram / WhatsApp.");
        eprintln!("============================================================");
    }
    let worker_id = format!("worker:{}", Uuid::new_v4());
    let (recovered_task_ids, adopted_resume_task_ids) = {
        let db = db_pool
            .get()
            .map_err(|e| anyhow::anyhow!("get db conn: {e}"))?;
        let recovered = recover_stale_running_tasks_on_startup(
            &db,
            config.worker.running_no_progress_timeout_seconds.max(1),
        )?;
        let resume_lease_seconds = config
            .worker
            .task_heartbeat_seconds
            .max(5)
            .saturating_mul(4)
            .max(300) as i64;
        let adopted =
            adopt_recoverable_resume_executions_on_startup(&db, &worker_id, resume_lease_seconds)?;
        repo::child_task_graph::reconcile_child_task_graphs_after_restart(&db, &now_ts())?;
        (recovered, adopted)
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
    if !adopted_resume_task_ids.is_empty() {
        info!(
            "startup durable resume adoption: transferred {} task(s) to the new worker generation",
            adopted_resume_task_ids.len()
        );
    }
    let isolation_cleanup_age_seconds =
        startup_isolation_cleanup_age_seconds(config.worker.running_no_progress_timeout_seconds);
    let protected_isolation_task_keys = {
        let db = db_pool
            .get()
            .map_err(|error| anyhow::anyhow!("get db conn: {error}"))?;
        let mut stmt = db.prepare(
            "SELECT task_id FROM tasks
             WHERE status IN ('queued', 'running')
                OR COALESCE(result_json, '') LIKE '%pinned_until_explicit_apply_or_discard%'",
        )?;
        let task_keys = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect::<HashSet<_>>();
        task_keys
    };
    let isolation_cleanup = execution_isolation::cleanup_abandoned_isolation_workspaces_protected(
        &workspace_root,
        now_ts_u64(),
        isolation_cleanup_age_seconds,
        &protected_isolation_task_keys,
    );
    if isolation_cleanup.removed > 0
        || isolation_cleanup.artifacts_removed > 0
        || !isolation_cleanup.errors.is_empty()
    {
        info!(
            "startup isolation cleanup removed={} artifacts_removed={} skipped={} errors={} older_than_seconds={}",
            isolation_cleanup.removed,
            isolation_cleanup.artifacts_removed,
            isolation_cleanup.skipped,
            isolation_cleanup.errors.len(),
            isolation_cleanup_age_seconds
        );
        if !isolation_cleanup.errors.is_empty() {
            warn!(
                "startup isolation cleanup errors={}",
                crate::truncate_for_log(&json!(isolation_cleanup.errors).to_string())
            );
        }
    } else {
        info!(
            "startup isolation cleanup: no abandoned isolation workspaces found (older_than_seconds={})",
            isolation_cleanup_age_seconds
        );
    }

    let memory_runtime = load_memory_runtime_config(&workspace_root, &config.memory);
    let command_intent = load_command_intent_runtime(&config.command_intent);
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
    let effective_skill_runner_path = bootstrap::resolve_skill_runner_path(&workspace_root);
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
            "Active provider: name={}, type={}, model={}, timeout_seconds={}",
            p.config.name, p.config.provider_type, p.config.model, p.config.timeout_seconds
        );
    }
    info!(
        "run_cmd config: timeout_seconds={}, idle_timeout_seconds={}, async_runtime_deadline_default=none, async_retention_seconds={}, terminate_grace_seconds={}, max_output_bytes={}, max_cmd_length={}, allow_outside_workspace={}, allow_sudo={}",
        config.tools.cmd_timeout_seconds.max(1),
        config.tools.cmd_idle_timeout_seconds.max(1),
        config.tools.cmd_async_retention_seconds.max(1),
        config.tools.cmd_terminate_grace_seconds.max(1),
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
    let agents_by_id = runtime::provider_runtime::build_agent_runtime_snapshot(&config);
    let agent_runtime_leases = agents_by_id
        .values()
        .map(|agent| (agent.runtime_digest.clone(), agent.clone()))
        .collect();

    let telegram_runtime_bots = config.telegram_runtime_bots();
    let telegram_bot_token = telegram_runtime_bots
        .iter()
        .map(|bot| bot.bot_token.trim())
        .find(|token| !token.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| config.telegram.bot_token.clone());
    let telegram_bot_tokens = Arc::new(
        telegram_runtime_bots
            .iter()
            .filter_map(|bot| {
                let token = bot.bot_token.trim();
                (!token.is_empty()).then(|| (bot.name.clone(), token.to_string()))
            })
            .collect::<HashMap<_, _>>(),
    );
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
    let whatsapp_app_secret = if config.whatsapp_cloud.app_secret.trim().is_empty() {
        config.whatsapp.app_secret.clone()
    } else {
        config.whatsapp_cloud.app_secret.clone()
    };
    let whatsapp_phone_number_id = if config.whatsapp_cloud.phone_number_id.trim().is_empty() {
        config.whatsapp.phone_number_id.clone()
    } else {
        config.whatsapp_cloud.phone_number_id.clone()
    };
    let (whatsapp_out_of_window_template_name, whatsapp_out_of_window_template_language) = if config
        .whatsapp_cloud
        .out_of_window_template_name
        .trim()
        .is_empty()
    {
        (
            config.whatsapp.out_of_window_template_name.clone(),
            config.whatsapp.out_of_window_template_language.clone(),
        )
    } else {
        (
            config.whatsapp_cloud.out_of_window_template_name.clone(),
            config
                .whatsapp_cloud
                .out_of_window_template_language
                .clone(),
        )
    };

    // Phase 4: 统一 skill 视图重建（启动与 reload 复用）
    let admission_overlay =
        load_skill_admission_snapshot(&workspace_root, &config).map_err(|e| {
            error!("startup: skill admission overlay failed: {}", e);
            anyhow::anyhow!(e)
        })?;
    let views = build_skill_views_with_overlay(
        &workspace_root,
        config.skills.registry_path.as_deref(),
        &config.skills.skill_switches,
        &config.skills.uninstalled_skills,
        Some(&admission_overlay),
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

    // §P4.1 收尾：registry 必须覆盖所有 HOST_TOOL_DESCRIPTORS（且 kind=builtin），
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
    info!(
        "routing default_locator_search_dir={}",
        default_locator_search_dir.display(),
    );

    let mcp_runtime = Arc::new(crate::mcp_runtime::McpRuntime::new(config.mcp.clone()));
    mcp_runtime.start().await;
    mcp_runtime.spawn_health_monitor().await;
    for snapshot in mcp_runtime.lifecycle_snapshots() {
        info!(
            server_id = snapshot.server_id,
            state = snapshot.state.as_token(),
            transport = snapshot.transport,
            tool_count = snapshot.tool_count,
            error_code = snapshot.last_error_code.unwrap_or_default(),
            "mcp_server_lifecycle"
        );
    }
    let initial_skill_views: SkillViewsSnapshot =
        assemble_skill_views_snapshot(views, &admission_overlay);

    let state = AppState {
        core: crate::CoreServices {
            db: db_pool,
            audit_db: audit_db_pool,
            skill_storage,
            llm_providers,
            agents_by_id: Arc::new(RwLock::new(Arc::new(agents_by_id))),
            agent_runtime_leases: Arc::new(RwLock::new(agent_runtime_leases)),
            http_client: Client::new(),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(initial_skill_views))),
            active_provider_type,
            mcp_runtime,
            browser_sessions: crate::browser_session_service::BrowserSessionService::new(
                &workspace_root,
            ),
        },
        skill_rt: crate::SkillRuntime {
            skill_timeout_seconds: config.skills.skill_timeout_seconds,
            skill_runner_path: effective_skill_runner_path,
            skill_global_max_concurrency: config.skills.skill_max_concurrency.max(1),
            skill_semaphore: Arc::new(Semaphore::new(config.skills.skill_max_concurrency.max(1))),
            skill_concurrency_gates: Arc::new(
                crate::runtime::state::SkillConcurrencyGates::default(),
            ),
            runner_pool: Arc::new(crate::skills::runner_pool::WarmRunnerPool::new(
                config.skills.runner_warm_pool_enabled,
                config.skills.runner_warm_pool_max_idle_per_skill,
                config.skills.runner_warm_pool_min_available_memory_mib,
                config.skills.runner_warm_pool_idle_timeout_seconds,
            )),
            tools_policy: Arc::new(tools_policy),
            cmd_timeout_seconds: config.tools.cmd_timeout_seconds.max(1),
            cmd_idle_timeout_seconds: config.tools.cmd_idle_timeout_seconds.max(1),
            cmd_async_retention_seconds: config.tools.cmd_async_retention_seconds.max(1),
            cmd_terminate_grace_seconds: config.tools.cmd_terminate_grace_seconds.max(1),
            cmd_max_output_bytes: config.tools.cmd_max_output_bytes.max(128),
            max_cmd_length: config.tools.max_cmd_length.max(16),
            workspace_root,
            default_locator_search_dir,
        },
        policy: crate::PolicyConfig {
            maintenance: config.maintenance.clone(),
            memory: memory_runtime,
            routing,
            limits: config.limits.clone(),
            llm_cost_governance: config.llm.cost_governance.clone(),
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
            worker_id,
            started_at: Instant::now(),
            queue_limit: config.worker.queue_limit,
            remote_executor: config.worker.remote_executor.clone(),
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
            active_running_task_ids: Arc::new(Mutex::new(HashSet::new())),
            task_cancellation_tokens: Arc::new(Mutex::new(HashMap::new())),
        },
        metrics: crate::TaskMetricsRegistry::default(),
        channels: ChannelConfig {
            telegram_bot_token,
            telegram_bot_tokens,
            telegram_configured_bot_names,
            whatsapp_cloud_enabled,
            whatsapp_api_base,
            whatsapp_access_token,
            whatsapp_app_secret,
            whatsapp_phone_number_id,
            whatsapp_out_of_window_template_name,
            whatsapp_out_of_window_template_language,
            whatsapp_web_enabled: config.whatsapp_web.enabled,
            whatsapp_web_bridge_base_url: config.whatsapp_web.bridge_base_url.clone(),
            whatsapp_web_allow_proactive_send: config.whatsapp_web.allow_proactive_send,
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
            workspace_instructions: config.workspace_instructions.clone(),
            auto_review: config.auto_review.clone(),
        },
        ask_states: AskStateRegistry::default(),
    };

    match state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!(error))
        .and_then(|db| {
            communication_preferences::migrate_legacy_telegram_voice_preferences(
                &db,
                &config.telegram.voice_reply_mode_by_chat,
            )
        }) {
        Ok(report) if report.discovered > 0 => info!(
            discovered = report.discovered,
            migrated = report.migrated,
            already_current = report.already_current,
            binding_missing = report.binding_missing,
            invalid = report.invalid,
            "legacy_communication_preference_migration_completed"
        ),
        Ok(_) => {}
        Err(error) => {
            warn!(error = %error, "legacy_communication_preference_migration_failed")
        }
    }

    let recovered_cancel_escalations = crate::local_process_job::recover_pending_cancel_escalations(
        &state.skill_rt.workspace_root,
        crate::now_ts_u64() as i64,
    );
    if recovered_cancel_escalations > 0 {
        info!(
            recovered_cancel_escalations,
            "restored pending local process cancellation escalation"
        );
    }
    spawn_worker(
        state.clone(),
        config.worker.poll_interval_ms,
        config.worker.concurrency.max(1),
    );
    match memory::jobs::reconcile_missing_turn_jobs(&state) {
        Ok(repaired) if repaired > 0 => {
            info!(
                repaired,
                "memory_durable_job_outbox_reconciliation_completed"
            )
        }
        Ok(_) => {}
        Err(error) => warn!(error = %error, "memory_durable_job_outbox_reconciliation_failed"),
    }
    memory::jobs::spawn_memory_job_workers(state.clone(), config.memory.background_job_concurrency);
    memory::embedding_jobs::spawn_embedding_workers(
        state.clone(),
        config.memory.background_job_concurrency,
    );
    spawn_cleanup_worker(state.clone());
    spawn_schedule_worker(state.clone());
    spawn_channel_terminal_delivery_worker(state.clone());
    http::ui_routes::spawn_nni_heartbeat_worker(state.clone());

    let api = Router::new()
        .merge(http::ui_routes::build_ui_router())
        .route("/tasks", post(submit_task))
        .route(
            "/tasks/:task_id/events",
            get(http::task_events::stream_task_events),
        )
        .route(
            "/tasks/:task_id/events/artifacts/:artifact_id",
            get(http::task_events::get_task_event_artifact),
        )
        .route(
            "/tasks/:task_id/artifacts",
            get(http::task_artifacts::list_task_artifacts),
        )
        .route(
            "/tasks/:task_id/artifacts/:artifact_id/content",
            get(http::task_artifacts::get_task_artifact_content)
                .head(http::task_artifacts::head_task_artifact_content),
        )
        .route("/classifiers/direct", post(classify_direct))
        .merge(http::memory_routes::router())
        .route(
            "/channel/preferences",
            get(communication_preferences::get_handler)
                .put(communication_preferences::update_handler),
        )
        .route(
            "/tasks/conversation-history",
            get(http::conversation_history::list_conversation_history),
        )
        .route(
            "/ui/attachment-constraints",
            get(http::ui_attachment_constraints::get_ui_attachment_constraints),
        )
        .route(
            "/tasks/:task_id/conversation-body/:field",
            get(http::conversation_history::get_conversation_body_range),
        )
        .route(
            "/tasks/conversations/:conversation_id/title",
            put(http::conversation_history::update_conversation_title),
        )
        .route(
            "/tasks/conversations/:conversation_id",
            delete(http::conversation_history::archive_conversation),
        )
        .route("/tasks/:task_id", get(get_task))
        .route(
            "/tasks/:task_id/delivery",
            post(http::task_delivery::deliver_task_result),
        )
        .route("/tasks/active", post(list_active_tasks))
        .route("/tasks/automation-runs", post(list_automation_runs))
        .route("/tasks/cancel", post(cancel_tasks))
        .route("/tasks/cancel-one", post(cancel_one_task))
        .route("/tasks/cancel-by-task-id", post(cancel_task_by_id_handler))
        .route("/tasks/resume-by-task-id", post(resume_task_by_id))
        .route("/tasks/approval-grants", get(list_approval_scope_grants))
        .route(
            "/tasks/approval-grants/revoke",
            post(revoke_approval_scope_grant),
        )
        .route("/tasks/pause-by-task-id", post(pause_task_by_id))
        .route("/tasks/steer-by-task-id", post(steer_task_by_id))
        .route(
            "/tasks/retry-child-by-task-id",
            post(retry_child_task_by_id),
        )
        .route(
            "/tasks/stop-child-tasks-by-parent",
            post(stop_child_tasks_by_parent),
        )
        .route(
            "/tasks/close-child-by-task-id",
            post(close_child_task_by_id),
        )
        .route("/tasks/goal-by-task-id", post(goal_by_task_id))
        .route("/admin/reload-skills", post(reload_skills_handler))
        .route("/admin/hooks/status", get(get_hook_status))
        .route(
            "/admin/mcp/config",
            get(get_mcp_config).post(update_mcp_config),
        )
        .route("/admin/mcp/servers", get(list_mcp_servers))
        .route("/admin/mcp/tools", get(list_mcp_tools))
        .route("/admin/mcp/servers/:server_id/test", post(test_mcp_server))
        .route(
            "/internal/channel-events/whatsapp-cloud",
            post(whatsapp_cloud_events::handle_whatsapp_cloud_events),
        )
        .route(
            "/internal/channel-events/whatsapp-cloud/accepted",
            post(whatsapp_cloud_events::handle_whatsapp_cloud_accepted),
        )
        .with_state(state.clone());

    let app = Router::new().nest("/v1", api).layer(api_cors_layer());

    let clawd_listen = clawd_internal_listen()?;
    let listener = tokio::net::TcpListener::bind(&clawd_listen).await?;
    info!("clawd internal listener bound to {}", clawd_listen);

    // §3.5d: prompts hot-reload via SIGHUP。
    // 行为见 [`crate::PromptsConfig`] / [`bootstrap::reload_runtime_prompts`]。
    // 仅在 unix + reload_on_sighup=true 时启用；其它 target / 显式禁用直接跳过。
    spawn_prompts_sighup_listener(state.clone(), config.prompts.clone());

    let serve_result = axum::serve(listener, app).await;
    state.core.mcp_runtime.stop().await;
    serve_result?;
    Ok(())
}

fn clawd_internal_listen() -> anyhow::Result<String> {
    let listen = claw_core::product_identity::env_string("INTERNAL_LISTEN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| claw_core::config::CLAWD_INTERNAL_LISTEN.to_string());
    validate_clawd_internal_listen(&listen)
}

fn validate_clawd_internal_listen(listen: &str) -> anyhow::Result<String> {
    let address = listen
        .parse::<std::net::SocketAddr>()
        .map_err(|_| anyhow::anyhow!("APP_INTERNAL_LISTEN must be a loopback socket address"))?;
    if !address.ip().is_loopback() {
        anyhow::bail!("APP_INTERNAL_LISTEN must use a loopback address");
    }
    Ok(address.to_string())
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

#[cfg(test)]
#[path = "internal_listener_tests.rs"]
mod internal_listener_tests;

async fn submit_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut req): Json<SubmitTaskRequest>,
) -> (StatusCode, Json<ApiResponse<SubmitTaskResponse>>) {
    if worker::conversation_compaction::is_conversation_compaction_payload(&req.payload) {
        if !matches!(req.kind, claw_core::types::TaskKind::Ask) {
            return api_err::<SubmitTaskResponse>(
                StatusCode::BAD_REQUEST,
                "conversation_compaction_task_kind_invalid",
            );
        }
        if let Err(error) =
            worker::conversation_compaction::validate_conversation_compaction_payload(&req.payload)
        {
            return api_err::<SubmitTaskResponse>(StatusCode::BAD_REQUEST, error);
        }
    }
    if worker::run_capability::is_direct_capability_payload(&req.payload) {
        if !matches!(req.kind, claw_core::types::TaskKind::Ask) {
            return api_err::<SubmitTaskResponse>(
                StatusCode::BAD_REQUEST,
                "run_capability_task_kind_invalid",
            );
        }
        if let Err(error) = worker::run_capability::parse_direct_capability_request(&req.payload) {
            return api_err::<SubmitTaskResponse>(StatusCode::BAD_REQUEST, error.to_string());
        }
    }
    if req.user_key.is_none() {
        req.user_key = auth_key_from_headers(&headers)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
    }
    if let Err(error) = hydrate_submit_task_from_ingress(&mut req) {
        return api_err::<SubmitTaskResponse>(StatusCode::BAD_REQUEST, error);
    }
    if let Some(idempotency_key) = req.idempotency_key.take() {
        let idempotency_key = idempotency_key.trim();
        if idempotency_key.chars().count() > 512 {
            return api_err::<SubmitTaskResponse>(
                StatusCode::BAD_REQUEST,
                "idempotency_key_too_long",
            );
        }
        if !idempotency_key.is_empty() {
            req.idempotency_key = Some(idempotency_key.to_string());
        }
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
            return api_err::<SubmitTaskResponse>(StatusCode::UNAUTHORIZED, "auth_key_invalid");
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
    let client_origin = task_execution_policy::client_origin_from_headers(&headers);
    let requested_execution_mode = task_execution_policy::execution_mode_from_headers(&headers);
    if let Err(error) = task_execution_policy::stamp_authenticated_submission_policy(
        &mut req.payload,
        submit_ctx.resolved_identity.as_ref(),
        client_origin,
        requested_execution_mode,
    ) {
        return api_err::<SubmitTaskResponse>(error.status_code(), error.as_token());
    }
    if let Err(error) =
        task_model_selection::validate_and_stamp_task_model_selection(&state, &mut req.payload)
    {
        return api_err::<SubmitTaskResponse>(StatusCode::BAD_REQUEST, error);
    }

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
    match find_task_by_idempotency_key(
        &state,
        req.idempotency_key.as_deref(),
        effective_user_id,
        channel,
    ) {
        Ok(Some(existing_id)) => {
            info!(
                "task_submit idempotency_reuse task_id={} user_id={} chat_id={}",
                existing_id, effective_user_id, effective_chat_id
            );
            return api_ok(SubmitTaskResponse {
                task_id: existing_id,
            });
        }
        Ok(None) => {}
        Err(error) => {
            error!("task idempotency lookup failed: {error}");
            return api_err::<SubmitTaskResponse>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_idempotency_lookup_failed",
            );
        }
    }

    let task_id = Uuid::new_v4();
    let call_id = task_id.to_string();
    if let Err(err) = ui_attachments::materialize_ui_task_attachments(
        &state,
        &mut req.payload,
        effective_user_id,
        effective_chat_id,
        &call_id,
    ) {
        warn!(
            "ui attachment materialize failed call_id={} user_id={} chat_id={} err={}",
            call_id, effective_user_id, effective_chat_id, err
        );
        return api_err::<SubmitTaskResponse>(StatusCode::BAD_REQUEST, err);
    }
    let kind = task_kind_name(&req.kind);
    let mut ingress = build_channel_ingress_snapshot(
        req.ingress.as_ref(),
        channel,
        effective_user_id,
        effective_chat_id,
        normalized_external_user_id.as_deref(),
        normalized_external_chat_id.as_deref(),
        &req.payload,
    );
    let platform_locale = ingress
        .platform_locale
        .as_deref()
        .or(ingress.locale.as_deref());
    match state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!(error))
        .and_then(|db| {
            communication_preferences::resolve_locale(
                &db,
                effective_user_id,
                effective_chat_id,
                effective_user_key.as_deref(),
                platform_locale,
                &state.policy.command_intent.default_locale,
            )
        }) {
        Ok(resolved) => {
            ingress.locale = Some(resolved.locale);
            ingress.locale_source = Some(resolved.source.to_string());
        }
        Err(error) => {
            warn!(error = %error, "channel_locale_resolution_failed");
            ingress.locale = communication_preferences::normalize_locale(
                &state.policy.command_intent.default_locale,
            )
            .or_else(|| Some("en-US".to_string()));
            ingress.locale_source = Some("safe_default".to_string());
        }
    }
    let whatsapp_cloud_inbound = (channel == ChannelKind::Whatsapp
        && ingress.adapter == "whatsapp_cloud")
        .then(|| {
            Some((
                ingress
                    .external_user_id
                    .as_deref()
                    .or(ingress.external_chat_id.as_deref())?
                    .to_string(),
                ingress.received_at_ts?,
            ))
        })
        .flatten();
    let message_id = ingress.message_id.clone();
    let payload = build_submit_task_payload(
        req.payload,
        ingress,
        channel,
        normalized_external_user_id.as_deref(),
        normalized_external_chat_id.as_deref(),
        effective_user_key.as_deref(),
        &effective_agent_id,
        state.agent_task_snapshot(&effective_agent_id),
        &call_id,
    );
    let payload_text = payload.to_string();
    let execution_mode = task_execution_policy::stamped_execution_mode(&payload);

    let write_result = insert_submitted_task(
        &state,
        &task_id,
        effective_user_id,
        effective_chat_id,
        effective_user_key.as_deref(),
        channel,
        normalized_external_user_id.as_deref(),
        normalized_external_chat_id.as_deref(),
        message_id.as_deref(),
        req.idempotency_key.as_deref(),
        kind,
        &payload_text,
    );

    let (persisted_task_id, inserted) = match write_result {
        Ok(result) => result,
        Err(err) => {
            error!("Insert task failed: {}", err);
            return api_err::<SubmitTaskResponse>(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database error",
            );
        }
    };
    if !inserted {
        return api_ok(SubmitTaskResponse {
            task_id: persisted_task_id,
        });
    }
    if let Some((external_user_id, received_at_ts)) = whatsapp_cloud_inbound {
        if let Err(err) = repo::record_whatsapp_cloud_inbound(
            &state.core.db,
            &state.channels.whatsapp_phone_number_id,
            &external_user_id,
            received_at_ts,
        ) {
            warn!(
                event = "whatsapp_cloud_window_record_failed",
                task_id = %task_id,
                diagnostic = %err,
                "whatsapp_cloud_inbound_window_record_failed"
            );
        }
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
            execution_mode,
        )),
        None,
    );
    info!(
        "task_submit accepted call_id={} task_id={} kind={} user_id={} chat_id={} execution_mode={}",
        task_id, task_id, kind, effective_user_id, effective_chat_id, execution_mode
    );
    if let Err(err) = task_event_transport::publish_event(
        &state,
        &call_id,
        "task_submitted",
        json!({
            "kind": kind,
            "channel": channel,
            "task_status": "queued",
            "execution_mode": execution_mode,
        }),
    ) {
        warn!(
            "task submit event publish failed task_id={} error={}",
            task_id,
            truncate_for_log(&err.to_string())
        );
    }

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
            let provided_key = auth_key_from_headers(&headers).map(str::to_string);
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
                        "task_owner_mismatch",
                    );
                }
                Err(TaskViewerAccessError::InvalidUserKey) => {
                    return api_err::<TaskQueryResponse>(
                        StatusCode::UNAUTHORIZED,
                        "auth_key_invalid",
                    );
                }
            }
            api_ok(crate::visible_text::sanitize_task_query_response_for_delivery(task))
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
    let Some(raw_key) = auth_key_from_headers(headers)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Err(api_err::<T>(StatusCode::UNAUTHORIZED, "auth_key_required"));
    };
    match resolve_auth_identity_by_key(state, raw_key) {
        Ok(Some(identity)) => Ok(identity),
        Ok(None) => Err(api_err::<T>(StatusCode::UNAUTHORIZED, "auth_key_invalid")),
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
        claim_attempt: 0,
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
            "source": source
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
    execution_state: String,
    summary: String,
    age_seconds: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    lifecycle: Option<serde_json::Value>,
}

/// Phase 4: 重载 skill 视图。POST /v1/admin/reload-skills。与现有管理接口一致：需 x-agent-key 鉴权。
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
