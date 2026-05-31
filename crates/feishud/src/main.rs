//! Feishu (Lark) 应用机器人通道
//! 支持两种入站模式：webhook（事件回调）、long_connection（长连接收事件）
//! 文本 / 图片 / 文件 / 音频 / 视频等媒体：下载落盘（可配置目录）后提交 clawd ask。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use claw_core::channel_commands::ChannelCommandCatalog;
use claw_core::channel_i18n::{text_from_path, text_with_vars_from_path};
use claw_core::types::{
    ApiResponse, AuthIdentity, BindChannelKeyRequest, ChannelKind, DetectFeishuBindSessionRequest,
    DetectFeishuBindSessionResponse, FeishuBindSessionStatusResponse, ResolveChannelBindingRequest,
    ResolveChannelBindingResponse, SubmitTaskRequest, SubmitTaskResponse, TaskKind,
    TaskQueryResponse, TaskStatus,
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[derive(Clone)]
struct AppState {
    config: FeishuConfig,
    client: Client,
    /// tenant_access_token 缓存 (token, expires_at_secs)
    token_cache: Arc<RwLock<Option<(String, u64)>>>,
    /// 工作区根目录（用于解析相对落盘路径）
    workspace_root: PathBuf,
    /// 未绑定用户等待 key 回填状态（按 chat_id）
    pending_key_bind_by_chat: Arc<Mutex<HashSet<String>>>,
}

#[derive(Clone, Deserialize)]
struct FeishuConfig {
    #[serde(default)]
    feishu: FeishuSection,
}

/// 入站模式：webhook = 飞书回调本服务；long_connection = 本服务主动连飞书收事件
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FeishuMode {
    #[default]
    Webhook,
    LongConnection,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct FeishuSection {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    mode: FeishuMode,
    #[serde(default = "default_listen")]
    listen: String,
    #[serde(default = "default_clawd_base_url")]
    clawd_base_url: String,
    /// Open API 根地址（中国站默认 open.feishu.cn；国际 Lark 用 open.larksuite.com）
    #[serde(default = "default_feishu_api_base_url")]
    api_base_url: String,
    #[serde(default)]
    app_id: String,
    #[serde(default)]
    app_secret: String,
    #[serde(default)]
    verification_token: String,
    #[serde(default)]
    encrypt_key: String,
    #[serde(default = "default_request_timeout")]
    request_timeout_seconds: u64,
    /// 任务投递软超时阈值（秒）；超过后提示“仍在执行”，并继续轮询
    #[serde(default = "default_task_delivery_timeout")]
    task_delivery_timeout_seconds: u64,
    #[serde(default = "default_text_chunk_chars")]
    text_chunk_chars: usize,
    #[serde(default = "default_feishu_language")]
    language: String,
    #[serde(default = "default_feishu_i18n_path")]
    i18n_path: String,
    #[serde(default = "default_feishu_image_inbox_dir")]
    image_inbox_dir: String,
    #[serde(default = "default_feishu_video_inbox_dir")]
    video_inbox_dir: String,
    #[serde(default = "default_feishu_audio_inbox_dir")]
    audio_inbox_dir: String,
    #[serde(default = "default_feishu_file_inbox_dir")]
    file_inbox_dir: String,
}

fn default_listen() -> String {
    "0.0.0.0:8789".to_string()
}
fn default_clawd_base_url() -> String {
    "http://127.0.0.1:8787".to_string()
}
fn default_request_timeout() -> u64 {
    30
}
fn default_task_delivery_timeout() -> u64 {
    600
}
fn default_text_chunk_chars() -> usize {
    4000
}

fn default_feishu_language() -> String {
    "zh-CN".to_string()
}

fn default_feishu_i18n_path() -> String {
    "configs/i18n/feishud.zh-CN.toml".to_string()
}

fn default_feishu_api_base_url() -> String {
    "https://open.feishu.cn".to_string()
}

fn default_feishu_image_inbox_dir() -> String {
    "data/feishud/image".to_string()
}

fn default_feishu_video_inbox_dir() -> String {
    "data/feishud/video".to_string()
}

fn default_feishu_audio_inbox_dir() -> String {
    "data/feishud/audio".to_string()
}

fn default_feishu_file_inbox_dir() -> String {
    "data/feishud/file".to_string()
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn apply_string_env(target: &mut String, key: &str) {
    if let Some(value) = env_non_empty(key) {
        *target = value;
    }
}

fn apply_env_overrides(config: &mut FeishuConfig) {
    apply_string_env(&mut config.feishu.app_id, "FEISHU_APP_ID");
    apply_string_env(&mut config.feishu.app_secret, "FEISHU_APP_SECRET");
    apply_string_env(
        &mut config.feishu.verification_token,
        "FEISHU_VERIFICATION_TOKEN",
    );
    apply_string_env(&mut config.feishu.encrypt_key, "FEISHU_ENCRYPT_KEY");
    apply_string_env(&mut config.feishu.language, "FEISHU_I18N_LANGUAGE");
    apply_string_env(&mut config.feishu.i18n_path, "FEISHU_I18N_PATH");
}

fn resolve_i18n_path(language: &str, configured_path: &str) -> String {
    let lang = language.trim();
    if !lang.is_empty() {
        let candidate = format!("configs/i18n/feishud.{lang}.toml");
        if Path::new(&candidate).exists() {
            return candidate;
        }
    }
    configured_path.to_string()
}

fn feishu_t(config: &FeishuConfig, key: &str, fallback: &str) -> String {
    text_from_path(&config.feishu.i18n_path, key, fallback)
}

fn feishu_t_with(
    config: &FeishuConfig,
    key: &str,
    vars: &[(&str, &str)],
    fallback: &str,
) -> String {
    text_with_vars_from_path(&config.feishu.i18n_path, key, vars, fallback)
}

fn current_ts_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn safe_feishu_storage_segment(raw: &str, fallback: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return fallback.to_string();
    }
    let mut out = String::new();
    for c in t.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

fn build_feishu_inbox_rel_path(root_dir: &str, chat_id: &str, file_name: &str) -> String {
    let seg = safe_feishu_storage_segment(chat_id, "unknown");
    format!("{}/{}/{}", root_dir.trim_end_matches('/'), seg, file_name)
}

/// 解析 `im.message.receive_v1` 公共字段。
fn parse_im_receive_v1(body: &Value) -> Option<(String, String, String, String, Value)> {
    let header = body.get("header")?;
    if header.get("event_type").and_then(|v| v.as_str())? != "im.message.receive_v1" {
        return None;
    }
    let event = body.get("event")?;
    let message = event.get("message")?;
    let message_id = message
        .get("message_id")
        .and_then(|v| v.as_str())?
        .to_string();
    let message_type = message
        .get("message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let content_str = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let content: Value = serde_json::from_str(content_str).ok()?;
    let sender = event.get("sender")?;
    let sender_id = sender.get("sender_id")?;
    let open_id = sender_id
        .get("open_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let chat_id = message
        .get("chat_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if chat_id.is_empty() {
        return None;
    }
    Some((open_id, chat_id, message_id, message_type, content))
}

/// 飞书消息资源下载：`type=image` 或 `type=file`（音频/视频/普通文件均用 file）。
fn feishu_resource_key_and_query_type(
    message_type: &str,
    content: &Value,
) -> Option<(String, &'static str)> {
    match message_type {
        "image" | "sticker" => {
            let key = content.get("image_key").and_then(|v| v.as_str())?;
            Some((key.to_string(), "image"))
        }
        "file" | "audio" | "media" => {
            let key = content.get("file_key").and_then(|v| v.as_str())?;
            Some((key.to_string(), "file"))
        }
        _ => None,
    }
}

fn feishu_inbox_root_for_message_type<'a>(
    message_type: &str,
    section: &'a FeishuSection,
) -> &'a str {
    match message_type {
        "image" | "sticker" => section.image_inbox_dir.as_str(),
        "media" => section.video_inbox_dir.as_str(),
        "audio" => section.audio_inbox_dir.as_str(),
        "file" => section.file_inbox_dir.as_str(),
        _ => section.file_inbox_dir.as_str(),
    }
}

fn feishu_saved_file_name(message_type: &str, content: &Value, ts: u64) -> String {
    if let Some(name) = content.get("file_name").and_then(|v| v.as_str()) {
        let n = name.trim();
        if !n.is_empty() && !n.contains('/') && !n.contains('\\') {
            let safe = safe_feishu_storage_segment(n, "file");
            return format!("{}_{}", ts, safe);
        }
    }
    let ext = match message_type {
        "image" | "sticker" => "jpg",
        "media" => "mp4",
        "audio" => "m4a",
        "file" => "bin",
        _ => "bin",
    };
    format!("{}.{}", ts, ext)
}

fn feishu_media_kind_label_zh(message_type: &str) -> &'static str {
    match message_type {
        "image" => "图片",
        "sticker" => "表情",
        "media" => "视频",
        "audio" => "语音",
        "file" => "文件",
        _ => "媒体",
    }
}

/// 入站媒体（图片 / 文件 / 音频 / 视频）解析结果。
#[derive(Clone)]
struct FeishuMediaCtx {
    open_id: String,
    chat_id: String,
    message_id: String,
    message_type: String,
    resource_key: String,
    query_type: &'static str,
    content: Value,
}

fn parse_im_media_from_event_body(body: &Value) -> Option<FeishuMediaCtx> {
    let (open_id, chat_id, message_id, message_type, content) = parse_im_receive_v1(body)?;
    if message_type == "text" {
        return None;
    }
    let (resource_key, query_type) = feishu_resource_key_and_query_type(&message_type, &content)?;
    Some(FeishuMediaCtx {
        open_id,
        chat_id,
        message_id,
        message_type,
        resource_key,
        query_type,
        content,
    })
}

/// 将飞书字符串 ID 稳定映射为 i64（供 clawd user_id/chat_id 使用）
fn feishu_id_to_i64(s: &str) -> i64 {
    let mut h: i64 = 0;
    for b in s.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as i64);
    }
    h
}

