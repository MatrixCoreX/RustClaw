use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio as StdProcessStdio};
use tokio::process::Command;

use super::super::{
    bind_channel_identity, create_auth_key, current_rss_bytes, delete_auth_key_by_id,
    exchange_credential_status_for_user_key, feishud_process_stats, larkd_process_stats, list_auth_keys,
    mask_secret, oldest_running_task_age_seconds, reload_skill_views, resolve_auth_identity_by_key,
    resolve_channel_binding_identity, task_count_by_status, telegramd_process_stats,
    update_auth_key_by_id, upsert_exchange_credential_for_user_key, wa_webd_process_stats,
    whatsappd_process_stats, ApiResponse, AppState, HealthResponse, LocalInteractionContext,
};
use claw_core::skill_registry::SkillKind;
use claw_core::types::{
    AuthIdentity, BindChannelKeyRequest, ExchangeCredentialStatus, ResolveChannelBindingRequest,
    ResolveChannelBindingResponse, UiKeyVerifyRequest, UpsertExchangeCredentialRequest,
};

const UI_HIDDEN_SKILLS: &[&str] = &["chat"];

fn hide_skill_in_ui(state: &AppState, name: &str) -> bool {
    let canonical = state.resolve_canonical_skill_name(name);
    UI_HIDDEN_SKILLS.iter().any(|s| *s == canonical)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceAction {
    Start,
    Stop,
    Restart,
}

pub(crate) fn build_ui_router() -> Router<AppState> {
    Router::new()
        .route("/auth/ui-key/verify", post(verify_ui_key))
        .route("/auth/me", get(auth_me))
        .route("/auth/channel/resolve", post(resolve_channel_binding))
        .route("/auth/channel/bind", post(bind_channel_key))
        .route(
            "/auth/crypto-credentials",
            get(get_crypto_credentials).post(upsert_crypto_credentials),
        )
        .route("/health", get(health))
        .route("/skills", get(list_skills))
        .route(
            "/skills/config",
            get(get_skills_config).post(update_skills_config),
        )
        .route("/skills/import", post(import_external_skill))
        .route("/skills/import/upload", post(import_external_skill_upload))
        .route("/skills/uninstall", post(uninstall_external_skill))
        .route("/llm/config", get(get_llm_config).post(update_llm_config))
        .route("/logs/latest", get(logs_latest))
        .route("/whatsapp-web/login-status", get(whatsapp_web_login_status))
        .route("/whatsapp-web/logout", post(whatsapp_web_logout))
        .route("/services/:service/:action", post(control_service))
        .route("/system/restart", post(restart_system))
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
        .route("/admin/auth-keys", get(get_auth_keys).post(create_auth_key_handler))
        .route(
            "/admin/auth-keys/:key_id",
            put(update_auth_key_handler).delete(delete_auth_key_handler),
        )
}

fn ui_auth_error(message: &str) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(message.to_string()),
        }),
    )
}

pub(crate) fn require_ui_identity(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthIdentity, (StatusCode, Json<ApiResponse<Value>>)> {
    let Some(raw_key) = headers
        .get("x-rustclaw-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Err(ui_auth_error("Missing X-RustClaw-Key header"));
    };
    match resolve_auth_identity_by_key(state, raw_key) {
        Ok(Some(identity)) => Ok(identity),
        Ok(None) => Err(ui_auth_error("Invalid key")),
        Err(err) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("auth lookup failed: {err}")),
            }),
        )),
    }
}

async fn verify_ui_key(
    State(state): State<AppState>,
    Json(req): Json<UiKeyVerifyRequest>,
) -> (StatusCode, Json<ApiResponse<AuthIdentity>>) {
    match resolve_auth_identity_by_key(&state, &req.user_key) {
        Ok(Some(identity)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(identity),
                error: None,
            }),
        ),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Invalid key".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("auth lookup failed: {err}")),
            }),
        ),
    }
}

async fn auth_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<AuthIdentity>>) {
    match require_ui_identity(&state, &headers) {
        Ok(identity) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(identity),
                error: None,
            }),
        ),
        Err((status, Json(resp))) => (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        ),
    }
}

async fn resolve_channel_binding(
    State(state): State<AppState>,
    Json(req): Json<ResolveChannelBindingRequest>,
) -> (StatusCode, Json<ApiResponse<ResolveChannelBindingResponse>>) {
    match resolve_channel_binding_identity(
        &state,
        match req.channel {
            claw_core::types::ChannelKind::Telegram => "telegram",
            claw_core::types::ChannelKind::Whatsapp => "whatsapp",
            claw_core::types::ChannelKind::Ui => "ui",
            claw_core::types::ChannelKind::Feishu => "feishu",
            claw_core::types::ChannelKind::Lark => "lark",
        },
        req.external_user_id.as_deref(),
        req.external_chat_id.as_deref(),
    ) {
        Ok(identity) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(ResolveChannelBindingResponse {
                    bound: identity.is_some(),
                    identity,
                }),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("resolve channel binding failed: {err}")),
            }),
        ),
    }
}

async fn bind_channel_key(
    State(state): State<AppState>,
    Json(req): Json<BindChannelKeyRequest>,
) -> (StatusCode, Json<ApiResponse<AuthIdentity>>) {
    match bind_channel_identity(
        &state,
        match req.channel {
            claw_core::types::ChannelKind::Telegram => "telegram",
            claw_core::types::ChannelKind::Whatsapp => "whatsapp",
            claw_core::types::ChannelKind::Ui => "ui",
            claw_core::types::ChannelKind::Feishu => "feishu",
            claw_core::types::ChannelKind::Lark => "lark",
        },
        req.external_user_id.as_deref(),
        req.external_chat_id.as_deref(),
        &req.user_key,
    ) {
        Ok(Some(identity)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(identity),
                error: None,
            }),
        ),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("Invalid key".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("bind channel key failed: {err}")),
            }),
        ),
    }
}

async fn get_crypto_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Vec<ExchangeCredentialStatus>>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: resp.ok,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    match exchange_credential_status_for_user_key(&state, &identity.user_key) {
        Ok(mut statuses) => {
            for status in &mut statuses {
                status.api_key_masked = status.api_key_masked.as_deref().map(mask_secret);
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(statuses),
                    error: None,
                }),
            )
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read crypto credentials failed: {err}")),
            }),
        ),
    }
}

async fn upsert_crypto_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpsertExchangeCredentialRequest>,
) -> (StatusCode, Json<ApiResponse<ExchangeCredentialStatus>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err((status, Json(resp))) => {
            return (
                status,
                Json(ApiResponse {
                    ok: resp.ok,
                    data: None,
                    error: resp.error,
                }),
            );
        }
    };
    match upsert_exchange_credential_for_user_key(
        &state,
        &identity.user_key,
        &req.exchange,
        &req.api_key,
        &req.api_secret,
        req.passphrase.as_deref(),
    ) {
        Ok(status) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(ExchangeCredentialStatus {
                    exchange: status.exchange,
                    configured: status.configured,
                    api_key_masked: status.api_key_masked.as_deref().map(mask_secret),
                    updated_at: status.updated_at,
                }),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(err.to_string()),
            }),
        ),
    }
}

#[derive(Debug, serde::Deserialize, Default)]
struct LogsLatestQuery {
    file: Option<String>,
    lines: Option<usize>,
}

fn normalize_log_file_name(raw: Option<&str>) -> String {
    let fallback = "agent_trace.log".to_string();
    let candidate = raw.unwrap_or("").trim();
    if candidate.is_empty() {
        return fallback;
    }
    let allowed = [
        "agent_trace.log",
        "model_io.log",
        "routing.log",
        "act_plan.log",
        "clawd.log",
        "telegramd.log",
        "whatsappd.log",
        "whatsapp_webd.log",
    ];
    if allowed.iter().any(|v| v.eq_ignore_ascii_case(candidate)) {
        return candidate.to_string();
    }
    fallback
}

fn read_last_lines(path: &std::path::Path, limit_lines: usize) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let total_size = file.metadata()?.len();
    let max_tail_bytes: u64 = 512 * 1024;
    let read_from = total_size.saturating_sub(max_tail_bytes);
    if read_from > 0 {
        file.seek(SeekFrom::Start(read_from))?;
    }
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let content = String::from_utf8_lossy(&buf);
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Ok(String::new());
    }
    let start = lines.len().saturating_sub(limit_lines);
    Ok(lines[start..].join("\n"))
}

fn auth_user_summary_counts(state: &AppState) -> anyhow::Result<(usize, usize)> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    let user_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM auth_keys WHERE enabled = 1",
        [],
        |row| row.get(0),
    )?;
    let bound_channel_count: i64 =
        db.query_row("SELECT COUNT(*) FROM channel_bindings", [], |row| {
            row.get(0)
        })?;
    Ok((
        user_count.max(0) as usize,
        bound_channel_count.max(0) as usize,
    ))
}

async fn logs_latest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LogsLatestQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let file_name = normalize_log_file_name(query.file.as_deref());
    let lines = query.lines.unwrap_or(200).clamp(20, 2000);
    let path = state.workspace_root.join("logs").join(&file_name);
    let raw = match read_last_lines(&path, lines) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read log failed: {err}")),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "file": file_name,
                "lines": lines,
                "text": raw,
            })),
            error: None,
        }),
    )
}

fn shell_escape_arg(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

fn parse_service_action(raw: &str) -> Option<ServiceAction> {
    match raw {
        "start" => Some(ServiceAction::Start),
        "stop" => Some(ServiceAction::Stop),
        "restart" => Some(ServiceAction::Restart),
        _ => None,
    }
}

fn service_start_script(service: &str) -> Option<&'static str> {
    match service {
        "telegramd" => Some("start-telegramd.sh"),
        "whatsappd" => Some("start-whatsappd.sh"),
        "whatsapp_webd" => Some("start-whatsapp-webd.sh"),
        "feishud" => Some("start-feishud.sh"),
        "larkd" => Some("start-larkd.sh"),
        _ => None,
    }
}

fn service_process_name(service: &str) -> Option<&'static str> {
    match service {
        "telegramd" => Some("telegramd"),
        "whatsappd" => Some("whatsappd"),
        "whatsapp_webd" => Some("whatsapp_webd"),
        "feishud" => Some("feishud"),
        "larkd" => Some("larkd"),
        _ => None,
    }
}

