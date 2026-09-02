use serde_json::Value;

use crate::AppState;
use claw_core::hard_rules::types::MainFlowRules;

fn external_chat_id_from_payload(payload: &Value) -> Option<String> {
    payload
        .get("external_chat_id")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn ingress_reply_target_from_payload(payload: &Value) -> Option<String> {
    payload
        .pointer("/channel_ingress/reply_target/external_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn ingress_context_token_from_payload(payload: &Value) -> Option<&str> {
    payload
        .pointer("/channel_ingress/context_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            payload
                .get("context_token")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
}

fn latest_wechat_inbound_context_token(
    state: &AppState,
    task: &crate::ClaimedTask,
    to_user_id: &str,
) -> Result<Option<String>, String> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| format!("db pool: {error}"))?;
    let principal_id = task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|user_key| crate::repo::auth::principal_id_for_user_key(&db, user_key))
        .transpose()
        .map_err(|error| format!("resolve wechat delivery principal: {error}"))?
        .flatten();
    let mut statement = db
        .prepare(
            "SELECT external_user_id, payload_json
             FROM tasks
             WHERE channel = 'wechat'
               AND kind = 'ask'
               AND (
                    (?1 IS NOT NULL AND principal_id = ?1)
                    OR (
                        (principal_id IS NULL OR TRIM(principal_id) = '')
                        AND user_id = ?2
                    )
               )
             ORDER BY CAST(created_at AS INTEGER) DESC, rowid DESC
             LIMIT 32",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            rusqlite::params![principal_id.as_deref(), task.user_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (external_user_id, payload_json) = row.map_err(|error| error.to_string())?;
        let Ok(payload) = serde_json::from_str::<Value>(&payload_json) else {
            continue;
        };
        if payload
            .get("schedule_triggered")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let reply_target = ingress_reply_target_from_payload(&payload);
        let row_user_id = external_user_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or(reply_target.as_deref());
        if row_user_id != Some(to_user_id) {
            continue;
        }
        if let Some(context_token) = ingress_context_token_from_payload(&payload) {
            return Ok(Some(context_token.to_string()));
        }
    }
    Ok(None)
}

pub(crate) fn runtime_channel_from_payload(
    state: &AppState,
    payload: &Value,
) -> crate::RuntimeChannel {
    let ch = payload
        .get("channel")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if is_whatsapp_channel_value(crate::main_flow_rules(state), &ch) {
        return crate::RuntimeChannel::Whatsapp;
    }
    if ch == "wechat" {
        return crate::RuntimeChannel::Wechat;
    }
    if ch == "feishu" {
        return crate::RuntimeChannel::Feishu;
    }
    if ch == "lark" {
        return crate::RuntimeChannel::Lark;
    }
    crate::RuntimeChannel::Telegram
}

fn is_whatsapp_channel_value(rules: &MainFlowRules, raw: &str) -> bool {
    let channel = raw.trim().to_ascii_lowercase();
    rules
        .runtime_whatsapp_channel_aliases
        .iter()
        .any(|v| v == &channel)
}

pub(crate) fn task_payload_value(task: &crate::ClaimedTask) -> Option<Value> {
    serde_json::from_str::<Value>(&task.payload_json).ok()
}

pub(crate) fn task_runtime_channel(
    state: &AppState,
    task: &crate::ClaimedTask,
) -> crate::RuntimeChannel {
    let ch = task.channel.trim().to_ascii_lowercase();
    if is_whatsapp_channel_value(crate::main_flow_rules(state), &ch) {
        return crate::RuntimeChannel::Whatsapp;
    }
    if ch == "wechat" {
        return crate::RuntimeChannel::Wechat;
    }
    if ch == "feishu" {
        return crate::RuntimeChannel::Feishu;
    }
    if ch == "lark" {
        return crate::RuntimeChannel::Lark;
    }
    let Some(payload) = task_payload_value(task) else {
        return crate::RuntimeChannel::Telegram;
    };
    runtime_channel_from_payload(state, &payload)
}

pub(crate) fn task_external_chat_id(task: &crate::ClaimedTask) -> Option<String> {
    if let Some(v) = task
        .external_chat_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(v);
    }
    let payload = task_payload_value(task)?;
    external_chat_id_from_payload(&payload)
}

