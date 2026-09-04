//! Channel text sending with safe chunking (Telegram, WhatsApp Cloud, WhatsApp Web Bridge, Feishu, Lark).
//! Used when clawd delivers task results directly to a channel (e.g. schedule_triggered notify).

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};

use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use toml::Value as TomlValue;
use tracing::{info, warn};

use claw_core::channel_chunk::{chunk_text_for_channel, SEGMENT_PREFIX_MAX_CHARS};
use claw_core::channel_delivery::{
    ChannelConversationWindow, ChannelConversationWindowState, ChannelDeliverySource,
};
use claw_core::channel_open_platform::{
    chunk_open_platform_text, open_platform_contract, plan_open_platform_media,
    preflight_open_platform_media, process_open_platform_rate_limiter,
    process_open_platform_token_cache, validate_open_platform_content, OpenPlatformContentError,
    OpenPlatformMediaPlan, OpenPlatformMessageType, OpenPlatformOutboundMediaKind,
    OpenPlatformRegion, OpenPlatformUploadEndpoint,
};
use claw_core::channel_provider_error::{ChannelProviderError, ChannelProviderTransportKind};
use claw_core::wechat_reply_media::{
    extract_wechat_outbound_media, strip_wechat_delivery_lines, WechatOutboundKind,
    WechatOutboundMedia, WechatOutboundSource,
};

use crate::AppState;
use wechat_ilink::http::IlinkAuth;
use wechat_ilink::{
    download_remote_media_to_temp, send_weixin_file_from_file_with_client_id,
    send_weixin_image_from_file_with_client_id, send_weixin_video_from_file_with_client_id,
    WechatMessageItem, WechatSendMessageRequest,
};

/// Feishu 中国站发送配置（定时任务主动推送用，从 configs/channels/feishu.toml 可选加载）
#[derive(Clone, Debug)]
pub struct FeishuSendConfig {
    pub app_id: String,
    pub app_secret: String,
    pub api_base_url: String,
}

/// Lark 国际版发送配置（定时任务主动推送用，从 configs/channels/lark.toml 可选加载）
#[derive(Clone, Debug)]
pub struct LarkSendConfig {
    pub app_id: String,
    pub app_secret: String,
    pub api_base_url: String,
}

/// WeChat 发送配置（文本 + CDN 媒体，与 OpenClaw weixin 对齐）
#[derive(Clone, Debug)]
pub struct WechatSendConfig {
    pub api_base_url: String,
    pub bot_token: String,
    pub wechat_uin_base64: Option<String>,
    pub text_chunk_chars: usize,
    /// Optional `SKRouteTag` (same as OpenClaw weixin plugin / gateway routing).
    pub sk_route_tag: Option<String>,
    /// CDN root for outbound images/videos/files (`novac2c.cdn.weixin.qq.com/c2c`).
    pub cdn_base_url: String,
}

#[derive(Debug, Deserialize)]
struct PersistedWechatSession {
    #[serde(default)]
    bot_token: String,
    #[serde(default)]
    base_url: Option<String>,
}

/// Max characters per Telegram message (conservative; platform limit ~4096).
const TELEGRAM_TEXT_CHUNK_CHARS: usize = 3500;

/// Local transport chunk target. It is not an official WhatsApp Web limit.
const WHATSAPP_TEXT_CHUNK_CHARS: usize = 3500;
const CLAWD_WECHAT_CHANNEL_VERSION: &str = env!("CARGO_PKG_VERSION");
const WECHAT_MEDIA_OUTBOUND_TEMP_DIR: &str = "/tmp/agent-runtime/wechat/media/outbound-temp";
const CHANNEL_MEDIA_OUTBOUND_TEMP_DIR: &str = "/tmp/agent-runtime/channel/media/outbound-temp";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ChannelSendOutcome {
    pub(crate) provider_message_ids: Vec<String>,
}

tokio::task_local! {
    static CHANNEL_SEND_PROVIDER_MESSAGE_IDS: RefCell<Vec<String>>;
}

pub(crate) async fn capture_channel_send_progress<F, T>(future: F) -> (T, Vec<String>)
where
    F: Future<Output = T>,
{
    CHANNEL_SEND_PROVIDER_MESSAGE_IDS
        .scope(RefCell::new(Vec::new()), async move {
            let result = future.await;
            let provider_message_ids =
                CHANNEL_SEND_PROVIDER_MESSAGE_IDS.with(|ids| ids.borrow().clone());
            (result, provider_message_ids)
        })
        .await
}

fn record_provider_message_id(provider_message_id: &str) {
    if provider_message_id.trim().is_empty() {
        return;
    }
    let _ = CHANNEL_SEND_PROVIDER_MESSAGE_IDS.try_with(|ids| {
        ids.borrow_mut().push(provider_message_id.to_string());
    });
}

fn record_provider_message_ids(provider_message_ids: &[String]) {
    for provider_message_id in provider_message_ids {
        record_provider_message_id(provider_message_id);
    }
}

fn provider_http_error(
    source_adapter: &str,
    operation: &str,
    status: reqwest::StatusCode,
    response_body: &str,
) -> String {
    if source_adapter == claw_core::channel_whatsapp_cloud::WHATSAPP_CLOUD_SOURCE_ADAPTER {
        return claw_core::channel_whatsapp_cloud::provider_error_from_response(
            operation,
            status.as_u16(),
            response_body,
        )
        .to_string();
    }
    let open_platform_region = [OpenPlatformRegion::Feishu, OpenPlatformRegion::Lark]
        .into_iter()
        .find(|region| open_platform_contract(*region).source_adapter == source_adapter);
    if let Some(region) = open_platform_region {
        return claw_core::channel_open_platform::open_platform_provider_error(
            region,
            operation,
            status.as_u16(),
            response_body,
        )
        .to_string();
    }
    ChannelProviderError::from_http_response(
        source_adapter,
        operation,
        status.as_u16(),
        response_body,
    )
    .to_string()
}

