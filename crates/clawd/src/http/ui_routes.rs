use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::process::{Command as StdCommand, Stdio as StdProcessStdio};
use tokio::process::Command;

use super::super::{
    bind_channel_identity, current_rss_bytes, exchange_credential_status_for_user_key, feishud_process_stats,
    larkd_process_stats, mask_secret, oldest_running_task_age_seconds, resolve_auth_identity_by_key,
    resolve_channel_binding_identity, task_count_by_status, telegramd_process_stats,
    upsert_exchange_credential_for_user_key, wa_webd_process_stats, whatsappd_process_stats,
    ApiResponse, AppState, HealthResponse, LocalInteractionContext,
};
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
        .route("/logs/latest", get(logs_latest))
        .route("/whatsapp-web/login-status", get(whatsapp_web_login_status))
        .route("/whatsapp-web/logout", post(whatsapp_web_logout))
        .route("/services/:service/:action", post(control_service))
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
            // The start command may return success even if script preflight exits quickly
            // (for example, service disabled or missing required config). Verify process is up.
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            if !service_is_running(service.as_str()) {
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
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
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
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            if !service_is_running(service.as_str()) {
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

#[derive(Debug, Deserialize)]
struct UpdateSkillsConfigRequest {
    #[serde(default)]
    skill_switches: HashMap<String, bool>,
}

// ---------- Model config (GET/POST /admin/model-config) ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelConfigItem {
    vendor: String,
    model: String,
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

fn default_model_item() -> ModelConfigItem {
    ModelConfigItem {
        vendor: String::new(),
        model: String::new(),
    }
}

fn read_model_config(state: &AppState) -> anyhow::Result<ModelConfigResponse> {
    let root = &state.workspace_root;

    let config_path = root.join("configs/config.toml");
    let config_raw = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|_| String::new());
    let config: toml::Value = toml::from_str(&config_raw)
        .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
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
        })
        .unwrap_or_else(default_model_item);

    let image_path = root.join("configs/image.toml");
    let image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let image: toml::Value = toml::from_str(&image_raw)
        .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    let read_image_section = |section: &str| -> ModelConfigItem {
        image
            .get(section)
            .and_then(|t| t.as_table())
            .map(|t| ModelConfigItem {
                vendor: t
                    .get("default_vendor")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                model: t
                    .get("default_model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
            .unwrap_or_else(default_model_item)
    };
    let image_edit = read_image_section("image_edit");
    let image_generation = read_image_section("image_generation");
    let image_vision = read_image_section("image_vision");

    let audio_path = root.join("configs/audio.toml");
    let audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let audio: toml::Value = toml::from_str(&audio_raw)
        .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    let audio_transcribe = audio
        .get("audio_transcribe")
        .and_then(|t| t.as_table())
        .map(|t| ModelConfigItem {
            vendor: t
                .get("default_vendor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            model: t
                .get("default_model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .unwrap_or_else(default_model_item);

    let audio_synthesize = audio
        .get("audio_synthesize")
        .and_then(|t| t.as_table())
        .map(|t| ModelConfigItem {
            vendor: t
                .get("default_vendor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            model: t
                .get("default_model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .unwrap_or_else(default_model_item);

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
        let mut value: toml::Value = toml::from_str(&raw)?;
        if let Some(t) = value.get_mut("llm").and_then(|v| v.as_table_mut()) {
            t.insert("selected_vendor".to_string(), toml::Value::String(llm.vendor.clone()));
            t.insert("selected_model".to_string(), toml::Value::String(llm.model.clone()));
        } else {
            let mut tbl = toml::map::Map::new();
            tbl.insert("selected_vendor".to_string(), toml::Value::String(llm.vendor.clone()));
            tbl.insert("selected_model".to_string(), toml::Value::String(llm.model.clone()));
            value
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("config.toml root is not a table"))?
                .insert("llm".to_string(), toml::Value::Table(tbl));
        }
        std::fs::write(&path, toml::to_string_pretty(&value)?)?;
    }

    let mut image_modified = false;
    let image_path = root.join("configs/image.toml");
    let image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let mut image: toml::Value = toml::from_str(&image_raw)
        .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    for (section, item) in [
        ("image_edit", req.image_edit.as_ref()),
        ("image_generation", req.image_generation.as_ref()),
        ("image_vision", req.image_vision.as_ref()),
    ] {
        if let Some(ref it) = item {
            image_modified = true;
            let tbl = image
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("image.toml root is not a table"))?
                .entry(section.to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if let Some(t) = tbl.as_table_mut() {
                t.insert("default_vendor".to_string(), toml::Value::String(it.vendor.clone()));
                t.insert("default_model".to_string(), toml::Value::String(it.model.clone()));
            }
        }
    }
    if image_modified {
        std::fs::write(&image_path, toml::to_string_pretty(&image)?)?;
    }

    let mut audio_modified = false;
    let audio_path = root.join("configs/audio.toml");
    let audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let mut audio: toml::Value = toml::from_str(&audio_raw)
        .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));

    if let Some(ref it) = req.audio_transcribe {
        audio_modified = true;
        let tbl = audio
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("audio.toml root is not a table"))?
            .entry("audio_transcribe".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if let Some(t) = tbl.as_table_mut() {
            t.insert("default_vendor".to_string(), toml::Value::String(it.vendor.clone()));
            t.insert("default_model".to_string(), toml::Value::String(it.model.clone()));
        }
    }
    if let Some(ref it) = req.audio_synthesize {
        audio_modified = true;
        let tbl = audio
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("audio.toml root is not a table"))?
            .entry("audio_synthesize".to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if let Some(t) = tbl.as_table_mut() {
            t.insert("default_vendor".to_string(), toml::Value::String(it.vendor.clone()));
            t.insert("default_model".to_string(), toml::Value::String(it.model.clone()));
        }
    }
    if audio_modified {
        std::fs::write(&audio_path, toml::to_string_pretty(&audio)?)?;
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

// ---------- Provider API keys (GET/POST /admin/provider-keys) ----------
// 主模型 config.toml [llm.<vendor>].api_key；图像 configs/image.toml [image_*].providers.<vendor>；声音 configs/audio.toml [audio_*].providers.<vendor>
// All GET return masked keys.

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProviderKeysResponse {
    /// 主模型 / LLM: config.toml [llm.<vendor>].api_key
    #[serde(default)]
    llm: HashMap<String, String>,
    /// 图像: image.toml [image_edit|image_generation|image_vision].providers.<vendor>.api_key
    #[serde(default)]
    image: HashMap<String, HashMap<String, String>>,
    /// 声音: audio.toml [audio_synthesize|audio_transcribe].providers.<vendor>.api_key
    #[serde(default)]
    audio: HashMap<String, HashMap<String, String>>,
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
    let config: toml::Value = toml::from_str(&config_raw)
        .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let llm = read_llm_provider_keys(&config);

    let image_path = root.join("configs/image.toml");
    let image_raw = std::fs::read_to_string(&image_path).unwrap_or_else(|_| String::new());
    let image: toml::Value = toml::from_str(&image_raw)
        .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let image_keys = read_image_provider_keys(&image);

    let audio_path = root.join("configs/audio.toml");
    let audio_raw = std::fs::read_to_string(&audio_path).unwrap_or_else(|_| String::new());
    let audio: toml::Value = toml::from_str(&audio_raw)
        .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
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
        let mut config: toml::Value = toml::from_str(&raw)?;
        let llm = config
            .get_mut("llm")
            .and_then(|v| v.as_table_mut())
            .ok_or_else(|| anyhow::anyhow!("config.toml has no [llm] table"))?;
        for (vendor, new_key) in &req.llm {
            if new_key.is_empty() {
                continue;
            }
            let entry = llm
                .entry(vendor.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            if let Some(t) = entry.as_table_mut() {
                t.insert("api_key".to_string(), toml::Value::String(new_key.clone()));
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&config)?)?;
    }

    if !req.image.is_empty() {
        let path = root.join("configs/image.toml");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
        let mut image: toml::Value = toml::from_str(&raw)
            .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        let root_t = image.as_table_mut().ok_or_else(|| anyhow::anyhow!("image.toml root not a table"))?;
        for (section, vendors) in &req.image {
            if vendors.is_empty() {
                continue;
            }
            let section_t = root_t
                .entry(section.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let providers = section_t
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("image.toml [{}] not a table", section))?
                .entry("providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let prov_t = providers.as_table_mut().ok_or_else(|| anyhow::anyhow!("providers not a table"))?;
            for (vendor, new_key) in vendors {
                if new_key.is_empty() {
                    continue;
                }
                let entry = prov_t
                    .entry(vendor.clone())
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                if let Some(t) = entry.as_table_mut() {
                    t.insert("api_key".to_string(), toml::Value::String(new_key.clone()));
                }
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&image)?)?;
    }

    if !req.audio.is_empty() {
        let path = root.join("configs/audio.toml");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| String::new());
        let mut audio: toml::Value = toml::from_str(&raw)
            .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
        let root_t = audio.as_table_mut().ok_or_else(|| anyhow::anyhow!("audio.toml root not a table"))?;
        for (section, vendors) in &req.audio {
            if vendors.is_empty() {
                continue;
            }
            let section_t = root_t
                .entry(section.clone())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let providers = section_t
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("audio.toml [{}] not a table", section))?
                .entry("providers".to_string())
                .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
            let prov_t = providers.as_table_mut().ok_or_else(|| anyhow::anyhow!("providers not a table"))?;
            for (vendor, new_key) in vendors {
                if new_key.is_empty() {
                    continue;
                }
                let entry = prov_t
                    .entry(vendor.clone())
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                if let Some(t) = entry.as_table_mut() {
                    t.insert("api_key".to_string(), toml::Value::String(new_key.clone()));
                }
            }
        }
        std::fs::write(&path, toml::to_string_pretty(&audio)?)?;
    }

    Ok(())
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
    let mut set: BTreeMap<String, bool> = baseline
        .iter()
        .map(|s| (state.resolve_canonical_skill_name(s), true))
        .collect();
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
    let base_skill_names: Vec<String> = claw_core::config::base_skill_names()
        .iter()
        .map(|s| s.to_string())
        .collect();
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
                "effective_enabled_skills_preview": effective,
                "runtime_enabled_skills": runtime_visible,
                "restart_required": true
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
    let path = state.workspace_root.join("configs/config.toml");
    if let Err(err) = std::fs::write(&path, updated) {
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
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "config_path": "configs/config.toml",
                "skill_switches": switches,
                "effective_enabled_skills_preview": effective,
                "restart_required": true
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