fn service_pid_file(service: &str) -> Option<&'static str> {
    match service {
        "telegramd" => Some("telegramd.pid"),
        "whatsappd" => Some("whatsappd.pid"),
        "whatsapp_webd" => Some("whatsapp_webd.pid"),
        "feishud" => Some("feishud.pid"),
        "larkd" => Some("larkd.pid"),
        _ => None,
    }
}

fn service_extra_process_names_on_stop(service: &str) -> &'static [&'static str] {
    match service {
        "whatsapp_webd" => &["services/wa-web-bridge/index.js", "wa-web-bridge/index.js"],
        _ => &[],
    }
}

fn service_is_running(service: &str) -> bool {
    match service {
        "telegramd" => telegramd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "whatsappd" => whatsappd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "whatsapp_webd" => wa_webd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "feishud" => feishud_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        "larkd" => larkd_process_stats()
            .map(|(count, _)| count > 0)
            .unwrap_or(false),
        _ => false,
    }
}

fn daemon_process_pids(process_name: &str) -> Option<Vec<u32>> {
    let entries = std::fs::read_dir("/proc").ok()?;
    let mut pids = Vec::new();
    let self_pid = std::process::id();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid_str = name.to_string_lossy();
        if !pid_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(pid_num) = pid_str.parse::<u32>() else {
            continue;
        };
        if pid_num == self_pid {
            continue;
        }
        let cmdline_path = format!("/proc/{pid_num}/cmdline");
        let bytes = match std::fs::read(&cmdline_path) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if bytes.is_empty() {
            continue;
        }
        let cmdline = String::from_utf8_lossy(&bytes);
        if cmdline.contains(process_name) {
            pids.push(pid_num);
        }
    }
    Some(pids)
}

fn runtime_profile_default() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// 轮询直到服务进程就绪，或超时。用于 Start/Restart。
async fn poll_service_running(service: &str, interval_ms: u64, timeout_secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    while std::time::Instant::now() < deadline {
        if service_is_running(service) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
    }
    false
}

/// 轮询直到服务进程已退出，或超时。用于 Stop。
async fn poll_service_stopped(service: &str, interval_ms: u64, timeout_secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    while std::time::Instant::now() < deadline {
        if !service_is_running(service) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
    }
    false
}

async fn control_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((service, action)): AxumPath<(String, String)>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let action = match parse_service_action(action.trim()) {
        Some(v) => v,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some("action must be start, stop, or restart".to_string()),
                }),
            );
        }
    };

    if service_start_script(service.as_str()).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("unsupported service".to_string()),
            }),
        );
    }

    match action {
        ServiceAction::Start => {
            if service_is_running(service.as_str()) {
                return (
                    StatusCode::OK,
                    Json(ApiResponse {
                        ok: true,
                        data: Some(json!({
                            "service": service,
                            "action": "start",
                            "status": "already_running"
                        })),
                        error: None,
                    }),
                );
            }
            let profile = std::env::var("RUSTCLAW_START_PROFILE")
                .ok()
                .filter(|v| matches!(v.as_str(), "debug" | "release"))
                .unwrap_or_else(|| runtime_profile_default().to_string());
            let Some(script_name) = service_start_script(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let workspace = state.workspace_root.to_string_lossy();
            let log_file = format!("logs/{}.log", service);
            let cmd = format!(
                "cd {} && mkdir -p logs .pids && nohup ./{} {} > {} 2>&1 &",
                shell_escape_arg(workspace.as_ref()),
                script_name,
                shell_escape_arg(profile.as_str()),
                shell_escape_arg(log_file.as_str())
            );
            let output = match Command::new("bash").arg("-lc").arg(cmd).output().await {
                Ok(v) => v,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some(format!("failed to start service process: {err}")),
                        }),
                    );
                }
            };
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("service start command failed: {detail}")),
                    }),
                );
            }
            // Start command returns immediately (nohup ... &). Poll until process is up or timeout.
            // Preflight (e.g. Telegram API) can take several seconds.
            if !poll_service_running(service.as_str(), 400, 12).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "service did not enter running state: {service}. check logs/{service}.log and channel config"
                        )),
                    }),
                );
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "service": service,
                        "action": "start",
                        "status": "starting",
                        "profile": profile
                    })),
                    error: None,
                }),
            )
        }
        ServiceAction::Stop => {
            let Some(process_name) = service_process_name(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let mut killed = 0usize;
            if let Some(pids) = daemon_process_pids(process_name) {
                for pid in pids {
                    let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                    let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                    killed += 1;
                }
            }
            for extra_name in service_extra_process_names_on_stop(service.as_str()) {
                if let Some(pids) = daemon_process_pids(extra_name) {
                    for pid in pids {
                        let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                        let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                        killed += 1;
                    }
                }
            }
            if killed == 0 && !service_is_running(service.as_str()) {
                return (
                    StatusCode::OK,
                    Json(ApiResponse {
                        ok: true,
                        data: Some(json!({
                            "service": service,
                            "action": "stop",
                            "status": "already_stopped"
                        })),
                        error: None,
                    }),
                );
            }
            // Wait until process is actually gone so UI refresh shows stopped state.
            if !poll_service_stopped(service.as_str(), 500, 15).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "service {service} did not stop within timeout. try again or check process manually."
                        )),
                    }),
                );
            }
            let Some(pid_file) = service_pid_file(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let workspace = state.workspace_root.to_string_lossy();
            let cmd = format!(
                "cd {} && rm -f .pids/{}",
                shell_escape_arg(workspace.as_ref()),
                shell_escape_arg(pid_file)
            );
            let output = match Command::new("bash").arg("-lc").arg(cmd).output().await {
                Ok(v) => v,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some(format!("failed to stop service process: {err}")),
                        }),
                    );
                }
            };
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("service stop command failed: {detail}")),
                    }),
                );
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "service": service,
                        "action": "stop",
                        "status": "stopped"
                    })),
                    error: None,
                }),
            )
        }
        ServiceAction::Restart => {
            let Some(process_name) = service_process_name(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            if let Some(pids) = daemon_process_pids(process_name) {
                for pid in pids {
                    let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                    let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                }
            }
            for extra_name in service_extra_process_names_on_stop(service.as_str()) {
                if let Some(pids) = daemon_process_pids(extra_name) {
                    for pid in pids {
                        let cmd = format!("kill -TERM {} >/dev/null 2>&1 || true", pid);
                        let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
                    }
                }
            }
            if let Some(pid_file) = service_pid_file(service.as_str()) {
                let workspace = state.workspace_root.to_string_lossy();
                let cmd = format!(
                    "cd {} && rm -f .pids/{}",
                    shell_escape_arg(workspace.as_ref()),
                    shell_escape_arg(pid_file)
                );
                let _ = Command::new("bash").arg("-lc").arg(cmd).output().await;
            }
            if !poll_service_stopped(service.as_str(), 500, 15).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "service {service} did not fully stop before restart. try again or check process manually."
                        )),
                    }),
                );
            }
            let profile = std::env::var("RUSTCLAW_START_PROFILE")
                .ok()
                .filter(|v| matches!(v.as_str(), "debug" | "release"))
                .unwrap_or_else(|| runtime_profile_default().to_string());
            let Some(script_name) = service_start_script(service.as_str()) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some("unsupported service".to_string()),
                    }),
                );
            };
            let workspace = state.workspace_root.to_string_lossy();
            let log_file = format!("logs/{}.log", service);
            let cmd = format!(
                "cd {} && mkdir -p logs .pids && nohup ./{} {} > {} 2>&1 &",
                shell_escape_arg(workspace.as_ref()),
                script_name,
                shell_escape_arg(profile.as_str()),
                shell_escape_arg(log_file.as_str())
            );
            let output = match Command::new("bash").arg("-lc").arg(cmd).output().await {
                Ok(v) => v,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some(format!("failed to start service process: {err}")),
                        }),
                    );
                }
            };
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("service restart start command failed: {detail}")),
                    }),
                );
            }
            if !poll_service_running(service.as_str(), 400, 12).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!(
                            "service did not enter running state after restart: {service}. check logs/{service}.log"
                        )),
                    }),
                );
            }
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({
                        "service": service,
                        "action": "restart",
                        "status": "restarted",
                        "profile": profile
                    })),
                    error: None,
                }),
            )
        }
    }
}

#[derive(Debug, Deserialize)]
struct CreateAuthKeyRequest {
    #[serde(default)]
    role: String,
}

#[derive(Debug, Deserialize)]
struct UpdateAuthKeyRequest {
    role: Option<String>,
    enabled: Option<bool>,
}

async fn get_auth_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can list auth keys".to_string()),
            }),
        );
    }
    match list_auth_keys(&state) {
        Ok(rows) => {
            let list: Vec<Value> = rows
                .into_iter()
                .map(|(key_id, user_key_masked, role, enabled, created_at, last_used_at)| {
                    json!({
                        "key_id": key_id,
                        "user_key_masked": user_key_masked,
                        "role": role,
                        "enabled": enabled != 0,
                        "created_at": created_at,
                        "last_used_at": last_used_at,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({ "keys": list })),
                    error: None,
                }),
            )
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("list auth keys failed: {err}")),
            }),
        ),
    }
}

async fn update_auth_key_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(key_id): AxumPath<i64>,
    Json(req): Json<UpdateAuthKeyRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can update auth keys".to_string()),
            }),
        );
    }

    let role = req.role.as_deref();
    let role = role.map(str::trim).filter(|v| !v.is_empty());
    match update_auth_key_by_id(&state, key_id, role, req.enabled, &identity.user_key) {
        Ok(true) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "updated": true })),
                error: None,
            }),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("auth key not found".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("update auth key failed: {err}")),
            }),
        ),
    }
}

async fn delete_auth_key_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(key_id): AxumPath<i64>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can delete auth keys".to_string()),
            }),
        );
    }

    match delete_auth_key_by_id(&state, key_id, &identity.user_key) {
        Ok(true) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "deleted": true })),
                error: None,
            }),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("auth key not found".to_string()),
            }),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("delete auth key failed: {err}")),
            }),
        ),
    }
}

async fn create_auth_key_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateAuthKeyRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can create auth keys".to_string()),
            }),
        );
    }
    match create_auth_key(&state, req.role.as_str()) {
        Ok(user_key) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({ "user_key": user_key })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("create auth key failed: {err}")),
            }),
        ),
    }
}

