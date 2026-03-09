use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow};
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use claw_core::config::AppConfig;
use claw_core::types::{
    ApiResponse, AuthIdentity, BindChannelKeyRequest, ChannelKind, ResolveChannelBindingRequest,
    ResolveChannelBindingResponse, SubmitTaskRequest, SubmitTaskResponse, TaskKind, TaskQueryResponse,
    TaskStatus,
};
use hmac::{Hmac, Mac};
use reqwest::Client;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use tracing::{info, warn};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
struct AppState {
    clawd_base_url: String,
    client: Client,
    api_base: String,
    access_token: String,
    app_secret: String,
    verify_token: String,
    phone_number_id: String,
    poll_interval_ms: u64,
    task_wait_seconds: u64,
    quick_result_wait_seconds: u64,
    image_inbox_dir: String,
    audio_inbox_dir: String,
    inbound_dedup: Arc<Mutex<HashMap<String, u64>>>,
    pending_key_bind: Arc<Mutex<HashSet<String>>>,
    bound_identity_by_user: Arc<Mutex<HashMap<String, AuthIdentity>>>,
}

#[derive(Debug, Deserialize)]
struct VerifyQuery {
    #[serde(rename = "hub.mode")]
    mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    challenge: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WaWebhookPayload {
    #[serde(default)]
    entry: Vec<WaEntry>,
}

#[derive(Debug, Deserialize)]
struct WaEntry {
    #[serde(default)]
    changes: Vec<WaChange>,
}

#[derive(Debug, Deserialize)]
struct WaChange {
    value: WaValue,
}

#[derive(Debug, Deserialize)]
struct WaValue {
    #[serde(default)]
    messages: Vec<WaMessage>,
}

#[derive(Debug, Deserialize)]
struct WaMessage {
    #[serde(default)]
    from: String,
    #[serde(rename = "id", default)]
    id: String,
    #[serde(rename = "type", default)]
    message_type: String,
    #[serde(default)]
    text: Option<WaText>,
    #[serde(default)]
    image: Option<WaMedia>,
    #[serde(default)]
    audio: Option<WaMedia>,
    #[serde(default)]
    document: Option<WaMedia>,
}

#[derive(Debug, Deserialize)]
struct WaText {
    #[serde(default)]
    body: String,
}

#[derive(Debug, Deserialize)]
struct WaMedia {
    #[serde(default)]
    id: String,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WaMediaMeta {
    url: String,
    #[serde(rename = "mime_type", default)]
    _mime_type: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .with_target(false)
        .compact()
        .init();

    let config = AppConfig::load("configs/config.toml")?;
    if !config.whatsapp.enabled {
        warn!("whatsappd disabled by config [whatsapp].enabled=false");
    }

    let clawd_base_url = config
        .server
        .clawd_base_url
        .clone()
        .unwrap_or_else(|| {
            let listen = config.server.listen.as_str();
            let host = if listen.starts_with("0.0.0.0:") {
                listen.replacen("0.0.0.0", "127.0.0.1", 1)
            } else {
                listen.to_string()
            };
            format!("http://{}", host)
        });
    let state = AppState {
        clawd_base_url,
        client: Client::builder()
            .timeout(Duration::from_secs(config.server.request_timeout_seconds.max(5)))
            .build()
            .context("build reqwest client failed")?,
        api_base: config.whatsapp.api_base.trim_end_matches('/').to_string(),
        access_token: config.whatsapp.access_token.clone(),
        app_secret: config.whatsapp.app_secret.clone(),
        verify_token: config.whatsapp.verify_token.clone(),
        phone_number_id: config.whatsapp.phone_number_id.clone(),
        poll_interval_ms: config.worker.poll_interval_ms.max(100),
        task_wait_seconds: config.worker.task_timeout_seconds.max(1),
        quick_result_wait_seconds: config.whatsapp.quick_result_wait_seconds.max(1),
        image_inbox_dir: config.whatsapp.image_inbox_dir.clone(),
        audio_inbox_dir: config.whatsapp.audio_inbox_dir.clone(),
        inbound_dedup: Arc::new(Mutex::new(HashMap::new())),
        pending_key_bind: Arc::new(Mutex::new(HashSet::new())),
        bound_identity_by_user: Arc::new(Mutex::new(HashMap::new())),
    };

    let webhook_path = normalize_webhook_path(&config.whatsapp.webhook_path);
    let app = Router::new()
        .route(&webhook_path, get(verify_webhook).post(handle_webhook))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(&config.whatsapp.webhook_listen).await?;
    info!(
        "whatsappd started: listen={} webhook_path={}",
        config.whatsapp.webhook_listen, webhook_path
    );
    axum::serve(listener, app).await?;
    Ok(())
}

fn normalize_webhook_path(path: &str) -> String {
    let p = path.trim();
    if p.is_empty() {
        "/webhook".to_string()
    } else if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{p}")
    }
}

async fn verify_webhook(
    State(state): State<AppState>,
    Query(query): Query<VerifyQuery>,
) -> impl IntoResponse {
    let mode_ok = query.mode.as_deref() == Some("subscribe");
    let token_ok = query.verify_token.as_deref() == Some(state.verify_token.as_str());
    if mode_ok && token_ok {
        let challenge = query.challenge.unwrap_or_default();
        return (StatusCode::OK, challenge);
    }
    (StatusCode::FORBIDDEN, "forbidden".to_string())
}

async fn handle_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Err(err) = verify_signature(&state.app_secret, &headers, &body) {
        warn!("webhook signature verify failed: {}", err);
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    let payload: WaWebhookPayload = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(err) => {
            warn!("parse webhook payload failed: {}", err);
            return (StatusCode::BAD_REQUEST, "bad request").into_response();
        }
    };