fn provider_transport_error(
    source_adapter: &str,
    operation: &str,
    error: &reqwest::Error,
) -> String {
    let kind = if error.is_timeout() {
        ChannelProviderTransportKind::Timeout
    } else if error.is_connect() {
        ChannelProviderTransportKind::Connect
    } else if error.is_request() {
        ChannelProviderTransportKind::Request
    } else if error.is_body() {
        ChannelProviderTransportKind::Body
    } else if error.is_decode() {
        ChannelProviderTransportKind::Decode
    } else {
        ChannelProviderTransportKind::Unknown
    };
    ChannelProviderError::from_transport(source_adapter, operation, kind, &error.to_string())
        .to_string()
}

fn provider_invalid_response(
    source_adapter: &str,
    operation: &str,
    diagnostic_material: &str,
) -> String {
    ChannelProviderError::invalid_response(source_adapter, operation, diagnostic_material)
        .to_string()
}

fn provider_content_error(
    source_adapter: &str,
    operation: &str,
    error: OpenPlatformContentError,
) -> String {
    ChannelProviderError::from_machine_failure(
        source_adapter,
        operation,
        claw_core::channel_provider_error::ChannelProviderFailureClass::PayloadRejected,
        None,
        Some(error.error_code()),
        None,
        &format!("{}:{}", error.actual_bytes, error.max_bytes),
    )
    .to_string()
}

fn default_wechat_cdn_base_url() -> String {
    "https://novac2c.cdn.weixin.qq.com/c2c".to_string()
}

fn wechat_delivery_client_id(delivery_id: &str, part_kind: &str, part_index: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(delivery_id.as_bytes());
    hasher.update([0]);
    hasher.update(part_kind.as_bytes());
    hasher.update([0]);
    hasher.update(part_index.to_string().as_bytes());
    let digest = hasher.finalize();
    format!("clawd-wechat-{}", hex::encode(&digest[..16]))
}

async fn materialize_wechat_outbound_media(
    state: &AppState,
    media: &WechatOutboundMedia,
) -> Result<PathBuf, String> {
    match &media.source {
        WechatOutboundSource::LocalPath(path) => Ok(path.clone()),
        WechatOutboundSource::RemoteUrl(url) => {
            download_remote_media_to_temp(
                &state.core.http_client,
                url,
                Path::new(WECHAT_MEDIA_OUTBOUND_TEMP_DIR),
                "clawd-wechat",
            )
            .await
        }
    }
}

async fn materialize_channel_outbound_media(
    state: &AppState,
    media: &WechatOutboundMedia,
    channel_tag: &str,
) -> Result<PathBuf, String> {
    match &media.source {
        WechatOutboundSource::LocalPath(path) => Ok(path.clone()),
        WechatOutboundSource::RemoteUrl(url) => {
            download_remote_media_to_temp(
                &state.core.http_client,
                url,
                Path::new(CHANNEL_MEDIA_OUTBOUND_TEMP_DIR),
                channel_tag,
            )
            .await
        }
    }
}

#[cfg(test)]
pub(crate) async fn send_telegram_message(
    state: &AppState,
    chat_id: i64,
    text: &str,
) -> Result<ChannelSendOutcome, String> {
    send_telegram_message_for_bot(state, None, chat_id, text).await
}

pub(crate) async fn send_telegram_message_for_bot(
    state: &AppState,
    bot_name: Option<&str>,
    chat_id: i64,
    text: &str,
) -> Result<ChannelSendOutcome, String> {
    let requested_bot = bot_name.map(str::trim).filter(|value| !value.is_empty());
    let token = match requested_bot {
        Some(name) => state
            .channels
            .telegram_bot_tokens
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| "telegram_bot_instance_not_configured".to_string())?,
        None => state.channels.telegram_bot_token.trim(),
    };
    if token.is_empty() {
        return Err("telegram bot token is empty".to_string());
    }
    let media = extract_wechat_outbound_media(text, &state.skill_rt.workspace_root);
    let stripped = strip_wechat_delivery_lines(text);
    let send_text = if stripped.trim().is_empty() && media.is_empty() && !text.trim().is_empty() {
        text
    } else {
        stripped.as_str()
    };
    let (send_text, url_buttons) = extract_telegram_url_buttons(send_text);
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let chunks = chunk_text_for_channel(
        &send_text,
        TELEGRAM_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    let n = chunks.len();
    let mut provider_message_ids = Vec::new();
    if n > 1 {
        info!(
            "send_chunks channel=telegram chat_id={} original_len={} chunk_count={}",
            chat_id,
            send_text.len(),
            n
        );
    }
    for (i, chunk) in chunks.into_iter().enumerate() {
        let body_text = if n > 1 {
            format!("（{}/{}）\n{}", i + 1, n, chunk)
        } else {
            chunk
        };
        if n > 1 {
            info!(
                "send_chunk channel=telegram chat_id={} index={} total={}",
                chat_id,
                i + 1,
                n
            );
        }
        let mut request_body = json!({
            "chat_id": chat_id,
            "text": body_text
        });
        if i + 1 == n && !url_buttons.is_empty() {
            request_body["reply_markup"] = json!({
                "inline_keyboard": url_buttons
                    .iter()
                    .map(|(label, url)| vec![json!({"text": label, "url": url})])
                    .collect::<Vec<_>>()
            });
        }
        let resp = state
            .core
            .http_client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|error| provider_transport_error("telegram_bot", "send_text", &error))?;
        let status = resp.status();
        let response_body = resp
            .text()
            .await
            .map_err(|error| provider_transport_error("telegram_bot", "send_text", &error))?;
        if !status.is_success() {
            return Err(provider_http_error(
                "telegram_bot",
                "send_text",
                status,
                &response_body,
            ));
        }
        let provider_message_id = telegram_message_id("send_text", &response_body)?;
        record_provider_message_id(&provider_message_id);
        provider_message_ids.push(provider_message_id);
    }
    for item in &media {
        let path = materialize_channel_outbound_media(state, item, "telegram").await?;
        let (method, field, max_bytes, label) = match item.kind {
            WechatOutboundKind::Image => {
                let size = claw_core::channel_media_limits::validate_local_media_file(
                    &path,
                    "Telegram",
                    "image",
                    claw_core::channel_media_limits::telegram_file_max_bytes(),
                )?;
                if size <= claw_core::channel_media_limits::telegram_image_max_bytes() {
                    (
                        "sendPhoto",
                        "photo",
                        claw_core::channel_media_limits::telegram_image_max_bytes(),
                        "image",
                    )
                } else {
                    (
                        "sendDocument",
                        "document",
                        claw_core::channel_media_limits::telegram_file_max_bytes(),
                        "file",
                    )
                }
            }
            WechatOutboundKind::Video => (
                "sendVideo",
                "video",
                claw_core::channel_media_limits::telegram_file_max_bytes(),
                "video",
            ),
            WechatOutboundKind::Audio => (
                "sendAudio",
                "audio",
                claw_core::channel_media_limits::telegram_file_max_bytes(),
                "audio",
            ),
            WechatOutboundKind::File => (
                "sendDocument",
                "document",
                claw_core::channel_media_limits::telegram_file_max_bytes(),
                "file",
            ),
        };
        claw_core::channel_media_limits::validate_local_media_file(
            &path, "Telegram", label, max_bytes,
        )?;
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|err| format!("read Telegram outbound media failed: {err}"))?;
        let filename = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("file.bin")
            .to_string();
        let form = Form::new()
            .text("chat_id", chat_id.to_string())
            .part(field, Part::bytes(bytes).file_name(filename));
        let resp = state
            .core
            .http_client
            .post(format!("https://api.telegram.org/bot{token}/{method}"))
            .multipart(form)
            .send()
            .await
            .map_err(|error| provider_transport_error("telegram_bot", "send_media", &error))?;
        let status = resp.status();
        let response_body = resp
            .text()
            .await
            .map_err(|error| provider_transport_error("telegram_bot", "send_media", &error))?;
        if !status.is_success() {
            return Err(provider_http_error(
                "telegram_bot",
                "send_media",
                status,
                &response_body,
            ));
        }
        let provider_message_id = telegram_message_id("send_media", &response_body)?;
        record_provider_message_id(&provider_message_id);
        provider_message_ids.push(provider_message_id);
    }
    Ok(ChannelSendOutcome {
        provider_message_ids,
    })
}