async fn restart_system(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(resp) => return resp,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only admin can restart RustClaw".to_string()),
            }),
        );
    }

    if !std::path::Path::new("/.dockerenv").exists() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("frontend restart is only available in Docker deployment".to_string()),
            }),
        );
    }

    let mut cmd = Command::new("bash");
    cmd.arg("-lc")
        .arg("sleep 1 && kill -TERM 1 >/dev/null 2>&1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    if let Err(err) = cmd.spawn() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("failed to schedule restart: {err}")),
            }),
        );
    }

    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "status": "restarting",
                "mode": "docker",
            })),
            error: None,
        }),
    )
}

async fn health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<HealthResponse>>) {
    if let Err((status, Json(resp))) = require_ui_identity(&state, &headers) {
        return (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        );
    }
    let queue_length = task_count_by_status(&state, "queued").unwrap_or_default();
    let running_length = task_count_by_status(&state, "running").unwrap_or_default();
    let running_oldest_age_seconds = oldest_running_task_age_seconds(&state).unwrap_or(0);
    let telegramd_stats = telegramd_process_stats();
    let whatsappd_stats = whatsappd_process_stats();
    let wa_webd_stats = wa_webd_process_stats();
    let telegramd_process_count = telegramd_stats.map(|(count, _)| count);
    let telegramd_memory_rss_bytes = telegramd_stats.map(|(_, rss_bytes)| rss_bytes);
    let telegramd_healthy = telegramd_process_count.map(|count| count > 0);
    let whatsappd_process_count = whatsappd_stats.map(|(count, _)| count);
    let whatsappd_memory_rss_bytes = whatsappd_stats.map(|(_, rss_bytes)| rss_bytes);
    let whatsappd_healthy = whatsappd_process_count.map(|count| count > 0);
    let wa_webd_process_count = wa_webd_stats.map(|(count, _)| count);
    let wa_webd_memory_rss_bytes = wa_webd_stats.map(|(_, rss_bytes)| rss_bytes);
    let wa_webd_healthy = wa_webd_process_count.map(|count| count > 0);
    let feishud_stats = feishud_process_stats();
    let feishud_process_count = feishud_stats.map(|(count, _)| count);
    let feishud_memory_rss_bytes = feishud_stats.map(|(_, rss_bytes)| rss_bytes);
    let feishud_healthy = feishud_process_count.map(|count| count > 0);
    let larkd_stats = larkd_process_stats();
    let larkd_process_count = larkd_stats.map(|(count, _)| count);
    let larkd_memory_rss_bytes = larkd_stats.map(|(_, rss_bytes)| rss_bytes);
    let larkd_healthy = larkd_process_count.map(|count| count > 0);
    let (user_count, bound_channel_count) = auth_user_summary_counts(&state).unwrap_or_default();
    let data = HealthResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        queue_length,
        worker_state: "running".to_string(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        memory_rss_bytes: current_rss_bytes(),
        running_length,
        task_timeout_seconds: state.worker_task_timeout_seconds,
        running_oldest_age_seconds,
        telegramd_healthy,
        telegramd_process_count,
        telegramd_memory_rss_bytes,
        whatsappd_healthy,
        whatsappd_process_count,
        whatsappd_memory_rss_bytes,
        telegram_bot_healthy: telegramd_healthy,
        telegram_bot_process_count: telegramd_process_count,
        telegram_bot_memory_rss_bytes: telegramd_memory_rss_bytes,
        whatsapp_cloud_healthy: whatsappd_healthy,
        whatsapp_cloud_process_count: whatsappd_process_count,
        whatsapp_cloud_memory_rss_bytes: whatsappd_memory_rss_bytes,
        whatsapp_web_healthy: wa_webd_healthy,
        whatsapp_web_process_count: wa_webd_process_count,
        whatsapp_web_memory_rss_bytes: wa_webd_memory_rss_bytes,
        feishud_healthy,
        feishud_process_count,
        feishud_memory_rss_bytes,
        larkd_healthy,
        larkd_process_count,
        larkd_memory_rss_bytes,
        user_count,
        bound_channel_count,
        future_adapters_enabled: state.future_adapters_enabled.as_ref().clone(),
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

async fn list_skills(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let mut skills: Vec<String> = state.get_skills_list().iter().cloned().collect();
    skills.retain(|s| !hide_skill_in_ui(&state, s));
    skills.sort_unstable();
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "skills": skills,
                "skill_runner_path": state.skill_runner_path.display().to_string(),
            })),
            error: None,
        }),
    )
}

async fn import_external_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ImportSkillRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let source = req.source.trim();
    if source.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("source is required".to_string()),
            }),
        );
    }
    let enabled = req.enabled.unwrap_or(true);

    let raw_name = guess_bundle_name_from_path_or_source(source, "external-skill");
    let canonical_name = slugify_skill_name(&raw_name);
    let bundle_rel_dir = format!("third_party/clawhub/{canonical_name}");
    let bundle_dir = state.workspace_root.join(&bundle_rel_dir);
    if bundle_dir.exists() {
        if let Err(err) = std::fs::remove_dir_all(&bundle_dir) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("remove old imported bundle failed: {err}")),
                }),
            );
        }
    }

    let skill_md = match materialize_import_source(&state, source, &bundle_dir).await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(err),
                }),
            );
        }
    };
    finalize_imported_bundle(&state, &bundle_dir, &bundle_rel_dir, source, enabled, &skill_md)
}

async fn import_external_skill_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }

    let mut bundle_name = String::new();
    let mut enabled = true;
    let mut uploaded_files: Vec<(PathBuf, Vec<u8>)> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "bundle_name" => {
                if let Ok(text) = field.text().await {
                    bundle_name = text.trim().to_string();
                }
            }
            "enabled" => {
                if let Ok(text) = field.text().await {
                    enabled = text.trim() != "false";
                }
            }
            "files" => {
                let raw_path = field
                    .file_name()
                    .map(str::to_string)
                    .unwrap_or_else(|| "SKILL.md".to_string());
                let Some(rel_path) = sanitize_upload_relative_path(&raw_path) else {
                    continue;
                };
                let Ok(bytes) = field.bytes().await else {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some("read uploaded file failed".to_string()),
                        }),
                    );
                };
                uploaded_files.push((rel_path, bytes.to_vec()));
            }
            _ => {}
        }
    }

    if uploaded_files.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("no uploaded files found".to_string()),
            }),
        );
    }

    let guessed_name = if !bundle_name.trim().is_empty() {
        bundle_name.trim().to_string()
    } else {
        uploaded_files
            .first()
            .and_then(|(path, _)| path.components().next())
            .and_then(|part| match part {
                std::path::Component::Normal(v) => v.to_str(),
                _ => None,
            })
            .unwrap_or("uploaded-skill")
            .to_string()
    };
    let canonical_name = slugify_skill_name(&guessed_name);
    let bundle_rel_dir = format!("third_party/clawhub/{canonical_name}");
    let bundle_dir = state.workspace_root.join(&bundle_rel_dir);
    if bundle_dir.exists() {
        if let Err(err) = std::fs::remove_dir_all(&bundle_dir) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("remove old uploaded bundle failed: {err}")),
                }),
            );
        }
    }
    if let Err(err) = std::fs::create_dir_all(&bundle_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("create upload bundle dir failed: {err}")),
            }),
        );
    }

    let mut skill_md_path = None;
    for (rel_path, bytes) in uploaded_files {
        let normalized = rel_path
            .strip_prefix(&guessed_name)
            .ok()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or(rel_path);
        let target_path = bundle_dir.join(&normalized);
        if let Some(parent) = target_path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("create uploaded subdirectory failed: {err}")),
                    }),
                );
            }
        }
        if let Err(err) = std::fs::write(&target_path, bytes) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("write uploaded file failed: {err}")),
                }),
            );
        }
        if normalized
            .file_name()
            .and_then(|v| v.to_str())
            .map(|name| name.eq_ignore_ascii_case("SKILL.md"))
            .unwrap_or(false)
        {
            skill_md_path = Some(target_path);
        }
    }

    let skill_md_path = skill_md_path.unwrap_or_else(|| bundle_dir.join("SKILL.md"));
    let skill_md = match std::fs::read_to_string(&skill_md_path) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("uploaded bundle is missing readable SKILL.md: {err}")),
                }),
            );
        }
    };

    finalize_imported_bundle(
        &state,
        &bundle_dir,
        &bundle_rel_dir,
        &format!("upload:{guessed_name}"),
        enabled,
        &skill_md,
    )
}

#[derive(Debug, Deserialize)]
struct UpdateSkillsConfigRequest {
    #[serde(default)]
    skill_switches: HashMap<String, bool>,
}

