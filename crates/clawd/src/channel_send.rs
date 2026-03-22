//! Channel text sending with safe chunking (Telegram, WhatsApp Cloud, WhatsApp Web Bridge, Feishu, Lark).
//! Used when clawd delivers task results directly to a channel (e.g. schedule_triggered notify).

use std::path::Path;

use serde::Deserialize;
use serde_json::json;
use toml::Value as TomlValue;
use tracing::info;

use claw_core::channel_chunk::{chunk_text_for_channel, SEGMENT_PREFIX_MAX_CHARS};
use claw_core::wechat_reply_media::{
    extract_wechat_outbound_media, strip_wechat_delivery_lines, WechatOutboundKind,
};

use crate::AppState;
use wechat_ilink::http::IlinkAuth;
use wechat_ilink::{
    send_weixin_file_from_file, send_weixin_image_from_file, send_weixin_video_from_file,
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

/// Max characters per WhatsApp text message (conservative; platform limit ~4096).
const WHATSAPP_TEXT_CHUNK_CHARS: usize = 3500;
const WECHAT_SEND_MESSAGE_TYPE: i64 = 2;
const WECHAT_SEND_MESSAGE_STATE: i64 = 2;
const CLAWD_WECHAT_CHANNEL_VERSION: &str = env!("CARGO_PKG_VERSION");

fn default_wechat_cdn_base_url() -> String {
    "https://novac2c.cdn.weixin.qq.com/c2c".to_string()
}

pub(crate) async fn send_telegram_message(
    state: &AppState,
    chat_id: i64,
    text: &str,
) -> Result<(), String> {
    let token = state.telegram_bot_token.trim();
    if token.is_empty() {
        return Err("telegram bot token is empty".to_string());
    }
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let chunks = chunk_text_for_channel(
        text,
        TELEGRAM_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    let n = chunks.len();
    if n > 1 {
        info!(
            "send_chunks channel=telegram chat_id={} original_len={} chunk_count={}",
            chat_id,
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
                "send_chunk channel=telegram chat_id={} index={} total={}",
                chat_id,
                i + 1,
                n
            );
        }
        let resp = state
            .http_client
            .post(&url)
            .json(&json!({
                "chat_id": chat_id,
                "text": body
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("status={status} body={body}"));
        }
    }
    Ok(())
}

pub(crate) async fn send_whatsapp_cloud_text_message(
    state: &AppState,
    to: &str,
    text: &str,
) -> Result<(), String> {
    let token = state.whatsapp_access_token.trim();
    if token.is_empty() {
        return Err("whatsapp access_token is empty".to_string());
    }
    let phone_number_id = state.whatsapp_phone_number_id.trim();
    if phone_number_id.is_empty() {
        return Err("whatsapp phone_number_id is empty".to_string());
    }
    let base = state.whatsapp_api_base.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("whatsapp api_base is empty".to_string());
    }
    let url = format!("{base}/v23.0/{phone_number_id}/messages");
    let chunks = chunk_text_for_channel(
        text,
        WHATSAPP_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    let n = chunks.len();
    if n > 1 {
        info!(
            "send_chunks channel=whatsapp_cloud to={} original_len={} chunk_count={}",
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
                "send_chunk channel=whatsapp_cloud to={} index={} total={}",
                to,
                i + 1,
                n
            );
        }
        let resp = state
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
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("status={status} body={body}"));
        }
    }
    Ok(())
}