fn extract_telegram_url_buttons(text: &str) -> (String, Vec<(String, String)>) {
    let mut kept_lines = Vec::new();
    let mut buttons = Vec::new();
    let mut seen_labels = HashMap::<String, usize>::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(body) = trimmed.strip_prefix("BUTTON:").map(str::trim) else {
            kept_lines.push(line.to_string());
            continue;
        };
        let Some((label, url)) = body
            .split_once('：')
            .or_else(|| body.split_once(':'))
            .map(|(label, url)| (label.trim(), url.trim()))
        else {
            kept_lines.push(line.to_string());
            continue;
        };
        if label.is_empty() || !(url.starts_with("https://") || url.starts_with("http://")) {
            kept_lines.push(line.to_string());
            continue;
        }
        let count = seen_labels.entry(label.to_string()).or_default();
        *count += 1;
        let unique_label = if *count == 1 {
            label.to_string()
        } else {
            format!("{label} {count}")
        };
        buttons.push((unique_label, url.to_string()));
    }
    (kept_lines.join("\n").trim().to_string(), buttons)
}

fn telegram_message_id(operation: &str, response_body: &str) -> Result<String, String> {
    let value = serde_json::from_str::<serde_json::Value>(response_body)
        .map_err(|_| provider_invalid_response("telegram_bot", operation, response_body))?;
    if value.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(provider_invalid_response(
            "telegram_bot",
            operation,
            response_body,
        ));
    }
    let message_id = value
        .pointer("/result/message_id")
        .and_then(serde_json::Value::as_i64)
        .filter(|message_id| *message_id > 0)
        .ok_or_else(|| provider_invalid_response("telegram_bot", operation, response_body))?;
    Ok(message_id.to_string())
}