#[derive(Debug, Deserialize)]
struct ImportSkillRequest {
    source: String,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateLlmConfigRequest {
    selected_vendor: String,
    selected_model: String,
    #[serde(default)]
    vendor_base_url: Option<String>,
    #[serde(default)]
    vendor_api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelConfigItem {
    vendor: String,
    model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct ModelConfigResponse {
    llm: ModelConfigItem,
    image_edit: ModelConfigItem,
    image_generation: ModelConfigItem,
    image_vision: ModelConfigItem,
    audio_transcribe: ModelConfigItem,
    audio_synthesize: ModelConfigItem,
    restart_required: bool,
}

#[derive(Debug, Deserialize)]
struct ModelConfigUpdateRequest {
    llm: Option<ModelConfigItem>,
    image_edit: Option<ModelConfigItem>,
    image_generation: Option<ModelConfigItem>,
    image_vision: Option<ModelConfigItem>,
    audio_transcribe: Option<ModelConfigItem>,
    audio_synthesize: Option<ModelConfigItem>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProviderKeysResponse {
    #[serde(default)]
    llm: HashMap<String, String>,
    #[serde(default)]
    image: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    audio: HashMap<String, HashMap<String, String>>,
}

fn default_model_item() -> ModelConfigItem {
    ModelConfigItem {
        vendor: String::new(),
        model: String::new(),
        base_url: None,
        api_key: None,
    }
}

fn read_model_config(state: &AppState) -> anyhow::Result<ModelConfigResponse> {
    let root = &state.workspace_root;

    let config_path = root.join("configs/config.toml");
    let config_raw = std::fs::read_to_string(&config_path).unwrap_or_else(|_| String::new());
    let config: toml::Value =
        toml::from_str(&config_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let llm = config
        .get("llm")
        .and_then(|t| t.as_table())
        .map(|t| ModelConfigItem {
            vendor: t
                .get("selected_vendor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            model: t
                .get("selected_model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            base_url: None,
            api_key: None,
        })
        .unwrap_or_else(default_model_item);

    let image_path = root.join("configs/image.toml");
    let image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let image: toml::Value =
        toml::from_str(&image_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    let read_image_section = |section: &str| -> ModelConfigItem {
        let sec = image.get(section).and_then(|t| t.as_table());
        let vendor = sec
            .and_then(|t| t.get("default_vendor").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        let model = sec
            .and_then(|t| t.get("default_model").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        let (base_url, api_key) = if vendor.is_empty() {
            (None, None)
        } else {
            let prov = image
                .get(section)
                .and_then(|t| t.get("providers"))
                .and_then(|p| p.get(&vendor))
                .and_then(|v| v.as_table());
            let base_url = prov
                .and_then(|t| t.get("base_url").and_then(|v| v.as_str()))
                .map(|s| s.to_string());
            let api_key = prov
                .and_then(|t| t.get("api_key").and_then(|v| v.as_str()))
                .map(mask_secret);
            (base_url, api_key)
        };
        ModelConfigItem {
            vendor,
            model,
            base_url,
            api_key,
        }
    };
    let image_edit = read_image_section("image_edit");
    let image_generation = read_image_section("image_generation");
    let image_vision = read_image_section("image_vision");

    let audio_path = root.join("configs/audio.toml");
    let audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let audio: toml::Value =
        toml::from_str(&audio_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    let read_audio_section = |section: &str| -> ModelConfigItem {
        let sec = audio.get(section).and_then(|t| t.as_table());
        let vendor = sec
            .and_then(|t| t.get("default_vendor").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        let model = sec
            .and_then(|t| t.get("default_model").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        let (base_url, api_key) = if vendor.is_empty() {
            (None, None)
        } else {
            let prov = audio
                .get(section)
                .and_then(|t| t.get("providers"))
                .and_then(|p| p.get(&vendor))
                .and_then(|v| v.as_table());
            let base_url = prov
                .and_then(|t| t.get("base_url").and_then(|v| v.as_str()))
                .map(|s| s.to_string());
            let api_key = prov
                .and_then(|t| t.get("api_key").and_then(|v| v.as_str()))
                .map(mask_secret);
            (base_url, api_key)
        };
        ModelConfigItem {
            vendor,
            model,
            base_url,
            api_key,
        }
    };
    let audio_transcribe = read_audio_section("audio_transcribe");
    let audio_synthesize = read_audio_section("audio_synthesize");

    Ok(ModelConfigResponse {
        llm,
        image_edit,
        image_generation,
        image_vision,
        audio_transcribe,
        audio_synthesize,
        restart_required: true,
    })
}

fn write_model_config(state: &AppState, req: &ModelConfigUpdateRequest) -> anyhow::Result<()> {
    let root = &state.workspace_root;

    if let Some(ref llm) = req.llm {
        let path = root.join("configs/config.toml");
        let raw = std::fs::read_to_string(&path)?;
        let updated_vendor = upsert_string_key_in_section(
            &raw,
            "llm",
            "selected_vendor",
            &format!("selected_vendor = {:?}", llm.vendor.trim()),
        );
        let updated = upsert_string_key_in_section(
            &updated_vendor,
            "llm",
            "selected_model",
            &format!("selected_model = {:?}", llm.model.trim()),
        );
        std::fs::write(&path, updated)?;
    }

    let image_path = root.join("configs/image.toml");
    let mut image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let mut image_modified = false;

    for (section, item) in [
        ("image_edit", req.image_edit.as_ref()),
        ("image_generation", req.image_generation.as_ref()),
        ("image_vision", req.image_vision.as_ref()),
    ] {
        if let Some(it) = item {
            image_modified = true;
            let vendor = it.vendor.trim();
            image_raw = upsert_string_key_in_section(
                &image_raw,
                section,
                "default_vendor",
                &format!("default_vendor = {:?}", vendor),
            );
            image_raw = upsert_string_key_in_section(
                &image_raw,
                section,
                "default_model",
                &format!("default_model = {:?}", it.model.trim()),
            );
            if !vendor.is_empty() {
                let provider_section = format!("{section}.providers.{vendor}");
                if let Some(ref u) = it.base_url {
                    let trimmed = u.trim();
                    image_raw = if trimmed.is_empty() {
                        remove_key_in_section(&image_raw, &provider_section, "base_url")
                    } else {
                        upsert_string_key_in_section(
                            &image_raw,
                            &provider_section,
                            "base_url",
                            &format!("base_url = {:?}", trimmed),
                        )
                    };
                }
                if let Some(ref k) = it.api_key {
                    let trimmed = k.trim();
                    if !trimmed.contains("***") {
                        image_raw = if trimmed.is_empty() {
                            remove_key_in_section(&image_raw, &provider_section, "api_key")
                        } else {
                            upsert_string_key_in_section(
                                &image_raw,
                                &provider_section,
                                "api_key",
                                &format!("api_key = {:?}", trimmed),
                            )
                        };
                    }
                }
            }
        }
    }
    if image_modified {
        std::fs::write(&image_path, image_raw)?;
    }

    let audio_path = root.join("configs/audio.toml");
    let mut audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let mut audio_modified = false;

    for (section, item) in [
        ("audio_transcribe", req.audio_transcribe.as_ref()),
        ("audio_synthesize", req.audio_synthesize.as_ref()),
    ] {
        if let Some(it) = item {
            audio_modified = true;
            let vendor = it.vendor.trim();
            audio_raw = upsert_string_key_in_section(
                &audio_raw,
                section,
                "default_vendor",
                &format!("default_vendor = {:?}", vendor),
            );
            audio_raw = upsert_string_key_in_section(
                &audio_raw,
                section,
                "default_model",
                &format!("default_model = {:?}", it.model.trim()),
            );
            if !vendor.is_empty() {
                let provider_section = format!("{section}.providers.{vendor}");
                if let Some(ref u) = it.base_url {
                    let trimmed = u.trim();
                    audio_raw = if trimmed.is_empty() {
                        remove_key_in_section(&audio_raw, &provider_section, "base_url")
                    } else {
                        upsert_string_key_in_section(
                            &audio_raw,
                            &provider_section,
                            "base_url",
                            &format!("base_url = {:?}", trimmed),
                        )
                    };
                }
                if let Some(ref k) = it.api_key {
                    let trimmed = k.trim();
                    if !trimmed.contains("***") {
                        audio_raw = if trimmed.is_empty() {
                            remove_key_in_section(&audio_raw, &provider_section, "api_key")
                        } else {
                            upsert_string_key_in_section(
                                &audio_raw,
                                &provider_section,
                                "api_key",
                                &format!("api_key = {:?}", trimmed),
                            )
                        };
                    }
                }
            }
        }
    }
    if audio_modified {
        std::fs::write(&audio_path, audio_raw)?;
    }

    Ok(())
}

fn read_llm_provider_keys(config: &toml::Value) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(llm) = config.get("llm").and_then(|v| v.as_table()) else {
        return out;
    };
    for (k, v) in llm {
        if let Some(tbl) = v.as_table() {
            if let Some(ak) = tbl.get("api_key").and_then(|a| a.as_str()) {
                out.insert(k.clone(), mask_secret(ak));
            }
        }
    }
    out
}

fn read_image_provider_keys(image: &toml::Value) -> HashMap<String, HashMap<String, String>> {
    let mut out = HashMap::new();
    for section in ["image_edit", "image_generation", "image_vision"] {
        let mut vendors = HashMap::new();
        if let Some(providers) = image
            .get(section)
            .and_then(|v| v.get("providers"))
            .and_then(|v| v.as_table())
        {
            for (vendor, tbl) in providers {
                if let Some(t) = tbl.as_table() {
                    if let Some(ak) = t.get("api_key").and_then(|a| a.as_str()) {
                        vendors.insert(vendor.clone(), mask_secret(ak));
                    }
                }
            }
        }
        out.insert(section.to_string(), vendors);
    }
    out
}

fn read_audio_provider_keys(audio: &toml::Value) -> HashMap<String, HashMap<String, String>> {
    let mut out = HashMap::new();
    for section in ["audio_synthesize", "audio_transcribe"] {
        let mut vendors = HashMap::new();
        if let Some(providers) = audio
            .get(section)
            .and_then(|v| v.get("providers"))
            .and_then(|v| v.as_table())
        {
            for (vendor, tbl) in providers {
                if let Some(t) = tbl.as_table() {
                    if let Some(ak) = t.get("api_key").and_then(|a| a.as_str()) {
                        vendors.insert(vendor.clone(), mask_secret(ak));
                    }
                }
            }
        }
        out.insert(section.to_string(), vendors);
    }
    out
}

fn read_provider_keys(state: &AppState) -> anyhow::Result<ProviderKeysResponse> {
    let root = &state.workspace_root;

    let config_path = root.join("configs/config.toml");
    let config_raw = std::fs::read_to_string(&config_path).unwrap_or_else(|_| String::new());
    let config: toml::Value =
        toml::from_str(&config_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let llm = read_llm_provider_keys(&config);

    let image_path = root.join("configs/image.toml");
    let image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let image: toml::Value =
        toml::from_str(&image_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let image_keys = read_image_provider_keys(&image);

    let audio_path = root.join("configs/audio.toml");
    let audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let audio: toml::Value =
        toml::from_str(&audio_raw).unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let audio_keys = read_audio_provider_keys(&audio);

    Ok(ProviderKeysResponse {
        llm,
        image: image_keys,
        audio: audio_keys,
    })
}

fn write_provider_keys(state: &AppState, req: &ProviderKeysResponse) -> anyhow::Result<()> {
    let root = &state.workspace_root;

    if !req.llm.is_empty() {
        let path = root.join("configs/config.toml");
        let raw = std::fs::read_to_string(&path)?;
        let mut updated = raw;
        for (vendor, new_key) in &req.llm {
            let section = format!("llm.{}", vendor.trim());
            let trimmed = new_key.trim();
            updated = if trimmed.is_empty() {
                remove_key_in_section(&updated, &section, "api_key")
            } else {
                upsert_string_key_in_section(
                    &updated,
                    &section,
                    "api_key",
                    &format!("api_key = {:?}", trimmed),
                )
            };
        }
        std::fs::write(&path, updated)?;
    }

    if !req.image.is_empty() {
        let path = root.join("configs/image.toml");
        let mut updated = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
        for (section, vendors) in &req.image {
            for (vendor, new_key) in vendors {
                let provider_section = format!("{}.providers.{}", section, vendor.trim());
                let trimmed = new_key.trim();
                updated = if trimmed.is_empty() {
                    remove_key_in_section(&updated, &provider_section, "api_key")
                } else {
                    upsert_string_key_in_section(
                        &updated,
                        &provider_section,
                        "api_key",
                        &format!("api_key = {:?}", trimmed),
                    )
                };
            }
        }
        std::fs::write(&path, updated)?;
    }

    if !req.audio.is_empty() {
        let path = root.join("configs/audio.toml");
        let mut updated = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
        for (section, vendors) in &req.audio {
            for (vendor, new_key) in vendors {
                let provider_section = format!("{}.providers.{}", section, vendor.trim());
                let trimmed = new_key.trim();
                updated = if trimmed.is_empty() {
                    remove_key_in_section(&updated, &provider_section, "api_key")
                } else {
                    upsert_string_key_in_section(
                        &updated,
                        &provider_section,
                        "api_key",
                        &format!("api_key = {:?}", trimmed),
                    )
                };
            }
        }
        std::fs::write(&path, updated)?;
    }

    Ok(())
}

async fn get_model_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<ModelConfigResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    match read_model_config(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read model config failed: {err}")),
            }),
        ),
    }
}

async fn update_model_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ModelConfigUpdateRequest>,
) -> (StatusCode, Json<ApiResponse<ModelConfigResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    if let Err(err) = write_model_config(&state, &req) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write model config failed: {err}")),
            }),
        );
    }
    match read_model_config(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: None,
                error: Some(format!("saved but re-read failed: {err}")),
            }),
        ),
    }
}

async fn get_provider_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<ProviderKeysResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    match read_provider_keys(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("read provider keys failed: {err}")),
            }),
        ),
    }
}