pub(crate) async fn send_whatsapp_web_bridge_text_message(
    state: &AppState,
    to: &str,
    text: &str,
) -> Result<(), String> {
    let base = state
        .whatsapp_web_bridge_base_url
        .trim()
        .trim_end_matches('/');
    if base.is_empty() {
        return Err("whatsapp_web.bridge_base_url is empty".to_string());
    }
    let url = format!("{base}/v1/send-text");
    let chunks = chunk_text_for_channel(
        text,
        WHATSAPP_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    let n = chunks.len();
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
            .http_client
            .post(&url)
            .json(&json!({
                "to": to,
                "text": body
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("wa-web bridge status={status} body={body}"));
        }
    }
    Ok(())
}

/// Max characters per Feishu/Lark text message (conservative; platform limit ~4096).
const FEISHU_LARK_TEXT_CHUNK_CHARS: usize = 3500;
const WECHAT_TEXT_CHUNK_CHARS: usize = 1200;

pub(crate) async fn send_wechat_text_message(
    state: &AppState,
    to_user_id: &str,
    context_token: Option<&str>,
    text: &str,
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
    let media = extract_wechat_outbound_media(text, &state.workspace_root);
    let stripped = strip_wechat_delivery_lines(text);
    let send_text = if stripped.trim().is_empty() && media.is_empty() && !text.trim().is_empty() {
        text
    } else {
        stripped.as_str()
    };
    let url = format!("{base}/ilink/bot/sendmessage");
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
    for (i, chunk) in chunks.into_iter().enumerate() {
        let body = if n > 1 {
            format!("（{}/{}）\n{}", i + 1, n, chunk)
        } else {
            chunk
        };
        let mut req = state
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("AuthorizationType", "ilink_bot_token")
            .header("Authorization", format!("Bearer {token}"));
        if let Some(uin) = config.wechat_uin_base64.as_deref() {
            req = req.header("X-WECHAT-UIN", uin);
        }
        if let Some(tag) = config.sk_route_tag.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            req = req.header("SKRouteTag", tag);
        }
        let resp = req
            .json(&json!({
                "base_info": {
                    "channel_version": env!("CARGO_PKG_VERSION")
                },
                "msg": {
                    "from_user_id": "",
                    "to_user_id": to_user_id,
                    "client_id": format!("clawd-{}", i + 1),
                    "message_type": WECHAT_SEND_MESSAGE_TYPE,
                    "message_state": WECHAT_SEND_MESSAGE_STATE,
                    "item_list": [{
                        "type": 1,
                        "text_item": { "text": body }
                    }],
                    "context_token": context_token
                }
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("wechat send status={status} body={body}"));
        }
    }
    let timeout_ms: u64 = 30_000;
    for (p, kind) in media {
        match kind {
            WechatOutboundKind::Image => {
                send_weixin_image_from_file(
                    &state.http_client,
                    base,
                    token,
                    auth,
                    cdn,
                    to_user_id,
                    Some(context_token),
                    &p,
                    CLAWD_WECHAT_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await?
            }
            WechatOutboundKind::Video => {
                send_weixin_video_from_file(
                    &state.http_client,
                    base,
                    token,
                    auth,
                    cdn,
                    to_user_id,
                    Some(context_token),
                    &p,
                    CLAWD_WECHAT_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await?
            }
            WechatOutboundKind::File => {
                let fname = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file");
                send_weixin_file_from_file(
                    &state.http_client,
                    base,
                    token,
                    auth,
                    cdn,
                    to_user_id,
                    Some(context_token),
                    &p,
                    fname,
                    CLAWD_WECHAT_CHANNEL_VERSION,
                    timeout_ms,
                )
                .await?
            }
        }
    }
    Ok(())
}

fn resolve_wechat_send_config(state: &AppState) -> Option<WechatSendConfig> {
    let fallback = state.wechat_send_config.clone();
    let loaded = load_wechat_send_config_from_workspace(&state.workspace_root);
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
    api_base: &str,
    app_id: &str,
    app_secret: &str,
) -> Result<String, String> {
    let base = api_base.trim_end_matches('/');
    let url = format!("{base}/open-apis/auth/v3/tenant_access_token/internal");
    let body = json!({ "app_id": app_id, "app_secret": app_secret });
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("token request status={status} body={text}"));
    }
    #[derive(serde::Deserialize)]
    struct TokenResp {
        tenant_access_token: Option<String>,
    }
    let data: TokenResp = resp.json().await.map_err(|e| e.to_string())?;
    data.tenant_access_token
        .ok_or_else(|| "token response missing tenant_access_token".to_string())
}

pub(crate) async fn send_feishu_text_message(
    state: &AppState,
    receive_id: &str,
    text: &str,
) -> Result<(), String> {
    let config = state.feishu_send_config.as_ref().ok_or_else(|| {
        "feishu send not configured (configs/channels/feishu.toml app_id/app_secret)".to_string()
    })?;
    let token = get_tenant_access_token(
        &state.http_client,
        &config.api_base_url,
        &config.app_id,
        &config.app_secret,
    )
    .await?;
    let base = config.api_base_url.trim_end_matches('/');
    let url = format!("{base}/open-apis/im/v1/messages?receive_id_type=chat_id");
    let chunks = chunk_text_for_channel(
        text,
        FEISHU_LARK_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    let n = chunks.len();
    if n > 1 {
        info!(
            "send_chunks channel=feishu receive_id={} original_len={} chunk_count={}",
            receive_id,
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
        let resp = state
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "receive_id": receive_id,
                "msg_type": "text",
                "content": json!({ "text": body }).to_string()
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let resp_body = resp.text().await.unwrap_or_default();
            return Err(format!("feishu send status={status} body={resp_body}"));
        }
    }
    Ok(())
}

pub(crate) async fn send_lark_text_message(
    state: &AppState,
    receive_id: &str,
    text: &str,
) -> Result<(), String> {
    let config = state.lark_send_config.as_ref().ok_or_else(|| {
        "lark send not configured (configs/channels/lark.toml app_id/app_secret)".to_string()
    })?;
    let token = get_tenant_access_token(
        &state.http_client,
        &config.api_base_url,
        &config.app_id,
        &config.app_secret,
    )
    .await?;
    let base = config.api_base_url.trim_end_matches('/');
    let url = format!("{base}/open-apis/im/v1/messages?receive_id_type=chat_id");
    let chunks = chunk_text_for_channel(
        text,
        FEISHU_LARK_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS),
    );
    let n = chunks.len();
    if n > 1 {
        info!(
            "send_chunks channel=lark receive_id={} original_len={} chunk_count={}",
            receive_id,
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
        let resp = state
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "receive_id": receive_id,
                "msg_type": "text",
                "content": json!({ "text": body }).to_string()
            }))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let resp_body = resp.text().await.unwrap_or_default();
            return Err(format!("lark send status={status} body={resp_body}"));
        }
    }
    Ok(())
}