fn resolve_whatsapp_delivery_route(
    state: &AppState,
    payload: &Value,
) -> crate::WhatsappDeliveryRoute {
    let rules = crate::main_flow_rules(state);
    let adapter = payload
        .get("adapter")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if rules.whatsapp_web_adapters.iter().any(|a| a == &adapter) {
        return crate::WhatsappDeliveryRoute::WebBridge;
    }
    if rules.whatsapp_cloud_adapters.iter().any(|a| a == &adapter) {
        return crate::WhatsappDeliveryRoute::Cloud;
    }
    if state.channels.whatsapp_web_enabled && !state.channels.whatsapp_cloud_enabled {
        return crate::WhatsappDeliveryRoute::WebBridge;
    }
    crate::WhatsappDeliveryRoute::Cloud
}

pub(crate) async fn send_task_channel_message(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    text: &str,
    delivery_id: &str,
    conversation_window: &claw_core::channel_delivery::ChannelConversationWindow,
    delivery_source: claw_core::channel_delivery::ChannelDeliverySource,
) -> Result<crate::channel_send::ChannelSendOutcome, String> {
    match runtime_channel_from_payload(state, payload) {
        crate::RuntimeChannel::Telegram => {
            let target_chat_id = task_external_chat_id(task)
                .or_else(|| external_chat_id_from_payload(payload))
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(task.chat_id);
            let bot_name = payload
                .get("telegram_bot_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            crate::channel_send::send_telegram_message_for_bot(
                state,
                bot_name,
                target_chat_id,
                text,
            )
            .await
        }
        crate::RuntimeChannel::Whatsapp => {
            let to = task_external_chat_id(task)
                .or_else(|| external_chat_id_from_payload(payload))
                .ok_or_else(|| "missing external_chat_id for whatsapp task".to_string())?;
            match resolve_whatsapp_delivery_route(state, payload) {
                crate::WhatsappDeliveryRoute::WebBridge => {
                    crate::channel_send::send_whatsapp_web_bridge_text_message(
                        state,
                        &to,
                        text,
                        delivery_source,
                    )
                    .await
                }
                crate::WhatsappDeliveryRoute::Cloud => {
                    crate::channel_send::send_whatsapp_cloud_text_message(
                        state,
                        &to,
                        text,
                        conversation_window,
                    )
                    .await
                }
            }
        }
        crate::RuntimeChannel::Wechat => {
            let to_user_id = ingress_reply_target_from_payload(payload)
                .or_else(|| task.external_user_id.clone())
                .or_else(|| task_external_chat_id(task))
                .or_else(|| external_chat_id_from_payload(payload))
                .ok_or_else(|| "missing external_chat_id for wechat task".to_string())?;
            let latest_context_token = if delivery_source
                == claw_core::channel_delivery::ChannelDeliverySource::ScheduledTask
            {
                latest_wechat_inbound_context_token(state, task, &to_user_id)?
            } else {
                None
            };
            let context_token = latest_context_token
                .as_deref()
                .or_else(|| ingress_context_token_from_payload(payload));
            crate::channel_send::send_wechat_text_message(
                state,
                &to_user_id,
                context_token,
                text,
                delivery_id,
            )
            .await
            .map(|_| crate::channel_send::ChannelSendOutcome::default())
        }
        crate::RuntimeChannel::Feishu => {
            let receive_id = task_external_chat_id(task)
                .or_else(|| external_chat_id_from_payload(payload))
                .ok_or_else(|| "missing external_chat_id for feishu task".to_string())?;
            crate::channel_send::send_feishu_text_message(state, &receive_id, text).await
        }
        crate::RuntimeChannel::Lark => {
            let receive_id = task_external_chat_id(task)
                .or_else(|| external_chat_id_from_payload(payload))
                .ok_or_else(|| "missing external_chat_id for lark task".to_string())?;
            crate::channel_send::send_lark_text_message(state, &receive_id, text).await
        }
    }
}

#[cfg(test)]
#[path = "channels_tests.rs"]
mod tests;