    for entry in payload.entry {
        for change in entry.changes {
            for msg in change.value.messages {
                if let Err(err) = handle_inbound_message(&state, msg).await {
                    warn!("handle inbound message failed: {}", err);
                }
            }
        }
    }
    (StatusCode::OK, "ok").into_response()
}

fn verify_signature(app_secret: &str, headers: &HeaderMap, body: &[u8]) -> anyhow::Result<()> {
    if app_secret.trim().is_empty() {
        return Err(anyhow!("app_secret is empty"));
    }
    let header = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| anyhow!("x-hub-signature-256 missing"))?;
    let provided = header
        .strip_prefix("sha256=")
        .ok_or_else(|| anyhow!("x-hub-signature-256 prefix invalid"))?;
    let mut mac =
        HmacSha256::new_from_slice(app_secret.as_bytes()).map_err(|_| anyhow!("invalid app_secret"))?;
    mac.update(body);
    let digest = mac.finalize().into_bytes();
    let expected = hex::encode(digest);
    if expected.eq_ignore_ascii_case(provided) {
        Ok(())
    } else {
        Err(anyhow!("signature mismatch"))
    }
}

fn should_expect_key_reply(state: &AppState, wa_id: &str) -> bool {
    state
        .pending_key_bind
        .lock()
        .ok()
        .is_some_and(|set| set.contains(wa_id))
}

fn set_expect_key_reply(state: &AppState, wa_id: &str, enabled: bool) {
    if let Ok(mut set) = state.pending_key_bind.lock() {
        if enabled {
            set.insert(wa_id.to_string());
        } else {
            set.remove(wa_id);
        }
    }
}

fn store_bound_identity(state: &AppState, wa_id: &str, identity: &AuthIdentity) {
    if let Ok(mut map) = state.bound_identity_by_user.lock() {
        map.insert(wa_id.to_string(), identity.clone());
    }
}

fn bound_user_key_for_wa(state: &AppState, wa_id: &str) -> Option<String> {
    state
        .bound_identity_by_user
        .lock()
        .ok()
        .and_then(|map| map.get(wa_id).map(|identity| identity.user_key.clone()))
}