async fn update_provider_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ProviderKeysResponse>,
) -> (StatusCode, Json<ApiResponse<ProviderKeysResponse>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return (
            resp.0,
            Json(ApiResponse {
                ok: resp.1.ok,
                data: None,
                error: resp.1.error.clone(),
            }),
        );
    }
    if let Err(err) = write_provider_keys(&state, &req) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write provider keys failed: {err}")),
            }),
        );
    }
    match read_provider_keys(&state) {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: None,
                error: Some(format!("saved but re-read failed: {err}")),
            }),
        ),
    }
}

async fn restart_clawd(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let workspace = state.workspace_root.to_string_lossy();
    let pid = std::process::id();
    let script = format!(
        "sleep 2; kill {pid} 2>/dev/null; sleep 1; cd {workspace} && ./start-clawd.sh"
    );
    let mut cmd = StdCommand::new("nohup");
    cmd.arg("bash")
        .arg("-c")
        .arg(&script)
        .current_dir(&state.workspace_root)
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null());
    match cmd.spawn() {
        Ok(_) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "message": "restart triggered; clawd will restart in a few seconds",
                    "restart_triggered": true
                })),
                error: None,
            }),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("failed to spawn restart process: {err}")),
            }),
        ),
    }
}

fn read_skill_config_file(state: &AppState) -> anyhow::Result<(String, toml::Value)> {
    let path = state.workspace_root.join("configs/config.toml");
    let raw = std::fs::read_to_string(&path)?;
    let parsed = toml::from_str::<toml::Value>(&raw)?;
    Ok((raw, parsed))
}

fn write_runtime_config_file(state: &AppState, raw: &str) -> std::io::Result<()> {
    let active_path = state.workspace_root.join("configs/config.toml");
    std::fs::write(&active_path, raw)?;

    let mounted_path = state.workspace_root.join("docker/config/config.toml");
    if let Some(parent) = mounted_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&mounted_path, raw)?;
    Ok(())
}

fn read_skills_registry_file(state: &AppState) -> std::io::Result<String> {
    let path = state.workspace_root.join("configs/skills_registry.toml");
    match std::fs::read_to_string(path) {
        Ok(raw) => Ok(raw),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    }
}

fn write_skills_registry_file(state: &AppState, raw: &str) -> std::io::Result<()> {
    let active_path = state.workspace_root.join("configs/skills_registry.toml");
    if let Some(parent) = active_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&active_path, raw)?;

    let mounted_path = state
        .workspace_root
        .join("docker/config/skills_registry.toml");
    if let Some(parent) = mounted_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&mounted_path, raw)?;
    Ok(())
}

#[derive(Debug, Default)]
struct ParsedSkillFrontmatter {
    name: String,
    description: String,
    metadata: Option<Value>,
}

#[derive(Debug)]
struct ImportedSkillPlan {
    canonical_name: String,
    display_name: String,
    description: String,
    external_kind: String,
    aliases: Vec<String>,
    prompt_rel_path: String,
    bundle_rel_dir: String,
    entry_file: String,
    runtime: Option<String>,
    require_bins: Vec<String>,
    require_py_modules: Vec<String>,
    source_url: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct UninstallExternalSkillRequest {
    skill_name: String,
}

fn normalize_remote_skill_source(source: &str) -> String {
    let trimmed = source.trim();
    if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        if let Some((repo_part, path_part)) = rest.split_once("/blob/") {
            if let Some((branch, file_path)) = path_part.split_once('/') {
                return format!(
                    "https://raw.githubusercontent.com/{repo_part}/{branch}/{file_path}"
                );
            }
        }
    }
    trimmed.to_string()
}

fn slugify_skill_name(input: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in input.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };
        if mapped == '_' {
            if out.is_empty() || last_was_sep {
                continue;
            }
            last_was_sep = true;
            out.push('_');
        } else {
            last_was_sep = false;
            out.push(mapped);
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "external_skill".to_string()
    } else if out.chars().next().unwrap_or('a').is_ascii_digit() {
        format!("ext_{out}")
    } else {
        out
    }
}

fn parse_skill_frontmatter(skill_md: &str) -> ParsedSkillFrontmatter {
    let mut parsed = ParsedSkillFrontmatter::default();
    let mut lines = skill_md.lines();
    if lines.next().map(str::trim) != Some("---") {
        return parsed;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if trimmed.is_empty() {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key {
            "name" => parsed.name = value.to_string(),
            "description" => parsed.description = value.to_string(),
            "metadata" => {
                if let Ok(meta) = serde_json::from_str::<Value>(value) {
                    parsed.metadata = Some(meta);
                }
            }
            _ => {}
        }
    }
    parsed
}

fn scan_bundle_files(root: &Path, current: &Path, acc: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            scan_bundle_files(root, &path, acc)?;
            continue;
        }
        if file_type.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            acc.push(rel);
        }
    }
    Ok(())
}

fn extract_required_bins(metadata: Option<&Value>) -> Vec<String> {
    let mut bins = Vec::new();
    let sources = [
        metadata,
        metadata.and_then(|m| m.get("openclaw")),
        metadata
            .and_then(|m| m.get("openclaw"))
            .and_then(|m| m.get("requires")),
    ];
    for source in sources.into_iter().flatten() {
        if let Some(arr) = source.get("bins").and_then(|v| v.as_array()) {
            for item in arr.iter().filter_map(|v| v.as_str()) {
                let item = item.trim();
                if !item.is_empty() && !bins.iter().any(|existing| existing == item) {
                    bins.push(item.to_string());
                }
            }
        }
    }
    bins
}

