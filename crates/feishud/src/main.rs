//! Feishu (Lark) 应用机器人通道 - Phase 1 最小可用
//! 支持两种入站模式：webhook（事件回调）、long_connection（长连接收事件）
//! 仅支持文本消息 → clawd ask → 轮询结果 → 文本回发

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use claw_core::types::{
    ApiResponse, AuthIdentity, BindChannelKeyRequest, ChannelKind, ResolveChannelBindingRequest,
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
    /// 整条任务轮询最长等待时间（秒），与 request_timeout_seconds 分离
    #[serde(default = "default_task_delivery_timeout")]
    task_delivery_timeout_seconds: u64,
    #[serde(default = "default_text_chunk_chars")]
    text_chunk_chars: usize,
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
    180
}
fn default_text_chunk_chars() -> usize {
    4000
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
    let header = body.get("header")?;
    if header.get("event_type").and_then(|v| v.as_str())? != "im.message.receive_v1" {
        return None;
    }
    let event = body.get("event")?;
    let message = event.get("message")?;
    if message.get("message_type").and_then(|v| v.as_str())? != "text" {
        return None;
    }
    let content_str = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let content: Value = serde_json::from_str(content_str).ok()?;
    let text = content
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if text.is_empty() {
        return None;
    }
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

/// 入站文本统一入口：先 resolve 绑定，已绑定则提交 ask；未绑定则尝试用当前文本 bind，成功则提示重发，失败则提示发 key。
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
            let _ = send_feishu_text(
                &config,
                &client,
                &token_cache,
                &chat_id,
                "身份校验暂时不可用，请稍后重试。",
            )
            .await;
            return;
        }
    };

    if let Some(ident) = identity {
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
    if trimmed.is_empty() {
        info!(
            "feishud: unbound user prompted for key (empty text) external_chat_id={}",
            chat_id
        );
        let _ = send_feishu_text(
            &config,
            &client,
            &token_cache,
            &chat_id,
            "请先发送你的 RustClaw key 完成绑定。",
        )
        .await;
        return;
    }

    info!(
        "feishud: bind attempt external_chat_id={} key_len={}",
        chat_id,
        trimmed.len()
    );
    match bind_feishu_identity(&client, &base, &open_id, &chat_id, trimmed).await {
        Ok(Some(_)) => {
            info!("feishud: bind success external_chat_id={}", chat_id);
            let _ = send_feishu_text(
                &config,
                &client,
                &token_cache,
                &chat_id,
                "绑定成功，请重新发送你的问题。",
            )
            .await;
        }
        Ok(None) => {
            warn!(
                "feishud: bind failure (invalid key) external_chat_id={}",
                chat_id
            );
            let _ = send_feishu_text(
                &config,
                &client,
                &token_cache,
                &chat_id,
                "key 无效或绑定失败，请发送有效 key 完成绑定。",
            )
            .await;
        }
        Err(e) => {
            warn!(
                "feishud: bind request failed err={} external_chat_id={}",
                e, chat_id
            );
            let _ = send_feishu_text(
                &config,
                &client,
                &token_cache,
                &chat_id,
                "绑定请求失败，请稍后重试。",
            )
            .await;
        }
    }
}

