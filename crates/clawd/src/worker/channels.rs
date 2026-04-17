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
) -> Result<(), String> {
    match runtime_channel_from_payload(state, payload) {
        crate::RuntimeChannel::Telegram => {
            let target_chat_id = task_external_chat_id(task)
                .or_else(|| external_chat_id_from_payload(payload))
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(task.chat_id);
            crate::channel_send::send_telegram_message(state, target_chat_id, text).await
        }
        crate::RuntimeChannel::Whatsapp => {
            let to = task_external_chat_id(task)
                .or_else(|| external_chat_id_from_payload(payload))
                .ok_or_else(|| "missing external_chat_id for whatsapp task".to_string())?;
            match resolve_whatsapp_delivery_route(state, payload) {
                crate::WhatsappDeliveryRoute::WebBridge => {
                    crate::channel_send::send_whatsapp_web_bridge_text_message(state, &to, text)
                        .await
                }
                crate::WhatsappDeliveryRoute::Cloud => {
                    crate::channel_send::send_whatsapp_cloud_text_message(state, &to, text).await
                }
            }
        }
        crate::RuntimeChannel::Wechat => {
            let to_user_id = task_external_chat_id(task)
                .or_else(|| external_chat_id_from_payload(payload))
                .ok_or_else(|| "missing external_chat_id for wechat task".to_string())?;
            let context_token = payload
                .get("context_token")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty());
            crate::channel_send::send_wechat_text_message(state, &to_user_id, context_token, text)
                .await
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
