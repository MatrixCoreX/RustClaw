mod binding;
mod config_cache;
mod config_section;
mod helpers;
mod ilink;
mod incoming;
mod login_routes;
mod reply_delivery;
mod task_flow;
mod wechat_api;
mod wechat_silk_wav;

use binding::*;
use helpers::*;
use incoming::*;
use login_routes::*;
use reply_delivery::*;
use task_flow::*;
use wechat_api::*;

use std::collections::{HashMap, HashSet};
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
use claw_core::channel_commands::ChannelCommandCatalog;
use claw_core::types::{
    ApiResponse, AuthIdentity, BindChannelKeyRequest, ChannelKind, ResolveChannelBindingRequest,
    ResolveChannelBindingResponse, SubmitTaskRequest, SubmitTaskResponse, TaskKind,
    TaskQueryResponse, TaskStatus,
};
use claw_core::wechat_reply_media::{
    extract_wechat_outbound_media, strip_wechat_delivery_lines, WechatOutboundKind,
    WechatOutboundMedia, WechatOutboundSource,
};
use config_cache::WeixinConfigManager;
use config_section::{AppConfig, WechatSection};
use qrcodegen::{QrCode, QrCodeEcc};
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};
use wechat_ilink::http::IlinkAuth;
use wechat_ilink::{
    download_decrypted_media, download_remote_media_to_temp, parse_aes_key_base64,
    parse_aes_key_hex_or_base64_media, send_weixin_file_from_file, send_weixin_image_from_file,
    send_weixin_video_from_file,
};

const SESSION_EXPIRED_ERRCODE: i64 = -14;
const MAX_CONSECUTIVE_FAILURES: usize = 3;
const RETRY_DELAY_MS: u64 = 2_000;
const BACKOFF_DELAY_MS: u64 = 30_000;
const ACTIVE_LOGIN_TTL_MS: u64 = 5 * 60_000;
const WECHAT_TEXT_CHUNK_CHARS: usize = 1200;
const WECHATD_CHANNEL_VERSION: &str = env!("CARGO_PKG_VERSION");
const WECHAT_MEDIA_OUTBOUND_TEMP_DIR: &str = "/tmp/rustclaw/wechatd/media/outbound-temp";

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

fn apply_wechat_env_overrides(config: &mut AppConfig) {
    apply_string_env(&mut config.wechat.bot_token, "WECHAT_BOT_TOKEN");
    apply_string_env(&mut config.wechat.wechat_uin_base64, "WECHAT_UIN_BASE64");
    apply_string_env(&mut config.wechat.sk_route_tag, "WECHAT_SK_ROUTE_TAG");
}

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
    pending_key_bind_by_user: Arc<RwLock<HashSet<String>>>,
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

    let config_path = std::env::var("WECHAT_CONFIG_PATH")
        .unwrap_or_else(|_| "configs/channels/wechat.toml".to_string());
    let raw = std::fs::read_to_string(&config_path)
        .with_context(|| format!("read wechat config failed: {config_path}"))?;
    let mut config: AppConfig = toml::from_str(&raw).context("parse wechat config failed")?;
    apply_wechat_env_overrides(&mut config);
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
        .timeout(Duration::from_secs(
            config.wechat.request_timeout_seconds.max(5),
        ))
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
        pending_key_bind_by_user: Arc::new(RwLock::new(HashSet::new())),
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
#[path = "main_tests.rs"]
mod tests;