/// 从成功任务的 result_json 取回复正文：优先 messages 数组（与 telegramd 一致），其次 text，否则占位。
fn feishu_task_success_text(task: &TaskQueryResponse) -> String {
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

        let clawd_base = config.feishu.clawd_base_url.clone();
        let chat_id_delivery = chat_id.clone();

        info!(
            "feishud: task delivery started task_id={} chat_id={} task_delivery_timeout_seconds={}",
            task_id, chat_id_delivery, delivery_timeout_secs
        );
        let started = std::time::Instant::now();
        let mut last_seen_status: Option<TaskStatus> = None;
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
                        warn!("feishud: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=poll_failed", task_id, started.elapsed().as_secs(), delivery_timeout_secs, last_seen_status);
                        let _ = send_feishu_text(
                            &config,
                            &client,
                            &token_cache,
                            &chat_id_delivery,
                            "请求处理超时，请稍后重试。",
                        )
                        .await;
                        break;
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
                    warn!("feishud: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=http status={}", task_id, started.elapsed().as_secs(), delivery_timeout_secs, last_seen_status, status);
                    let _ = send_feishu_text(
                        &config,
                        &client,
                        &token_cache,
                        &chat_id_delivery,
                        "请求处理超时，请稍后重试。",
                    )
                    .await;
                    break;
                }
                tokio::time::sleep(poll_interval).await;
                continue;
            }
            let body: ApiResponse<TaskQueryResponse> = match resp.json().await {
                Ok(b) => b,
                Err(e) => {
                    debug!("feishud: poll parse failed task_id={} err={}", task_id, e);
                    if started.elapsed() > Duration::from_secs(delivery_timeout_secs) {
                        warn!("feishud: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=parse_failed", task_id, started.elapsed().as_secs(), delivery_timeout_secs, last_seen_status);
                        let _ = send_feishu_text(
                            &config,
                            &client,
                            &token_cache,
                            &chat_id_delivery,
                            "请求处理超时，请稍后重试。",
                        )
                        .await;
                        break;
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
                    warn!("feishud: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?} reason=no_task_data error={}", task_id, started.elapsed().as_secs(), delivery_timeout_secs, last_seen_status, err_msg);
                    let _ = send_feishu_text(
                        &config,
                        &client,
                        &token_cache,
                        &chat_id_delivery,
                        "请求处理超时，请稍后重试。",
                    )
                    .await;
                    break;
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
                        warn!("feishud: task delivery timeout task_id={} elapsed_secs={} timeout_limit_secs={} last_seen_status={:?}", task_id, started.elapsed().as_secs(), delivery_timeout_secs, last_seen_status);
                        let _ = send_feishu_text(
                            &config,
                            &client,
                            &token_cache,
                            &chat_id_delivery,
                            "请求处理超时，请稍后重试。",
                        )
                        .await;
                        break;
                    }
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }
                TaskStatus::Succeeded => {
                    let to_send = feishu_task_success_text(task);
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
                    info!(
                        "feishud: task delivery success task_id={} (result sent)",
                        task_id
                    );
                    break;
                }
                TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Timeout => {
                    let detail = task.error_text.as_deref().unwrap_or("任务失败").to_string();
                    let _ = send_feishu_text(
                        &config,
                        &client,
                        &token_cache,
                        &chat_id_delivery,
                        &format!("处理失败：{}", detail),
                    )
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

    let Some((open_id, chat_id, text)) = parse_im_text_from_event_body(&body_json) else {
        info!("feishud: event ignored (not im.message.receive_v1 text or missing chat_id)");
        return Json(json!({})).into_response();
    };

    tokio::spawn(handle_incoming_feishu_text(state, open_id, chat_id, text));
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
    let url = "https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal";
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
    let url = "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id";
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
                            warn!("feishud: long_connection event parse failed with reason: {}", e);
                            return Ok(());
                        }
                    };
                    let event_type = body
                        .get("header")
                        .and_then(|h| h.get("event_type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    tracing::debug!("feishud: long_connection raw event received event_type={} body_len={}", event_type, body_len);

                    match parse_im_text_from_event_body(&body) {
                        Some((open_id, chat_id, text)) => {
                            info!("feishud: long_connection event parse success chat_id={} open_id={} text_len={}", chat_id, open_id, text.len());
                            let state = (*state).clone();
                            tokio::spawn(handle_incoming_feishu_text(state, open_id, chat_id, text));
                        }
                        None => {
                            let header_ok = body
                                .get("header")
                                .and_then(|h| h.get("event_type"))
                                .and_then(|v| v.as_str())
                                == Some("im.message.receive_v1");
                            if header_ok {
                                info!("feishud: long_connection event parse skipped (not text / missing fields) event_type={} body_len={}", event_type, body_len);
                            } else {
                                tracing::debug!("feishud: long_connection event parse skipped (not im.message.receive_v1) event_type={}", event_type);
                            }
                        }
                    }
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
    let config: FeishuConfig = {
        let raw = std::fs::read_to_string(&config_path)
            .map_err(|e| anyhow::anyhow!("read config {}: {}", config_path, e))?;
        toml::from_str(&raw).map_err(|e| anyhow::anyhow!("parse config: {}", e))?
    };

    if !config.feishu.enabled {
        tracing::info!("feishud: disabled in config, exiting");
        return Ok(());
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(config.feishu.request_timeout_seconds))
        .build()?;

    let state = AppState {
        config: config.clone(),
        client: client.clone(),
        token_cache: Arc::new(RwLock::new(None)),
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