fn infer_python_modules(script_path: &Path) -> Vec<String> {
    let mut modules = Vec::new();
    let Ok(raw) = std::fs::read_to_string(script_path) else {
        return modules;
    };
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("import ") {
            for item in rest.split(',') {
                let name = item
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .trim();
                if name == "akshare" && !modules.iter().any(|m| m == name) {
                    modules.push(name.to_string());
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("from ") {
            let name = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("")
                .trim();
            if name == "akshare" && !modules.iter().any(|m| m == name) {
                modules.push(name.to_string());
            }
        }
    }
    modules
}

fn detect_import_plan(
    skill_md: &str,
    bundle_dir: &Path,
    bundle_rel_dir: &str,
    source: &str,
    enabled: bool,
) -> anyhow::Result<ImportedSkillPlan> {
    let frontmatter = parse_skill_frontmatter(skill_md);
    let mut files = Vec::new();
    scan_bundle_files(bundle_dir, bundle_dir, &mut files)?;
    files.sort();

    let display_name = if !frontmatter.name.trim().is_empty() {
        frontmatter.name.trim().to_string()
    } else {
        bundle_dir
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("external-skill")
            .to_string()
    };
    let canonical_name = slugify_skill_name(&display_name);
    let mut aliases = Vec::new();
    let alias = display_name.trim().to_ascii_lowercase();
    if !alias.is_empty() && alias != canonical_name {
        aliases.push(alias);
    }

    let mut require_bins = extract_required_bins(frontmatter.metadata.as_ref());
    let mut require_py_modules = Vec::new();
    let mut external_kind = "prompt_bundle".to_string();
    let mut entry_file = "SKILL.md".to_string();
    let mut runtime = None;

    let first_python = files.iter().find(|path| path.ends_with(".py")).cloned();
    let first_node = files
        .iter()
        .find(|path| path.ends_with(".js") || path.ends_with(".mjs") || path.ends_with(".cjs"))
        .cloned();
    if let Some(py_entry) = first_python {
        external_kind = "local_script".to_string();
        entry_file = py_entry.clone();
        runtime = Some("python3".to_string());
        if !require_bins.iter().any(|item| item == "python3") {
            require_bins.push("python3".to_string());
        }
        require_py_modules = infer_python_modules(&bundle_dir.join(&py_entry));
    } else if let Some(node_entry) = first_node {
        external_kind = "local_script".to_string();
        entry_file = node_entry;
        runtime = Some("node".to_string());
        if !require_bins.iter().any(|item| item == "node") {
            require_bins.push("node".to_string());
        }
    } else if skill_md.contains("```bash")
        || skill_md.contains("```sh")
        || !require_bins.is_empty()
        || skill_md.contains("curl ")
        || skill_md.contains("jq ")
    {
        external_kind = "local_shell_recipe".to_string();
    }

    let description = if !frontmatter.description.trim().is_empty() {
        frontmatter.description.trim().to_string()
    } else {
        "Imported external skill".to_string()
    };
    let prompt_rel_path = format!("prompts/skills/{canonical_name}.md");
    Ok(ImportedSkillPlan {
        canonical_name,
        display_name,
        description,
        external_kind,
        aliases,
        prompt_rel_path,
        bundle_rel_dir: bundle_rel_dir.to_string(),
        entry_file,
        runtime,
        require_bins,
        require_py_modules,
        source_url: source.to_string(),
        enabled,
    })
}

fn render_string_array(items: &[String]) -> String {
    if items.is_empty() {
        "[]".to_string()
    } else {
        let body = items
            .iter()
            .map(|item| format!("{item:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{body}]")
    }
}

fn render_imported_skill_registry_block(plan: &ImportedSkillPlan) -> String {
    let mut lines = Vec::new();
    lines.push("[[skills]]".to_string());
    lines.push(format!("name = {:?}", plan.canonical_name));
    lines.push(format!("enabled = {}", plan.enabled));
    lines.push("kind = \"external\"".to_string());
    lines.push(format!("aliases = {}", render_string_array(&plan.aliases)));
    lines.push("timeout_seconds = 60".to_string());
    lines.push(format!("prompt_file = {:?}", plan.prompt_rel_path));
    lines.push("output_kind = \"text\"".to_string());
    lines.push(format!("external_kind = {:?}", plan.external_kind));
    lines.push(format!("external_bundle_dir = {:?}", plan.bundle_rel_dir));
    lines.push(format!("external_entry_file = {:?}", plan.entry_file));
    if let Some(runtime) = &plan.runtime {
        lines.push(format!("external_runtime = {:?}", runtime));
    }
    lines.push(format!(
        "external_require_bins = {}",
        render_string_array(&plan.require_bins)
    ));
    lines.push(format!(
        "external_require_py_modules = {}",
        render_string_array(&plan.require_py_modules)
    ));
    lines.push(format!("external_source_url = {:?}", plan.source_url));
    lines.join("\n")
}

fn render_imported_skill_prompt(plan: &ImportedSkillPlan, skill_md: &str) -> String {
    let normalized_skill_md = skill_md.trim();
    let mut out = String::new();
    out.push_str("<!-- AUTO-GENERATED: external skill importer -->\n");
    out.push_str(&format!("# {}\n\n", plan.display_name));
    out.push_str("RustClaw imported external skill wrapper.\n\n");
    out.push_str("## RustClaw Wrapper\n");
    out.push_str(&format!(
        "- This is an imported external skill: `{}`.\n",
        plan.display_name
    ));
    out.push_str(&format!("- Description: {}\n", plan.description));
    out.push_str(&format!("- Runtime mode: `{}`\n", plan.external_kind));
    out.push_str(&format!("- Bundle directory: `{}`\n", plan.bundle_rel_dir));
    out.push_str(&format!("- Entry file: `{}`\n", plan.entry_file));
    if let Some(runtime) = &plan.runtime {
        out.push_str(&format!("- Runtime binary: `{runtime}`\n"));
    }
    if !plan.require_bins.is_empty() {
        out.push_str(&format!(
            "- Required local commands: {}\n",
            plan.require_bins.join(", ")
        ));
    }
    if !plan.require_py_modules.is_empty() {
        out.push_str(&format!(
            "- Required Python packages: {}\n",
            plan.require_py_modules.join(", ")
        ));
    }
    out.push_str(&format!("- Source: `{}`\n", plan.source_url));
    out.push_str("\n## Calling Rules\n");
    out.push_str("- Prefer the original `SKILL.md` below over your own guesses.\n");
    out.push_str(
        "- Follow the documented commands, options, examples, and parameter names from the original `SKILL.md` exactly.\n",
    );
    out.push_str(
        "- Do not invent unsupported CLI flags, JSON fields, shell fragments, or action names that are not grounded in the original `SKILL.md` or the entry file.\n",
    );
    match plan.external_kind.as_str() {
        "local_script" => {
            out.push_str(
                "- This skill runs a local script. Stay close to the script's real supported options and examples from the original `SKILL.md`.\n",
            );
            out.push_str(
                "- If the original `SKILL.md` shows a concrete command example, mirror that option shape instead of inventing a higher-level parameter.\n",
            );
        }
        "local_shell_recipe" => {
            out.push_str(
                "- This skill runs shell recipes inside its bundle directory.\n",
            );
            out.push_str(
                "- Keep the command close to the examples shown in the original `SKILL.md`.\n",
            );
            out.push_str(
                "- Prefer short, explicit commands. Reuse the documented recipes instead of inventing unrelated shell pipelines.\n",
            );
        }
        _ => {
            out.push_str(
                "- This prompt file lets the imported skill appear in RustClaw immediately.\n",
            );
            out.push_str(
                "- Runtime execution may still require a dedicated executor for this external kind.\n",
            );
        }
    }
    out.push_str(
        "- Avoid adding internal metadata fields yourself; RustClaw will inject its own runtime context.\n",
    );
    if !normalized_skill_md.is_empty() {
        out.push_str("\n## Original SKILL.md\n\n");
        out.push_str(normalized_skill_md);
        out.push('\n');
    }
    out
}

fn parse_registry_block_name(block: &[&str]) -> Option<String> {
    for line in block {
        let trimmed = line.trim();
        if !trimmed.starts_with("name") {
            continue;
        }
        let Some((lhs, rhs)) = trimmed.split_once('=') else {
            continue;
        };
        if lhs.trim() != "name" {
            continue;
        }
        let rhs = rhs.trim();
        let parsed = toml::from_str::<toml::Value>(&format!("value = {rhs}")).ok()?;
        let value = parsed.get("value")?.as_str()?.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn remove_skill_registry_block(raw: &str, skill_name: &str) -> (String, bool) {
    let mut out: Vec<String> = Vec::new();
    let lines: Vec<&str> = raw.lines().collect();
    let mut idx = 0usize;
    let mut removed = false;
    while idx < lines.len() {
        if lines[idx].trim() != "[[skills]]" {
            out.push(lines[idx].to_string());
            idx += 1;
            continue;
        }
        let start = idx;
        idx += 1;
        while idx < lines.len() && lines[idx].trim() != "[[skills]]" {
            idx += 1;
        }
        let block = &lines[start..idx];
        let block_name = parse_registry_block_name(block)
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default();
        if block_name == skill_name {
            removed = true;
            continue;
        }
        out.extend(block.iter().map(|line| (*line).to_string()));
    }
    let mut rendered = out.join("\n");
    if raw.ends_with('\n') {
        rendered.push('\n');
    }
    (rendered, removed)
}

fn remove_managed_prompt_file(path: &Path) -> std::io::Result<bool> {
    let raw = match std::fs::read_to_string(path) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    if raw.contains("<!-- AUTO-GENERATED: external skill importer -->") {
        std::fs::remove_file(path)?;
        return Ok(true);
    }
    Ok(false)
}

fn remove_runtime_skill_switch(raw: &str, state: &AppState, skill_name: &str) -> String {
    let parsed = toml::from_str::<toml::Value>(raw).unwrap_or_else(|_| toml::Value::Table(Default::default()));
    let mut switches = collect_skill_switches(&parsed, state);
    switches.remove(skill_name);
    let rendered = render_switches_inline_table(&switches);
    upsert_skill_switches_line(raw, &rendered)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else if file_type.is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn sanitize_upload_relative_path(input: &str) -> Option<PathBuf> {
    let trimmed = input.trim().replace('\\', "/");
    if trimmed.is_empty() {
        return None;
    }
    let path = Path::new(&trimmed);
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => out.push(part),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn guess_bundle_name_from_path_or_source(source: &str, fallback: &str) -> String {
    let source_hint = Path::new(source);
    let mut raw_name = source_hint
        .file_name()
        .and_then(|v| v.to_str())
        .map(|v| v.trim_end_matches(".md"))
        .map(|v| v.trim_end_matches(".git"))
        .filter(|v| !v.is_empty())
        .unwrap_or(fallback)
        .to_string();
    if raw_name.eq_ignore_ascii_case("skill") {
        if let Some(parent_name) = source_hint
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|v| v.to_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            raw_name = parent_name.to_string();
        }
    }
    raw_name
}

fn finalize_imported_bundle(
    state: &AppState,
    bundle_dir: &Path,
    bundle_rel_dir: &str,
    source: &str,
    enabled: bool,
    skill_md: &str,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let plan = match detect_import_plan(skill_md, bundle_dir, bundle_rel_dir, source, enabled) {
        Ok(plan) => plan,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("analyze imported skill failed: {err}")),
                }),
            );
        }
    };

    let prompt_path = state.workspace_root.join(&plan.prompt_rel_path);
    if let Some(parent) = prompt_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("create prompt directory failed: {err}")),
                }),
            );
        }
    }
    if let Err(err) = std::fs::write(&prompt_path, render_imported_skill_prompt(&plan, skill_md)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write prompt file failed: {err}")),
            }),
        );
    }

    let mut registry_raw = match read_skills_registry_file(state) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills registry failed: {err}")),
                }),
            );
        }
    };
    if !registry_raw.ends_with('\n') && !registry_raw.is_empty() {
        registry_raw.push('\n');
    }
    registry_raw.push('\n');
    registry_raw.push_str(&render_imported_skill_registry_block(&plan));
    registry_raw.push('\n');
    if let Err(err) = write_skills_registry_file(state, &registry_raw) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills registry failed: {err}")),
            }),
        );
    }

    let reload = match reload_skill_views(state) {
        Ok(result) => result,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("reload skill views failed: {err}")),
                }),
            );
        }
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "skill_name": plan.canonical_name,
                "display_name": plan.display_name,
                "description": plan.description,
                "external_kind": plan.external_kind,
                "bundle_dir": plan.bundle_rel_dir,
                "entry_file": plan.entry_file,
                "runtime": plan.runtime,
                "require_bins": plan.require_bins,
                "require_py_modules": plan.require_py_modules,
                "prompt_file": plan.prompt_rel_path,
                "source": plan.source_url,
                "reload": reload
            })),
            error: None,
        }),
    )
}

