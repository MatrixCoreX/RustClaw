mod config_cache;
mod config_section;
mod ilink;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use axum::extract::State as AxumState;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use claw_core::channel_chunk::{chunk_text_for_channel, SEGMENT_PREFIX_MAX_CHARS};
use claw_core::types::{
    ApiResponse, AuthIdentity, BindChannelKeyRequest, ChannelKind, ResolveChannelBindingRequest,
    ResolveChannelBindingResponse, SubmitTaskRequest, SubmitTaskResponse, TaskKind,
    TaskQueryResponse, TaskStatus,
};
use config_section::{AppConfig, WechatSection};
use config_cache::WeixinConfigManager;
use qrcodegen::{QrCode, QrCodeEcc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

const SESSION_EXPIRED_ERRCODE: i64 = -14;
const MAX_CONSECUTIVE_FAILURES: usize = 3;
const RETRY_DELAY_MS: u64 = 2_000;
const BACKOFF_DELAY_MS: u64 = 30_000;
const ACTIVE_LOGIN_TTL_MS: u64 = 5 * 60_000;
const WECHAT_TEXT_CHUNK_CHARS: usize = 1200;

#[derive(Clone, Serialize, Deserialize)]
struct WechatRuntimeStatus {
    healthy: bool,
    status: String,
    last_event_ts: Option<u64>,
    last_peer: Option<String>,
    last_error: Option<String>,
    account_label: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
struct PersistedSession {
    bot_token: String,
    account_id: Option<String>,
    base_url: Option<String>,
    user_id: Option<String>,
    saved_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ActiveLogin {
    session_key: String,
    qrcode: String,
    qrcode_url: String,
    started_at_ms: u64,
    status: String,
    message: String,
}

#[derive(Deserialize)]
struct LoginStartRequest {
    #[serde(default)]
    force: bool,
    #[serde(default)]
    bot_type: Option<String>,
}

#[derive(Serialize)]
struct LoginStartResponse {
    session_key: String,
    qrcode_url: String,
    message: String,
}

#[derive(Deserialize)]
struct LoginWaitRequest {
    session_key: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    bot_type: Option<String>,
}

#[derive(Serialize)]
struct LoginWaitResponse {
    connected: bool,
    qr_status: String,
    message: String,
    account_id: Option<String>,
    user_id: Option<String>,
}

#[derive(Serialize)]
struct LoginStatusResponse {
    connected: bool,
    qr_ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    qr_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    qrcode_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_update_ts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_label: Option<String>,
    status: String,
}

#[derive(Clone)]
struct State {
    config: WechatSection,
    client: Client,
    status: Arc<RwLock<WechatRuntimeStatus>>,
    status_path: Arc<PathBuf>,
    session_path: Arc<PathBuf>,
    session: Arc<RwLock<Option<PersistedSession>>>,
    active_logins: Arc<RwLock<HashMap<String, ActiveLogin>>>,
    context_tokens: Arc<RwLock<HashMap<String, String>>>,
    sync_buf_path: Arc<PathBuf>,
    config_cache: Arc<Mutex<WeixinConfigManager>>,
}

#[derive(Serialize)]
struct GetUpdatesReq<'a> {
    get_updates_buf: &'a str,
    base_info: ilink::BaseInfo,
}

#[derive(Debug, Deserialize)]
struct GetUpdatesResp {
    #[serde(default)]
    ret: Option<i64>,
    #[serde(default)]
    errcode: Option<i64>,
    #[serde(default)]
    errmsg: Option<String>,
    #[serde(default)]
    msgs: Vec<WeixinMessage>,
    #[serde(default)]
    get_updates_buf: Option<String>,
    #[serde(default)]
    longpolling_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct WeixinMessage {
    #[serde(default)]
    from_user_id: Option<String>,
    #[serde(default, rename = "to_user_id")]
    _to_user_id: Option<String>,
    #[serde(default)]
    create_time_ms: Option<u64>,
    #[serde(default)]
    item_list: Option<Vec<MessageItem>>,
    #[serde(default)]
    context_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MessageItem {
    #[serde(default)]
    r#type: Option<i64>,
    #[serde(default)]
    text_item: Option<TextItem>,
    #[serde(default)]
    voice_item: Option<VoiceItem>,
}

#[derive(Debug, Clone, Deserialize)]
struct TextItem {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct VoiceItem {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Serialize)]
struct SendMessageReq {
    msg: OutboundMessage,
    base_info: ilink::BaseInfo,
}

#[derive(Serialize)]
struct OutboundMessage {
    from_user_id: String,
    to_user_id: String,
    client_id: String,
    message_type: i64,
    message_state: i64,
    item_list: Vec<OutboundMessageItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_token: Option<String>,
}

#[derive(Serialize)]
struct OutboundMessageItem {
    r#type: i64,
    text_item: OutboundTextItem,
}

#[derive(Serialize)]
struct OutboundTextItem {
    text: String,
}

#[derive(Deserialize)]
struct QRCodeResponse {
    qrcode: String,
    #[serde(rename = "qrcode_img_content")]
    qrcode_img_content: String,
}

#[derive(Deserialize)]
struct QRStatusResponse {
    status: String,
    #[serde(default)]
    bot_token: Option<String>,
    #[serde(default)]
    ilink_bot_id: Option<String>,
    #[serde(default)]
    baseurl: Option<String>,
    #[serde(default)]
    ilink_user_id: Option<String>,
}

fn current_ts_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_millis() as u64)
        .unwrap_or(0)
}

fn current_ts_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .unwrap_or(0)
}

fn workspace_root_from_config_path(config_path: &str) -> PathBuf {
    Path::new(config_path)
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn wechat_runtime_status_file_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join("run")
        .join("wechatd-status")
        .join("primary.json")
}

fn wechat_session_file_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join("data").join("wechatd").join("session.json")
}