/// 从已解析的 event 请求体（webhook 或等价结构）中解析 im.message.receive_v1 文本消息。
/// 返回 (open_id, chat_id, text)；若非该事件或非文本或缺少 chat_id 则返回 None。
fn parse_im_text_from_event_body(body: &Value) -> Option<(String, String, String)> {
    let (open_id, chat_id, _mid, message_type, content) = parse_im_receive_v1(body)?;
    if message_type != "text" {
        return None;
    }
    let text = content
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if text.is_empty() {
        return None;
    }
    Some((open_id, chat_id, text.to_string()))
}

/// 解析绑定：调用 clawd /v1/auth/channel/resolve，返回已绑定身份（若有）。
async fn resolve_feishu_identity(
    client: &Client,
    base_url: &str,
    open_id: &str,
    chat_id: &str,
) -> Result<Option<AuthIdentity>, String> {
    let url = format!("{}/v1/auth/channel/resolve", base_url.trim_end_matches('/'));
    let req = ResolveChannelBindingRequest {
        channel: ChannelKind::Feishu,
        external_user_id: Some(open_id.to_string()),
        external_chat_id: Some(chat_id.to_string()),
        telegram_bot_name: None,
    };
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("resolve request failed: {}", e))?;
    let status = resp.status();
    let body: ApiResponse<ResolveChannelBindingResponse> = resp
        .json()
        .await
        .map_err(|e| format!("resolve response parse failed: {}", e))?;
    if !status.is_success() || !body.ok {
        return Err(body.error.unwrap_or_else(|| "resolve failed".to_string()));
    }
    Ok(body.data.and_then(|d| d.identity))
}

/// 绑定 key：调用 clawd /v1/auth/channel/bind，成功返回身份。
async fn bind_feishu_identity(
    client: &Client,
    base_url: &str,
    open_id: &str,
    chat_id: &str,
    user_key: &str,
) -> Result<Option<AuthIdentity>, String> {
    let url = format!("{}/v1/auth/channel/bind", base_url.trim_end_matches('/'));
    let req = BindChannelKeyRequest {
        channel: ChannelKind::Feishu,
        external_user_id: Some(open_id.to_string()),
        external_chat_id: Some(chat_id.to_string()),
        telegram_bot_name: None,
        user_key: user_key.trim().to_string(),
    };
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("bind request failed: {}", e))?;
    let status = resp.status();
    let body: ApiResponse<AuthIdentity> = resp
        .json()
        .await
        .map_err(|e| format!("bind response parse failed: {}", e))?;
    if status.as_u16() == 401 || !body.ok {
        return Ok(None);
    }
    Ok(body.data)
}

