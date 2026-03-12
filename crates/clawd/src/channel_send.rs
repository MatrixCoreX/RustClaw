//! Channel text sending with safe chunking (Telegram, WhatsApp Cloud, WhatsApp Web Bridge).
//! Used when clawd delivers task results directly to a channel.

use serde_json::json;
use tracing::info;

use claw_core::channel_chunk::{chunk_text_for_channel, SEGMENT_PREFIX_MAX_CHARS};

use crate::AppState;

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