fn wechat_sync_buf_file_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join("data")
        .join("wechatd")
        .join("get_updates_buf.txt")
}

fn qr_svg_data_url(qr_content: &str) -> Result<String, String> {
    let qr = QrCode::encode_text(qr_content, QrCodeEcc::Medium)
        .map_err(|e| format!("encode QR svg failed: {e:?}"))?;
    let border = 4;
    let size = qr.size();
    let canvas = size + border * 2;
    let mut path = String::new();
    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                let px = x + border;
                let py = y + border;
                path.push_str(&format!("M{px},{py}h1v1h-1z"));
            }
        }
    }
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {canvas} {canvas}\" shape-rendering=\"crispEdges\"><rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/><path d=\"{path}\" fill=\"#111111\"/></svg>"
    );
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        BASE64_STANDARD.encode(svg)
    ))
}

fn qr_render_content(response: &QRCodeResponse) -> &str {
    let content = response.qrcode_img_content.trim();
    if content.is_empty() {
        response.qrcode.trim()
    } else {
        content
    }
}

fn stable_i64_from_string(input: &str) -> i64 {
    let mut h: i64 = 0;
    for b in input.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as i64);
    }
    h
}

fn extract_text_message(msg: &WeixinMessage) -> Option<String> {
    for item in msg.item_list.as_ref()? {
        if item.r#type == Some(1) {
            let text = item
                .text_item
                .as_ref()
                .and_then(|v| v.text.as_deref())
                .map(str::trim)
                .unwrap_or("");
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        if item.r#type == Some(3) {
            let text = item
                .voice_item
                .as_ref()
                .and_then(|v| v.text.as_deref())
                .map(str::trim)
                .unwrap_or("");
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// True when the message carries image / video / file / raw voice items (no usable text).
fn has_non_text_media_items(msg: &WeixinMessage) -> bool {
    let Some(items) = msg.item_list.as_ref() else {
        return false;
    };
    for it in items {
        let t = it.r#type.unwrap_or(0);
        if t == 2 || t == 4 || t == 5 {
            return true;
        }
        if t == 3 {
            let voice_text = it
                .voice_item
                .as_ref()
                .and_then(|v| v.text.as_deref())
                .map(str::trim)
                .unwrap_or("");
            if voice_text.is_empty() {
                return true;
            }
        }
    }
    false
}

fn active_login_is_fresh(login: &ActiveLogin) -> bool {
    current_ts_ms().saturating_sub(login.started_at_ms) < ACTIVE_LOGIN_TTL_MS
}

fn runtime_status_is_connected(status: &str) -> bool {
    matches!(status, "connected" | "polling" | "message_received")
}

async fn write_json_file<T: Serialize>(path: &Path, value: &T) {
    let Some(parent) = path.parent() else {
        return;
    };
    if tokio::fs::create_dir_all(parent).await.is_err() {
        return;
    }
    let Ok(raw) = serde_json::to_vec_pretty(value) else {
        return;
    };
    let _ = tokio::fs::write(path, raw).await;
}

async fn write_text_file(path: &Path, content: &str) {
    let Some(parent) = path.parent() else {
        return;
    };
    if tokio::fs::create_dir_all(parent).await.is_err() {
        return;
    }
    let _ = tokio::fs::write(path, content).await;
}

fn load_text_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn load_session_file(path: &Path) -> Option<PersistedSession> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

async fn update_status(
    state: &State,
    mut mutate: impl FnMut(&mut WechatRuntimeStatus),
) {
    let snapshot = {
        let mut guard = state.status.write().await;
        mutate(&mut guard);
        guard.clone()
    };
    write_json_file(&state.status_path, &snapshot).await;
}

async fn healthz(AxumState(state): AxumState<State>) -> Json<WechatRuntimeStatus> {
    Json(state.status.read().await.clone())
}

fn build_login_status_response(
    status: &WechatRuntimeStatus,
    active_login: Option<&ActiveLogin>,
) -> LoginStatusResponse {
    LoginStatusResponse {
        connected: runtime_status_is_connected(&status.status),
        qr_ready: active_login.is_some(),
        session_key: active_login.map(|login| login.session_key.clone()),
        qr_status: active_login.map(|login| login.status.clone()),
        qrcode_url: active_login.map(|login| login.qrcode_url.clone()),
        message: active_login.map(|login| login.message.clone()),
        last_update_ts: status.last_event_ts,
        last_error: status.last_error.clone(),
        account_label: status.account_label.clone(),
        status: status.status.clone(),
    }
}

async fn login_status(AxumState(state): AxumState<State>) -> Json<LoginStatusResponse> {
    let status = state.status.read().await.clone();
    let active_login = {
        let logins = state.active_logins.read().await;
        logins
            .values()
            .find(|login| active_login_is_fresh(login))
            .cloned()
    };
    Json(build_login_status_response(&status, active_login.as_ref()))
}

async fn login_qr_start(
    AxumState(state): AxumState<State>,
    Json(req): Json<LoginStartRequest>,
) -> Result<Json<LoginStartResponse>, (axum::http::StatusCode, String)> {
    let session_key = "primary".to_string();
    let mut active = state.active_logins.write().await;
    if !req.force {
        if let Some(existing) = active.get(&session_key) {
            if active_login_is_fresh(existing) {
                return Ok(Json(LoginStartResponse {
                    session_key,
                    qrcode_url: existing.qrcode_url.clone(),
                    message: "二维码已就绪，请使用微信扫描。".to_string(),
                }));
            }
        }
    }
    let response = fetch_qrcode(
        &state.client,
        &state.config,
        req.bot_type.as_deref().unwrap_or("3"),
    )
    .await
    .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, e))?;
    let qrcode_url = qr_svg_data_url(qr_render_content(&response))
        .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, e))?;
    active.insert(
        session_key.clone(),
        ActiveLogin {
            session_key: session_key.clone(),
            qrcode: response.qrcode.clone(),
            qrcode_url: qrcode_url.clone(),
            started_at_ms: current_ts_ms(),
            status: "wait".to_string(),
            message: "二维码已生成，等待扫码。".to_string(),
        },
    );
    update_status(&state, |status| {
        status.healthy = true;
        status.status = "qr_ready".to_string();
        status.last_event_ts = Some(current_ts_ms());
        status.last_error = None;
    })
    .await;
    Ok(Json(LoginStartResponse {
        session_key,
        qrcode_url,
        message: "使用微信扫描二维码完成连接。".to_string(),
    }))
}