fn extract_pending_bind_token_candidate(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("/start") {
        let candidate = rest.trim();
        if candidate.starts_with("pb-") {
            return Some(candidate.to_string());
        }
    }
    if trimmed.starts_with("pb-") {
        return Some(trimmed.to_string());
    }
    None
}

async fn detect_pending_feishu_bind(
    client: &Client,
    base_url: &str,
    open_id: &str,
    chat_id: &str,
    bind_token: Option<&str>,
) -> Result<Option<FeishuBindSessionStatusResponse>, String> {
    let url = format!(
        "{}/v1/auth/channel-binds/feishu/detect",
        base_url.trim_end_matches('/')
    );
    let req = DetectFeishuBindSessionRequest {
        bind_token: bind_token.map(|token| token.trim().to_string()),
        external_user_id: open_id.to_string(),
        external_chat_id: chat_id.to_string(),
    };
    let resp = client
        .post(&url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("detect request failed: {}", e))?;
    let status = resp.status();
    let body: ApiResponse<DetectFeishuBindSessionResponse> = resp
        .json()
        .await
        .map_err(|e| format!("detect response parse failed: {}", e))?;
    if !status.is_success() || !body.ok {
        return Err(body.error.unwrap_or_else(|| "detect failed".to_string()));
    }
    Ok(body
        .data
        .and_then(|data| if data.matched { data.session } else { None }))
}

const FEISHU_I18N_IDENTITY_CHECK_UNAVAILABLE_KEY: &str = "feishu.msg.identity_check_unavailable";
const FEISHU_I18N_BIND_REQUIRED_KEY: &str = "feishu.msg.bind_key_required_for_chat";
const FEISHU_I18N_BIND_HELP_KEY: &str = "feishu.msg.bind_help";
const FEISHU_I18N_BIND_SUCCESS_KEY: &str = "feishu.msg.bind_success";
const FEISHU_I18N_BIND_INVALID_KEY: &str = "feishu.msg.bind_invalid";
const FEISHU_I18N_BIND_REQUEST_FAILED_KEY: &str = "feishu.msg.bind_request_failed";
const FEISHU_I18N_MEDIA_DOWNLOAD_FAILED_KEY: &str = "feishu.msg.media_download_failed";
const FEISHU_I18N_MEDIA_FILE_TOO_LARGE_KEY: &str = "feishu.msg.media_file_too_large";
const FEISHU_I18N_REQUEST_TIMEOUT_RETRY_LATER_KEY: &str = "feishu.msg.request_timeout_retry_later";
const FEISHU_I18N_TASK_DONE_FALLBACK_TEXT_KEY: &str = "feishu.msg.task_done_fallback_text";
const FEISHU_I18N_TASK_FAILED_FALLBACK_ERROR_KEY: &str = "feishu.msg.task_failed_fallback_error";
const FEISHU_I18N_PROCESS_FAILED_WITH_ERROR_KEY: &str = "feishu.msg.process_failed_with_error";

const FEISHU_IDENTITY_CHECK_UNAVAILABLE_FALLBACK: &str = "身份校验暂时不可用，请稍后重试。";
const FEISHU_BIND_REQUIRED_FALLBACK: &str =
    "请先发送你的 key 进行绑定，然后再继续聊天或使用功能。\nPlease send your key first to bind this account before chatting or using features.";
const FEISHU_BIND_HELP_FALLBACK: &str =
    "欢迎使用 RustClaw。\n请先发送 /key <your_key> 完成绑定。\nWelcome to RustClaw.\nPlease send /key <your_key> first to bind this account.";
const FEISHU_BIND_SUCCESS_FALLBACK: &str =
    "绑定成功，请重新发送你的问题。\nKey bound successfully. Please send your previous message again.";
const FEISHU_BIND_INVALID_FALLBACK: &str =
    "key 无效或绑定失败，请发送有效 key 完成绑定。\nInvalid key. Please try again.";
const FEISHU_BIND_REQUEST_FAILED_FALLBACK: &str =
    "绑定请求失败，请稍后重试。\nBind request failed, please try again later.";
const FEISHU_MEDIA_DOWNLOAD_FAILED_FALLBACK: &str = "媒体下载失败，请稍后重试。";
const FEISHU_MEDIA_FILE_TOO_LARGE_FALLBACK: &str = "媒体文件过大，已拒绝保存。";
const FEISHU_REQUEST_TIMEOUT_RETRY_LATER_FALLBACK: &str =
    "你的任务正在持续执行（任务编号：{task_id}），执行完了给你回复。";
const FEISHU_TASK_DONE_FALLBACK_TEXT_FALLBACK: &str = "处理完成。";
const FEISHU_TASK_FAILED_FALLBACK_ERROR_FALLBACK: &str = "任务失败";
const FEISHU_PROCESS_FAILED_WITH_ERROR_FALLBACK: &str = "处理失败：{error}";

fn should_expect_key_reply(state: &AppState, chat_id: &str) -> bool {
    state
        .pending_key_bind_by_chat
        .lock()
        .ok()
        .is_some_and(|set| set.contains(chat_id))
}

fn set_expect_key_reply(state: &AppState, chat_id: &str, enabled: bool) {
    if let Ok(mut set) = state.pending_key_bind_by_chat.lock() {
        if enabled {
            set.insert(chat_id.to_string());
        } else {
            set.remove(chat_id);
        }
    }
}

fn is_unbound_allowed_command(text: &str) -> bool {
    static COMMAND_CATALOG: OnceLock<ChannelCommandCatalog> = OnceLock::new();
    COMMAND_CATALOG
        .get_or_init(ChannelCommandCatalog::default)
        .allows_unbound_command(text, "feishu")
}

