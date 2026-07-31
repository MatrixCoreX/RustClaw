use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context};
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use claw_core::channel_chunk::{chunk_text_for_channel, SEGMENT_PREFIX_MAX_CHARS};
use claw_core::channel_commands::{ChannelCommandCatalog, CoreCommandAction};
use claw_core::channel_i18n::{text_from_path, text_with_vars_from_path};
use claw_core::config::AppConfig;
use claw_core::types::{
    ApiResponse, AuthIdentity, BindChannelKeyRequest, BindChannelKeyResponse, ChannelKind,
    PendingChannelRequestStatus, PendingChannelRequestStoreRequest, ResolveChannelBindingRequest,
    ResolveChannelBindingResponse, SubmitTaskRequest, SubmitTaskResponse, TaskKind,
    TaskQueryResponse, TaskStatus,
};
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use tracing::{info, warn};

type HmacSha256 = Hmac<Sha256>;
const WA_I18N_BIND_REQUIRED_KEY: &str = "whatsapp_cloud.msg.bind_key_required_for_chat";
const WA_I18N_BIND_SUCCESS_KEY: &str = "whatsapp_cloud.msg.bind_success";
const WA_I18N_PENDING_RESUME_STOPPED_KEY: &str = "whatsapp_cloud.msg.pending_resume_stopped";
const WA_I18N_BIND_INVALID_KEY: &str = "whatsapp_cloud.msg.bind_invalid";
const WA_I18N_BIND_HELP_KEY: &str = "whatsapp_cloud.msg.bind_help";
const WA_I18N_RUN_USAGE_KEY: &str = "whatsapp_cloud.msg.run_usage";
const WHATSAPP_TEXT_CHUNK_CHARS: usize = 3500;
const WA_I18N_REQUEST_TIMEOUT_RETRY_LATER_KEY: &str =
    "whatsapp_cloud.msg.request_timeout_retry_later";

const WA_BIND_REQUIRED_FALLBACK: &str = "message_key=whatsapp_cloud.msg.bind_key_required_for_chat";
const WA_BIND_SUCCESS_FALLBACK: &str = "message_key=whatsapp_cloud.msg.bind_success";
const WA_PENDING_RESUME_STOPPED_FALLBACK: &str =
    "message_key=whatsapp_cloud.msg.pending_resume_stopped";
const WA_BIND_INVALID_FALLBACK: &str = "message_key=whatsapp_cloud.msg.bind_invalid";
const WA_BIND_HELP_FALLBACK: &str = "message_key=whatsapp_cloud.msg.bind_help";
const WA_RUN_USAGE_FALLBACK: &str = "message_key=whatsapp_cloud.msg.run_usage";
const WA_REQUEST_TIMEOUT_RETRY_LATER_FALLBACK: &str =
    "message_key=whatsapp_cloud.msg.request_timeout_retry_later task_id={task_id}";

fn whatsapp_provider_http_error(
    operation: &str,
    status: reqwest::StatusCode,
    response_body: &str,
) -> anyhow::Error {
    anyhow!(
        claw_core::channel_whatsapp_cloud::provider_error_from_response(
            operation,
            status.as_u16(),
            response_body,
        )
    )
}

fn whatsapp_provider_invalid_response(operation: &str, diagnostic_material: &str) -> anyhow::Error {
    anyhow!(
        claw_core::channel_provider_error::ChannelProviderError::invalid_response(
            "whatsapp_cloud",
            operation,
            diagnostic_material,
        )
    )
}

#[derive(Clone)]
struct AppState {
    clawd_base_url: String,
    i18n_path: String,
    language: String,
    command_catalog: Arc<ChannelCommandCatalog>,
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
    last_inbound_at_by_user: Arc<Mutex<HashMap<String, u64>>>,
    pending_key_bind: Arc<Mutex<HashSet<String>>>,
    bound_identity_by_user: Arc<Mutex<HashMap<String, AuthIdentity>>>,
}

fn wa_t(state: &AppState, key: &str, fallback: &str) -> String {
    text_from_path(&state.i18n_path, key, fallback)
}