async fn login_qr_wait(
    AxumState(state): AxumState<State>,
    Json(req): Json<LoginWaitRequest>,
) -> Result<Json<LoginWaitResponse>, (axum::http::StatusCode, String)> {
    let timeout_ms = req.timeout_ms.unwrap_or(480_000).max(1_000);
    let deadline = current_ts_ms().saturating_add(timeout_ms);
    let mut refresh_count = 1usize;
    loop {
        let active_login = {
            let logins = state.active_logins.read().await;
            logins.get(&req.session_key).cloned()
        }
        .ok_or_else(|| {
            (
                axum::http::StatusCode::BAD_REQUEST,
                "当前没有进行中的登录，请先发起登录。".to_string(),
            )
        })?;

        if !active_login_is_fresh(&active_login) {
            state.active_logins.write().await.remove(&req.session_key);
            return Ok(Json(LoginWaitResponse {
                connected: false,
                qr_status: "expired".to_string(),
                message: "二维码已过期，请重新生成。".to_string(),
                account_id: None,
                user_id: None,
            }));
        }

        let status = poll_qr_status(&state.client, &state.config, &active_login.qrcode)
            .await
            .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, e))?;
        match status.status.as_str() {
            "wait" | "scaned" => {
                let qr_message = if status.status == "scaned" {
                    "二维码已被扫描，请在手机上确认登录。".to_string()
                } else {
                    "二维码已生成，等待扫码。".to_string()
                };
                if let Some(login) = state.active_logins.write().await.get_mut(&req.session_key) {
                    login.status = status.status.clone();
                    login.message = qr_message.clone();
                }
                if status.status == "scaned" {
                    update_status(&state, |runtime| {
                        runtime.healthy = true;
                        runtime.status = "qr_scanned".to_string();
                        runtime.last_event_ts = Some(current_ts_ms());
                        runtime.last_error = None;
                    })
                    .await;
                }
                if current_ts_ms().saturating_add(1_000) >= deadline {
                    return Ok(Json(LoginWaitResponse {
                        connected: false,
                        qr_status: status.status.clone(),
                        message: qr_message,
                        account_id: None,
                        user_id: None,
                    }));
                }
            }
            "expired" => {
                refresh_count = refresh_count.saturating_add(1);
                if refresh_count > 3 {
                    state.active_logins.write().await.remove(&req.session_key);
                    return Ok(Json(LoginWaitResponse {
                        connected: false,
                        qr_status: "expired".to_string(),
                        message: "登录超时：二维码多次过期，请重新开始登录流程。".to_string(),
                        account_id: None,
                        user_id: None,
                    }));
                }
                let refreshed = fetch_qrcode(
                    &state.client,
                    &state.config,
                    req.bot_type.as_deref().unwrap_or("3"),
                )
                .await
                .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, e))?;
                let qrcode_url = qr_svg_data_url(qr_render_content(&refreshed))
                    .map_err(|e| (axum::http::StatusCode::BAD_GATEWAY, e))?;
                state.active_logins.write().await.insert(
                    req.session_key.clone(),
                    ActiveLogin {
                        session_key: req.session_key.clone(),
                        qrcode: refreshed.qrcode,
                        qrcode_url,
                        started_at_ms: current_ts_ms(),
                        status: "wait".to_string(),
                        message: "二维码已刷新，等待扫码。".to_string(),
                    },
                );
            }
            "confirmed" => {
                let bot_token = status.bot_token.clone().unwrap_or_default();
                let account_id = status.ilink_bot_id.clone();
                if bot_token.trim().is_empty() || account_id.is_none() {
                    return Ok(Json(LoginWaitResponse {
                        connected: false,
                        qr_status: "confirmed".to_string(),
                        message: "登录失败：服务器未返回完整 bot_token / ilink_bot_id。".to_string(),
                        account_id: None,
                        user_id: None,
                    }));
                }
                let session = PersistedSession {
                    bot_token,
                    account_id: account_id.clone(),
                    base_url: status.baseurl.clone(),
                    user_id: status.ilink_user_id.clone(),
                    saved_at: Some(current_ts_secs().to_string()),
                };
                *state.session.write().await = Some(session.clone());
                write_json_file(&state.session_path, &session).await;
                state.active_logins.write().await.remove(&req.session_key);
                update_status(&state, |runtime| {
                    runtime.healthy = true;
                    runtime.status = "connected".to_string();
                    runtime.last_event_ts = Some(current_ts_ms());
                    runtime.last_error = None;
                    runtime.account_label = account_id.clone();
                })
                .await;
                return Ok(Json(LoginWaitResponse {
                    connected: true,
                    qr_status: "confirmed".to_string(),
                    message: "与微信连接成功。".to_string(),
                    account_id,
                    user_id: status.ilink_user_id,
                }));
            }
            other => {
                warn!("wechatd: unexpected qr status={}", other);
            }
        }
        if current_ts_ms() >= deadline {
            return Ok(Json(LoginWaitResponse {
                connected: false,
                qr_status: "wait".to_string(),
                message: "登录超时，请重试。".to_string(),
                account_id: None,
                user_id: None,
            }));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn session_token(config: &WechatSection, session: Option<&PersistedSession>) -> Option<String> {
    if let Some(existing) = session {
        if !existing.bot_token.trim().is_empty() {
            return Some(existing.bot_token.trim().to_string());
        }
    }
    let config_token = config.bot_token.trim();
    if config_token.is_empty() || config_token == "REPLACE_ME" {
        None
    } else {
        Some(config_token.to_string())
    }
}

fn session_base_url(config: &WechatSection, session: Option<&PersistedSession>) -> String {
    session
        .and_then(|s| s.base_url.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| config.api_base_url.trim())
        .to_string()
}

async fn fetch_qrcode(
    client: &Client,
    section: &WechatSection,
    bot_type: &str,
) -> Result<QRCodeResponse, String> {
    let base = format!("{}/", section.api_base_url.trim_end_matches('/'));
    let url = format!(
        "{}ilink/bot/get_bot_qrcode?bot_type={}",
        base, bot_type
    );
    let req = client.get(&url);
    let response = ilink::apply_route_tag(req, section)
        .send()
        .await
        .map_err(|e| format!("fetch QR code failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("fetch QR code status={status} body={body}"));
    }
    response
        .json()
        .await
        .map_err(|e| format!("parse QR code response failed: {e}"))
}

async fn poll_qr_status(
    client: &Client,
    section: &WechatSection,
    qrcode: &str,
) -> Result<QRStatusResponse, String> {
    let base = format!("{}/", section.api_base_url.trim_end_matches('/'));
    let url = format!("{}ilink/bot/get_qrcode_status?qrcode={}", base, qrcode);
    let req = client
        .get(&url)
        .header("iLink-App-ClientVersion", "1");
    let response = ilink::apply_route_tag(req, section)
        .send()
        .await
        .map_err(|e| format!("poll QR status failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("poll QR status={status} body={body}"));
    }
    response
        .json()
        .await
        .map_err(|e| format!("parse QR status failed: {e}"))
}

async fn get_updates(
    client: &Client,
    config: &WechatSection,
    base_url: &str,
    token: &str,
    get_updates_buf: &str,
    timeout_ms: u64,
) -> Result<GetUpdatesResp, String> {
    let value = ilink::post_json(
        client,
        config,
        base_url,
        token,
        "ilink/bot/getupdates",
        &GetUpdatesReq {
            get_updates_buf,
            base_info: ilink::base_info(),
        },
        timeout_ms,
    )
    .await?;
    serde_json::from_value(value).map_err(|e| format!("getupdates decode failed: {e}"))
}

async fn send_text_message(
    client: &Client,
    config: &WechatSection,
    base_url: &str,
    token: &str,
    to_user_id: &str,
    context_token: Option<&str>,
    text: &str,
) -> Result<(), String> {
    let chunks = chunk_text_for_channel(
        text,
        config
            .text_chunk_chars
            .max(1)
            .min(WECHAT_TEXT_CHUNK_CHARS)
            .saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    let chunk_count = chunks.len();
    for (index, chunk) in chunks.into_iter().enumerate() {
        let body = SendMessageReq {
            msg: OutboundMessage {
                from_user_id: String::new(),
                to_user_id: to_user_id.to_string(),
                client_id: format!("wechatd-{}", current_ts_ms()),
                message_type: 2,
                message_state: 2,
                item_list: vec![OutboundMessageItem {
                    r#type: 1,
                    text_item: OutboundTextItem {
                        text: if chunk_count > 1 {
                            format!("（{}/{}）\n{}", index + 1, chunk_count, chunk)
                        } else {
                            chunk
                        },
                    },
                }],
                context_token: context_token.map(str::to_string),
            },
            base_info: ilink::base_info(),
        };
        let _ = ilink::post_json(
            client,
            config,
            base_url,
            token,
            "ilink/bot/sendmessage",
            &body,
            config.request_timeout_seconds.max(1) * 1_000,
        )
        .await?;
    }
    Ok(())
}

async fn resolve_wechat_identity(
    client: &Client,
    base_url: &str,
    external_user_id: &str,
    external_chat_id: &str,
) -> Result<Option<AuthIdentity>, String> {
    let url = format!("{}/v1/auth/channel/resolve", base_url.trim_end_matches('/'));
    let req = ResolveChannelBindingRequest {
        channel: ChannelKind::Wechat,
        external_user_id: Some(external_user_id.to_string()),
        external_chat_id: Some(external_chat_id.to_string()),
        telegram_bot_name: None,
    };
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("resolve request failed: {e}"))?;
    let status = resp.status();
    let body: ApiResponse<ResolveChannelBindingResponse> = resp
        .json()
        .await
        .map_err(|e| format!("resolve response parse failed: {e}"))?;
    if !status.is_success() || !body.ok {
        return Err(body.error.unwrap_or_else(|| "resolve failed".to_string()));
    }
    Ok(body.data.and_then(|d| d.identity))
}

async fn bind_wechat_identity(
    client: &Client,
    base_url: &str,
    external_user_id: &str,
    external_chat_id: &str,
    user_key: &str,
) -> Result<Option<AuthIdentity>, String> {
    let url = format!("{}/v1/auth/channel/bind", base_url.trim_end_matches('/'));
    let req = BindChannelKeyRequest {
        channel: ChannelKind::Wechat,
        external_user_id: Some(external_user_id.to_string()),
        external_chat_id: Some(external_chat_id.to_string()),
        telegram_bot_name: None,
        user_key: user_key.trim().to_string(),
    };
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("bind request failed: {e}"))?;
    let status = resp.status();
    let body: ApiResponse<AuthIdentity> = resp
        .json()
        .await
        .map_err(|e| format!("bind response parse failed: {e}"))?;
    if status.as_u16() == 401 || !body.ok {
        return Ok(None);
    }
    Ok(body.data)
}

fn task_success_text(task: &TaskQueryResponse) -> String {
    if let Some(messages) = task
        .result_json
        .as_ref()
        .and_then(|v| v.get("messages"))
        .and_then(|v| v.as_array())
    {
        let parts: Vec<String> = messages
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if !parts.is_empty() {
            return parts.join("\n\n");
        }
    }
    task.result_json
        .as_ref()
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "处理完成。".to_string())
}

/// Refresh `ilink/bot/sendtyping` while clawd runs (`keepaliveIntervalMs` ≈ 5s in OpenClaw weixin).
struct WechatTypingHeartbeat {
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl WechatTypingHeartbeat {
    fn start(
        client: Client,
        section: WechatSection,
        base_url: String,
        token: String,
        to_user_id: String,
        typing_ticket: String,
        interval: Duration,
    ) -> Self {
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                let _ = ilink::send_typing_once(
                    &client,
                    &section,
                    &base_url,
                    &token,
                    &to_user_id,
                    &typing_ticket,
                    1,
                )
                .await;
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = &mut stop_rx => {
                        let _ = ilink::send_typing_once(
                            &client,
                            &section,
                            &base_url,
                            &token,
                            &to_user_id,
                            &typing_ticket,
                            2,
                        )
                        .await;
                        break;
                    }
                }
            }
        });
        Self {
            stop_tx: Some(stop_tx),
        }
    }

    fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for WechatTypingHeartbeat {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn resolve_typing_ticket_for_peer(
    state: &State,
    from_user_id: &str,
    context_token: Option<&str>,
) -> Option<String> {
    let session_guard = state.session.read().await;
    let token = session_token(&state.config, session_guard.as_ref())?;
    let base_url = session_base_url(&state.config, session_guard.as_ref());
    drop(session_guard);
    let mut mgr = state.config_cache.lock().await;
    let ticket = mgr
        .typing_ticket_for_user(
            &state.client,
            &state.config,
            &base_url,
            &token,
            from_user_id,
            context_token,
        )
        .await;
    let t = ticket.trim();
    if t.is_empty() {
        None
    } else {
        Some(ticket)
    }
}

async fn submit_wechat_task_and_reply(
    state: State,
    from_user_id: String,
    text: String,
    context_token: Option<String>,
    user_key: Option<String>,
    typing_ticket: Option<String>,
) {
    let submit_req = SubmitTaskRequest {
        user_id: Some(stable_i64_from_string(&from_user_id)),
        chat_id: Some(stable_i64_from_string(&from_user_id)),
        user_key: user_key.clone(),
        channel: Some(ChannelKind::Wechat),
        external_user_id: Some(from_user_id.clone()),
        external_chat_id: Some(from_user_id.clone()),
        kind: TaskKind::Ask,
        payload: json!({
            "text": text,
            "agent_mode": true,
            "channel": "wechat",
            "context_token": context_token
        }),
    };
    let submit_url = format!("{}/v1/tasks", state.config.clawd_base_url.trim_end_matches('/'));
    let submit_resp = match state.client.post(&submit_url).json(&submit_req).send().await {
        Ok(resp) => resp,
        Err(err) => {
            warn!("wechatd: task submit failed err={}", err);
            return;
        }
    };
    if !submit_resp.status().is_success() {
        warn!(
            "wechatd: task submit failed status={} body={}",
            submit_resp.status(),
            submit_resp.text().await.unwrap_or_default()
        );
        return;
    }
    let submit_body: ApiResponse<SubmitTaskResponse> = match submit_resp.json().await {
        Ok(body) => body,
        Err(err) => {
            warn!("wechatd: task submit parse failed err={}", err);
            return;
        }
    };
    let Some(task_data) = submit_body.data else {
        warn!("wechatd: task submit missing task_id");
        return;
    };
    let task_id = task_data.task_id.to_string();
    let started = std::time::Instant::now();
    let (poll_token, poll_base) = {
        let g = state.session.read().await;
        (
            session_token(&state.config, g.as_ref()),
            session_base_url(&state.config, g.as_ref()),
        )
    };
    let interval = Duration::from_secs(state.config.typing_refresh_interval_secs.max(1));
    let _typing_guard = match (&typing_ticket, &poll_token) {
        (Some(ticket), Some(tok)) if !ticket.trim().is_empty() => Some(WechatTypingHeartbeat::start(
            state.client.clone(),
            state.config.clone(),
            poll_base,
            tok.clone(),
            from_user_id.clone(),
            ticket.clone(),
            interval,
        )),
        _ => None,
    };
    loop {
        let url = format!(
            "{}/v1/tasks/{}",
            state.config.clawd_base_url.trim_end_matches('/'),
            task_id
        );
        let mut req = state.client.get(&url);
        if let Some(ref key) = user_key {
            let k = key.trim();
            if !k.is_empty() {
                req = req.header("X-RustClaw-Key", k);
            }
        }
        let resp = match req.send().await {
            Ok(resp) => resp,
            Err(err) => {
                if started.elapsed()
                    > Duration::from_secs(state.config.request_timeout_seconds.max(30))
                {
                    warn!("wechatd: poll task timeout err={}", err);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1500)).await;
                continue;
            }
        };
        if !resp.status().is_success() {
            if started.elapsed() > Duration::from_secs(state.config.request_timeout_seconds.max(30))
            {
                warn!("wechatd: poll task status timeout status={}", resp.status());
                break;
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
            continue;
        }
        let body: ApiResponse<TaskQueryResponse> = match resp.json().await {
            Ok(body) => body,
            Err(err) => {
                warn!("wechatd: poll task parse failed err={}", err);
                tokio::time::sleep(Duration::from_millis(1500)).await;
                continue;
            }
        };
        let Some(task) = body.data else {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            continue;
        };
        match task.status {
            TaskStatus::Queued | TaskStatus::Running => {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                continue;
            }
            TaskStatus::Succeeded => {
                let reply_text = task_success_text(&task);
                let session_guard = state.session.read().await;
                let token = session_token(&state.config, session_guard.as_ref());
                let base_url = session_base_url(&state.config, session_guard.as_ref());
                drop(session_guard);
                if let Some(token) = token {
                    if let Err(err) = send_text_message(
                        &state.client,
                        &state.config,
                        &base_url,
                        &token,
                        &from_user_id,
                        context_token.as_deref(),
                        &reply_text,
                    )
                    .await
                    {
                        warn!("wechatd: send reply failed err={}", err);
                    }
                }
                break;
            }
            TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                let error_text = task
                    .error_text
                    .unwrap_or_else(|| "请求处理失败，请稍后重试。".to_string());
                let session_guard = state.session.read().await;
                let token = session_token(&state.config, session_guard.as_ref());
                let base_url = session_base_url(&state.config, session_guard.as_ref());
                drop(session_guard);
                if let Some(token) = token {
                    let _ = send_text_message(
                        &state.client,
                        &state.config,
                        &base_url,
                        &token,
                        &from_user_id,
                        context_token.as_deref(),
                        &error_text,
                    )
                    .await;
                }
                break;
            }
        }
    }
}

