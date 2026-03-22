mod config_cache;
mod config_section;
mod ilink;
mod wechat_silk_wav;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use axum::extract::State as AxumState;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use claw_core::channel_chunk::{chunk_text_for_channel, SEGMENT_PREFIX_MAX_CHARS};
use claw_core::wechat_reply_media::{
    extract_wechat_outbound_media, strip_wechat_delivery_lines, WechatOutboundKind,
};
use claw_core::types::{
    ApiResponse, AuthIdentity, BindChannelKeyRequest, ChannelKind, ResolveChannelBindingRequest,
    ResolveChannelBindingResponse, SubmitTaskRequest, SubmitTaskResponse, TaskKind,
    TaskQueryResponse, TaskStatus,
};
use config_section::{AppConfig, WechatSection};
use config_cache::WeixinConfigManager;
use qrcodegen::{QrCode, QrCodeEcc};
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};
use wechat_ilink::http::IlinkAuth;
use wechat_ilink::{
    download_decrypted_media, parse_aes_key_base64, parse_aes_key_hex_or_base64_media,
    send_weixin_file_from_file, send_weixin_image_from_file, send_weixin_video_from_file,
};

const SESSION_EXPIRED_ERRCODE: i64 = -14;
const MAX_CONSECUTIVE_FAILURES: usize = 3;
const RETRY_DELAY_MS: u64 = 2_000;
const BACKOFF_DELAY_MS: u64 = 30_000;
const ACTIVE_LOGIN_TTL_MS: u64 = 5 * 60_000;
const WECHAT_TEXT_CHUNK_CHARS: usize = 1200;
const WECHATD_CHANNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