fn extract_bind_key_candidate(text: &str, expect_key_reply: bool) -> Option<String> {
    let trimmed = text.trim();
    trimmed
        .strip_prefix("/key")
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            if expect_key_reply && !trimmed.is_empty() && !trimmed.starts_with('/') {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
}

/// 入站文本统一入口：先 resolve 绑定，已绑定则提交 ask；未绑定仅允许 /start /help /key 和等待态回 key，其他文本统一提示先绑定。
/// webhook 与 long_connection 均通过此函数复用同一套逻辑。
async fn handle_incoming_feishu_text(
    state: AppState,
    open_id: String,
    chat_id: String,
    text: String,
) {
    let base = state.config.feishu.clawd_base_url.clone();
    let client = state.client.clone();
    let config = state.config.clone();
    let token_cache = state.token_cache.clone();

    info!(
        "feishud: binding resolve start external_chat_id={}",
        chat_id
    );
    let identity = match resolve_feishu_identity(&client, &base, &open_id, &chat_id).await {
        Ok(ident) => ident,
        Err(e) => {
            warn!("feishud: binding resolve failed err={}", e);
            let msg = feishu_t(
                &config,
                FEISHU_I18N_IDENTITY_CHECK_UNAVAILABLE_KEY,
                FEISHU_IDENTITY_CHECK_UNAVAILABLE_FALLBACK,
            );
            let _ = send_feishu_text(&config, &client, &token_cache, &chat_id, &msg).await;
            return;
        }
    };

    if let Some(ident) = identity {
        set_expect_key_reply(&state, &chat_id, false);
        info!(
            "feishud: binding resolve result bound=true external_chat_id={}",
            chat_id
        );
        handle_text_message_to_clawd(state, open_id, chat_id, text, Some(ident.user_key));
        return;
    }

    info!(
        "feishud: binding resolve result bound=false external_chat_id={}",
        chat_id
    );
    let trimmed = text.trim();
    let pending_bind_token = extract_pending_bind_token_candidate(trimmed);
    if let Some(bind_token) = pending_bind_token.as_deref() {
        match detect_pending_feishu_bind(&client, &base, &open_id, &chat_id, Some(bind_token)).await
        {
            Ok(Some(_session)) => {
                set_expect_key_reply(&state, &chat_id, false);
                info!(
                    "feishud: pending bind finalized external_chat_id={}",
                    chat_id
                );
                let msg = feishu_t(
                    &config,
                    FEISHU_I18N_BIND_SUCCESS_KEY,
                    FEISHU_BIND_SUCCESS_FALLBACK,
                );
                let _ = send_feishu_text(&config, &client, &token_cache, &chat_id, &msg).await;
                return;
            }
            Ok(None) => {}
            Err(e) => {
                warn!(
                    "feishud: pending bind detect failed err={} external_chat_id={}",
                    e, chat_id
                );
            }
        }
    }
    if is_unbound_allowed_command(trimmed) {
        set_expect_key_reply(&state, &chat_id, true);
        let msg = feishu_t(
            &config,
            FEISHU_I18N_BIND_HELP_KEY,
            FEISHU_BIND_HELP_FALLBACK,
        );
        let _ = send_feishu_text(&config, &client, &token_cache, &chat_id, &msg).await;
        return;
    }
    let maybe_candidate =
        extract_bind_key_candidate(trimmed, should_expect_key_reply(&state, &chat_id));
    if let Some(candidate) = maybe_candidate {
        info!(
            "feishud: bind attempt external_chat_id={} key_len={}",
            chat_id,
            candidate.len()
        );
        match bind_feishu_identity(&client, &base, &open_id, &chat_id, &candidate).await {
            Ok(Some(_)) => {
                set_expect_key_reply(&state, &chat_id, false);
                info!("feishud: bind success external_chat_id={}", chat_id);
                let msg = feishu_t(
                    &config,
                    FEISHU_I18N_BIND_SUCCESS_KEY,
                    FEISHU_BIND_SUCCESS_FALLBACK,
                );
                let _ = send_feishu_text(&config, &client, &token_cache, &chat_id, &msg).await;
            }
            Ok(None) => {
                set_expect_key_reply(&state, &chat_id, true);
                warn!(
                    "feishud: bind failure (invalid key) external_chat_id={}",
                    chat_id
                );
                let msg = feishu_t(
                    &config,
                    FEISHU_I18N_BIND_INVALID_KEY,
                    FEISHU_BIND_INVALID_FALLBACK,
                );
                let _ = send_feishu_text(&config, &client, &token_cache, &chat_id, &msg).await;
            }
            Err(e) => {
                set_expect_key_reply(&state, &chat_id, true);
                warn!(
                    "feishud: bind request failed err={} external_chat_id={}",
                    e, chat_id
                );
                let msg = feishu_t(
                    &config,
                    FEISHU_I18N_BIND_REQUEST_FAILED_KEY,
                    FEISHU_BIND_REQUEST_FAILED_FALLBACK,
                );
                let _ = send_feishu_text(&config, &client, &token_cache, &chat_id, &msg).await;
            }
        }
        return;
    }
    if trimmed.is_empty() {
        info!(
            "feishud: unbound user prompted for key (empty text) external_chat_id={}",
            chat_id
        );
    }
    set_expect_key_reply(&state, &chat_id, true);
    let msg = feishu_t(
        &config,
        FEISHU_I18N_BIND_REQUIRED_KEY,
        FEISHU_BIND_REQUIRED_FALLBACK,
    );
    let _ = send_feishu_text(&config, &client, &token_cache, &chat_id, &msg).await;
}

/// 入站媒体：下载并落盘后，将提示文本交给 clawd ask（与文本链路一致）。
async fn handle_incoming_feishu_media(state: AppState, ctx: FeishuMediaCtx) {
    let base = state.config.feishu.clawd_base_url.clone();
    let client = state.client.clone();
    let config = state.config.clone();
    let token_cache = state.token_cache.clone();
    let workspace_root = state.workspace_root.clone();

    info!(
        "feishud: media inbound message_type={} chat_id={} message_id={}",
        ctx.message_type, ctx.chat_id, ctx.message_id
    );

    let identity = match resolve_feishu_identity(&client, &base, &ctx.open_id, &ctx.chat_id).await {
        Ok(ident) => ident,
        Err(e) => {
            warn!("feishud: media binding resolve failed err={}", e);
            let msg = feishu_t(
                &config,
                FEISHU_I18N_IDENTITY_CHECK_UNAVAILABLE_KEY,
                FEISHU_IDENTITY_CHECK_UNAVAILABLE_FALLBACK,
            );
            let _ = send_feishu_text(&config, &client, &token_cache, &ctx.chat_id, &msg).await;
            return;
        }
    };

    let Some(ident) = identity else {
        set_expect_key_reply(&state, &ctx.chat_id, true);
        let msg = feishu_t(
            &config,
            FEISHU_I18N_BIND_REQUIRED_KEY,
            FEISHU_BIND_REQUIRED_FALLBACK,
        );
        let _ = send_feishu_text(&config, &client, &token_cache, &ctx.chat_id, &msg).await;
        return;
    };

    let token = match get_tenant_access_token(&config.feishu, &client, &token_cache).await {
        Ok(t) => t,
        Err(e) => {
            warn!("feishud: media token failed err={}", e);
            return;
        }
    };

    let api_base = config.feishu.api_base_url.trim();
    let bytes = match download_feishu_message_resource(
        &client,
        api_base,
        &token,
        &ctx.message_id,
        &ctx.resource_key,
        ctx.query_type,
    )
    .await
    {
        Ok(b) => b,
        Err(e) => {
            warn!("feishud: media download failed err={}", e);
            let msg = feishu_t(
                &config,
                FEISHU_I18N_MEDIA_DOWNLOAD_FAILED_KEY,
                FEISHU_MEDIA_DOWNLOAD_FAILED_FALLBACK,
            );
            let _ = send_feishu_text(&config, &client, &token_cache, &ctx.chat_id, &msg).await;
            return;
        }
    };

    let max_len = match ctx.message_type.as_str() {
        "image" | "sticker" => 25 * 1024 * 1024,
        "audio" => 20 * 1024 * 1024,
        _ => 100 * 1024 * 1024,
    };
    if bytes.len() > max_len {
        warn!(
            "feishud: media too large len={} max={}",
            bytes.len(),
            max_len
        );
        let msg = feishu_t(
            &config,
            FEISHU_I18N_MEDIA_FILE_TOO_LARGE_KEY,
            FEISHU_MEDIA_FILE_TOO_LARGE_FALLBACK,
        );
        let _ = send_feishu_text(&config, &client, &token_cache, &ctx.chat_id, &msg).await;
        return;
    }

    let ts = current_ts_ms();
    let root_dir = feishu_inbox_root_for_message_type(&ctx.message_type, &config.feishu);
    let fname = feishu_saved_file_name(&ctx.message_type, &ctx.content, ts);
    let rel = build_feishu_inbox_rel_path(root_dir, &ctx.chat_id, &fname);
    let abs = workspace_root.join(&rel);
    if let Some(parent) = abs.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            warn!("feishud: create media inbox dir failed err={}", e);
            return;
        }
    }
    if let Err(e) = tokio::fs::write(&abs, &bytes).await {
        warn!("feishud: write media file failed err={}", e);
        return;
    }

    let label = feishu_media_kind_label_zh(&ctx.message_type);
    let hint = format!(
        "用户发来{}，已保存为工作区相对路径：{}。请根据能力回复或调用工具处理。",
        label, rel
    );
    handle_text_message_to_clawd(state, ctx.open_id, ctx.chat_id, hint, Some(ident.user_key));
}