async fn handle_incoming_message(state: State, msg: WeixinMessage) {
    let Some(from_user_id) = msg.from_user_id.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string) else {
        return;
    };
    let text = match extract_text_message(&msg) {
        Some(t) => t,
        None => {
            if has_non_text_media_items(&msg) {
                let session_guard = state.session.read().await;
                let token = session_token(&state.config, session_guard.as_ref());
                let base_url = session_base_url(&state.config, session_guard.as_ref());
                drop(session_guard);
                if let Some(token) = token {
                    let _ = send_text_message(
                        &state.client,
                        &state.config,
                        &base_url,
                        &token,
                        &from_user_id,
                        msg.context_token.as_deref(),
                        "当前仅支持文本与已转文字的语音。图片/视频/文件等媒体将后续对齐 CDN 加解密与上传流程。",
                    )
                    .await;
                }
            }
            return;
        }
    };
    if let Some(token) = msg.context_token.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        state
            .context_tokens
            .write()
            .await
            .insert(from_user_id.clone(), token.to_string());
    }
    update_status(&state, |status| {
        status.healthy = true;
        status.status = "message_received".to_string();
        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
        status.last_peer = Some(from_user_id.clone());
        status.last_error = None;
    })
    .await;

    let identity = match resolve_wechat_identity(
        &state.client,
        &state.config.clawd_base_url,
        &from_user_id,
        &from_user_id,
    )
    .await
    {
        Ok(identity) => identity,
        Err(err) => {
            warn!("wechatd: resolve identity failed err={}", err);
            return;
        }
    };
    if let Some(identity) = identity {
        let typing_ticket = resolve_typing_ticket_for_peer(
            &state,
            &from_user_id,
            msg.context_token.as_deref(),
        )
        .await;
        tokio::spawn(submit_wechat_task_and_reply(
            state,
            from_user_id,
            text,
            msg.context_token,
            Some(identity.user_key),
            typing_ticket,
        ));
        return;
    }

    match bind_wechat_identity(
        &state.client,
        &state.config.clawd_base_url,
        &from_user_id,
        &from_user_id,
        text.trim(),
    )
    .await
    {
        Ok(Some(_)) => {
            let session_guard = state.session.read().await;
            let token = session_token(&state.config, session_guard.as_ref());
            let base_url = session_base_url(&state.config, session_guard.as_ref());
            drop(session_guard);
            if let Some(token) = token {
                let _ = send_text_message(
                    &state.client,
                    &state.config,
                    &base_url,
                    &token,
                    &from_user_id,
                    msg.context_token.as_deref(),
                    "绑定成功，请重新发送你的问题。",
                )
                .await;
            }
        }
        Ok(None) => {
            let session_guard = state.session.read().await;
            let token = session_token(&state.config, session_guard.as_ref());
            let base_url = session_base_url(&state.config, session_guard.as_ref());
            drop(session_guard);
            if let Some(token) = token {
                let _ = send_text_message(
                    &state.client,
                    &state.config,
                    &base_url,
                    &token,
                    &from_user_id,
                    msg.context_token.as_deref(),
                    "请先发送你的 RustClaw key 完成绑定。",
                )
                .await;
            }
        }
        Err(err) => {
            warn!("wechatd: bind request failed err={}", err);
        }
    }
}

