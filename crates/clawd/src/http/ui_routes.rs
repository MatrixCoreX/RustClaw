use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use rusqlite::OptionalExtension;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{BufRead, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio as StdProcessStdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::Semaphore;

use super::super::{
    attach_pending_channel_bind_session_install_flow, bind_channel_identity,
    channel_gateway_process_stats, create_auth_key, create_pending_channel_bind_session,
    current_rss_bytes, daemon_process_pids_by_name, delete_auth_key_by_id,
    exchange_credential_status_for_user_key, factory_reset_auth_state, feishud_process_stats,
    finalize_pending_channel_bind_session, get_auth_key_value_by_id,
    get_pending_channel_bind_session_by_id, get_pending_channel_bind_session_by_token,
    has_channel_binding_for_user_key, larkd_process_stats, list_auth_keys,
    mark_pending_channel_bind_session_detected, mark_pending_channel_bind_session_expired,
    mark_pending_channel_bind_session_failed, mask_secret, oldest_running_task_age_seconds,
    reload_skill_views, reset_channel_binding_state_for_user_key, resolve_auth_identity_by_key,
    resolve_channel_binding_identity, task_count_by_status, telegramd_process_stats,
    update_auth_key_by_id, upsert_exchange_credential_for_user_key, upsert_webd_login_account,
    verify_webd_password_login, wa_webd_process_stats, webd_process_stats, wechatd_process_stats,
    whatsappd_process_stats, ApiResponse, AppState, FactoryResetDbResult, HealthResponse,
    LlmProviderRuntime, LocalInteractionContext, PendingChannelBindSession,
};
use crate::ClaimedTask;
use claw_core::types::{
    AuthIdentity, BindChannelKeyRequest, DetectFeishuBindSessionRequest,
    DetectFeishuBindSessionResponse, ExchangeCredentialStatus, FeishuBindSessionStatusResponse,
    GatewayInstanceRuntimeStatus, ResolveChannelBindingRequest, ResolveChannelBindingResponse,
    StartFeishuBindSessionRequest, TelegramBotRuntimeStatus, UiKeyVerifyRequest,
    UpsertExchangeCredentialRequest,
};
use claw_core::{
    prompt_layers,
    skill_registry::{PlannerCapabilityKind, SkillKind},
};

const TELEGRAM_BOT_HEARTBEAT_STALE_SECONDS: i64 = 45;
const FEISHU_BIND_SESSION_DEFAULT_TTL_SECONDS: u64 = 600;
const FEISHU_BIND_SESSION_MIN_TTL_SECONDS: u64 = 60;
const FEISHU_BIND_SESSION_MAX_TTL_SECONDS: u64 = 1800;
const FEISHU_OFFICIAL_ACCOUNTS_BASE_URL: &str = "https://accounts.feishu.cn";
const WORKSPACE_UPDATE_TIMEOUT_SECONDS: u64 = 3600;
const WORKSPACE_UPDATE_LOG_MAX_CHARS: usize = 12000;
const WORKSPACE_UPDATE_PATH_BATCH_SIZE: usize = 128;
const WORKSPACE_UPDATE_PATH_LIST_MAX_BYTES: usize = 32 * 1024 * 1024;
const WORKSPACE_UPDATE_PATH_LIST_MAX_ITEMS: usize = 250_000;
const NNI_SIGNATURE_HELPER_TIMEOUT_SECONDS: u64 = 12;
const FEISHU_CONFIG_TEMPLATE: &str = include_str!("../../templates/feishu_china_config.toml");
const LLM_CONNECTIVITY_TEST_PROMPT: &str = "Reply with OK only.";

#[derive(Debug, Clone, Serialize)]
struct WorkspaceUpdateStatus {
    status: String,
    step: String,
    mode: String,
    started_ts: Option<i64>,
    finished_ts: Option<i64>,
    old_commit: Option<String>,
    new_commit: Option<String>,
    remote_commit: Option<String>,
    exit_code: Option<i32>,
    stdout_tail: String,
    stderr_tail: String,
    error: Option<String>,
    next_step: Option<String>,
    next_step_key: Option<String>,
    next_step_args: BTreeMap<String, Value>,
}

impl Default for WorkspaceUpdateStatus {
    fn default() -> Self {
        Self {
            status: "idle".to_string(),
            step: "idle".to_string(),
            mode: "full".to_string(),
            started_ts: None,
            finished_ts: None,
            old_commit: None,
            new_commit: None,
            remote_commit: None,
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            error: None,
            next_step: None,
            next_step_key: None,
            next_step_args: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
struct WorkspaceUpdateCommandOutput {
    exit_code: Option<i32>,
    stdout_tail: String,
    stderr_tail: String,
}

#[derive(Debug, Default)]
struct WorkspaceUpdateConflictPaths {
    tracked: Vec<String>,
    untracked: Vec<String>,
}

impl WorkspaceUpdateConflictPaths {
    fn is_empty(&self) -> bool {
        self.tracked.is_empty() && self.untracked.is_empty()
    }

    fn len(&self) -> usize {
        self.tracked.len() + self.untracked.len()
    }
}

#[derive(Debug, Default)]
struct WorkspaceUpdateControl {
    cancel_requested: bool,
    active_child_pid: Option<u32>,
}

static WORKSPACE_UPDATE_STATE: OnceLock<Arc<Mutex<WorkspaceUpdateStatus>>> = OnceLock::new();
static WORKSPACE_UPDATE_CONTROL: OnceLock<Arc<Mutex<WorkspaceUpdateControl>>> = OnceLock::new();

pub(crate) fn build_ui_router() -> Router<AppState> {
    Router::new()
        .route("/auth/ui-key/verify", post(verify_ui_key))
        .route("/auth/me", get(auth_me))
        .route("/auth/channel/resolve", post(resolve_channel_binding))
        .route("/auth/channel/bind", post(bind_channel_key))
        .route(
            "/auth/channel-binds/feishu/detect",
            post(detect_feishu_bind_session_handler),
        )
        .route(
            "/auth/crypto-credentials",
            get(get_crypto_credentials).post(upsert_crypto_credentials),
        )
        .route("/health", get(health))
        .route("/skills", get(list_skills))
        .route("/capabilities", get(list_capabilities))
        .route(
            "/skills/config",
            get(get_skills_config).post(update_skills_config),
        )
        .route(
            "/telegram/config",
            get(get_telegram_config).post(update_telegram_config),
        )
        .route(
            "/wechat/config",
            get(get_wechat_config).post(update_wechat_config),
        )
        .route(
            "/feishu/config",
            get(get_feishu_config).post(update_feishu_config),
        )
        .route("/admin/feishu/reset", post(reset_feishu_config_handler))
        .route("/skills/import", post(import_external_skill))
        .route("/skills/import/upload", post(import_external_skill_upload))
        .route("/skills/uninstall", post(uninstall_external_skill))
        .route("/llm/config", get(get_llm_config).post(update_llm_config))
        .route("/llm/test", post(test_llm_config))
        .route("/models/catalog", get(get_model_catalog))
        .route("/nni/device/status", get(nni_device_status))
        .route("/nni/device/action", post(nni_device_action))
        .route("/nni/config", get(get_nni_config).post(update_nni_config))
        .route("/nni/join/request", post(nni_join_request))
        .route("/nni/join/verify", post(nni_join_verify))
        .route("/nni/records", get(nni_request_records))
        .route("/nni/records/clear", post(nni_clear_request_records))
        .route("/nni/heartbeat/records", get(nni_request_records))
        .route("/nni/heartbeat/errors", get(nni_heartbeat_errors))
        .route(
            "/nni/heartbeat/errors/clear",
            post(nni_clear_heartbeat_errors),
        )
        .route("/logs/latest", get(logs_latest))
        .route("/debug/tasks/:task_id", get(task_debug_detail))
        .route("/debug/recent-robot-tasks", get(recent_robot_tasks))
        .route("/debug/usage-records", get(usage_records))
        .route("/debug/usage-records/:record_id", get(usage_record_detail))
        .route("/observability/slo", get(observability_slo_metrics))
        .route("/wechat/login-status", get(wechat_login_status))
        .route("/wechat/login-qr/start", post(wechat_login_qr_start))
        .route("/wechat/login-qr/wait", post(wechat_login_qr_wait))
        .route("/whatsapp-web/login-status", get(whatsapp_web_login_status))
        .route("/whatsapp-web/logout", post(whatsapp_web_logout))
        .route("/services/:service/:action", post(control_service))
        .route("/system/restart", post(restart_system))
        .route("/pi-app/status", get(pi_app_status))
        .route("/pi-app/restart", post(restart_pi_app))
        .route(
            "/admin/workspace-update",
            get(get_workspace_update).post(start_workspace_update),
        )
        .route(
            "/admin/workspace-update/build-ui",
            post(start_workspace_update_ui_only),
        )
        .route(
            "/admin/workspace-update/build-clawd",
            post(start_workspace_update_clawd_only),
        )
        .route(
            "/admin/workspace-update/deploy-release",
            post(start_workspace_update_release_deploy),
        )
        .route(
            "/admin/workspace-update/cancel",
            post(cancel_workspace_update),
        )
        .route("/admin/factory-reset", post(factory_reset_handler))
        .route("/local/interaction-context", get(local_interaction_context))
        .route(
            "/admin/model-config",
            get(get_model_config).post(update_model_config),
        )
        .route(
            "/admin/provider-keys",
            get(get_provider_keys).post(update_provider_keys),
        )
        .route("/admin/restart-clawd", post(restart_clawd))
        .route(
            "/admin/auth-keys",
            get(get_auth_keys).post(create_auth_key_handler),
        )
        .route(
            "/admin/auth-keys/:key_id/full",
            get(get_auth_key_full_handler),
        )
        .route(
            "/admin/channel-binds/feishu/start",
            post(start_feishu_bind_session_handler),
        )
        .route(
            "/admin/channel-binds/feishu/:session_id",
            get(get_feishu_bind_session_handler),
        )
        .route(
            "/admin/auth-keys/:key_id",
            put(update_auth_key_handler).delete(delete_auth_key_handler),
        )
        .route(
            "/internal/webd/verify-login",
            post(webd_internal_verify_login),
        )
        .route("/internal/llm/text", post(internal_llm_text))
        .route("/admin/webd-accounts", post(admin_upsert_webd_account))
}

#[derive(Debug, Deserialize)]
struct InternalLlmTextRequest {
    #[serde(default)]
    skill_name: String,
    #[serde(default)]
    prompt_source: String,
    #[serde(default)]
    vendor: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    system: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    prompt: String,
    temperature: Option<f64>,
    max_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
struct InternalLlmTextResponse {
    text: String,
    prompt_source: String,
    model: String,
    provider: String,
}

#[derive(Debug)]
struct NniSignatureHelperOutput {
    ok: bool,
    payload: Value,
    error: Option<String>,
    stderr_tail: String,
    exit_code: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct NniDeviceActionRequest {
    action: String,
    #[serde(default)]
    timestamp: Option<i64>,
    #[serde(default)]
    challenge: Option<String>,
}

include!("ui_routes/config_helpers.rs");
include!("ui_routes/nni_internal_llm.rs");
include!("ui_routes/nni_request_records.rs");
include!("ui_routes/nni_remote_join.rs");
include!("ui_routes/auth_feishu_bind.rs");
include!("ui_routes/factory_reset.rs");
include!("ui_routes/channel_config.rs");
include!("ui_routes/task_debug_trace.rs");
include!("ui_routes/logs_usage_debug.rs");
include!("ui_routes/slo_metrics.rs");
include!("ui_routes/service_control.rs");
include!("ui_routes/workspace_update.rs");
include!("ui_routes/health_skills_import.rs");
include!("ui_routes/model_provider_config.rs");
include!("ui_routes/skill_import_config.rs");
include!("ui_routes/llm_skill_config.rs");
include!("ui_routes/messaging_login.rs");

#[cfg(test)]
#[path = "ui_routes_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ui_routes/slo_metrics_tests.rs"]
mod slo_metrics_tests;