/// webhook / 长连接统一分发：文本走绑定与 ask；媒体先落盘再 ask。
fn dispatch_im_incoming_event(state: AppState, body: Value) {
    if let Some((open_id, chat_id, text)) = parse_im_text_from_event_body(&body) {
        tokio::spawn(handle_incoming_feishu_text(state, open_id, chat_id, text));
        return;
    }
    if let Some(ctx) = parse_im_media_from_event_body(&body) {
        tokio::spawn(handle_incoming_feishu_media(state, ctx));
        return;
    }
    debug!("feishud: im.message.receive_v1 ignored (unsupported type or missing fields)");
}

/// 从成功任务的 result_json 取回复正文：优先逐条发送 messages，其次 text，否则占位。
fn feishu_task_success_messages(task: &TaskQueryResponse, config: &FeishuConfig) -> Vec<String> {
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
            return parts;
        }
    }
    vec![task
        .result_json
        .as_ref()
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            feishu_t(
                config,
                FEISHU_I18N_TASK_DONE_FALLBACK_TEXT_KEY,
                FEISHU_TASK_DONE_FALLBACK_TEXT_FALLBACK,
            )
        })]
}

/// 共享主链：提交任务并 spawn 轮询与回发。供 webhook 与 long_connection 复用。
/// `user_key`: 已绑定身份时传入，否则为 None（未绑定不应调用此函数）。
fn handle_text_message_to_clawd(
    state: AppState,
    open_id: String,
    chat_id: String,
    text: String,
    user_key: Option<String>,
) {
    let user_id = feishu_id_to_i64(if open_id.is_empty() {
        &chat_id
    } else {
        &open_id
    });
    let chat_id_i64 = feishu_id_to_i64(&chat_id);

    let submit_req = SubmitTaskRequest {
        user_id: Some(user_id),
        chat_id: Some(chat_id_i64),
        user_key: user_key.clone(),
        channel: Some(ChannelKind::Feishu),
        external_user_id: Some(open_id.clone()),
        external_chat_id: Some(chat_id.clone()),
        kind: TaskKind::Ask,
        payload: json!({
            "text": text,
            "agent_mode": true
        }),
    };

    let submit_url = format!("{}/v1/tasks", state.config.feishu.clawd_base_url);
    let client = state.client.clone();
    let config = state.config.clone();
    let token_cache = state.token_cache.clone();
    let poll_interval = Duration::from_millis(1500);
    let delivery_timeout_secs = state.config.feishu.task_delivery_timeout_seconds;
    let chunk_chars = state.config.feishu.text_chunk_chars.max(100);
    let user_key_poll = user_key.clone();

    tokio::spawn(async move {
        let submit_resp = match client.post(&submit_url).json(&submit_req).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!("feishud: task submit failed err={}", e);
                return;
            }
        };

        if !submit_resp.status().is_success() {
            let status = submit_resp.status();
            let resp_body = submit_resp.text().await.unwrap_or_default();
            warn!(
                "feishud: task submit failed status={} body_len={}",
                status,
                resp_body.len()
            );
            return;
        }

        let submit_body: ApiResponse<SubmitTaskResponse> = match submit_resp.json().await {
            Ok(b) => b,
            Err(e) => {
                warn!("feishud: task submit response parse failed err={}", e);
                return;
            }
        };

        let Some(data) = submit_body.data else {
            warn!("feishud: task submit no task_id");
            return;
        };
        let task_id = data.task_id.to_string();
        info!(
            "feishud: bound user task submitted task_id={} external_chat_id={}",
            task_id, chat_id
        );
        let running_notice_text = feishu_t_with(
            &config,
            FEISHU_I18N_REQUEST_TIMEOUT_RETRY_LATER_KEY,
            &[("task_id", task_id.as_str())],
            FEISHU_REQUEST_TIMEOUT_RETRY_LATER_FALLBACK,
        );

        let clawd_base = config.feishu.clawd_base_url.clone();
        let chat_id_delivery = chat_id.clone();

        info!(
            "feishud: task delivery started task_id={} chat_id={} task_delivery_timeout_seconds={}",
            task_id, chat_id_delivery, delivery_timeout_secs
        );
        let started = std::time::Instant::now();
        let mut last_seen_status: Option<TaskStatus> = None;
        let mut timeout_logged = false;
        loop {
            let url = format!("{}/v1/tasks/{}", clawd_base, task_id);
            let mut req = client.get(&url);
            if let Some(ref key) = user_key_poll {
                let k = key.trim();
                if !k.is_empty() {
                    req = req.header("X-RustClaw-Key", k);
                }
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    warn!("feishud: poll failed task_id={} err={}", task_id, e);
                    if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                        if !timeout_logged {
                            warn!("feishud: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=poll_failed (continue_polling=true)", task_id, started.elapsed().as_secs(), delivery_timeout_secs, last_seen_status);
                            let _ = send_feishu_text(
                                &config,
                                &client,
                                &token_cache,
                                &chat_id_delivery,
                                &running_notice_text,
                            )
                            .await;
                            timeout_logged = true;
                        }
                    }
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }
            };
            if !resp.status().is_success() {
                let status = resp.status();
                let body_preview = resp.text().await.unwrap_or_default();
                if body_preview.len() > 200 {
                    debug!(
                        "feishud: poll http error task_id={} status={} body_len={}",
                        task_id,
                        status,
                        body_preview.len()
                    );
                } else {
                    debug!(
                        "feishud: poll http error task_id={} status={} body={}",
                        task_id, status, body_preview
                    );
                }
                if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                    if !timeout_logged {
                        warn!("feishud: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=http status={} (continue_polling=true)", task_id, started.elapsed().as_secs(), delivery_timeout_secs, last_seen_status, status);
                        let _ = send_feishu_text(
                            &config,
                            &client,
                            &token_cache,
                            &chat_id_delivery,
                            &running_notice_text,
                        )
                        .await;
                        timeout_logged = true;
                    }
                }
                tokio::time::sleep(poll_interval).await;
                continue;
            }
            let body: ApiResponse<TaskQueryResponse> = match resp.json().await {
                Ok(b) => b,
                Err(e) => {
                    debug!("feishud: poll parse failed task_id={} err={}", task_id, e);
                    if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                        if !timeout_logged {
                            warn!("feishud: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=parse_failed (continue_polling=true)", task_id, started.elapsed().as_secs(), delivery_timeout_secs, last_seen_status);
                            let _ = send_feishu_text(
                                &config,
                                &client,
                                &token_cache,
                                &chat_id_delivery,
                                &running_notice_text,
                            )
                            .await;
                            timeout_logged = true;
                        }
                    }
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }
            };
            let Some(ref task) = body.data else {
                let err_msg = body.error.as_deref().unwrap_or("no data");
                debug!(
                    "feishud: poll no data task_id={} ok={} error={}",
                    task_id, body.ok, err_msg
                );
                if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                    if !timeout_logged {
                        warn!("feishud: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=no_task_data error={} (continue_polling=true)", task_id, started.elapsed().as_secs(), delivery_timeout_secs, last_seen_status, err_msg);
                        let _ = send_feishu_text(
                            &config,
                            &client,
                            &token_cache,
                            &chat_id_delivery,
                            &running_notice_text,
                        )
                        .await;
                        timeout_logged = true;
                    }
                }
                tokio::time::sleep(poll_interval).await;
                continue;
            };
            last_seen_status = Some(task.status.clone());
            let msg_len = task
                .result_json
                .as_ref()
                .and_then(|v| v.get("messages"))
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let text_len = task
                .result_json
                .as_ref()
                .and_then(|v| v.get("text"))
                .and_then(|v| v.as_str())
                .map(|s| s.len())
                .unwrap_or(0);
            debug!("feishud: poll task_id={} status={:?} result_json={} messages_len={} text_len={} elapsed_secs={}", task_id, task.status, task.result_json.is_some(), msg_len, text_len, started.elapsed().as_secs());
            match task.status {
                TaskStatus::Queued | TaskStatus::Running => {
                    if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                        if !timeout_logged {
                            warn!("feishud: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} (continue_polling=true)", task_id, started.elapsed().as_secs(), delivery_timeout_secs, last_seen_status);
                            let _ = send_feishu_text(
                                &config,
                                &client,
                                &token_cache,
                                &chat_id_delivery,
                                &running_notice_text,
                            )
                            .await;
                            timeout_logged = true;
                        }
                    }
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }
                TaskStatus::Succeeded => {
                    for to_send in feishu_task_success_messages(task, &config) {
                        for chunk in chunk_text_utf8(to_send.as_str(), chunk_chars) {
                            if let Err(e) = send_feishu_text(
                                &config,
                                &client,
                                &token_cache,
                                &chat_id_delivery,
                                &chunk,
                            )
                            .await
                            {
                                warn!(
                                    "feishud: send success text failed task_id={} err={}",
                                    task_id, e
                                );
                            }
                        }
                    }
                    info!(
                        "feishud: task delivery success task_id={} (result sent)",
                        task_id
                    );
                    break;
                }
                TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                    let detail = task.error_text.clone().unwrap_or_else(|| {
                        feishu_t(
                            &config,
                            FEISHU_I18N_TASK_FAILED_FALLBACK_ERROR_KEY,
                            FEISHU_TASK_FAILED_FALLBACK_ERROR_FALLBACK,
                        )
                    });
                    let msg = feishu_t_with(
                        &config,
                        FEISHU_I18N_PROCESS_FAILED_WITH_ERROR_KEY,
                        &[("error", &detail)],
                        FEISHU_PROCESS_FAILED_WITH_ERROR_FALLBACK,
                    );
                    let _ =
                        send_feishu_text(&config, &client, &token_cache, &chat_id_delivery, &msg)
                            .await;
                    info!(
                        "feishud: task delivery failure task_id={} status={:?}",
                        task_id, task.status
                    );
                    break;
                }
            }
        }
    });
}