fn wechat_ilink_auth(sec: &WechatSection) -> IlinkAuth<'_> {
    IlinkAuth {
        sk_route_tag: sec.sk_route_tag.as_str(),
        wechat_uin_base64: sec.wechat_uin_base64.as_str(),
    }
}

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
    workspace_root: PathBuf,
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
struct CdnMedia {
    #[serde(default)]
    encrypt_query_param: Option<String>,
    #[serde(default)]
    aes_key: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    encrypt_type: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct ImageItemSerde {
    #[serde(default)]
    media: Option<CdnMedia>,
    #[serde(default)]
    aeskey: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct VideoItemSerde {
    #[serde(default)]
    media: Option<CdnMedia>,
    #[serde(default)]
    aeskey: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileItemSerde {
    #[serde(default)]
    media: Option<CdnMedia>,
    #[serde(default)]
    file_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MessageItem {
    #[serde(default)]
    r#type: Option<i64>,
    #[serde(default)]
    ref_msg: Option<RefMessage>,
    #[serde(default)]
    text_item: Option<TextItem>,
    #[serde(default)]
    voice_item: Option<VoiceItem>,
    #[serde(default)]
    image_item: Option<ImageItemSerde>,
    #[serde(default)]
    video_item: Option<VideoItemSerde>,
    #[serde(default)]
    file_item: Option<FileItemSerde>,
}

#[derive(Debug, Clone, Deserialize)]
struct RefMessage {
    #[serde(default)]
    message_item: Option<Box<MessageItem>>,
    #[serde(default)]
    title: Option<String>,
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
    #[serde(default)]
    media: Option<CdnMedia>,
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

fn is_media_item(item: &MessageItem) -> bool {
    matches!(item.r#type, Some(2 | 3 | 4 | 5))
}

fn body_from_message_item(item: &MessageItem) -> String {
    if item.r#type == Some(1) {
        let text = item
            .text_item
            .as_ref()
            .and_then(|v| v.text.as_deref())
            .map(str::trim)
            .unwrap_or("");
        if text.is_empty() {
            return String::new();
        }
        let Some(ref_msg) = item.ref_msg.as_ref() else {
            return text.to_string();
        };
        if ref_msg
            .message_item
            .as_deref()
            .map(is_media_item)
            .unwrap_or(false)
        {
            return text.to_string();
        }
        let mut quoted_parts = Vec::new();
        if let Some(title) = ref_msg.title.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            quoted_parts.push(title.to_string());
        }
        if let Some(ref_item) = ref_msg.message_item.as_deref() {
            let ref_body = body_from_message_item(ref_item);
            if !ref_body.trim().is_empty() {
                quoted_parts.push(ref_body);
            }
        }
        if quoted_parts.is_empty() {
            text.to_string()
        } else {
            format!("[引用: {}]\n{}", quoted_parts.join(" | "), text)
        }
    } else if item.r#type == Some(3) {
        item.voice_item
            .as_ref()
            .and_then(|v| v.text.as_deref())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .unwrap_or_default()
    } else {
        String::new()
    }
}

fn body_from_item_list(items: &[MessageItem]) -> String {
    for item in items {
        let body = body_from_message_item(item);
        if !body.trim().is_empty() {
            return body;
        }
    }
    String::new()
}

fn first_item_or_ref_item(
    msg: &WeixinMessage,
    mut matches: impl FnMut(&MessageItem) -> bool,
) -> Option<MessageItem> {
    let items = msg.item_list.as_ref()?;
    for item in items {
        if matches(item) {
            return Some(item.clone());
        }
    }
    for item in items {
        let Some(ref_item) = item.ref_msg.as_ref().and_then(|v| v.message_item.as_deref()) else {
            continue;
        };
        if matches(ref_item) {
            return Some(ref_item.clone());
        }
    }
    None
}

fn extract_text_message(msg: &WeixinMessage) -> Option<String> {
    let body = body_from_item_list(msg.item_list.as_ref()?);
    (!body.trim().is_empty()).then_some(body)
}

/// True when the message carries image / video / file / raw voice items (no usable text).
fn has_non_text_media_items(msg: &WeixinMessage) -> bool {
    first_item_or_ref_item(msg, |it| {
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
            return voice_text.is_empty();
        }
        false
    })
    .is_some()
}

fn safe_inbox_user_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn build_wechat_inbox_rel_path(root_dir: &str, user_id: &str, file_name: &str) -> String {
    let base = root_dir.trim().trim_end_matches('/');
    let seg = safe_inbox_user_segment(user_id);
    let safe_name = sanitize_inbox_filename(file_name);
    if base.is_empty() {
        format!("{seg}/{safe_name}")
    } else {
        format!("{base}/{seg}/{safe_name}")
    }
}

fn inbound_image_decrypt_params(msg: &WeixinMessage) -> Option<(String, [u8; 16])> {
    let it = first_item_or_ref_item(msg, |it| {
        it.r#type == Some(2)
            && it
                .image_item
                .as_ref()
                .and_then(|img| img.media.as_ref())
                .and_then(|media| media.encrypt_query_param.as_deref())
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
    })?;
    let img = it.image_item.as_ref()?;
    let media = img.media.as_ref()?;
    let ep = media.encrypt_query_param.as_deref()?.trim();
    let key = parse_aes_key_hex_or_base64_media(
        img.aeskey.as_deref(),
        media.aes_key.as_deref(),
        "inbound-image",
    )
    .ok()?;
    Some((ep.to_string(), key))
}

fn inbound_voice_decrypt_params(msg: &WeixinMessage) -> Option<(String, [u8; 16])> {
    let it = first_item_or_ref_item(msg, |it| {
        if it.r#type != Some(3) {
            return false;
        }
        let Some(vo) = it.voice_item.as_ref() else {
            return false;
        };
        if !vo.text.as_deref().map(str::trim).unwrap_or("").is_empty() {
            return false;
        }
        vo.media
            .as_ref()
            .and_then(|media| media.encrypt_query_param.as_deref())
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    })?;
    let vo = it.voice_item.as_ref()?;
    let media = vo.media.as_ref()?;
    let ep = media.encrypt_query_param.as_deref()?.trim();
    let ak = media.aes_key.as_deref()?;
    let key = parse_aes_key_base64(ak, "inbound-voice").ok()?;
    Some((ep.to_string(), key))
}

fn inbound_video_decrypt_params(msg: &WeixinMessage) -> Option<(String, [u8; 16])> {
    let it = first_item_or_ref_item(msg, |it| {
        it.r#type == Some(5)
            && it
                .video_item
                .as_ref()
                .and_then(|v| v.media.as_ref())
                .and_then(|media| media.encrypt_query_param.as_deref())
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
    })?;
    let v = it.video_item.as_ref()?;
    let media = v.media.as_ref()?;
    let ep = media.encrypt_query_param.as_deref()?.trim();
    let key = parse_aes_key_hex_or_base64_media(
        v.aeskey.as_deref(),
        media.aes_key.as_deref(),
        "inbound-video",
    )
    .ok()?;
    Some((ep.to_string(), key))
}

fn inbound_file_decrypt_params(msg: &WeixinMessage) -> Option<(String, [u8; 16], String)> {
    let it = first_item_or_ref_item(msg, |it| {
        it.r#type == Some(4)
            && it
                .file_item
                .as_ref()
                .and_then(|f| f.media.as_ref())
                .and_then(|media| media.encrypt_query_param.as_deref())
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
    })?;
    let f = it.file_item.as_ref()?;
    let media = f.media.as_ref()?;
    let ep = media.encrypt_query_param.as_deref()?.trim();
    let ak = media.aes_key.as_deref()?;
    let key = parse_aes_key_base64(ak, "inbound-file").ok()?;
    let raw = f.file_name.as_deref().unwrap_or("attachment.bin").trim();
    let safe = sanitize_inbox_filename(raw);
    Some((ep.to_string(), key, safe))
}

fn sanitize_inbox_filename(name: &str) -> String {
    let name = name.trim();
    let base = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let mut s: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        s = "attachment.bin".to_string();
    }
    if s.len() > 120 {
        s.truncate(120);
    }
    s
}