async fn resolve_whatsapp_identity(state: &AppState, wa_id: &str) -> anyhow::Result<Option<AuthIdentity>> {
    let url = format!("{}/v1/auth/channel/resolve", state.clawd_base_url);
    let req = ResolveChannelBindingRequest {
        channel: ChannelKind::Whatsapp,
        external_user_id: Some(wa_id.to_string()),
        external_chat_id: Some(wa_id.to_string()),
    };
    let resp = state.client.post(&url).json(&req).send().await?;
    let status = resp.status();
    let body: ApiResponse<ResolveChannelBindingResponse> = resp.json().await?;
    if !status.is_success() || !body.ok {
        return Err(anyhow!(
            "resolve whatsapp identity failed: {}",
            body.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    Ok(body.data.and_then(|v| v.identity))
}

async fn bind_whatsapp_identity(
    state: &AppState,
    wa_id: &str,
    user_key: &str,
) -> anyhow::Result<Option<AuthIdentity>> {
    let url = format!("{}/v1/auth/channel/bind", state.clawd_base_url);
    let req = BindChannelKeyRequest {
        channel: ChannelKind::Whatsapp,
        external_user_id: Some(wa_id.to_string()),
        external_chat_id: Some(wa_id.to_string()),
        user_key: user_key.trim().to_string(),
    };
    let resp = state.client.post(&url).json(&req).send().await?;
    let status = resp.status();
    let body: ApiResponse<AuthIdentity> = resp.json().await?;
    if !status.is_success() {
        if status.as_u16() == 401 {
            return Ok(None);
        }
        return Err(anyhow!(
            "bind whatsapp identity failed: {}",
            body.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    if !body.ok {
        return Ok(None);
    }
    Ok(body.data)
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn dedup_message_key(msg: &WaMessage) -> String {
    if !msg.id.trim().is_empty() {
        return format!("wa_msg:{}", msg.id.trim());
    }
    let text = msg
        .text
        .as_ref()
        .map(|t| t.body.trim())
        .unwrap_or("");
    format!("wa_fallback:{}:{}:{}", msg.from.trim(), msg.message_type.trim(), text)
}

fn should_process_inbound(state: &AppState, msg: &WaMessage) -> bool {
    const DEDUP_WINDOW_SECONDS: u64 = 10 * 60;
    let key = dedup_message_key(msg);
    if key.trim().is_empty() {
        return true;
    }
    let now = now_ts();
    let mut guard = match state.inbound_dedup.lock() {
        Ok(g) => g,
        Err(_) => return true,
    };
    guard.retain(|_, ts| now.saturating_sub(*ts) <= DEDUP_WINDOW_SECONDS);
    if let Some(last_ts) = guard.get(&key) {
        if now.saturating_sub(*last_ts) <= DEDUP_WINDOW_SECONDS {
            return false;
        }
    }
    guard.insert(key, now);
    true
}

async fn handle_inbound_message(state: &AppState, msg: WaMessage) -> anyhow::Result<()> {
    if !should_process_inbound(state, &msg) {
        info!(
            "skip duplicated inbound message: wa_id={} msg_id={} type={}",
            msg.from,
            msg.id,
            msg.message_type
        );
        return Ok(());
    }
    if msg.from.trim().is_empty() {
        return Ok(());
    }
    let identity = match resolve_whatsapp_identity(state, &msg.from).await? {
        Some(identity) => {
            set_expect_key_reply(state, &msg.from, false);
            identity
        }
        None => {
            let text = msg
                .text
                .as_ref()
                .map(|v| v.body.trim().to_string())
                .unwrap_or_default();
            let maybe_candidate = text
                .strip_prefix("/key")
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    if should_expect_key_reply(state, &msg.from) && !text.is_empty() {
                        Some(text.clone())
                    } else {
                        None
                    }
                });
            if let Some(candidate) = maybe_candidate {
                if let Some(identity) = bind_whatsapp_identity(state, &msg.from, &candidate).await? {
                    set_expect_key_reply(state, &msg.from, false);
                    store_bound_identity(state, &msg.from, &identity);
                    let _ = send_whatsapp_text(
                        state,
                        &msg.from,
                        "Key 绑定成功，请重新发送刚才的消息。\nKey bound successfully. Please send your previous message again.",
                    )
                    .await;
                    identity
                } else {
                    set_expect_key_reply(state, &msg.from, true);
                    let _ = send_whatsapp_text(
                        state,
                        &msg.from,
                        "Key 无效，请重新输入。\nInvalid key. Please try again.",
                    )
                    .await;
                    return Ok(());
                }
            } else {
                set_expect_key_reply(state, &msg.from, true);
                let _ = send_whatsapp_text(
                    state,
                    &msg.from,
                    "请先发送你的 key 进行绑定。\nPlease send your key first to bind this account.",
                )
                .await;
                return Ok(());
            }
        }
    };
    store_bound_identity(state, &msg.from, &identity);
    let user_id = identity.user_id;
    let chat_id = user_id;

    match msg.message_type.as_str() {
        "text" => {
            let text = msg.text.map(|v| v.body).unwrap_or_default();
            if text.trim().is_empty() {
                return Ok(());
            }
            if text.trim_start().starts_with("/run") {
                handle_run_command(state, &msg.from, user_id, chat_id, &text).await?;
            } else {
                let payload = json!({ "text": text.trim(), "agent_mode": true });
                let task_id = submit_task_only(
                    state,
                    user_id,
                    chat_id,
                    &msg.from,
                    TaskKind::Ask,
                    payload,
                )
                .await?;
                let delivered = try_deliver_quick_result(state, &msg.from, &task_id, None).await?;
                if !delivered {
                    spawn_task_result_delivery(state.clone(), msg.from.clone(), task_id, None);
                }
            }
        }
        "image" => {
            if let Some(media) = msg.image {
                handle_image_message(state, &msg.from, user_id, chat_id, &media).await?;
            }
        }
        "audio" => {
            if let Some(media) = msg.audio {
                handle_audio_message(state, &msg.from, user_id, chat_id, &media).await?;
            }
        }
        "document" => {
            if let Some(media) = msg.document {
                if media
                    .mime_type
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .starts_with("image/")
                {
                    handle_image_message(state, &msg.from, user_id, chat_id, &media).await?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_run_command(
    state: &AppState,
    wa_id: &str,
    user_id: i64,
    chat_id: i64,
    text: &str,
) -> anyhow::Result<()> {
    let rest = text.trim().strip_prefix("/run").unwrap_or_default().trim();
    if rest.is_empty() {
        send_whatsapp_text(state, wa_id, "Usage: /run <skill_name> <args>").await?;
        return Ok(());
    }
    let mut parts = rest.splitn(2, ' ');
    let skill_name = parts.next().unwrap_or_default().trim();
    let args = parts.next().unwrap_or_default().trim();
    if skill_name.is_empty() {
        send_whatsapp_text(state, wa_id, "Usage: /run <skill_name> <args>").await?;
        return Ok(());
    }
    let payload = json!({
        "skill_name": skill_name,
        "args": args
    });
    let task_id = submit_task_only(state, user_id, chat_id, wa_id, TaskKind::RunSkill, payload).await?;
    let delivered = try_deliver_quick_result(state, wa_id, &task_id, None).await?;
    if !delivered {
        spawn_task_result_delivery(state.clone(), wa_id.to_string(), task_id, None);
    }
    Ok(())
}

async fn handle_image_message(
    state: &AppState,
    wa_id: &str,
    user_id: i64,
    chat_id: i64,
    media: &WaMedia,
) -> anyhow::Result<()> {
    if media.id.trim().is_empty() {
        return Ok(());
    }
    let ext = media
        .mime_type
        .as_deref()
        .and_then(ext_from_mime)
        .unwrap_or("jpg");
    let rel_path = build_inbox_rel_path(&state.image_inbox_dir, wa_id, user_id, ext);
    let abs_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&rel_path);
    download_whatsapp_media(state, &media.id, &abs_path).await?;
    let payload = json!({
        "skill_name": "image_vision",
        "args": {
            "action": "describe",
            "images": [{"path": rel_path}],
            "detail_level": "normal"
        }
    });
    let task_id = submit_task_only(state, user_id, chat_id, wa_id, TaskKind::RunSkill, payload).await?;
    let delivered = try_deliver_quick_result(state, wa_id, &task_id, None).await?;
    if !delivered {
        spawn_task_result_delivery(state.clone(), wa_id.to_string(), task_id, None);
    }
    Ok(())
}

async fn handle_audio_message(
    state: &AppState,
    wa_id: &str,
    user_id: i64,
    chat_id: i64,
    media: &WaMedia,
) -> anyhow::Result<()> {
    if media.id.trim().is_empty() {
        return Ok(());
    }
    let ext = media
        .mime_type
        .as_deref()
        .and_then(ext_from_mime)
        .unwrap_or("ogg");
    let rel_path = build_inbox_rel_path(&state.audio_inbox_dir, wa_id, user_id, ext);
    let abs_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&rel_path);
    download_whatsapp_media(state, &media.id, &abs_path).await?;
    let transcribe_payload = json!({
        "skill_name": "audio_transcribe",
        "args": {
            "audio": {"path": rel_path}
        }
    });
    let task_id = submit_task_only(state, user_id, chat_id, wa_id, TaskKind::RunSkill, transcribe_payload).await?;
    let delivered = try_deliver_quick_result(state, wa_id, &task_id, Some(120)).await?;
    if !delivered {
        spawn_task_result_delivery(state.clone(), wa_id.to_string(), task_id, Some(120));
    }
    Ok(())
}

fn ext_from_mime(mime: &str) -> Option<&'static str> {
    let v = mime.to_ascii_lowercase();
    if v.contains("jpeg") {
        Some("jpg")
    } else if v.contains("png") {
        Some("png")
    } else if v.contains("webp") {
        Some("webp")
    } else if v.contains("ogg") {
        Some("ogg")
    } else if v.contains("mpeg") || v.contains("mp3") {
        Some("mp3")
    } else if v.contains("wav") {
        Some("wav")
    } else {
        None
    }
}

fn build_inbox_rel_path(base_dir: &str, wa_id: &str, user_id: i64, ext: &str) -> String {
    let clean_id = wa_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}/wa_{}_{}_{}.{}", base_dir, clean_id, user_id, ts, ext)
}

async fn download_whatsapp_media(state: &AppState, media_id: &str, local_path: &Path) -> anyhow::Result<()> {
    let meta_url = format!("{}/v23.0/{}", state.api_base, media_id);
    let meta = state
        .client
        .get(&meta_url)
        .bearer_auth(state.access_token.trim())
        .send()
        .await
        .context("request media meta failed")?;
    if !meta.status().is_success() {
        let status = meta.status();
        let body = meta.text().await.unwrap_or_default();
        return Err(anyhow!("media meta http {}: {}", status, body));
    }
    let meta_body: WaMediaMeta = meta.json().await.context("decode media meta failed")?;
    let bytes = state
        .client
        .get(&meta_body.url)
        .bearer_auth(state.access_token.trim())
        .send()
        .await
        .context("download media failed")?
        .bytes()
        .await
        .context("read media bytes failed")?;
    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(local_path, &bytes)?;
    Ok(())
}

async fn submit_task_only(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    wa_id: &str,
    kind: TaskKind,
    payload: Value,
) -> anyhow::Result<String> {
    let user_key = state
        .bound_identity_by_user
        .lock()
        .ok()
        .and_then(|map| map.get(wa_id).map(|identity| identity.user_key.clone()));
    let mut payload = payload;
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "adapter".to_string(),
            Value::String("whatsapp_cloud".to_string()),
        );
    }
    let req = SubmitTaskRequest {
        user_id,
        chat_id,
        user_key,
        channel: Some(ChannelKind::Whatsapp),
        external_user_id: Some(wa_id.to_string()),
        external_chat_id: Some(wa_id.to_string()),
        kind,
        payload,
    };
    let url = format!("{}/v1/tasks", state.clawd_base_url);
    let resp = state
        .client
        .post(&url)
        .json(&req)
        .send()
        .await
        .context("submit task request failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("submit task http {}: {}", status, body));
    }
    let body: ApiResponse<SubmitTaskResponse> = resp.json().await.context("decode submit task response failed")?;
    if !body.ok {
        return Err(anyhow!(
            "submit task rejected: {}",
            body.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    let task_id = body
        .data
        .ok_or_else(|| anyhow!("submit task missing task_id"))?
        .task_id;
    Ok(task_id.to_string())
}

async fn query_task_status(
    state: &AppState,
    task_id: &str,
    user_key: Option<&str>,
) -> anyhow::Result<TaskQueryResponse> {
    let url = format!("{}/v1/tasks/{task_id}", state.clawd_base_url);
    let mut req = state.client.get(&url);
    if let Some(user_key) = user_key.map(str::trim).filter(|v| !v.is_empty()) {
        req = req.header("X-RustClaw-Key", user_key);
    }
    let resp = req.send().await.context("query task status failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("query task status http {}: {}", status, body));
    }
    let body: ApiResponse<TaskQueryResponse> = resp.json().await.context("decode query task response failed")?;
    if !body.ok {
        return Err(anyhow!(
            "query task failed: {}",
            body.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }
    body.data.ok_or_else(|| anyhow!("query task missing data"))
}

async fn poll_task_result(
    state: &AppState,
    task_id: &str,
    user_key: Option<&str>,
    wait_override_seconds: Option<u64>,
) -> anyhow::Result<String> {
    let poll_interval_ms = state.poll_interval_ms.max(1);
    let wait_seconds = wait_override_seconds.unwrap_or(state.task_wait_seconds).max(1);
    let max_rounds = ((wait_seconds * 1000) / poll_interval_ms).max(1);
    for _ in 0..max_rounds {
        let task = query_task_status(state, task_id, user_key).await?;
        match task.status {
            TaskStatus::Queued | TaskStatus::Running => {
                tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
            }
            TaskStatus::Succeeded => {
                let answer = task
                    .result_json
                    .as_ref()
                    .and_then(|v| v.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("done")
                    .to_string();
                return Ok(answer);
            }
            TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                let err = task.error_text.unwrap_or_else(|| "task failed".to_string());
                return Err(anyhow!("{}", err));
            }
        }
    }
    Err(anyhow!("task_result_wait_timeout"))
}

async fn try_deliver_quick_result(
    state: &AppState,
    wa_id: &str,
    task_id: &str,
    wait_override_seconds: Option<u64>,
) -> anyhow::Result<bool> {
    let wait = wait_override_seconds.or(Some(state.quick_result_wait_seconds));
    match poll_task_result(state, task_id, bound_user_key_for_wa(state, wa_id).as_deref(), wait).await {
        Ok(answer) => {
            send_answer(state, wa_id, &answer).await?;
            Ok(true)
        }
        Err(err) if err.to_string() == "task_result_wait_timeout" => Ok(false),
        Err(err) => {
            send_whatsapp_text(state, wa_id, &format!("处理失败：{}", err)).await?;
            Ok(true)
        }
    }
}

fn spawn_task_result_delivery(
    state: AppState,
    wa_id: String,
    task_id: String,
    wait_override_seconds: Option<u64>,
) {
    tokio::spawn(async move {
        let out = poll_task_result(
            &state,
            &task_id,
            bound_user_key_for_wa(&state, &wa_id).as_deref(),
            wait_override_seconds,
        )
        .await;
        match out {
            Ok(answer) => {
                let _ = send_answer(&state, &wa_id, &answer).await;
            }
            Err(err) => {
                let _ = send_whatsapp_text(&state, &wa_id, &format!("处理失败：{}", err)).await;
            }
        }
    });
}

async fn send_answer(state: &AppState, wa_id: &str, answer: &str) -> anyhow::Result<()> {
    const IMAGE_PREFIX: &str = "IMAGE_FILE:";
    const FILE_PREFIX: &str = "FILE:";
    const VOICE_PREFIX: &str = "VOICE_FILE:";

    let image_paths = extract_prefixed_paths(answer, IMAGE_PREFIX);
    let file_paths = extract_prefixed_paths(answer, FILE_PREFIX);
    let voice_paths = extract_prefixed_paths(answer, VOICE_PREFIX);
    let text_without_tokens = strip_prefixed_tokens(answer, &[IMAGE_PREFIX, FILE_PREFIX, VOICE_PREFIX])
        .trim()
        .to_string();

    if !text_without_tokens.is_empty() {
        send_whatsapp_text(state, wa_id, &text_without_tokens).await?;
    }

    for p in &image_paths {
        let media_id = upload_media(state, &p, "image/jpeg").await?;
        send_whatsapp_media_by_id(state, wa_id, "image", &media_id, None).await?;
    }
    for p in &file_paths {
        let media_id = upload_media(state, &p, "application/octet-stream").await?;
        let filename = Path::new(&p)
            .file_name()
            .and_then(|v| v.to_str())
            .map(|v| v.to_string());
        send_whatsapp_media_by_id(state, wa_id, "document", &media_id, filename.as_deref()).await?;
    }
    for p in &voice_paths {
        let media_id = upload_media(state, &p, "audio/ogg").await?;
        send_whatsapp_media_by_id(state, wa_id, "audio", &media_id, None).await?;
    }

    if text_without_tokens.is_empty() && image_paths.is_empty() && file_paths.is_empty() && voice_paths.is_empty() {
        send_whatsapp_text(state, wa_id, answer).await?;
    }
    Ok(())
}

fn extract_prefixed_paths(answer: &str, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in answer.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let cleaned = normalize_path_token(rest.trim());
            if !cleaned.is_empty() && Path::new(cleaned).exists() && Path::new(cleaned).is_file() {
                out.push(cleaned.to_string());
            }
        }
    }
    out
}

fn strip_prefixed_tokens(answer: &str, prefixes: &[&str]) -> String {
    answer
        .lines()
        .filter(|line| !prefixes.iter().any(|prefix| line.trim_start().starts_with(prefix)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_path_token(token: &str) -> &str {
    token.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '，' | ',' | ':' | '：' | ';'))
}

async fn upload_media(state: &AppState, path: &str, mime: &str) -> anyhow::Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read media file failed: {path}"))?;
    let filename = Path::new(path)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("file.bin")
        .to_string();
    let part = Part::bytes(bytes)
        .file_name(filename)
        .mime_str(mime)
        .context("invalid media mime")?;
    let form = Form::new()
        .text("messaging_product", "whatsapp")
        .part("file", part);
    let url = format!(
        "{}/v23.0/{}/media",
        state.api_base,
        state.phone_number_id.trim()
    );
    let resp = state
        .client
        .post(&url)
        .bearer_auth(state.access_token.trim())
        .multipart(form)
        .send()
        .await
        .context("upload media failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("upload media http {}: {}", status, body));
    }
    let body: Value = resp.json().await.context("decode upload media response failed")?;
    let media_id = body
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("upload media missing id"))?;
    Ok(media_id.to_string())
}

async fn send_whatsapp_media_by_id(
    state: &AppState,
    wa_id: &str,
    media_type: &str,
    media_id: &str,
    filename: Option<&str>,
) -> anyhow::Result<()> {
    let mut body = json!({
        "messaging_product": "whatsapp",
        "to": wa_id,
        "type": media_type,
    });
    match media_type {
        "image" => body["image"] = json!({ "id": media_id }),
        "audio" => body["audio"] = json!({ "id": media_id }),
        _ => {
            let mut doc = json!({ "id": media_id });
            if let Some(name) = filename {
                doc["filename"] = Value::String(name.to_string());
            }
            body["document"] = doc;
        }
    }

    let url = format!(
        "{}/v23.0/{}/messages",
        state.api_base,
        state.phone_number_id.trim()
    );
    let resp = state
        .client
        .post(&url)
        .bearer_auth(state.access_token.trim())
        .json(&body)
        .send()
        .await
        .context("send media message failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("send media http {}: {}", status, text));
    }
    Ok(())
}

async fn send_whatsapp_text(state: &AppState, wa_id: &str, text: &str) -> anyhow::Result<()> {
    let url = format!(
        "{}/v23.0/{}/messages",
        state.api_base,
        state.phone_number_id.trim()
    );
    let resp = state
        .client
        .post(&url)
        .bearer_auth(state.access_token.trim())
        .json(&json!({
            "messaging_product": "whatsapp",
            "to": wa_id,
            "type": "text",
            "text": { "body": text }
        }))
        .send()
        .await
        .context("send text message failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("send text message http {}: {}", status, body));
    }
    Ok(())
}