/// 飞书签名校验：签名字符串 = timestamp + nonce + encrypt_key + body，SHA256 十六进制小写。
/// 仅当配置了 encrypt_key 时执行；未配置时跳过（日志注明）。
const FEISHU_TIMESTAMP_TOLERANCE_SECS: u64 = 300;

fn verify_feishu_signature(
    headers: &HeaderMap,
    body: &str,
    encrypt_key: &str,
) -> Result<(), &'static str> {
    if encrypt_key.is_empty() {
        return Ok(());
    }
    let timestamp = headers
        .get("x-lark-request-timestamp")
        .and_then(|v| v.to_str().ok())
        .ok_or("timestamp missing")?;
    let nonce = headers
        .get("x-lark-request-nonce")
        .and_then(|v| v.to_str().ok())
        .ok_or("nonce missing")?;
    let signature = headers
        .get("x-lark-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or("signature missing")?;
    let ts: u64 = timestamp.parse().map_err(|_| "timestamp invalid")?;
    let now = std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now > ts && now - ts > FEISHU_TIMESTAMP_TOLERANCE_SECS {
        return Err("timestamp expired");
    }
    if ts > now && ts - now > FEISHU_TIMESTAMP_TOLERANCE_SECS {
        return Err("timestamp invalid");
    }
    let sign_string = format!("{}{}{}{}", timestamp, nonce, encrypt_key, body);
    let mut hasher = Sha256::new();
    hasher.update(sign_string.as_bytes());
    let out = hasher.finalize();
    let expected: String = out.iter().map(|b| format!("{:02x}", b)).collect();
    if expected.eq_ignore_ascii_case(signature) {
        Ok(())
    } else {
        Err("signature invalid")
    }
}