fn inbox_rel_suits_doc_parse(rel: &str) -> bool {
    Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "pdf" | "docx" | "md" | "txt" | "html" | "htm"
            )
        })
        .unwrap_or(false)
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
    let url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        "ilink/bot/getupdates"
    );
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("AuthorizationType", "ilink_bot_token")
        .header("Authorization", format!("Bearer {token}"))
        .header("X-WECHAT-UIN", ilink::build_wechat_uin_header(config))
        .json(&GetUpdatesReq {
            get_updates_buf,
            base_info: ilink::base_info(),
        })
        .timeout(Duration::from_millis(timeout_ms.max(1_000)));
    req = ilink::apply_route_tag(req, config);
    let response = match req.send().await {
        Ok(response) => response,
        Err(err) if err.is_timeout() => {
            return Ok(GetUpdatesResp {
                ret: Some(0),
                errcode: None,
                errmsg: None,
                msgs: Vec::new(),
                get_updates_buf: (!get_updates_buf.is_empty()).then_some(get_updates_buf.to_string()),
                longpolling_timeout_ms: None,
            });
        }
        Err(err) => return Err(format!("wechat request failed: {err}")),
    };
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("wechat request status={status} body={body}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("getupdates decode failed: {e}"))
}

fn normalized_context_token(context_token: Option<&str>) -> Option<&str> {
    context_token.map(str::trim).filter(|v| !v.is_empty())
}

fn context_token_store_key(account_id: &str, user_id: &str) -> String {
    format!("{account_id}:{user_id}")
}