pub(crate) async fn send_whatsapp_cloud_text_message(
    state: &AppState,
    to: &str,
    text: &str,
    conversation_window: &ChannelConversationWindow,
) -> Result<ChannelSendOutcome, String> {
    let token = state.channels.whatsapp_access_token.trim();
    if token.is_empty() {
        return Err("whatsapp access_token is empty".to_string());
    }
    let phone_number_id = state.channels.whatsapp_phone_number_id.trim();
    if phone_number_id.is_empty() {
        return Err("whatsapp phone_number_id is empty".to_string());
    }
    let base = state
        .channels
        .whatsapp_api_base
        .trim()
        .trim_end_matches('/');
    if base.is_empty() {
        return Err("whatsapp api_base is empty".to_string());
    }
    if conversation_window.state != ChannelConversationWindowState::Open {
        return send_whatsapp_cloud_template_message(state, to).await;
    }
    let media = extract_wechat_outbound_media(text, &state.skill_rt.workspace_root);
    let stripped = strip_wechat_delivery_lines(text);
    let send_text = if stripped.trim().is_empty() && media.is_empty() && !text.trim().is_empty() {
        text
    } else {
        stripped.as_str()
    };
    let url = format!("{base}/v23.0/{phone_number_id}/messages");
    let chunks = chunk_text_for_channel(
        send_text,
        WHATSAPP_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    let n = chunks.len();
    let mut provider_message_ids = Vec::new();
    if n > 1 {
        info!(
            "send_chunks channel=whatsapp_cloud to={} original_len={} chunk_count={}",
            to,
            send_text.len(),
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
                "send_chunk channel=whatsapp_cloud to={} index={} total={}",
                to,
                i + 1,
                n
            );
        }
        let resp = state
            .core
            .http_client
            .post(&url)
            .bearer_auth(token)
            .json(&json!({
                "messaging_product": "whatsapp",
                "to": to,
                "type": "text",
                "text": {
                    "body": body
                }
            }))
            .send()
            .await
            .map_err(|error| provider_transport_error("whatsapp_cloud", "send_text", &error))?;
        let status = resp.status();
        let response_body = resp
            .text()
            .await
            .map_err(|error| provider_transport_error("whatsapp_cloud", "send_text", &error))?;
        if !status.is_success() {
            return Err(provider_http_error(
                "whatsapp_cloud",
                "send_text",
                status,
                &response_body,
            ));
        }
        let message_ids =
            claw_core::channel_whatsapp_cloud::decode_message_ids("send_text", &response_body)
                .map_err(|error| error.to_string())?;
        record_provider_message_ids(&message_ids);
        provider_message_ids.extend(message_ids);
    }
    for item in &media {
        let path = materialize_channel_outbound_media(state, item, "whatsapp-cloud").await?;
        let (kind, message_type) = match item.kind {
            WechatOutboundKind::Image => (
                claw_core::channel_media_limits::WhatsappCloudMediaKind::Image,
                "image",
            ),
            WechatOutboundKind::Video => (
                claw_core::channel_media_limits::WhatsappCloudMediaKind::Video,
                "video",
            ),
            WechatOutboundKind::Audio => (
                claw_core::channel_media_limits::WhatsappCloudMediaKind::Audio,
                "audio",
            ),
            WechatOutboundKind::File => (
                claw_core::channel_media_limits::WhatsappCloudMediaKind::Document,
                "document",
            ),
        };
        let prepared = claw_core::channel_media_limits::prepare_whatsapp_cloud_media(
            &path,
            kind,
            Path::new(CHANNEL_MEDIA_OUTBOUND_TEMP_DIR)
                .join("whatsapp-compatible")
                .as_path(),
        )
        .await?;
        let filename = prepared
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("file.bin")
            .to_string();
        let bytes_result = tokio::fs::read(&prepared.path).await;
        if prepared.compatible_copy_created {
            let _ = tokio::fs::remove_file(&prepared.path).await;
        }
        let bytes =
            bytes_result.map_err(|err| format!("whatsapp_cloud_media_read_failed:{err}"))?;
        let part = Part::bytes(bytes)
            .file_name(filename.clone())
            .mime_str(prepared.mime_type)
            .map_err(|err| format!("prepare WhatsApp Cloud media failed: {err}"))?;
        let form = Form::new()
            .text("messaging_product", "whatsapp")
            .part("file", part);
        let upload_resp = state
            .core
            .http_client
            .post(format!("{base}/v23.0/{phone_number_id}/media"))
            .bearer_auth(token)
            .multipart(form)
            .send()
            .await
            .map_err(|error| provider_transport_error("whatsapp_cloud", "upload_media", &error))?;
        if !upload_resp.status().is_success() {
            let status = upload_resp.status();
            let body = upload_resp.text().await.unwrap_or_default();
            return Err(provider_http_error(
                "whatsapp_cloud",
                "upload_media",
                status,
                &body,
            ));
        }
        let upload_body: serde_json::Value = upload_resp.json().await.map_err(|error| {
            provider_invalid_response("whatsapp_cloud", "upload_media", &error.to_string())
        })?;
        let media_id = upload_body
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                provider_invalid_response("whatsapp_cloud", "upload_media", "missing_id")
            })?;
        let mut body = json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": message_type,
        });
        let mut media_body = json!({ "id": media_id });
        if message_type == "document" {
            media_body["filename"] = json!(filename);
        }
        body[message_type] = media_body;
        let send_resp = state
            .core
            .http_client
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|error| provider_transport_error("whatsapp_cloud", "send_media", &error))?;
        let status = send_resp.status();
        let response_body = send_resp
            .text()
            .await
            .map_err(|error| provider_transport_error("whatsapp_cloud", "send_media", &error))?;
        if !status.is_success() {
            return Err(provider_http_error(
                "whatsapp_cloud",
                "send_media",
                status,
                &response_body,
            ));
        }
        let message_ids =
            claw_core::channel_whatsapp_cloud::decode_message_ids("send_media", &response_body)
                .map_err(|error| error.to_string())?;
        record_provider_message_ids(&message_ids);
        provider_message_ids.extend(message_ids);
    }
    Ok(ChannelSendOutcome {
        provider_message_ids,
    })
}

async fn send_whatsapp_cloud_template_message(
    state: &AppState,
    to: &str,
) -> Result<ChannelSendOutcome, String> {
    let policy = claw_core::channel_whatsapp_cloud::WhatsappTemplatePolicy::from_config(
        &state.channels.whatsapp_out_of_window_template_name,
        &state.channels.whatsapp_out_of_window_template_language,
    )
    .ok_or_else(|| {
        ChannelProviderError::from_machine_failure(
            "whatsapp_cloud",
            "send_template",
            claw_core::channel_provider_error::ChannelProviderFailureClass::PayloadRejected,
            None,
            Some("conversation_window_closed"),
            None,
            "out_of_window_template_not_configured",
        )
        .to_string()
    })?;
    let token = state.channels.whatsapp_access_token.trim();
    let phone_number_id = state.channels.whatsapp_phone_number_id.trim();
    let base = state
        .channels
        .whatsapp_api_base
        .trim()
        .trim_end_matches('/');
    let response = state
        .core
        .http_client
        .post(format!("{base}/v23.0/{phone_number_id}/messages"))
        .bearer_auth(token)
        .json(&json!({
            "messaging_product": "whatsapp",
            "to": to,
            "type": "template",
            "template": {
                "name": policy.name,
                "language": {"code": policy.language, "policy": "deterministic"}
            }
        }))
        .send()
        .await
        .map_err(|error| provider_transport_error("whatsapp_cloud", "send_template", &error))?;
    let status = response.status();
    let response_body = response
        .text()
        .await
        .map_err(|error| provider_transport_error("whatsapp_cloud", "send_template", &error))?;
    if !status.is_success() {
        return Err(provider_http_error(
            "whatsapp_cloud",
            "send_template",
            status,
            &response_body,
        ));
    }
    let provider_message_ids =
        claw_core::channel_whatsapp_cloud::decode_message_ids("send_template", &response_body)
            .map_err(|error| error.to_string())?;
    record_provider_message_ids(&provider_message_ids);
    Ok(ChannelSendOutcome {
        provider_message_ids,
    })
}