fn wa_t_with(state: &AppState, key: &str, vars: &[(&str, &str)], fallback: &str) -> String {
    text_with_vars_from_path(&state.i18n_path, key, vars, fallback)
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
        .unwrap_or_else(|| claw_core::config::CLAWD_INTERNAL_BASE_URL.to_string());
    let i18n_path = resolve_i18n_path(&config.whatsapp.language, &config.whatsapp.i18n_path);
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let state = AppState {
        clawd_base_url,
        i18n_path,
        language: config.whatsapp.language.clone(),
        command_catalog: Arc::new(ChannelCommandCatalog::load_or_default(
            &workspace_root.join("configs/channel_commands.toml"),
        )),
        client: Client::builder()
            .timeout(Duration::from_secs(
                config.server.request_timeout_seconds.max(5),
            ))
            .build()
            .context("build reqwest client failed")?,
        api_base: config.whatsapp.api_base.trim_end_matches('/').to_string(),
        access_token: config.whatsapp.access_token.clone(),
        app_secret: config.whatsapp.app_secret.clone(),
        verify_token: config.whatsapp.verify_token.clone(),
        phone_number_id: config.whatsapp.phone_number_id.clone(),
        poll_interval_ms: config.worker.poll_interval_ms.max(100),
        task_wait_seconds: config.whatsapp.task_delivery_timeout_seconds.max(1),
        quick_result_wait_seconds: config.whatsapp.quick_result_wait_seconds.max(1),
        image_inbox_dir: config.whatsapp.image_inbox_dir.clone(),
        audio_inbox_dir: config.whatsapp.audio_inbox_dir.clone(),
        inbound_dedup: Arc::new(Mutex::new(HashMap::new())),
        last_inbound_at_by_user: Arc::new(Mutex::new(HashMap::new())),
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

fn resolve_i18n_path(language: &str, configured_path: &str) -> String {
    let lang = language.trim();
    if !lang.is_empty() {
        let candidate = format!("configs/i18n/whatsapp-cloud.{lang}.toml");
        if Path::new(&candidate).exists() {
            return candidate;
        }
    }
    configured_path.to_string()
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

    let status_payload =
        serde_json::from_slice::<claw_core::channel_whatsapp_cloud::WhatsappWebhookPayload>(&body)
            .ok();
    if status_payload
        .as_ref()
        .is_some_and(|payload| payload.statuses().next().is_some())
    {
        if let Err(err) = forward_delivery_statuses(&state, &headers, &body).await {
            warn!("forward whatsapp delivery statuses failed: {}", err);
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "whatsapp_cloud_status_forward_failed",
            )
                .into_response();
        }
    }

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

async fn forward_delivery_statuses(
    state: &AppState,
    headers: &HeaderMap,
    body: &[u8],
) -> anyhow::Result<()> {
    let signature = headers
        .get("x-hub-signature-256")
        .ok_or_else(|| anyhow!("x-hub-signature-256 missing"))?;
    let response = state
        .client
        .post(format!(
            "{}/v1/internal/channel-events/whatsapp-cloud",
            state.clawd_base_url
        ))
        .header("x-hub-signature-256", signature)
        .body(body.to_vec())
        .send()
        .await
        .context("forward whatsapp status request failed")?;
    if !response.status().is_success() {
        return Err(anyhow!("forward whatsapp status rejected"));
    }
    Ok(())
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
    let mut mac = HmacSha256::new_from_slice(app_secret.as_bytes())
        .map_err(|_| anyhow!("invalid app_secret"))?;
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

fn is_unbound_allowed_command(
    command_catalog: &ChannelCommandCatalog,
    channel: &str,
    text: &str,
) -> bool {
    command_catalog.allows_unbound_command(text, channel)
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

async fn send_bind_required_prompt(state: &AppState, wa_id: &str) -> anyhow::Result<()> {
    set_expect_key_reply(state, wa_id, true);
    let text = wa_t(state, WA_I18N_BIND_REQUIRED_KEY, WA_BIND_REQUIRED_FALLBACK);
    send_whatsapp_text(state, wa_id, &text).await?;
    Ok(())
}

async fn resolve_whatsapp_identity(
    state: &AppState,
    wa_id: &str,
) -> anyhow::Result<Option<AuthIdentity>> {
    let url = format!("{}/v1/auth/channel/resolve", state.clawd_base_url);
    let req = ResolveChannelBindingRequest {
        channel: ChannelKind::Whatsapp,
        telegram_bot_name: None,
        external_user_id: Some(wa_id.to_string()),
        external_chat_id: Some(wa_id.to_string()),
    };
    let resp = state.client.post(&url).json(&req).send().await?;
    let status = resp.status();
    let body: ApiResponse<ResolveChannelBindingResponse> = resp.json().await?;
    if !status.is_success() || !body.ok {
        return Err(whatsapp_provider_invalid_response(
            "resolve_identity",
            "application_rejected",
        ));
    }
    Ok(body.data.and_then(|v| v.identity))
}

async fn bind_whatsapp_identity(
    state: &AppState,
    wa_id: &str,
    user_key: &str,
) -> anyhow::Result<Option<BindChannelKeyResponse>> {
    let url = format!("{}/v1/auth/channel/bind", state.clawd_base_url);
    let req = BindChannelKeyRequest {
        channel: ChannelKind::Whatsapp,
        telegram_bot_name: None,
        external_user_id: Some(wa_id.to_string()),
        external_chat_id: Some(wa_id.to_string()),
        user_key: user_key.trim().to_string(),
    };
    let resp = state.client.post(&url).json(&req).send().await?;
    let status = resp.status();
    let body: ApiResponse<BindChannelKeyResponse> = resp.json().await?;
    if !status.is_success() {
        if status.as_u16() == 401 {
            return Ok(None);
        }
        return Err(whatsapp_provider_invalid_response(
            "bind_identity",
            "application_rejected",
        ));
    }
    if !body.ok {
        return Ok(None);
    }
    Ok(body.data)
}

async fn store_pending_whatsapp_request(
    state: &AppState,
    msg: &WaMessage,
    text: &str,
) -> anyhow::Result<Option<PendingChannelRequestStatus>> {
    let prompt = text.trim();
    let media = msg
        .image
        .as_ref()
        .map(|media| ("image", media))
        .or_else(|| msg.audio.as_ref().map(|media| ("audio", media)))
        .or_else(|| msg.document.as_ref().map(|media| ("file", media)));
    if (prompt.is_empty() && media.is_none()) || msg.id.trim().is_empty() {
        return Ok(None);
    }
    let idempotency_key = format!("pending:whatsapp_cloud:{}", msg.id.trim());
    let mut ingress = claw_core::channel_ingress::ChannelIngressEnvelope::new(
        ChannelKind::Whatsapp,
        "whatsapp_cloud",
    )
    .with_external_ids(msg.from.clone(), msg.from.clone())
    .with_message_id(msg.id.clone())
    .with_received_at_ts(now_ts())
    .with_reply_target(claw_core::channel_ingress::ChannelReplyTarget::user(
        msg.from.clone(),
    ))
    .with_locale(state.language.clone());
    if let Some((kind, media)) = media {
        ingress
            .attachments
            .push(claw_core::channel_ingress::ChannelIngressAttachment {
                kind: kind.to_string(),
                path: format!("provider://whatsapp_cloud/{}", media.id),
                mime_type: media.mime_type.clone(),
                size: None,
            });
    }
    let request = SubmitTaskRequest {
        user_id: None,
        chat_id: None,
        user_key: None,
        channel: Some(ChannelKind::Whatsapp),
        external_user_id: Some(msg.from.clone()),
        external_chat_id: Some(msg.from.clone()),
        ingress: Some(ingress),
        idempotency_key: Some(idempotency_key.clone()),
        kind: TaskKind::Ask,
        payload: json!({ "text": prompt, "adapter": "whatsapp_cloud" }),
    };
    let url = format!("{}/v1/auth/channel/pending-request", state.clawd_base_url);
    let response = state
        .client
        .post(url)
        .json(&PendingChannelRequestStoreRequest {
            idempotency_key,
            expires_in_seconds: None,
            request,
        })
        .send()
        .await?;
    let status = response.status();
    let body: ApiResponse<PendingChannelRequestStatus> = response.json().await?;
    if !status.is_success() || !body.ok {
        return Err(whatsapp_provider_invalid_response(
            "store_pending_request",
            body.error.as_deref().unwrap_or("application_rejected"),
        ));
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
    let text = msg.text.as_ref().map(|t| t.body.trim()).unwrap_or("");
    format!(
        "wa_fallback:{}:{}:{}",
        msg.from.trim(),
        msg.message_type.trim(),
        text
    )
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
            msg.from, msg.id, msg.message_type
        );
        return Ok(());
    }
    if msg.from.trim().is_empty() {
        return Ok(());
    }
    if let Ok(mut windows) = state.last_inbound_at_by_user.lock() {
        windows.insert(msg.from.clone(), now_ts());
    }
    let inbound_text = msg
        .text
        .as_ref()
        .map(|v| v.body.trim().to_string())
        .unwrap_or_default();
    let slash_command = state
        .command_catalog
        .match_command(&inbound_text, "whatsapp");
    let core_action = slash_command
        .as_ref()
        .and_then(|command| command.definition.core_action());
    if let Some(candidate) = extract_bind_key_candidate(inbound_text.trim(), false) {
        if let Some(bind_result) = bind_whatsapp_identity(state, &msg.from, &candidate).await? {
            let identity = bind_result.identity;
            set_expect_key_reply(state, &msg.from, false);
            store_bound_identity(state, &msg.from, &identity);
            let text = wa_t(state, WA_I18N_BIND_SUCCESS_KEY, WA_BIND_SUCCESS_FALLBACK);
            let _ = send_whatsapp_text(state, &msg.from, &text).await;
            if let Some(resume) = bind_result.pending_resume {
                if let Some(task_id) = resume.task_id {
                    let target = resume.external_user_id.unwrap_or_else(|| msg.from.clone());
                    spawn_task_result_delivery(state.clone(), target, task_id.to_string(), None);
                } else if resume.error_code.is_some() {
                    let stopped = wa_t(
                        state,
                        WA_I18N_PENDING_RESUME_STOPPED_KEY,
                        WA_PENDING_RESUME_STOPPED_FALLBACK,
                    );
                    let _ = send_whatsapp_text(state, &msg.from, &stopped).await;
                }
            }
        } else {
            set_expect_key_reply(state, &msg.from, true);
            let text = wa_t(state, WA_I18N_BIND_INVALID_KEY, WA_BIND_INVALID_FALLBACK);
            let _ = send_whatsapp_text(state, &msg.from, &text).await;
        }
        return Ok(());
    }
    let identity = match resolve_whatsapp_identity(state, &msg.from).await? {
        Some(identity) => {
            set_expect_key_reply(state, &msg.from, false);
            identity
        }
        None => {
            let trimmed_text = inbound_text.trim();
            if is_unbound_allowed_command(state.command_catalog.as_ref(), "whatsapp", trimmed_text)
            {
                set_expect_key_reply(state, &msg.from, true);
                let text = wa_t(state, WA_I18N_BIND_HELP_KEY, WA_BIND_HELP_FALLBACK);
                let _ = send_whatsapp_text(state, &msg.from, &text).await;
                return Ok(());
            }
            let maybe_candidate =
                extract_bind_key_candidate(trimmed_text, should_expect_key_reply(state, &msg.from));
            if let Some(candidate) = maybe_candidate {
                if let Some(bind_result) =
                    bind_whatsapp_identity(state, &msg.from, &candidate).await?
                {
                    let identity = bind_result.identity;
                    set_expect_key_reply(state, &msg.from, false);
                    store_bound_identity(state, &msg.from, &identity);
                    let text = wa_t(state, WA_I18N_BIND_SUCCESS_KEY, WA_BIND_SUCCESS_FALLBACK);
                    let _ = send_whatsapp_text(state, &msg.from, &text).await;
                    if let Some(resume) = bind_result.pending_resume {
                        if let Some(task_id) = resume.task_id {
                            let target =
                                resume.external_user_id.unwrap_or_else(|| msg.from.clone());
                            spawn_task_result_delivery(
                                state.clone(),
                                target,
                                task_id.to_string(),
                                None,
                            );
                        } else if resume.error_code.is_some() {
                            let stopped = wa_t(
                                state,
                                WA_I18N_PENDING_RESUME_STOPPED_KEY,
                                WA_PENDING_RESUME_STOPPED_FALLBACK,
                            );
                            let _ = send_whatsapp_text(state, &msg.from, &stopped).await;
                        }
                    }
                } else {
                    set_expect_key_reply(state, &msg.from, true);
                    let text = wa_t(state, WA_I18N_BIND_INVALID_KEY, WA_BIND_INVALID_FALLBACK);
                    let _ = send_whatsapp_text(state, &msg.from, &text).await;
                }
                return Ok(());
            }
            if let Err(error) = store_pending_whatsapp_request(state, &msg, trimmed_text).await {
                warn!(
                    "whatsappd: pending request persistence failed wa_id={} error={}",
                    msg.from, error
                );
            }
            let _ = send_bind_required_prompt(state, &msg.from).await;
            return Ok(());
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
            if matches!(core_action, Some(CoreCommandAction::RunSkill)) {
                let command_tail = slash_command
                    .as_ref()
                    .map(|command| command.tail.as_str())
                    .unwrap_or_default();
                handle_run_command(state, &msg.from, user_id, chat_id, &msg.id, command_tail)
                    .await?;
            } else {
                let payload = json!({ "text": text.trim() });
                let task_id = submit_task_only(
                    state,
                    user_id,
                    chat_id,
                    &msg.from,
                    Some(&msg.id),
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
                handle_image_message(state, &msg.from, user_id, chat_id, &msg.id, &media).await?;
            }
        }
        "audio" => {
            if let Some(media) = msg.audio {
                handle_audio_message(state, &msg.from, user_id, chat_id, &msg.id, &media).await?;
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
                    handle_image_message(state, &msg.from, user_id, chat_id, &msg.id, &media)
                        .await?;
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
    message_id: &str,
    command_tail: &str,
) -> anyhow::Result<()> {
    let rest = command_tail.trim();
    if rest.is_empty() {
        let text = wa_t(state, WA_I18N_RUN_USAGE_KEY, WA_RUN_USAGE_FALLBACK);
        send_whatsapp_text(state, wa_id, &text).await?;
        return Ok(());
    }
    let mut parts = rest.splitn(2, ' ');
    let skill_name = parts.next().unwrap_or_default().trim();
    let args = parts.next().unwrap_or_default().trim();
    if skill_name.is_empty() {
        let text = wa_t(state, WA_I18N_RUN_USAGE_KEY, WA_RUN_USAGE_FALLBACK);
        send_whatsapp_text(state, wa_id, &text).await?;
        return Ok(());
    }
    let payload = json!({
        "skill_name": skill_name,
        "args": args
    });
    let task_id = submit_task_only(
        state,
        user_id,
        chat_id,
        wa_id,
        Some(message_id),
        TaskKind::RunSkill,
        payload,
    )
    .await?;
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
    message_id: &str,
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
    let task_id = submit_task_only(
        state,
        user_id,
        chat_id,
        wa_id,
        Some(message_id),
        TaskKind::RunSkill,
        payload,
    )
    .await?;
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
    message_id: &str,
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
    let task_id = submit_task_only(
        state,
        user_id,
        chat_id,
        wa_id,
        Some(message_id),
        TaskKind::RunSkill,
        transcribe_payload,
    )
    .await?;
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

async fn download_whatsapp_media(
    state: &AppState,
    media_id: &str,
    local_path: &Path,
) -> anyhow::Result<()> {
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
        return Err(whatsapp_provider_http_error(
            "fetch_media_metadata",
            status,
            &body,
        ));
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
    message_id: Option<&str>,
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
        user_id: Some(user_id),
        chat_id: Some(chat_id),
        user_key,
        channel: Some(ChannelKind::Whatsapp),
        external_user_id: Some(wa_id.to_string()),
        external_chat_id: Some(wa_id.to_string()),
        ingress: Some({
            let mut ingress = claw_core::channel_ingress::ChannelIngressEnvelope::new(
                ChannelKind::Whatsapp,
                "whatsapp_cloud",
            )
            .with_external_ids(wa_id.to_string(), wa_id.to_string())
            .with_received_at_ts(now_ts())
            .with_reply_target(claw_core::channel_ingress::ChannelReplyTarget::user(
                wa_id.to_string(),
            ))
            .with_locale(state.language.clone());
            if let Some(message_id) = message_id {
                ingress = ingress.with_message_id(message_id);
            }
            ingress
        }),
        idempotency_key: message_id.map(|value| format!("whatsapp_cloud:{value}")),
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
        return Err(whatsapp_provider_http_error("submit_task", status, &body));
    }
    let body: ApiResponse<SubmitTaskResponse> = resp
        .json()
        .await
        .context("decode submit task response failed")?;
    if !body.ok {
        return Err(whatsapp_provider_invalid_response(
            "submit_task",
            "application_rejected",
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
        req = req.header("X-Agent-Key", user_key);
    }
    let resp = req.send().await.context("query task status failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(whatsapp_provider_http_error("query_task", status, &body));
    }
    let body: ApiResponse<TaskQueryResponse> = resp
        .json()
        .await
        .context("decode query task response failed")?;
    if !body.ok {
        return Err(whatsapp_provider_invalid_response(
            "query_task",
            "application_rejected",
        ));
    }
    body.data.ok_or_else(|| anyhow!("query task missing data"))
}

async fn poll_task_result(
    state: &AppState,
    task_id: &str,
    user_key: Option<&str>,
    wait_override_seconds: Option<u64>,
) -> anyhow::Result<()> {
    let poll_interval_ms = state.poll_interval_ms.max(1);
    let wait_seconds = wait_override_seconds
        .unwrap_or(state.task_wait_seconds)
        .max(1);
    let max_rounds = ((wait_seconds * 1000) / poll_interval_ms).max(1);
    for _ in 0..max_rounds {
        let task = query_task_status(state, task_id, user_key).await?;
        match task.status {
            TaskStatus::Queued | TaskStatus::Running => {
                tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
            }
            TaskStatus::Succeeded
            | TaskStatus::Failed
            | TaskStatus::Canceled
            | TaskStatus::Timeout => return Ok(()),
        }
    }
    Err(anyhow!("task_result_wait_timeout"))
}

async fn poll_task_result_with_soft_timeout(
    state: &AppState,
    wa_id: &str,
    task_id: &str,
    user_key: Option<&str>,
    wait_override_seconds: Option<u64>,
) -> anyhow::Result<bool> {
    let poll_interval_ms = state.poll_interval_ms.max(1);
    let delivery_timeout_secs = wait_override_seconds
        .unwrap_or(state.task_wait_seconds)
        .max(1);
    let running_notice_text = wa_t_with(
        state,
        WA_I18N_REQUEST_TIMEOUT_RETRY_LATER_KEY,
        &[("task_id", task_id)],
        WA_REQUEST_TIMEOUT_RETRY_LATER_FALLBACK,
    );
    info!(
        "whatsappd: task delivery started task_id={} wa_id={} task_delivery_timeout_seconds={}",
        task_id, wa_id, delivery_timeout_secs
    );
    let started = std::time::Instant::now();
    let mut timeout_notice_sent = false;
    let mut last_seen_status: Option<TaskStatus> = None;

    loop {
        let task = match query_task_status(state, task_id, user_key).await {
            Ok(task) => task,
            Err(err) => {
                warn!("whatsappd: poll failed task_id={} err={}", task_id, err);
                if started.elapsed() > Duration::from_secs(delivery_timeout_secs)
                    && !timeout_notice_sent
                {
                    warn!(
                        "whatsappd: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=poll_failed (continue_polling=true)",
                        task_id,
                        started.elapsed().as_secs(),
                        delivery_timeout_secs,
                        last_seen_status
                    );
                    let _ = send_whatsapp_text(state, wa_id, &running_notice_text).await;
                    timeout_notice_sent = true;
                }
                tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
                continue;
            }
        };

        last_seen_status = Some(task.status.clone());
        match task.status {
            TaskStatus::Queued | TaskStatus::Running => {
                if started.elapsed() > Duration::from_secs(delivery_timeout_secs)
                    && !timeout_notice_sent
                {
                    warn!(
                        "whatsappd: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} (continue_polling=true)",
                        task_id,
                        started.elapsed().as_secs(),
                        delivery_timeout_secs,
                        last_seen_status
                    );
                    let _ = send_whatsapp_text(state, wa_id, &running_notice_text).await;
                    timeout_notice_sent = true;
                }
                tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
            }
            TaskStatus::Succeeded
            | TaskStatus::Failed
            | TaskStatus::Canceled
            | TaskStatus::Timeout => return Ok(timeout_notice_sent),
        }
    }
}

async fn try_deliver_quick_result(
    state: &AppState,
    wa_id: &str,
    task_id: &str,
    wait_override_seconds: Option<u64>,
) -> anyhow::Result<bool> {
    let wait = wait_override_seconds.or(Some(state.quick_result_wait_seconds));
    match poll_task_result(
        state,
        task_id,
        bound_user_key_for_wa(state, wa_id).as_deref(),
        wait,
    )
    .await
    {
        Ok(()) => {
            request_unified_terminal_delivery(state, wa_id, task_id, false).await?;
            Ok(true)
        }
        Err(err) if err.to_string() == "task_result_wait_timeout" => Ok(false),
        Err(err) => Err(err),
    }
}

fn spawn_task_result_delivery(
    state: AppState,
    wa_id: String,
    task_id: String,
    wait_override_seconds: Option<u64>,
) {
    tokio::spawn(async move {
        let out = poll_task_result_with_soft_timeout(
            &state,
            &wa_id,
            &task_id,
            bound_user_key_for_wa(&state, &wa_id).as_deref(),
            wait_override_seconds,
        )
        .await;
        match out {
            Ok(background) => {
                if let Err(error) =
                    request_unified_terminal_delivery(&state, &wa_id, &task_id, background).await
                {
                    warn!(
                        "whatsappd: unified terminal delivery failed task_id={} error={}",
                        task_id, error
                    );
                }
            }
            Err(error) => warn!(
                "whatsappd: task result polling failed task_id={} error={}",
                task_id, error
            ),
        }
    });
}

async fn request_unified_terminal_delivery(
    state: &AppState,
    wa_id: &str,
    task_id: &str,
    background: bool,
) -> anyhow::Result<()> {
    let user_key = bound_user_key_for_wa(state, wa_id)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("channel_task_delivery_bound_key_missing"))?;
    let source = if background {
        claw_core::channel_delivery::ChannelDeliverySource::BackgroundCompletion
    } else {
        claw_core::channel_delivery::ChannelDeliverySource::ImmediateDaemon
    };
    let result = claw_core::channel_delivery_client::request_task_delivery(
        &state.client,
        &state.clawd_base_url,
        task_id,
        &user_key,
        source,
    )
    .await?;
    if result.accepted {
        info!(
            "whatsappd: unified terminal delivery accepted task_id={} status={:?}",
            task_id, result.status
        );
        Ok(())
    } else {
        Err(anyhow!(result.error_code.unwrap_or_else(|| {
            "channel_task_delivery_not_accepted".to_string()
        })))
    }
}

async fn send_whatsapp_text(
    state: &AppState,
    wa_id: &str,
    text: &str,
) -> anyhow::Result<Vec<String>> {
    let url = format!(
        "{}/v23.0/{}/messages",
        state.api_base,
        state.phone_number_id.trim()
    );
    let chunks = chunk_text_for_channel(
        text,
        WHATSAPP_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    let n = chunks.len();
    let mut provider_message_ids = Vec::new();
    if n > 1 {
        info!(
            "send_chunks channel=whatsapp wa_id={} original_len={} chunk_count={}",
            wa_id,
            text.len(),
            n
        );
    }
    for (i, chunk) in chunks.into_iter().enumerate() {
        let body = if n > 1 {
            format!("（{}/{}）\n{}", i + 1, n, chunk)
        } else {
            chunk
        };
        if n > 1 {
            info!(
                "send_chunk channel=whatsapp wa_id={} index={} total={}",
                wa_id,
                i + 1,
                n
            );
        }
        let resp = state
            .client
            .post(&url)
            .bearer_auth(state.access_token.trim())
            .json(&json!({
                "messaging_product": "whatsapp",
                "to": wa_id,
                "type": "text",
                "text": { "body": body }
            }))
            .send()
            .await
            .context("send text message failed")?;
        let status = resp.status();
        let response_body = resp
            .text()
            .await
            .context("read send text response failed")?;
        if !status.is_success() {
            return Err(whatsapp_provider_http_error(
                "send_text",
                status,
                &response_body,
            ));
        }
        let ids =
            claw_core::channel_whatsapp_cloud::decode_message_ids("send_text", &response_body)?;
        info!(
            event = "whatsapp_cloud_message_accepted",
            provider_message_ids = ?ids,
            "whatsapp_cloud_text_accepted"
        );
        provider_message_ids.extend(ids);
    }
    Ok(provider_message_ids)
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