async fn materialize_import_source(
    state: &AppState,
    source: &str,
    dest_dir: &Path,
) -> Result<String, String> {
    let normalized = normalize_remote_skill_source(source);
    let src_path = Path::new(&normalized);
    if src_path.exists() {
        if src_path.is_dir() {
            copy_dir_recursive(src_path, dest_dir)
                .map_err(|err| format!("copy local bundle failed: {err}"))?;
            let skill_md = dest_dir.join("SKILL.md");
            return std::fs::read_to_string(&skill_md)
                .map_err(|err| format!("read copied SKILL.md failed: {err}"));
        }
        if src_path.is_file() {
            std::fs::create_dir_all(dest_dir)
                .map_err(|err| format!("create import dir failed: {err}"))?;
            std::fs::copy(src_path, dest_dir.join("SKILL.md"))
                .map_err(|err| format!("copy local SKILL.md failed: {err}"))?;
            return std::fs::read_to_string(dest_dir.join("SKILL.md"))
                .map_err(|err| format!("read copied SKILL.md failed: {err}"));
        }
    }

    let res = state
        .http_client
        .get(&normalized)
        .send()
        .await
        .map_err(|err| format!("download skill source failed: {err}"))?;
    let status = res.status();
    let body = res
        .text()
        .await
        .map_err(|err| format!("read skill source body failed: {err}"))?;
    if !status.is_success() {
        return Err(format!(
            "download skill source returned {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }
    std::fs::create_dir_all(dest_dir).map_err(|err| format!("create import dir failed: {err}"))?;
    std::fs::write(dest_dir.join("SKILL.md"), &body)
        .map_err(|err| format!("write downloaded SKILL.md failed: {err}"))?;
    Ok(body)
}

fn upsert_string_key_in_section(
    raw: &str,
    section_name: &str,
    key: &str,
    rendered_line: &str,
) -> String {
    let mut lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();
    let section_header = format!("[{section_name}]");
    let mut in_section = false;
    let mut section_seen = false;
    let mut inserted_or_replaced = false;
    let mut insert_index_in_section: Option<usize> = None;
    let mut section_end: Option<usize> = None;

    for idx in 0..lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed == section_header {
            in_section = true;
            section_seen = true;
            insert_index_in_section = Some(idx + 1);
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed != section_header {
            if in_section {
                section_end = Some(idx);
                break;
            }
            continue;
        }
        if in_section && trimmed.starts_with(key) && trimmed.contains('=') {
            lines[idx] = rendered_line.to_string();
            inserted_or_replaced = true;
            break;
        }
    }

    if !inserted_or_replaced && section_seen {
        let idx = insert_index_in_section.or(section_end).unwrap_or(lines.len());
        lines.insert(idx, rendered_line.to_string());
    } else if !inserted_or_replaced {
        if !lines.is_empty() && !lines.last().map(|s| s.trim().is_empty()).unwrap_or(false) {
            lines.push(String::new());
        }
        lines.push(section_header);
        lines.push(rendered_line.to_string());
    }

    let mut out = lines.join("\n");
    if raw.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn remove_key_in_section(raw: &str, section_name: &str, key: &str) -> String {
    let mut lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();
    let section_header = format!("[{section_name}]");
    let mut in_section = false;
    let mut remove_index: Option<usize> = None;

    for idx in 0..lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed == section_header {
            in_section = true;
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed != section_header {
            if in_section {
                break;
            }
            continue;
        }
        if in_section && trimmed.starts_with(key) && trimmed.contains('=') {
            remove_index = Some(idx);
            break;
        }
    }

    if let Some(idx) = remove_index {
        lines.remove(idx);
    }

    let mut out = lines.join("\n");
    if raw.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn llm_vendor_names() -> [&'static str; 8] {
    [
        "openai",
        "google",
        "anthropic",
        "grok",
        "deepseek",
        "qwen",
        "minimax",
        "custom",
    ]
}

fn collect_llm_vendor_info(value: &toml::Value) -> Vec<Value> {
    let mut vendors = Vec::new();
    let Some(llm) = value.get("llm").and_then(|v| v.as_table()) else {
        return vendors;
    };
    for vendor_name in llm_vendor_names() {
        let Some(vendor) = llm.get(vendor_name).and_then(|v| v.as_table()) else {
            continue;
        };
        let base_url = vendor
            .get("base_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let default_model = vendor
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let api_key_configured = vendor
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let api_key_masked = vendor
            .get("api_key")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(mask_secret);
        let mut models = vendor
            .get("models")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !default_model.is_empty() && !models.iter().any(|m| m == &default_model) {
            models.insert(0, default_model.clone());
        }
        vendors.push(json!({
            "name": vendor_name,
            "default_model": default_model,
            "models": models,
            "base_url": base_url,
            "api_key_configured": api_key_configured,
            "api_key_masked": api_key_masked
        }));
    }
    vendors
}

fn current_runtime_llm_info(state: &AppState) -> Value {
    if let Some(provider) = state.llm_providers.first() {
        let vendor = provider
            .config
            .name
            .strip_prefix("vendor-")
            .unwrap_or(provider.config.name.as_str())
            .to_string();
        return json!({
            "vendor": vendor,
            "model": provider.config.model,
            "provider_name": provider.config.name,
            "provider_type": provider.config.provider_type
        });
    }
    json!(null)
}

fn llm_restart_required(state: &AppState, selected_vendor: &str, selected_model: &str) -> bool {
    let runtime = current_runtime_llm_info(state);
    let runtime_vendor = runtime
        .get("vendor")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let runtime_model = runtime
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    runtime_vendor != selected_vendor.trim() || runtime_model != selected_model.trim()
}

fn skills_restart_required(runtime_visible: &[String], effective_visible: &[String]) -> bool {
    let mut runtime_sorted = runtime_visible.to_vec();
    runtime_sorted.sort_unstable();
    let mut effective_sorted = effective_visible.to_vec();
    effective_sorted.sort_unstable();
    runtime_sorted != effective_sorted
}

fn collect_skills_baseline(value: &toml::Value, state: &AppState) -> Vec<String> {
    value
        .get("skills")
        .and_then(|v| v.get("skills_list"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| state.resolve_canonical_skill_name(s))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn collect_skill_switches(value: &toml::Value, state: &AppState) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    let Some(tbl) = value
        .get("skills")
        .and_then(|v| v.get("skill_switches"))
        .and_then(|v| v.as_table())
    else {
        return out;
    };
    for (k, v) in tbl {
        let canonical = state.resolve_canonical_skill_name(k);
        if hide_skill_in_ui(state, &canonical) {
            continue;
        }
        if let Some(b) = v.as_bool() {
            out.insert(canonical, b);
        }
    }
    out
}

fn compute_effective_enabled(
    baseline: &[String],
    switches: &BTreeMap<String, bool>,
    state: &AppState,
) -> Vec<String> {
    let mut set: BTreeMap<String, bool> = BTreeMap::new();
    for skill in baseline {
        set.insert(state.resolve_canonical_skill_name(skill), true);
    }
    if let Some(registry) = state.get_skills_registry() {
        for skill in registry.enabled_names() {
            set.insert(state.resolve_canonical_skill_name(&skill), true);
        }
    }
    for (k, v) in switches {
        if *v {
            set.insert(state.resolve_canonical_skill_name(k), true);
        } else {
            set.remove(&state.resolve_canonical_skill_name(k));
        }
    }
    set.into_keys().collect()
}

fn render_switches_inline_table(switches: &BTreeMap<String, bool>) -> String {
    if switches.is_empty() {
        return "skill_switches = {}".to_string();
    }
    let pairs = switches
        .iter()
        .map(|(k, v)| format!("{k} = {v}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("skill_switches = {{ {pairs} }}")
}

fn upsert_skill_switches_line(raw: &str, rendered_line: &str) -> String {
    let mut lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();
    let mut in_skills = false;
    let mut inserted_or_replaced = false;
    let mut skills_section_seen = false;
    let mut insert_index_in_skills: Option<usize> = None;
    let mut skills_section_end: Option<usize> = None;

    for idx in 0..lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed == "[skills]" {
            in_skills = true;
            skills_section_seen = true;
            insert_index_in_skills = Some(idx + 1);
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed != "[skills]" {
            if in_skills {
                skills_section_end = Some(idx);
                break;
            }
            continue;
        }
        if in_skills && trimmed.starts_with("skill_switches") && trimmed.contains('=') {
            lines[idx] = rendered_line.to_string();
            inserted_or_replaced = true;
            break;
        }
        if in_skills && insert_index_in_skills.is_none() && !trimmed.is_empty() {
            insert_index_in_skills = Some(idx);
        }
        if in_skills && trimmed.starts_with("skills_list") && insert_index_in_skills.is_none() {
            insert_index_in_skills = Some(idx);
        }
    }

    if !inserted_or_replaced && skills_section_seen {
        let idx = insert_index_in_skills
            .or(skills_section_end)
            .unwrap_or(lines.len());
        lines.insert(idx, rendered_line.to_string());
    }

    let mut out = lines.join("\n");
    if raw.ends_with('\n') {
        out.push('\n');
    }
    out
}

async fn get_skills_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let parsed = match read_skill_config_file(&state) {
        Ok((_, v)) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills config failed: {err}")),
                }),
            );
        }
    };
    let baseline = collect_skills_baseline(&parsed, &state);
    let switches = collect_skill_switches(&parsed, &state);
    let mut baseline_visible = baseline
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    baseline_visible.sort_unstable();
    let mut runtime_visible = state
        .get_skills_list()
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    runtime_visible.sort_unstable();
    let managed = {
        let mut set: BTreeMap<String, bool> = BTreeMap::new();
        for s in &baseline_visible {
            set.insert(s.clone(), true);
        }
        for s in switches.keys() {
            set.insert(s.clone(), true);
        }
        for s in runtime_visible.iter() {
            set.insert(s.clone(), true);
        }
        set.into_keys().collect::<Vec<_>>()
    };
    let mut effective = compute_effective_enabled(&baseline, &switches, &state);
    effective.retain(|s| !hide_skill_in_ui(&state, s));
    let restart_required = skills_restart_required(&runtime_visible, &effective);
    let base_skill_names: Vec<String> = claw_core::config::base_skill_names()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let external_skill_names = state
        .get_skills_registry()
        .as_ref()
        .map(|registry| {
            registry
                .all_names()
                .into_iter()
                .filter(|name| {
                    !hide_skill_in_ui(&state, name)
                        && registry
                            .get(name)
                            .map(|entry| entry.kind == SkillKind::External)
                            .unwrap_or(false)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "skills_list": baseline_visible,
                "skill_switches": switches,
                "managed_skills": managed,
                "base_skill_names": base_skill_names,
                "external_skill_names": external_skill_names,
                "effective_enabled_skills_preview": effective,
                "runtime_enabled_skills": runtime_visible,
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn get_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let parsed = match read_skill_config_file(&state) {
        Ok((_, v)) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read llm config failed: {err}")),
                }),
            );
        }
    };
    let llm = parsed.get("llm").and_then(|v| v.as_table());
    let selected_vendor = llm
        .and_then(|tbl| tbl.get("selected_vendor"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let selected_model = llm
        .and_then(|tbl| tbl.get("selected_model"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let vendors = collect_llm_vendor_info(&parsed);
    let restart_required = llm_restart_required(&state, &selected_vendor, &selected_model);
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "selected_vendor": selected_vendor,
                "selected_model": selected_model,
                "vendors": vendors,
                "runtime": current_runtime_llm_info(&state),
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn update_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateLlmConfigRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let selected_vendor = req.selected_vendor.trim().to_ascii_lowercase();
    let selected_model = req.selected_model.trim().to_string();
    if selected_vendor.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("selected_vendor is required".to_string()),
            }),
        );
    }
    if selected_model.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("selected_model is required".to_string()),
            }),
        );
    }

    let (raw, parsed) = match read_skill_config_file(&state) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read llm config failed: {err}")),
                }),
            );
        }
    };
    let vendors = collect_llm_vendor_info(&parsed);
    let Some(vendor_info) = vendors.iter().find(|item| {
        item.get("name")
            .and_then(|v| v.as_str())
            .map(|name| name.eq_ignore_ascii_case(&selected_vendor))
            .unwrap_or(false)
    }) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("unsupported vendor: {selected_vendor}")),
            }),
        );
    };

    let allowed_models = vendor_info
        .get("models")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if selected_vendor != "custom"
        && !allowed_models.is_empty()
        && !allowed_models.iter().any(|m| m == &selected_model)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("model is not in the configured pool for vendor {selected_vendor}: {selected_model}")),
            }),
        );
    }

    let vendor_base_url = req.vendor_base_url.as_deref().map(str::trim).unwrap_or("");
    if vendor_base_url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("vendor_base_url is required".to_string()),
            }),
        );
    }

    let updated_vendor = upsert_string_key_in_section(
        &raw,
        "llm",
        "selected_vendor",
        &format!("selected_vendor = {:?}", selected_vendor),
    );
    let updated_raw = upsert_string_key_in_section(
        &updated_vendor,
        "llm",
        "selected_model",
        &format!("selected_model = {:?}", selected_model),
    );
    let updated_vendor_base_url = upsert_string_key_in_section(
        &updated_raw,
        &format!("llm.{selected_vendor}"),
        "base_url",
        &format!("base_url = {:?}", vendor_base_url),
    );
    let updated_vendor_model = upsert_string_key_in_section(
        &updated_vendor_base_url,
        &format!("llm.{selected_vendor}"),
        "model",
        &format!("model = {:?}", selected_model),
    );
    let vendor_api_key = req.vendor_api_key.as_deref().map(str::trim).unwrap_or("");
    let final_updated = if vendor_api_key.is_empty() {
        remove_key_in_section(
            &updated_vendor_model,
            &format!("llm.{selected_vendor}"),
            "api_key",
        )
    } else {
        upsert_string_key_in_section(
            &updated_vendor_model,
            &format!("llm.{selected_vendor}"),
            "api_key",
            &format!("api_key = {:?}", vendor_api_key),
        )
    };
    if let Err(err) = write_runtime_config_file(&state, &final_updated) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write llm config failed: {err}")),
            }),
        );
    }
    let restart_required = llm_restart_required(&state, &selected_vendor, &selected_model);

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "selected_vendor": selected_vendor,
                "selected_model": selected_model,
                "runtime": current_runtime_llm_info(&state),
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn update_skills_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateSkillsConfigRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let (raw, parsed) = match read_skill_config_file(&state) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills config failed: {err}")),
                }),
            );
        }
    };
    let baseline = collect_skills_baseline(&parsed, &state);
    let core_skills = claw_core::config::core_skills_always_enabled();
    let mut switches = BTreeMap::new();
    for (k, v) in req.skill_switches {
        let skill = state.resolve_canonical_skill_name(k.trim());
        if skill.is_empty() || hide_skill_in_ui(&state, &skill) {
            continue;
        }
        let is_core = core_skills.iter().any(|s| *s == skill);
        switches.insert(skill, if is_core { true } else { v });
    }
    let rendered = render_switches_inline_table(&switches);
    let updated = upsert_skill_switches_line(&raw, &rendered);
    if let Err(err) = write_runtime_config_file(&state, &updated) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills config failed: {err}")),
            }),
        );
    }
    let effective = compute_effective_enabled(&baseline, &switches, &state);
    let mut effective_visible = effective
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    effective_visible.sort_unstable();
    let mut runtime_visible = state
        .get_skills_list()
        .iter()
        .filter(|s| !hide_skill_in_ui(&state, s))
        .cloned()
        .collect::<Vec<_>>();
    runtime_visible.sort_unstable();
    let restart_required = skills_restart_required(&runtime_visible, &effective_visible);
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "skill_switches": switches,
                "effective_enabled_skills_preview": effective,
                "restart_required": restart_required
            })),
            error: None,
        }),
    )
}