/// verification_token 强校验：配置了则必须匹配，否则拒绝。
fn verify_verification_token(
    body: &Value,
    is_challenge: bool,
    expected: &str,
) -> Result<(), &'static str> {
    if expected.is_empty() {
        return Ok(());
    }
    let token = if is_challenge {
        body.get("token").and_then(|v| v.as_str()).unwrap_or("")
    } else {
        body.get("header")
            .and_then(|h| h.get("token"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
    };
    if token == expected {
        Ok(())
    } else {
        Err("token mismatch")
    }
}

async fn callback_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    info!("feishud: callback received body_len={}", body.len());

    if !state.config.feishu.encrypt_key.is_empty() {
        if let Err(reason) =
            verify_feishu_signature(&headers, &body, &state.config.feishu.encrypt_key)
        {
            warn!("feishud: signature verification failed reason={}", reason);
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "signature_invalid" })),
            )
                .into_response();
        }
        info!("feishud: signature verification success");
    } else {
        info!("feishud: signature check skipped (encrypt_key not set)");
    }

    let body_json: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            warn!("feishud: body parse failed err={}", e);
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid_json" })),
            )
                .into_response();
        }
    };

    if let Some(challenge) = body_json.get("challenge").and_then(|v| v.as_str()) {
        let typ = body_json.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if typ == "url_verification" {
            if let Err(reason) =
                verify_verification_token(&body_json, true, &state.config.feishu.verification_token)
            {
                warn!(
                    "feishud: challenge verification_token mismatch reason={}",
                    reason
                );
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "error": "token_mismatch" })),
                )
                    .into_response();
            }
            info!("feishud: challenge verification success returning challenge");
            return Json(json!({ "challenge": challenge })).into_response();
        }
    }

    if let Err(reason) =
        verify_verification_token(&body_json, false, &state.config.feishu.verification_token)
    {
        warn!(
            "feishud: event verification_token mismatch reason={}",
            reason
        );
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "token_mismatch" })),
        )
            .into_response();
    }
    info!("feishud: event token verification success");

    dispatch_im_incoming_event(state, body_json);
    Json(json!({})).into_response()
}

fn chunk_text_utf8(s: &str, max_chars: usize) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    if s.chars().count() <= max_chars {
        return vec![s.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for c in s.chars() {
        if current.chars().count() >= max_chars {
            out.push(std::mem::take(&mut current));
        }
        current.push(c);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// 下载消息内资源（图片 / 文件 / 音视频均走此接口，`type` 为 `image` 或 `file`）。
async fn download_feishu_message_resource(
    client: &Client,
    api_base: &str,
    token: &str,
    message_id: &str,
    resource_key: &str,
    query_type: &str,
) -> Result<Vec<u8>, String> {
    let base = api_base.trim_end_matches('/');
    let mid = urlencoding::encode(message_id);
    let key = urlencoding::encode(resource_key);
    let url = format!(
        "{}/open-apis/im/v1/messages/{}/resources/{}?type={}",
        base, mid, key, query_type
    );
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("download request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "download status={} url_tail=.../messages/.../resources/... body_len={}",
            status,
            text.len()
        ));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("download body read failed: {}", e))
}

async fn get_tenant_access_token(
    config: &FeishuSection,
    client: &Client,
    cache: &RwLock<Option<(String, u64)>>,
) -> Result<String, String> {
    let now_secs = std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .map(|d| d.as_secs())
        .unwrap_or(0);
    {
        let guard = cache.read().await;
        if let Some((ref token, exp)) = *guard {
            if exp > now_secs + 60 {
                return Ok(token.clone());
            }
        }
    }
    let url = format!(
        "{}/open-apis/auth/v3/tenant_access_token/internal",
        config.api_base_url.trim_end_matches('/')
    );
    let body = json!({
        "app_id": config.app_id,
        "app_secret": config.app_secret
    });
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("token request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("token request status={} body={}", status, text));
    }
    #[derive(Deserialize)]
    struct TokenResp {
        tenant_access_token: Option<String>,
        expire: Option<u64>,
    }
    let data: TokenResp = resp
        .json()
        .await
        .map_err(|e| format!("token parse failed: {}", e))?;
    let token = data
        .tenant_access_token
        .ok_or_else(|| "token response missing tenant_access_token".to_string())?;
    let expire = data.expire.unwrap_or(7200);
    let expires_at = now_secs + expire;
    {
        let mut guard = cache.write().await;
        *guard = Some((token.clone(), expires_at));
    }
    info!(
        "feishud: tenant_access_token refreshed expires_in={}",
        expire
    );
    Ok(token)
}

async fn send_feishu_text(
    config: &FeishuConfig,
    client: &Client,
    token_cache: &RwLock<Option<(String, u64)>>,
    receive_id: &str,
    text: &str,
) -> Result<(), String> {
    let token = get_tenant_access_token(&config.feishu, client, token_cache).await?;
    let url = format!(
        "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
        config.feishu.api_base_url.trim_end_matches('/')
    );
    let body = json!({
        "receive_id": receive_id,
        "msg_type": "text",
        "content": json!({ "text": text }).to_string()
    });
    let resp = client
        .post(url)
        .header("Authorization", format!("Bearer {}", token))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("send request failed: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "feishu send status={} body_len={}",
            status,
            body.len()
        ));
    }
    info!(
        "feishud: send success receive_id={} text_len={}",
        receive_id,
        text.len()
    );
    Ok(())
}