#[cfg(test)]
#[path = "channel_send_tests.rs"]
mod tests;

pub(crate) async fn send_whatsapp_web_bridge_text_message(
    state: &AppState,
    to: &str,
    text: &str,
    delivery_source: ChannelDeliverySource,
) -> Result<ChannelSendOutcome, String> {
    if matches!(
        delivery_source,
        ChannelDeliverySource::ScheduledTask | ChannelDeliverySource::ProactiveNotice
    ) && !state.channels.whatsapp_web_allow_proactive_send
    {
        return Err(ChannelProviderError::from_machine_failure(
            "whatsapp_web",
            "send_text",
            claw_core::channel_provider_error::ChannelProviderFailureClass::PermissionDenied,
            Some(403),
            Some("proactive_send_disabled"),
            None,
            "local_policy:proactive_send_disabled",
        )
        .to_string());
    }
    let base = state
        .channels
        .whatsapp_web_bridge_base_url
        .trim()
        .trim_end_matches('/');
    if base.is_empty() {
        return Err("whatsapp_web.bridge_base_url is empty".to_string());
    }
    if !extract_wechat_outbound_media(text, &state.skill_rt.workspace_root).is_empty() {
        return send_whatsapp_web_bridge_result(state, base, to, text, delivery_source).await;
    }
    let url = format!("{base}/v1/send-text");
    let chunks = chunk_text_for_channel(
        text,
        WHATSAPP_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    let n = chunks.len();
    let mut provider_message_ids = Vec::new();
    if n > 1 {
        info!(
            "send_chunks channel=whatsapp_web_bridge to={} original_len={} chunk_count={}",
            to,
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
                "send_chunk channel=whatsapp_web_bridge to={} index={} total={}",
                to,
                i + 1,
                n
            );
        }
        let resp = state
            .core
            .http_client
            .post(&url)
            .json(&json!({
                "to": to,
                "text": body,
                "delivery_source": delivery_source
            }))
            .send()
            .await
            .map_err(|error| provider_transport_error("whatsapp_web", "send_text", &error))?;
        let status = resp.status();
        let response_body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(provider_http_error(
                "whatsapp_web",
                "send_text",
                status,
                &response_body,
            ));
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response_body) {
            let message_ids = value
                .get("message_ids")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            record_provider_message_ids(&message_ids);
            provider_message_ids.extend(message_ids);
        }
    }
    Ok(ChannelSendOutcome {
        provider_message_ids,
    })
}

async fn send_whatsapp_web_bridge_result(
    state: &AppState,
    base: &str,
    to: &str,
    text: &str,
    delivery_source: ChannelDeliverySource,
) -> Result<ChannelSendOutcome, String> {
    let media = extract_wechat_outbound_media(text, &state.skill_rt.workspace_root);
    let text_without_media = strip_wechat_delivery_lines(text).trim().to_string();
    let mut structured_media = Vec::with_capacity(media.len());
    for item in &media {
        let path = materialize_channel_outbound_media(state, item, "whatsapp-web").await?;
        let kind = match item.kind {
            WechatOutboundKind::Image => "image",
            WechatOutboundKind::Video => "video",
            WechatOutboundKind::Audio => "audio",
            WechatOutboundKind::File => "file",
        };
        structured_media.push(json!({
            "kind": kind,
            "path": path,
        }));
    }
    let response = state
        .core
        .http_client
        .post(format!("{base}/v1/send-result"))
        .json(&json!({
            "schema_version": 1,
            "to": to,
            "text": text_without_media,
            "media": structured_media,
            "delivery_source": delivery_source
        }))
        .send()
        .await
        .map_err(|error| provider_transport_error("whatsapp_web", "send_result", &error))?;
    let status = response.status();
    let response_body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(provider_http_error(
            "whatsapp_web",
            "send_result",
            status,
            &response_body,
        ));
    }
    let provider_message_ids = serde_json::from_str::<serde_json::Value>(&response_body)
        .ok()
        .and_then(|value| value.get("message_ids").cloned())
        .and_then(|value| value.as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    record_provider_message_ids(&provider_message_ids);
    Ok(ChannelSendOutcome {
        provider_message_ids,
    })
}

/// Max characters per Feishu/Lark text message (conservative; platform limit ~4096).
const FEISHU_LARK_TEXT_CHUNK_CHARS: usize = 3500;
const WECHAT_TEXT_CHUNK_CHARS: usize = 1800;

fn record_wechat_part_result<T>(
    first_error: &mut Option<String>,
    result: Result<T, String>,
    part_kind: &str,
    part_index: usize,
) -> bool {
    match result {
        Ok(_) => true,
        Err(error) => {
            warn!(
                channel = "wechat",
                part_kind,
                part_index,
                error = %error,
                "wechat_delivery_part_failed_continuing"
            );
            if first_error.is_none() {
                *first_error = Some(error);
            }
            false
        }
    }
}