async fn uninstall_external_skill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UninstallExternalSkillRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let skill_name = state.resolve_canonical_skill_name(req.skill_name.trim());
    if skill_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("skill_name is required".to_string()),
            }),
        );
    }

    let Some(registry) = state.get_skills_registry() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("skills registry is not available".to_string()),
            }),
        );
    };
    let Some(entry) = registry.get(&skill_name).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("unknown skill: {skill_name}")),
            }),
        );
    };
    if entry.kind != SkillKind::External {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("only imported external skills can be uninstalled here".to_string()),
            }),
        );
    }

    let registry_raw = match read_skills_registry_file(&state) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills registry failed: {err}")),
                }),
            );
        }
    };
    let (updated_registry, removed_from_registry) = remove_skill_registry_block(&registry_raw, &skill_name);
    if !removed_from_registry {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("skill registry block not found for {skill_name}")),
            }),
        );
    }
    if let Err(err) = write_skills_registry_file(&state, &updated_registry) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills registry failed: {err}")),
            }),
        );
    }

    let mut removed_bundle = false;
    if let Some(bundle_rel) = entry.external_bundle_dir.as_deref() {
        let bundle_path = if Path::new(bundle_rel).is_absolute() {
            PathBuf::from(bundle_rel)
        } else {
            state.workspace_root.join(bundle_rel)
        };
        let allowed_root = state.workspace_root.join("third_party");
        if bundle_path.starts_with(&allowed_root) && bundle_path.exists() {
            match std::fs::remove_dir_all(&bundle_path) {
                Ok(_) => removed_bundle = true,
                Err(err) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiResponse {
                            ok: false,
                            data: None,
                            error: Some(format!("remove imported bundle failed: {err}")),
                        }),
                    );
                }
            }
        }
    }

    let mut removed_prompt = false;
    let prompt_rel = entry.prompt_file.trim();
    if !prompt_rel.is_empty() {
        let prompt_path = if Path::new(prompt_rel).is_absolute() {
            PathBuf::from(prompt_rel)
        } else {
            state.workspace_root.join(prompt_rel)
        };
        match remove_managed_prompt_file(&prompt_path) {
            Ok(value) => removed_prompt = value,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        ok: false,
                        data: None,
                        error: Some(format!("remove prompt file failed: {err}")),
                    }),
                );
            }
        }
    }

    let (runtime_raw, _) = match read_skill_config_file(&state) {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills config failed: {err}")),
                }),
            );
        }
    };
    let updated_runtime = remove_runtime_skill_switch(&runtime_raw, &state, &skill_name);
    if let Err(err) = write_runtime_config_file(&state, &updated_runtime) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills config failed: {err}")),
            }),
        );
    }

    let reload = match reload_skill_views(&state) {
        Ok(result) => result,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("reload skill views failed: {err}")),
                }),
            );
        }
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "skill_name": skill_name,
                "removed_bundle": removed_bundle,
                "removed_prompt": removed_prompt,
                "reload": reload,
            })),
            error: None,
        }),
    )
}

async fn whatsapp_web_login_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let base = state
        .whatsapp_web_bridge_base_url
        .trim()
        .trim_end_matches('/');
    if base.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("whatsapp_web.bridge_base_url is empty".to_string()),
            }),
        );
    }
    let url = format!("{base}/v1/login-status");
    let resp = match state.http_client.get(&url).send().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("request bridge login status failed: {err}")),
                }),
            );
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!(
                    "bridge login status failed: status={status} body={body}"
                )),
            }),
        );
    }
    let data = match resp.json::<Value>().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("decode bridge login status failed: {err}")),
                }),
            );
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        }),
    )
}

async fn whatsapp_web_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(resp) = require_ui_identity(&state, &headers) {
        return resp;
    }
    let base = state
        .whatsapp_web_bridge_base_url
        .trim()
        .trim_end_matches('/');
    if base.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some("whatsapp_web.bridge_base_url is empty".to_string()),
            }),
        );
    }
    let url = format!("{base}/v1/logout");
    let resp = match state.http_client.post(&url).send().await {
        Ok(v) => v,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("request bridge logout failed: {err}")),
                }),
            );
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return (
            StatusCode::BAD_GATEWAY,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("bridge logout failed: status={status} body={body}")),
            }),
        );
    }
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({ "ok": true })),
            error: None,
        }),
    )
}

async fn local_interaction_context(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<LocalInteractionContext>>) {
    match require_ui_identity(&state, &headers) {
        Ok(identity) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(LocalInteractionContext {
                    user_id: identity.user_id,
                    chat_id: identity.chat_id,
                    role: identity.role,
                }),
                error: None,
            }),
        ),
        Err((status, Json(resp))) => (
            status,
            Json(ApiResponse {
                ok: resp.ok,
                data: None,
                error: resp.error,
            }),
        ),
    }
}