/// 长连接模式：使用 open-lark 连飞书收事件，重连带退避。
async fn run_long_connection_loop(state: AppState) -> anyhow::Result<()> {
    use open_lark::client::ws_client::LarkWsClient;
    use open_lark::core::config::Config as LarkConfig;
    use open_lark::core::constants::AppType;
    use open_lark::event::dispatcher::EventDispatcherHandler;

    let app_id = state.config.feishu.app_id.clone();
    let app_secret = state.config.feishu.app_secret.clone();
    if app_id.is_empty() || app_secret.is_empty() {
        anyhow::bail!("feishud long_connection mode requires app_id and app_secret");
    }

    let lark_config: std::sync::Arc<LarkConfig> = std::sync::Arc::new(
        LarkConfig::builder()
            .app_id(&app_id)
            .app_secret(&app_secret)
            .app_type(AppType::SelfBuild)
            .enable_token_cache(true)
            .build(),
    );

    let state_arc = Arc::new(state);
    let mut backoff_secs = 5u64;
    const MAX_BACKOFF_SECS: u64 = 300;

    loop {
        info!("feishud: long connection starting (app_id={})", app_id);
        let handler = EventDispatcherHandler::builder()
            .register_p2_im_message_receive_v1_raw({
                let state = state_arc.clone();
                move |payload: &[u8]| {
                    let body_len = payload.len();
                    let body: Value = match serde_json::from_slice(payload) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!(
                                "feishud: long_connection event parse failed with reason: {}",
                                e
                            );
                            return Ok(());
                        }
                    };
                    let event_type = body
                        .get("header")
                        .and_then(|h| h.get("event_type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    tracing::debug!(
                        "feishud: long_connection raw event received event_type={} body_len={}",
                        event_type,
                        body_len
                    );

                    let st = (*state).clone();
                    dispatch_im_incoming_event(st, body);
                    Ok(())
                }
            })
            .map_err(|e| anyhow::anyhow!("register_p2_im_message_receive_v1_raw: {}", e))?
            .build();

        match LarkWsClient::open(lark_config.clone(), handler).await {
            Ok(()) => {
                warn!("feishud: long connection closed normally, reconnecting");
            }
            Err(e) => {
                warn!(
                    "feishud: long connection error: {}, reconnecting in {}s",
                    e, backoff_secs
                );
            }
        }
        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,feishud=debug".to_string()),
        )
        .init();
    let _ = tracing_log::LogTracer::init();

    let config_path = std::env::var("FEISHU_CONFIG_PATH")
        .unwrap_or_else(|_| "configs/channels/feishu.toml".to_string());
    let mut config: FeishuConfig = {
        let raw = std::fs::read_to_string(&config_path)
            .map_err(|e| anyhow::anyhow!("read config {}: {}", config_path, e))?;
        toml::from_str(&raw).map_err(|e| anyhow::anyhow!("parse config: {}", e))?
    };
    apply_env_overrides(&mut config);
    config.feishu.i18n_path = resolve_i18n_path(&config.feishu.language, &config.feishu.i18n_path);

    if !config.feishu.enabled {
        tracing::info!("feishud: disabled in config, exiting");
        return Ok(());
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(config.feishu.request_timeout_seconds))
        .build()?;

    let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let state = AppState {
        config: config.clone(),
        client: client.clone(),
        token_cache: Arc::new(RwLock::new(None)),
        workspace_root,
        pending_key_bind_by_chat: Arc::new(Mutex::new(HashSet::new())),
    };

    match config.feishu.mode {
        FeishuMode::Webhook => {
            let token_ok = !config.feishu.verification_token.trim().is_empty();
            let encrypt_ok = !config.feishu.encrypt_key.trim().is_empty();
            if !token_ok && !encrypt_ok {
                anyhow::bail!(
                    "feishud webhook mode requires verification_token or encrypt_key (at least one must be set)"
                );
            }
            let app = Router::new()
                .route("/", post(callback_handler))
                .with_state(state);
            let listen = config
                .feishu
                .listen
                .parse::<std::net::SocketAddr>()
                .map_err(|e| anyhow::anyhow!("listen address {}: {}", config.feishu.listen, e))?;
            info!(
                "feishud: mode=webhook listening on {} (Feishu app bot callback)",
                listen
            );
            axum::serve(tokio::net::TcpListener::bind(listen).await?, app).await?;
        }
        FeishuMode::LongConnection => {
            let listen = config
                .feishu
                .listen
                .parse::<std::net::SocketAddr>()
                .map_err(|e| anyhow::anyhow!("listen address {}: {}", config.feishu.listen, e))?;
            let health_app = Router::new().route("/health", get(|| async { "ok" }));
            let listener = tokio::net::TcpListener::bind(listen).await?;
            tokio::spawn(async move {
                if let Err(err) = axum::serve(listener, health_app).await {
                    tracing::warn!("feishud: health server exited err={}", err);
                }
            });
            info!(
                "feishud: mode=long_connection health check on {} (GET /health)",
                listen
            );
            run_long_connection_loop(state).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
