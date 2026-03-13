//! Channel text sending with safe chunking (Telegram, WhatsApp Cloud, WhatsApp Web Bridge, Feishu, Lark).
//! Used when clawd delivers task results directly to a channel (e.g. schedule_triggered notify).

use serde_json::json;
use tracing::info;

use claw_core::channel_chunk::{chunk_text_for_channel, SEGMENT_PREFIX_MAX_CHARS};

use crate::AppState;

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

/// Max characters per Telegram message (conservative; platform limit ~4096).
const TELEGRAM_TEXT_CHUNK_CHARS: usize = 3500;

/// Max characters per WhatsApp text message (conservative; platform limit ~4096).
const WHATSAPP_TEXT_CHUNK_CHARS: usize = 3500;

pub(crate) async fn send_telegram_message(state: &AppState, chat_id: i64, text: &str) -> Result<(), String> {
    let token = state.telegram_bot_token.trim();
    if token.is_empty() {
        return Err("telegram bot token is empty".to_string());
    }
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let chunks = chunk_text_for_channel(text, TELEGRAM_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS));
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
            info!("send_chunk channel=telegram chat_id={} index={} total={}", chat_id, i + 1, n);
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
    let chunks = chunk_text_for_channel(text, WHATSAPP_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS));
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
            info!("send_chunk channel=whatsapp_cloud to={} index={} total={}", to, i + 1, n);
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
    let chunks = chunk_text_for_channel(text, WHATSAPP_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS));
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
            info!("send_chunk channel=whatsapp_web_bridge to={} index={} total={}", to, i + 1, n);
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
    let config = state
        .feishu_send_config
        .as_ref()
        .ok_or_else(|| "feishu send not configured (configs/channels/feishu.toml app_id/app_secret)".to_string())?;
    let token = get_tenant_access_token(
        &state.http_client,
        &config.api_base_url,
        &config.app_id,
        &config.app_secret,
    )
    .await?;
    let base = config.api_base_url.trim_end_matches('/');
    let url = format!("{base}/open-apis/im/v1/messages?receive_id_type=chat_id");
    let chunks = chunk_text_for_channel(text, FEISHU_LARK_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS));
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
    let config = state
        .lark_send_config
        .as_ref()
        .ok_or_else(|| "lark send not configured (configs/channels/lark.toml app_id/app_secret)".to_string())?;
    let token = get_tenant_access_token(
        &state.http_client,
        &config.api_base_url,
        &config.app_id,
        &config.app_secret,
    )
    .await?;
    let base = config.api_base_url.trim_end_matches('/');
    let url = format!("{base}/open-apis/im/v1/messages?receive_id_type=chat_id");
    let chunks = chunk_text_for_channel(text, FEISHU_LARK_TEXT_CHUNK_CHARS.saturating_sub(SEGMENT_PREFIX_MAX_CHARS));
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