pub(crate) async fn send_wechat_text_message(
    state: &AppState,
    to_user_id: &str,
    context_token: Option<&str>,
    text: &str,
    delivery_id: &str,
) -> Result<(), String> {
    let context_token = context_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "wechat send requires context_token".to_string())?;
    let config = resolve_wechat_send_config(state).ok_or_else(|| {
        "wechat send not configured (configs/channels/wechat.toml api_base_url/bot_token)"
            .to_string()
    })?;
    let base = config.api_base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("wechat api_base_url is empty".to_string());
    }
    let token = config.bot_token.trim();
    if token.is_empty() {
        return Err("wechat bot_token is empty".to_string());
    }
    let cdn = config.cdn_base_url.trim();
    if cdn.is_empty() {
        return Err("wechat cdn_base_url is empty".to_string());
    }
    let auth = IlinkAuth {
        sk_route_tag: config.sk_route_tag.as_deref().unwrap_or(""),
        wechat_uin_base64: config.wechat_uin_base64.as_deref().unwrap_or(""),
    };
    let media = extract_wechat_outbound_media(text, &state.skill_rt.workspace_root);
    let stripped = strip_wechat_delivery_lines(text);
    let send_text = if stripped.trim().is_empty() && media.is_empty() && !text.trim().is_empty() {
        text
    } else {
        stripped.as_str()
    };
    let max_chunk = config
        .text_chunk_chars
        .max(1)
        .min(WECHAT_TEXT_CHUNK_CHARS)
        .saturating_sub(SEGMENT_PREFIX_MAX_CHARS);
    let chunks = chunk_text_for_channel(send_text, max_chunk);
    let n = chunks.len();
    if n > 1 {
        info!(
            "send_chunks channel=wechat to_user_id={} original_len={} chunk_count={}",
            to_user_id,
            send_text.len(),
            n
        );
    }
    let mut first_error = None;
    for (i, chunk) in chunks.into_iter().enumerate() {
        let body = if n > 1 {
            format!("（{}/{}）\n{}", i + 1, n, chunk)
        } else {
            chunk
        };
        let client_id = wechat_delivery_client_id(delivery_id, "text", i);
        let request = match WechatMessageItem::text(body).and_then(|item| {
            WechatSendMessageRequest::finish(
                to_user_id,
                context_token,
                client_id.clone(),
                None,
                item,
                CLAWD_WECHAT_CHANNEL_VERSION,
            )
        }) {
            Ok(request) => request,
            Err(error) => {
                record_wechat_part_result(&mut first_error, Err::<(), _>(error), "text", i);
                continue;
            }
        };
        let result = wechat_ilink::post_ilink_json(
            &state.core.http_client,
            base,
            token,
            auth,
            "ilink/bot/sendmessage",
            &request,
            30_000,
        )
        .await;
        if record_wechat_part_result(&mut first_error, result, "text", i) {
            record_provider_message_id(&client_id);
        }
    }
    let timeout_ms: u64 = 30_000;
    for (media_index, media) in media.into_iter().enumerate() {
        let part_kind = match media.kind {
            WechatOutboundKind::Image => "image",
            WechatOutboundKind::Video => "video",
            WechatOutboundKind::Audio => "audio",
            WechatOutboundKind::File => "file",
        };
        let file_path = match materialize_wechat_outbound_media(state, &media).await {
            Ok(path) => path,
            Err(error) => {
                record_wechat_part_result(
                    &mut first_error,
                    Err::<(), _>(error),
                    part_kind,
                    media_index,
                );
                continue;
            }
        };
        let client_id = wechat_delivery_client_id(delivery_id, part_kind, media_index);
        let result = match media.kind {
            WechatOutboundKind::Image => {
                send_weixin_image_from_file_with_client_id(
                    &state.core.http_client,
                    base,
                    token,
                    auth,
                    cdn,
                    to_user_id,
                    Some(context_token),
                    None,
                    Some(&client_id),
                    &file_path,
                    CLAWD_WECHAT_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await
            }
            WechatOutboundKind::Video => {
                send_weixin_video_from_file_with_client_id(
                    &state.core.http_client,
                    base,
                    token,
                    auth,
                    cdn,
                    to_user_id,
                    Some(context_token),
                    None,
                    Some(&client_id),
                    &file_path,
                    CLAWD_WECHAT_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await
            }
            WechatOutboundKind::Audio | WechatOutboundKind::File => {
                let fname = file_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file");
                send_weixin_file_from_file_with_client_id(
                    &state.core.http_client,
                    base,
                    token,
                    auth,
                    cdn,
                    to_user_id,
                    Some(context_token),
                    None,
                    Some(&client_id),
                    &file_path,
                    fname,
                    CLAWD_WECHAT_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await
            }
        };
        if record_wechat_part_result(&mut first_error, result, part_kind, media_index) {
            record_provider_message_id(&client_id);
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub(crate) fn resolve_wechat_send_config(state: &AppState) -> Option<WechatSendConfig> {
    let fallback = state.channels.wechat_send_config.clone();
    let loaded = load_wechat_send_config_from_workspace(&state.skill_rt.workspace_root);
    match (loaded, fallback) {
        (Some(loaded), Some(mut fallback)) => {
            if !loaded.api_base_url.trim().is_empty() {
                fallback.api_base_url = loaded.api_base_url;
            }
            if !loaded.bot_token.trim().is_empty() {
                fallback.bot_token = loaded.bot_token;
            }
            if loaded.wechat_uin_base64.is_some() {
                fallback.wechat_uin_base64 = loaded.wechat_uin_base64;
            }
            if loaded.sk_route_tag.is_some() {
                fallback.sk_route_tag = loaded.sk_route_tag;
            }
            if !loaded.cdn_base_url.trim().is_empty() {
                fallback.cdn_base_url = loaded.cdn_base_url;
            }
            fallback.text_chunk_chars = loaded.text_chunk_chars;
            Some(fallback)
        }
        (Some(loaded), None) => Some(loaded),
        (None, Some(fallback)) => Some(fallback),
        (None, None) => None,
    }
}

fn load_wechat_send_config_from_workspace(workspace_root: &Path) -> Option<WechatSendConfig> {
    let path = workspace_root.join("configs/channels/wechat.toml");
    let content = std::fs::read_to_string(&path).ok()?;
    let table: TomlValue = toml::from_str(&content).ok()?;
    let wechat = table.get("wechat")?.as_table()?;
    let enabled = wechat
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !enabled {
        return None;
    }
    let session = load_wechat_session(workspace_root);
    let api_base_url = wechat
        .get("api_base_url")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            session
                .as_ref()
                .and_then(|session| session.base_url.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })?;
    let configured_token = wechat
        .get("bot_token")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let bot_token = if configured_token.is_empty() || configured_token == "REPLACE_ME" {
        session
            .as_ref()
            .map(|session| session.bot_token.trim().to_string())
            .unwrap_or_default()
    } else {
        configured_token
    };
    if bot_token.is_empty() {
        return None;
    }
    let wechat_uin_base64 = wechat
        .get("wechat_uin_base64")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let text_chunk_chars = wechat
        .get("text_chunk_chars")
        .and_then(|v| v.as_integer())
        .map(|v| v.max(1) as usize)
        .unwrap_or(WECHAT_TEXT_CHUNK_CHARS);
    let sk_route_tag = wechat
        .get("sk_route_tag")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let cdn_base_url = wechat
        .get("cdn_base_url")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(default_wechat_cdn_base_url);
    Some(WechatSendConfig {
        api_base_url,
        bot_token,
        wechat_uin_base64,
        text_chunk_chars,
        sk_route_tag,
        cdn_base_url,
    })
}

fn load_wechat_session(workspace_root: &Path) -> Option<PersistedWechatSession> {
    let path = workspace_root.join("data/wechatd/session.json");
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

async fn get_tenant_access_token(
    client: &reqwest::Client,
    source_adapter: &str,
    api_base: &str,
    app_id: &str,
    app_secret: &str,
) -> Result<String, String> {
    let region = [OpenPlatformRegion::Feishu, OpenPlatformRegion::Lark]
        .into_iter()
        .find(|region| open_platform_contract(*region).source_adapter == source_adapter)
        .ok_or_else(|| "channel_open_platform_adapter_invalid".to_string())?;
    let base = api_base.trim_end_matches('/');
    let cache = process_open_platform_token_cache(region, base, app_id);
    let now_secs = std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    cache
        .token_or_refresh(now_secs, || async {
            let url = format!("{base}/open-apis/auth/v3/tenant_access_token/internal");
            let body = json!({ "app_id": app_id, "app_secret": app_secret });
            let resp = client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|error| provider_transport_error(source_adapter, "auth_token", &error))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(provider_http_error(
                    source_adapter,
                    "auth_token",
                    status,
                    &text,
                ));
            }
            #[derive(serde::Deserialize)]
            struct TokenResp {
                tenant_access_token: Option<String>,
                expire: Option<u64>,
            }
            let data: TokenResp = resp.json().await.map_err(|error| {
                provider_invalid_response(source_adapter, "auth_token", &error.to_string())
            })?;
            let token = data.tenant_access_token.ok_or_else(|| {
                provider_invalid_response(source_adapter, "auth_token", "missing_token")
            })?;
            Ok((token, data.expire.unwrap_or(7200)))
        })
        .await
}

async fn upload_open_platform_media(
    client: &reqwest::Client,
    source_adapter: &str,
    base: &str,
    token: &str,
    path: &Path,
    plan: OpenPlatformMediaPlan,
) -> Result<String, String> {
    let bytes = tokio::fs::read(path).await.map_err(|error| {
        provider_invalid_response(source_adapter, "upload_media", &error.to_string())
    })?;
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file.bin")
        .to_string();
    let part = Part::bytes(bytes).file_name(filename.clone());
    let (upload_url, form) = match plan.upload_endpoint {
        OpenPlatformUploadEndpoint::Image => (
            format!("{base}/open-apis/im/v1/images"),
            Form::new()
                .text("image_type", plan.form_file_type.to_string())
                .part("image", part),
        ),
        OpenPlatformUploadEndpoint::File => (
            format!("{base}/open-apis/im/v1/files"),
            Form::new()
                .text("file_type", plan.form_file_type.to_string())
                .text("file_name", filename)
                .part("file", part),
        ),
    };
    let upload_resp = client
        .post(upload_url)
        .header("Authorization", format!("Bearer {token}"))
        .multipart(form)
        .send()
        .await
        .map_err(|error| provider_transport_error(source_adapter, "upload_media", &error))?;
    let status = upload_resp.status().as_u16();
    let response_body = upload_resp.text().await.unwrap_or_default();
    let region = [OpenPlatformRegion::Feishu, OpenPlatformRegion::Lark]
        .into_iter()
        .find(|region| open_platform_contract(*region).source_adapter == source_adapter)
        .ok_or_else(|| "channel_open_platform_adapter_invalid".to_string())?;
    let upload_body = claw_core::channel_open_platform::decode_open_platform_response(
        region,
        "upload_media",
        status,
        &response_body,
    )
    .map_err(|error| error.to_string())?;
    let pointer = format!("/data/{}", plan.key_name);
    upload_body
        .pointer(&pointer)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            provider_invalid_response(source_adapter, "upload_media", "missing_media_key")
        })
}

pub(crate) async fn send_feishu_text_message(
    state: &AppState,
    receive_id: &str,
    text: &str,
) -> Result<ChannelSendOutcome, String> {
    let config = state.channels.feishu_send_config.as_ref().ok_or_else(|| {
        "feishu send not configured (configs/channels/feishu.toml app_id/app_secret)".to_string()
    })?;
    send_feishu_lark_answer(
        state,
        "feishu",
        claw_core::channel_capabilities::ChannelAdapterKind::FeishuOpenPlatform,
        &config.api_base_url,
        &config.app_id,
        &config.app_secret,
        receive_id,
        text,
    )
    .await
}

pub(crate) async fn send_lark_text_message(
    state: &AppState,
    receive_id: &str,
    text: &str,
) -> Result<ChannelSendOutcome, String> {
    let config = state.channels.lark_send_config.as_ref().ok_or_else(|| {
        "lark send not configured (configs/channels/lark.toml app_id/app_secret)".to_string()
    })?;
    send_feishu_lark_answer(
        state,
        "lark",
        claw_core::channel_capabilities::ChannelAdapterKind::LarkOpenPlatform,
        &config.api_base_url,
        &config.app_id,
        &config.app_secret,
        receive_id,
        text,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn send_feishu_lark_answer(
    state: &AppState,
    channel_tag: &str,
    capability_adapter: claw_core::channel_capabilities::ChannelAdapterKind,
    api_base_url: &str,
    app_id: &str,
    app_secret: &str,
    receive_id: &str,
    answer: &str,
) -> Result<ChannelSendOutcome, String> {
    let image_max_bytes = claw_core::channel_media_limits::required_channel_media_max_bytes(
        capability_adapter,
        claw_core::channel_capabilities::ChannelCapabilityKind::SendImage,
    );
    let file_max_bytes = claw_core::channel_media_limits::required_channel_media_max_bytes(
        capability_adapter,
        claw_core::channel_capabilities::ChannelCapabilityKind::SendFile,
    );
    let region = match capability_adapter {
        claw_core::channel_capabilities::ChannelAdapterKind::FeishuOpenPlatform => {
            OpenPlatformRegion::Feishu
        }
        claw_core::channel_capabilities::ChannelAdapterKind::LarkOpenPlatform => {
            OpenPlatformRegion::Lark
        }
        _ => return Err("channel_open_platform_adapter_invalid".to_string()),
    };
    let source_adapter = open_platform_contract(region).source_adapter;
    let mut outcome = ChannelSendOutcome::default();
    let token = get_tenant_access_token(
        &state.core.http_client,
        source_adapter,
        api_base_url,
        app_id,
        app_secret,
    )
    .await?;
    let base = api_base_url.trim_end_matches('/');
    let message_url = format!("{base}/open-apis/im/v1/messages?receive_id_type=chat_id");
    let media = extract_wechat_outbound_media(answer, &state.skill_rt.workspace_root);
    let stripped = strip_wechat_delivery_lines(answer);
    let send_text = if stripped.trim().is_empty() && media.is_empty() && !answer.trim().is_empty() {
        answer
    } else {
        stripped.as_str()
    };
    let chunks = chunk_open_platform_text(
        send_text,
        FEISHU_LARK_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    )
    .map_err(|error| provider_content_error(source_adapter, "send_text", error))?;
    let n = chunks.len();
    for (index, chunk) in chunks.into_iter().enumerate() {
        let body = if n > 1 {
            format!("（{}/{}）\n{}", index + 1, n, chunk)
        } else {
            chunk
        };
        let content = json!({ "text": body }).to_string();
        validate_open_platform_content(OpenPlatformMessageType::Text, &content)
            .map_err(|error| provider_content_error(source_adapter, "send_text", error))?;
        process_open_platform_rate_limiter()
            .acquire(region, receive_id)
            .await;
        let resp = state
            .core
            .http_client
            .post(&message_url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&json!({
                "receive_id": receive_id,
                "msg_type": "text",
                "content": content
            }))
            .send()
            .await
            .map_err(|error| provider_transport_error(source_adapter, "send_text", &error))?;
        let status = resp.status().as_u16();
        let response_body = resp.text().await.unwrap_or_default();
        let message_id = claw_core::channel_open_platform::open_platform_message_id(
            region,
            "send_text",
            status,
            &response_body,
        )
        .map_err(|error| error.to_string())?;
        record_provider_message_id(&message_id);
        outcome.provider_message_ids.push(message_id);
    }

    for item in &media {
        let path = materialize_channel_outbound_media(state, item, channel_tag).await?;
        let actual_bytes =
            preflight_open_platform_media(region, "send_media", &path, file_max_bytes)
                .map_err(|error| error.to_string())?;
        let media_kind = match item.kind {
            WechatOutboundKind::Image => OpenPlatformOutboundMediaKind::Image,
            WechatOutboundKind::Video => OpenPlatformOutboundMediaKind::Video,
            WechatOutboundKind::Audio => OpenPlatformOutboundMediaKind::Audio,
            WechatOutboundKind::File => OpenPlatformOutboundMediaKind::File,
        };
        let mut plan = plan_open_platform_media(region, media_kind, &path, actual_bytes);
        let media_key = match upload_open_platform_media(
            &state.core.http_client,
            source_adapter,
            base,
            &token,
            &path,
            plan,
        )
        .await
        {
            Ok(key) => key,
            Err(image_error) if plan.upload_endpoint == OpenPlatformUploadEndpoint::Image => {
                let diagnostic_id = ChannelProviderError::decode(&image_error)
                    .map(|error| error.diagnostic_id)
                    .unwrap_or_else(|| "invalid_provider_error".to_string());
                tracing::warn!(
                    "channel open platform image fallback adapter={} diagnostic_id={}",
                    source_adapter,
                    diagnostic_id
                );
                plan = plan_open_platform_media(
                    region,
                    OpenPlatformOutboundMediaKind::Image,
                    &path,
                    image_max_bytes + 1,
                );
                upload_open_platform_media(
                    &state.core.http_client,
                    source_adapter,
                    base,
                    &token,
                    &path,
                    plan,
                )
                .await?
            }
            Err(error) => return Err(error),
        };
        let content = match plan.key_name {
            "image_key" => json!({ "image_key": media_key }),
            "file_key" => json!({ "file_key": media_key }),
            _ => {
                return Err(provider_invalid_response(
                    source_adapter,
                    "send_media",
                    "unsupported_media_key",
                ))
            }
        }
        .to_string();
        let message_type = plan.message_type;
        validate_open_platform_content(message_type, &content)
            .map_err(|error| provider_content_error(source_adapter, "send_media", error))?;
        process_open_platform_rate_limiter()
            .acquire(region, receive_id)
            .await;
        let resp = state
            .core
            .http_client
            .post(&message_url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&json!({
                "receive_id": receive_id,
                "msg_type": message_type.as_str(),
                "content": content
            }))
            .send()
            .await
            .map_err(|error| provider_transport_error(source_adapter, "send_media", &error))?;
        let status = resp.status().as_u16();
        let response_body = resp.text().await.unwrap_or_default();
        let message_id = claw_core::channel_open_platform::open_platform_message_id(
            region,
            "send_media",
            status,
            &response_body,
        )
        .map_err(|error| error.to_string())?;
        record_provider_message_id(&message_id);
        outcome.provider_message_ids.push(message_id);
    }
    Ok(outcome)
}