fn session_account_id(session: Option<&PersistedSession>) -> String {
    session
        .and_then(|s| s.account_id.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("primary")
        .to_string()
}

async fn remember_context_token(state: &State, user_id: &str, token: &str) {
    let account_id = {
        let session = state.session.read().await;
        session_account_id(session.as_ref())
    };
    state
        .context_tokens
        .write()
        .await
        .insert(context_token_store_key(&account_id, user_id), token.to_string());
}

async fn resolve_delivery_context_token(
    state: &State,
    user_id: &str,
    explicit: Option<&str>,
) -> Option<String> {
    if let Some(token) = normalized_context_token(explicit) {
        remember_context_token(state, user_id, token).await;
        return Some(token.to_string());
    }
    let account_id = {
        let session = state.session.read().await;
        session_account_id(session.as_ref())
    };
    state
        .context_tokens
        .read()
        .await
        .get(&context_token_store_key(&account_id, user_id))
        .cloned()
}

fn markdown_to_plain_text(text: &str) -> String {
    static CODE_BLOCK_RE: OnceLock<Regex> = OnceLock::new();
    static IMAGE_RE: OnceLock<Regex> = OnceLock::new();
    static LINK_RE: OnceLock<Regex> = OnceLock::new();
    static TABLE_SEP_RE: OnceLock<Regex> = OnceLock::new();
    static HEADING_RE: OnceLock<Regex> = OnceLock::new();
    static QUOTE_RE: OnceLock<Regex> = OnceLock::new();
    static LIST_RE: OnceLock<Regex> = OnceLock::new();
    static ORDERED_LIST_RE: OnceLock<Regex> = OnceLock::new();
    static BOLD_STAR_RE: OnceLock<Regex> = OnceLock::new();
    static BOLD_UNDERSCORE_RE: OnceLock<Regex> = OnceLock::new();
    static ITALIC_STAR_RE: OnceLock<Regex> = OnceLock::new();
    static ITALIC_UNDERSCORE_RE: OnceLock<Regex> = OnceLock::new();

    let mut result = text.replace("\r\n", "\n");
    result = CODE_BLOCK_RE
        .get_or_init(|| Regex::new(r"(?s)```[^\n]*\n?(.*?)```").expect("valid code block regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = IMAGE_RE
        .get_or_init(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").expect("valid image regex"))
        .replace_all(&result, "")
        .into_owned();
    result = LINK_RE
        .get_or_init(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").expect("valid link regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = TABLE_SEP_RE
        .get_or_init(|| Regex::new(r"(?m)^\|[\s:|-]+\|$").expect("valid table separator regex"))
        .replace_all(&result, "")
        .into_owned();

    let mut lines = Vec::new();
    for line in result.lines() {
        let trimmed = line.trim();
        let mut normalized = if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() >= 2 {
            trimmed[1..trimmed.len() - 1]
                .split('|')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join("  ")
        } else {
            line.to_string()
        };
        normalized = HEADING_RE
            .get_or_init(|| Regex::new(r"^\s{0,3}#{1,6}\s+").expect("valid heading regex"))
            .replace(&normalized, "")
            .into_owned();
        normalized = QUOTE_RE
            .get_or_init(|| Regex::new(r"^\s*>\s?").expect("valid quote regex"))
            .replace(&normalized, "")
            .into_owned();
        normalized = LIST_RE
            .get_or_init(|| Regex::new(r"^\s*[-*+]\s+").expect("valid list regex"))
            .replace(&normalized, "")
            .into_owned();
        normalized = ORDERED_LIST_RE
            .get_or_init(|| Regex::new(r"^\s*\d+\.\s+").expect("valid ordered list regex"))
            .replace(&normalized, "")
            .into_owned();
        lines.push(normalized);
    }

    result = lines.join("\n");
    result = BOLD_STAR_RE
        .get_or_init(|| Regex::new(r"\*\*([^*\n]+)\*\*").expect("valid bold star regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = BOLD_UNDERSCORE_RE
        .get_or_init(|| Regex::new(r"__([^_\n]+)__").expect("valid bold underscore regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = ITALIC_STAR_RE
        .get_or_init(|| Regex::new(r"\*([^*\n]+)\*").expect("valid italic star regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = ITALIC_UNDERSCORE_RE
        .get_or_init(|| Regex::new(r"_([^_\n]+)_").expect("valid italic underscore regex"))
        .replace_all(&result, "$1")
        .into_owned();
    result = result.replace("~~", "");
    result = result.replace('`', "");

    let mut compact = Vec::new();
    let mut last_blank = false;
    for line in result.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            if !last_blank {
                compact.push(String::new());
            }
            last_blank = true;
        } else {
            compact.push(trimmed.to_string());
            last_blank = false;
        }
    }
    compact.join("\n").trim().to_string()
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
    let Some(context_token) = normalized_context_token(context_token) else {
        return Err("sendmessage requires context_token".to_string());
    };
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
                context_token: Some(context_token.to_string()),
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

async fn send_text_reply_via_session(
    state: &State,
    to_user_id: &str,
    context_token: Option<&str>,
    text: &str,
) {
    let session_guard = state.session.read().await;
    let token = session_token(&state.config, session_guard.as_ref());
    let base_url = session_base_url(&state.config, session_guard.as_ref());
    drop(session_guard);
    let Some(token) = token else {
        return;
    };
    let Some(context_token) = resolve_delivery_context_token(state, to_user_id, context_token).await else {
        return;
    };
    let _ = send_text_message(
        &state.client,
        &state.config,
        &base_url,
        &token,
        to_user_id,
        Some(context_token.as_str()),
        text,
    )
    .await;
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
    let context_token = resolve_delivery_context_token(state, from_user_id, context_token).await;
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
            context_token.as_deref(),
        )
        .await;
    let t = ticket.trim();
    if t.is_empty() {
        None
    } else {
        Some(ticket)
    }
}

async fn deliver_wechat_clawd_reply(
    state: &State,
    from_user_id: &str,
    context_token: Option<&str>,
    reply_text: &str,
) {
    let session_guard = state.session.read().await;
    let Some(token) = session_token(&state.config, session_guard.as_ref()) else {
        warn!("wechatd: deliver reply skipped (no session token)");
        return;
    };
    let base_url = session_base_url(&state.config, session_guard.as_ref());
    drop(session_guard);
    let Some(context_token) = resolve_delivery_context_token(state, from_user_id, context_token).await else {
        warn!("wechatd: deliver reply skipped (missing context_token)");
        return;
    };
    let timeout_ms = state.config.request_timeout_seconds.max(1) * 1_000;
    let cdn = state.config.cdn_base_url.trim();
    let auth = wechat_ilink_auth(&state.config);
    let media =
        extract_wechat_outbound_media(reply_text, &state.workspace_root);
    let stripped = markdown_to_plain_text(&strip_wechat_delivery_lines(reply_text));
    let no_outbound_media = media.is_empty();
    if !stripped.trim().is_empty() {
        if let Err(err) = send_text_message(
            &state.client,
            &state.config,
            &base_url,
            &token,
            from_user_id,
            Some(context_token.as_str()),
            stripped.trim(),
        )
        .await
        {
            warn!("wechatd: send reply text failed err={}", err);
        }
    }
    for (p, kind) in &media {
        let res = match kind {
            WechatOutboundKind::Image => {
                send_weixin_image_from_file(
                    &state.client,
                    &base_url,
                    &token,
                    auth,
                    cdn,
                    from_user_id,
                    Some(context_token.as_str()),
                    p,
                    WECHATD_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await
            }
            WechatOutboundKind::Video => {
                send_weixin_video_from_file(
                    &state.client,
                    &base_url,
                    &token,
                    auth,
                    cdn,
                    from_user_id,
                    Some(context_token.as_str()),
                    p,
                    WECHATD_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await
            }
            WechatOutboundKind::File => {
                let fname = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file");
                send_weixin_file_from_file(
                    &state.client,
                    &base_url,
                    &token,
                    auth,
                    cdn,
                    from_user_id,
                    Some(context_token.as_str()),
                    p,
                    fname,
                    WECHATD_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await
            }
        };
        if let Err(err) = res {
            warn!("wechatd: send reply media {:?} kind={:?} err={}", p, kind, err);
        }
    }
    if stripped.trim().is_empty() && no_outbound_media && !reply_text.trim().is_empty() {
        let fallback_text = markdown_to_plain_text(reply_text);
        if let Err(err) = send_text_message(
            &state.client,
            &state.config,
            &base_url,
            &token,
            from_user_id,
            Some(context_token.as_str()),
            &fallback_text,
        )
        .await
        {
            warn!("wechatd: send reply fallback text failed err={}", err);
        }
    }
}

async fn submit_wechat_task_with_payload(
    state: State,
    from_user_id: String,
    context_token: Option<String>,
    user_key: Option<String>,
    typing_ticket: Option<String>,
    kind: TaskKind,
    mut payload: Value,
) {
    if let Some(obj) = payload.as_object_mut() {
        obj.entry("channel")
            .or_insert(Value::String("wechat".to_string()));
        if let Some(ref ct) = context_token {
            let t = ct.trim();
            if !t.is_empty() {
                obj.entry("context_token")
                    .or_insert(Value::String(ct.clone()));
            }
        }
    }
    let submit_req = SubmitTaskRequest {
        user_id: Some(stable_i64_from_string(&from_user_id)),
        chat_id: Some(stable_i64_from_string(&from_user_id)),
        user_key: user_key.clone(),
        channel: Some(ChannelKind::Wechat),
        external_user_id: Some(from_user_id.clone()),
        external_chat_id: Some(from_user_id.clone()),
        kind,
        payload,
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
                deliver_wechat_clawd_reply(
                    &state,
                    &from_user_id,
                    context_token.as_deref(),
                    &reply_text,
                )
                .await;
                break;
            }
            TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                let error_text = task
                    .error_text
                    .unwrap_or_else(|| "请求处理失败，请稍后重试。".to_string());
                send_text_reply_via_session(&state, &from_user_id, context_token.as_deref(), &error_text).await;
                break;
            }
        }
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
    let payload = json!({
        "text": text,
        "agent_mode": true,
        "channel": "wechat",
        "context_token": context_token.clone(),
    });
    submit_wechat_task_with_payload(
        state,
        from_user_id,
        context_token,
        user_key,
        typing_ticket,
        TaskKind::Ask,
        payload,
    )
    .await;
}

async fn submit_wechat_run_skill_and_reply(
    state: State,
    from_user_id: String,
    context_token: Option<String>,
    user_key: Option<String>,
    typing_ticket: Option<String>,
    skill_name: &'static str,
    args: Value,
) {
    let payload = json!({
        "skill_name": skill_name,
        "args": args,
    });
    submit_wechat_task_with_payload(
        state,
        from_user_id,
        context_token,
        user_key,
        typing_ticket,
        TaskKind::RunSkill,
        payload,
    )
    .await;
}

async fn spawn_inbound_ask_flow(
    state: State,
    from_user_id: String,
    msg: WeixinMessage,
    ask_text: String,
    prefetched_typing_ticket: Option<String>,
) {
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
        let typing_ticket = prefetched_typing_ticket.filter(|ticket| !ticket.trim().is_empty());
        tokio::spawn(submit_wechat_task_and_reply(
            state,
            from_user_id,
            ask_text,
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
        "",
    )
    .await
    {
        Ok(Some(_)) => {
            send_text_reply_via_session(
                &state,
                &from_user_id,
                msg.context_token.as_deref(),
                "绑定成功。请再发一次该媒体以便处理。",
            )
            .await;
        }
        Ok(None) => {
            send_text_reply_via_session(
                &state,
                &from_user_id,
                msg.context_token.as_deref(),
                "请先发送你的 RustClaw key 完成绑定（文本消息）。",
            )
            .await;
        }
        Err(err) => {
            warn!("wechatd: bind request failed err={}", err);
        }
    }
}

async fn spawn_inbound_skill_flow(
    state: State,
    from_user_id: String,
    msg: WeixinMessage,
    skill_name: &'static str,
    args: Value,
    prefetched_typing_ticket: Option<String>,
) {
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
        let typing_ticket = prefetched_typing_ticket.filter(|ticket| !ticket.trim().is_empty());
        tokio::spawn(submit_wechat_run_skill_and_reply(
            state,
            from_user_id,
            msg.context_token,
            Some(identity.user_key),
            typing_ticket,
            skill_name,
            args,
        ));
        return;
    }
    match bind_wechat_identity(
        &state.client,
        &state.config.clawd_base_url,
        &from_user_id,
        &from_user_id,
        "",
    )
    .await
    {
        Ok(Some(_)) => {
            send_text_reply_via_session(
                &state,
                &from_user_id,
                msg.context_token.as_deref(),
                "绑定成功。请再发一次该媒体以便处理。",
            )
            .await;
        }
        Ok(None) => {
            send_text_reply_via_session(
                &state,
                &from_user_id,
                msg.context_token.as_deref(),
                "请先发送你的 RustClaw key 完成绑定（文本消息）。",
            )
            .await;
        }
        Err(err) => {
            warn!("wechatd: bind request failed err={}", err);
        }
    }
}

async fn handle_incoming_message(state: State, msg: WeixinMessage) {
    let Some(from_user_id) = msg.from_user_id.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string) else {
        return;
    };
    if let Some(token) = msg.context_token.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        remember_context_token(&state, &from_user_id, token).await;
    }
    let prefetched_typing_ticket =
        resolve_typing_ticket_for_peer(&state, &from_user_id, msg.context_token.as_deref()).await;

    if extract_text_message(&msg).is_none() {
        if let Some((ep, key)) = inbound_image_decrypt_params(&msg) {
            let cdn = state.config.cdn_base_url.trim();
            match download_decrypted_media(
                &state.client,
                &ep,
                &key,
                cdn,
                "inbound-image",
            )
            .await
            {
                Ok(bytes) => {
                    if bytes.len() > 25 * 1024 * 1024 {
                        warn!("wechatd: inbound image too large ({} bytes)", bytes.len());
                        return;
                    }
                    let rel = build_wechat_inbox_rel_path(
                        &state.config.image_inbox_dir,
                        &from_user_id,
                        &format!("{}.jpg", current_ts_ms()),
                    );
                    let abs = state.workspace_root.join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&abs, &bytes).await.is_err() {
                        warn!("wechatd: failed to write inbound image {}", rel);
                        return;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    return spawn_inbound_skill_flow(
                        state,
                        from_user_id,
                        msg,
                        "image_vision",
                        json!({
                            "action": "describe",
                            "images": [{"path": rel}],
                            "detail_level": "normal"
                        }),
                        prefetched_typing_ticket.clone(),
                    )
                    .await;
                }
                Err(err) => {
                    warn!("wechatd: inbound image decrypt/download failed: {}", err);
                }
            }
        }
        if let Some((ep, key)) = inbound_video_decrypt_params(&msg) {
            let cdn = state.config.cdn_base_url.trim();
            match download_decrypted_media(
                &state.client,
                &ep,
                &key,
                cdn,
                "inbound-video",
            )
            .await
            {
                Ok(bytes) => {
                    if bytes.len() > 100 * 1024 * 1024 {
                        warn!("wechatd: inbound video too large");
                        return;
                    }
                    let rel = build_wechat_inbox_rel_path(
                        &state.config.video_inbox_dir,
                        &from_user_id,
                        &format!("{}.mp4", current_ts_ms()),
                    );
                    let abs = state.workspace_root.join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&abs, &bytes).await.is_err() {
                        warn!("wechatd: failed to write inbound video {}", rel);
                        return;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    return;
                }
                Err(err) => {
                    warn!("wechatd: inbound video decrypt/download failed: {}", err);
                }
            }
        }
        if let Some((ep, key, safe_name)) = inbound_file_decrypt_params(&msg) {
            let cdn = state.config.cdn_base_url.trim();
            match download_decrypted_media(
                &state.client,
                &ep,
                &key,
                cdn,
                "inbound-file",
            )
            .await
            {
                Ok(bytes) => {
                    if bytes.len() > 100 * 1024 * 1024 {
                        warn!("wechatd: inbound file too large");
                        return;
                    }
                    let rel = build_wechat_inbox_rel_path(
                        &state.config.file_inbox_dir,
                        &from_user_id,
                        &format!("{}_{}", current_ts_ms(), safe_name),
                    );
                    let abs = state.workspace_root.join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&abs, &bytes).await.is_err() {
                        warn!("wechatd: failed to write inbound file {}", rel);
                        return;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    if inbox_rel_suits_doc_parse(&rel) {
                        return spawn_inbound_skill_flow(
                            state,
                            from_user_id,
                            msg,
                            "doc_parse",
                            json!({
                                "action": "parse_doc",
                                "path": rel,
                                "max_chars": 12000,
                                "include_metadata": true,
                                "table_mode": "basic"
                            }),
                            prefetched_typing_ticket.clone(),
                        )
                        .await;
                    }
                    let hint = format!(
                        "用户发来文件「{}」，已保存为工作区相对路径：{}。请根据能力回复或调用工具处理。",
                        safe_name, rel
                    );
                    return spawn_inbound_ask_flow(
                        state,
                        from_user_id,
                        msg,
                        hint,
                        prefetched_typing_ticket.clone(),
                    )
                    .await;
                }
                Err(err) => {
                    warn!("wechatd: inbound file decrypt/download failed: {}", err);
                }
            }
        }
        if let Some((ep, key)) = inbound_voice_decrypt_params(&msg) {
            let cdn = state.config.cdn_base_url.trim();
            match download_decrypted_media(
                &state.client,
                &ep,
                &key,
                cdn,
                "inbound-voice",
            )
            .await
            {
                Ok(bytes) => {
                    if bytes.len() > 20 * 1024 * 1024 {
                        warn!("wechatd: inbound voice too large");
                        return;
                    }
                    let ts = current_ts_ms();
                    let (rel, data_to_write) =
                        if let Some(wav) = wechat_silk_wav::try_silk_to_wav(&bytes) {
                            (
                                build_wechat_inbox_rel_path(
                                    &state.config.audio_inbox_dir,
                                    &from_user_id,
                                    &format!("v{}.wav", ts),
                                ),
                                wav,
                            )
                        } else {
                            (
                                build_wechat_inbox_rel_path(
                                    &state.config.audio_inbox_dir,
                                    &from_user_id,
                                    &format!("v{}.bin", ts),
                                ),
                                bytes,
                            )
                        };
                    let abs = state.workspace_root.join(&rel);
                    if let Some(parent) = abs.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if tokio::fs::write(&abs, &data_to_write).await.is_err() {
                        warn!("wechatd: failed to write inbound voice {}", rel);
                        return;
                    }
                    update_status(&state, |status| {
                        status.healthy = true;
                        status.status = "message_received".to_string();
                        status.last_event_ts = msg.create_time_ms.or(Some(current_ts_ms()));
                        status.last_peer = Some(from_user_id.clone());
                        status.last_error = None;
                    })
                    .await;
                    return spawn_inbound_skill_flow(
                        state,
                        from_user_id,
                        msg,
                        "audio_transcribe",
                        json!({ "audio": { "path": rel } }),
                        prefetched_typing_ticket.clone(),
                    )
                    .await;
                }
                Err(err) => {
                    warn!("wechatd: inbound voice decrypt/download failed: {}", err);
                }
            }
        }
    }

    let text = match extract_text_message(&msg) {
        Some(t) => t,
        None => {
            if has_non_text_media_items(&msg) {
                send_text_reply_via_session(
                    &state,
                    &from_user_id,
                    msg.context_token.as_deref(),
                    "收到媒体消息但未能完成 CDN 解密或类型不受支持（如部分系统表情等）。",
                )
                .await;
            }
            return;
        }
    };
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
        tokio::spawn(submit_wechat_task_and_reply(
            state,
            from_user_id,
            text,
            msg.context_token,
            Some(identity.user_key),
            prefetched_typing_ticket,
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
            send_text_reply_via_session(
                &state,
                &from_user_id,
                msg.context_token.as_deref(),
                "绑定成功，请重新发送你的问题。",
            )
            .await;
        }
        Ok(None) => {
            send_text_reply_via_session(
                &state,
                &from_user_id,
                msg.context_token.as_deref(),
                "请先发送你的 RustClaw key 完成绑定。",
            )
            .await;
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
        workspace_root: workspace_root.clone(),
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
                image_item: None,
                video_item: None,
                file_item: None,
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
                    media: None,
                }),
                image_item: None,
                video_item: None,
                file_item: None,
            }]),
            context_token: None,
        };
        assert_eq!(extract_text_message(&msg).as_deref(), Some("voice text"));
    }
}