async fn monitor_wechat_loop(state: State) {
    let mut next_timeout_ms = state.config.longpoll_timeout_ms.max(1_000);
    let mut consecutive_failures = 0usize;
    let mut get_updates_buf = load_text_file(&state.sync_buf_path).unwrap_or_default();
    loop {
        let session_guard = state.session.read().await;
        let token = session_token(&state.config, session_guard.as_ref());
        let base_url = session_base_url(&state.config, session_guard.as_ref());
        drop(session_guard);

        let Some(token) = token else {
            update_status(&state, |status| {
                status.healthy = true;
                status.status = "login_required".to_string();
                status.last_error = None;
            })
            .await;
            tokio::time::sleep(Duration::from_secs(3)).await;
            continue;
        };

        match get_updates(
            &state.client,
            &state.config,
            &base_url,
            &token,
            &get_updates_buf,
            next_timeout_ms,
        )
        .await
        {
            Ok(resp) => {
                if let Some(timeout_ms) = resp.longpolling_timeout_ms {
                    if timeout_ms > 0 {
                        next_timeout_ms = timeout_ms;
                    }
                }
                let is_error = resp.ret.unwrap_or(0) != 0 || resp.errcode.unwrap_or(0) != 0;
                if is_error {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    let errcode = resp.errcode.unwrap_or(resp.ret.unwrap_or(0));
                    if errcode == SESSION_EXPIRED_ERRCODE {
                        update_status(&state, |status| {
                            status.healthy = false;
                            status.status = "session_expired".to_string();
                            status.last_event_ts = Some(current_ts_ms());
                            status.last_error = Some("会话已过期，请重新扫码登录。".to_string());
                        })
                        .await;
                        *state.session.write().await = None;
                        let _ = tokio::fs::remove_file(&*state.session_path).await;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                    update_status(&state, |status| {
                        status.healthy = false;
                        status.status = "poll_error".to_string();
                        status.last_error = Some(
                            resp.errmsg
                                .clone()
                                .unwrap_or_else(|| format!("errcode={errcode}")),
                        );
                    })
                    .await;
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        tokio::time::sleep(Duration::from_millis(BACKOFF_DELAY_MS)).await;
                        consecutive_failures = 0;
                    } else {
                        tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
                    }
                    continue;
                }
                consecutive_failures = 0;
                if let Some(buf) = resp.get_updates_buf.as_deref().filter(|v| !v.is_empty()) {
                    get_updates_buf = buf.to_string();
                    write_text_file(&state.sync_buf_path, &get_updates_buf).await;
                }
                update_status(&state, |status| {
                    status.healthy = true;
                    status.status = "polling".to_string();
                    status.last_error = None;
                })
                .await;
                for msg in resp.msgs {
                    handle_incoming_message(state.clone(), msg).await;
                }
            }
            Err(err) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                update_status(&state, |status| {
                    status.healthy = false;
                    status.status = "poll_error".to_string();
                    status.last_error = Some(err.clone());
                })
                .await;
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    tokio::time::sleep(Duration::from_millis(BACKOFF_DELAY_MS)).await;
                    consecutive_failures = 0;
                } else {
                    tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,wechatd=debug".to_string()),
        )
        .init();

    let config_path =
        std::env::var("WECHAT_CONFIG_PATH").unwrap_or_else(|_| "configs/channels/wechat.toml".to_string());
    let raw = std::fs::read_to_string(&config_path)
        .with_context(|| format!("read wechat config failed: {config_path}"))?;
    let config: AppConfig = toml::from_str(&raw).context("parse wechat config failed")?;
    if !config.wechat.enabled {
        anyhow::bail!("wechat.enabled=false; nothing to do");
    }
    if config.wechat.listen.trim().is_empty()
        || config.wechat.clawd_base_url.trim().is_empty()
        || config.wechat.api_base_url.trim().is_empty()
    {
        anyhow::bail!("wechatd requires listen, clawd_base_url, and api_base_url");
    }

    let workspace_root = workspace_root_from_config_path(&config_path);
    let status_path = Arc::new(wechat_runtime_status_file_path(&workspace_root));
    let session_path = wechat_session_file_path(&workspace_root);
    let sync_buf_path = Arc::new(wechat_sync_buf_file_path(&workspace_root));
    let starting = WechatRuntimeStatus {
        healthy: true,
        status: "starting".to_string(),
        last_event_ts: None,
        last_peer: None,
        last_error: None,
        account_label: Some("primary".to_string()),
    };
    write_json_file(&status_path, &starting).await;

    let client = Client::builder()
        .timeout(Duration::from_secs(config.wechat.request_timeout_seconds.max(5)))
        .build()?;
    let persisted_session = load_session_file(&session_path);
    let state = State {
        config: config.wechat.clone(),
        client,
        status: Arc::new(RwLock::new(starting)),
        status_path,
        session_path: Arc::new(session_path),
        session: Arc::new(RwLock::new(persisted_session)),
        active_logins: Arc::new(RwLock::new(HashMap::new())),
        context_tokens: Arc::new(RwLock::new(HashMap::new())),
        sync_buf_path,
        config_cache: Arc::new(Mutex::new(WeixinConfigManager::new())),
    };
    let session_snapshot = state.session.read().await.clone();
    update_status(&state, |status| {
        status.healthy = true;
        status.status = if session_token(&state.config, session_snapshot.as_ref()).is_some() {
            "polling".to_string()
        } else {
            "login_required".to_string()
        };
        status.account_label = session_snapshot
            .as_ref()
            .and_then(|s| s.account_id.clone())
            .or_else(|| Some("primary".to_string()));
        status.last_error = None;
    })
    .await;

    tokio::spawn(monitor_wechat_loop(state.clone()));

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/login/status", get(login_status))
        .route("/login/qr/start", post(login_qr_start))
        .route("/login/qr/wait", post(login_qr_wait))
        .with_state(state);
    info!("wechatd: listening on {}", config.wechat.listen);
    let listener = tokio::net::TcpListener::bind(&config.wechat.listen).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_login_status_response, extract_text_message, qr_render_content, qr_svg_data_url,
        wechat_runtime_status_file_path, workspace_root_from_config_path, ActiveLogin, MessageItem,
        QRCodeResponse, TextItem, VoiceItem, WechatRuntimeStatus, WeixinMessage,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn workspace_root_comes_from_channel_config_path() {
        let root = workspace_root_from_config_path("/tmp/demo/configs/channels/wechat.toml");
        assert_eq!(root, PathBuf::from("/tmp/demo"));
    }

    #[test]
    fn runtime_status_path_is_under_run_directory() {
        let path = wechat_runtime_status_file_path(Path::new("/tmp/demo"));
        assert_eq!(
            path,
            PathBuf::from("/tmp/demo/run/wechatd-status/primary.json")
        );
    }

    #[test]
    fn qr_svg_data_url_returns_svg_data_uri() {
        let data_url = qr_svg_data_url("https://example.com/qr-login").expect("qr svg");
        assert!(data_url.starts_with("data:image/svg+xml;base64,"));
        assert!(data_url.len() > "data:image/svg+xml;base64,".len());
    }

    #[test]
    fn qr_render_content_prefers_img_content() {
        let response = QRCodeResponse {
            qrcode: "909101143a13a8526f377cf9f2655903".to_string(),
            qrcode_img_content: "https://example.com/wechat-login".to_string(),
        };

        assert_eq!(
            qr_render_content(&response),
            "https://example.com/wechat-login"
        );
    }

    #[test]
    fn qr_render_content_falls_back_to_qrcode_id() {
        let response = QRCodeResponse {
            qrcode: "909101143a13a8526f377cf9f2655903".to_string(),
            qrcode_img_content: "   ".to_string(),
        };

        assert_eq!(
            qr_render_content(&response),
            "909101143a13a8526f377cf9f2655903"
        );
    }

    #[test]
    fn login_status_response_includes_session_key_for_active_qr() {
        let status = WechatRuntimeStatus {
            healthy: true,
            status: "qr_ready".to_string(),
            last_event_ts: Some(123),
            last_peer: None,
            last_error: None,
            account_label: Some("primary".to_string()),
        };
        let active = ActiveLogin {
            session_key: "primary".to_string(),
            qrcode: "qr-id".to_string(),
            qrcode_url: "data:image/svg+xml;base64,abc".to_string(),
            started_at_ms: 100,
            status: "wait".to_string(),
            message: "二维码已生成".to_string(),
        };

        let response = build_login_status_response(&status, Some(&active));

        assert_eq!(response.session_key.as_deref(), Some("primary"));
        assert_eq!(response.qr_status.as_deref(), Some("wait"));
        assert_eq!(response.qrcode_url.as_deref(), Some("data:image/svg+xml;base64,abc"));
    }

    #[test]
    fn login_status_response_omits_session_key_without_active_qr() {
        let status = WechatRuntimeStatus {
            healthy: true,
            status: "connected".to_string(),
            last_event_ts: Some(123),
            last_peer: None,
            last_error: None,
            account_label: Some("bot-1".to_string()),
        };

        let response = build_login_status_response(&status, None);

        assert!(response.session_key.is_none());
        assert_eq!(response.connected, true);
        assert_eq!(response.qr_ready, false);
    }

    #[test]
    fn extract_text_message_prefers_text_items() {
        let msg = WeixinMessage {
            from_user_id: Some("u1".to_string()),
            _to_user_id: None,
            create_time_ms: None,
            item_list: Some(vec![MessageItem {
                r#type: Some(1),
                text_item: Some(TextItem {
                    text: Some("hello".to_string()),
                }),
                voice_item: None,
            }]),
            context_token: Some("ctx".to_string()),
        };
        assert_eq!(extract_text_message(&msg).as_deref(), Some("hello"));
    }

    #[test]
    fn extract_text_message_falls_back_to_voice_transcript() {
        let msg = WeixinMessage {
            from_user_id: Some("u1".to_string()),
            _to_user_id: None,
            create_time_ms: None,
            item_list: Some(vec![MessageItem {
                r#type: Some(3),
                text_item: None,
                voice_item: Some(VoiceItem {
                    text: Some("voice text".to_string()),
                }),
            }]),
            context_token: None,
        };
        assert_eq!(extract_text_message(&msg).as_deref(), Some("voice text"));
    }
}
