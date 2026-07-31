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
    conversation_window: &claw_core::channel_delivery::ChannelConversationWindow,
) -> Result<crate::channel_send::ChannelSendOutcome, String> {
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
                        .map(|_| crate::channel_send::ChannelSendOutcome::default())
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
            let context_token = ingress_context_token_from_payload(payload);
            crate::channel_send::send_wechat_text_message(state, &to_user_id, context_token, text)
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whatsapp_delivery_keeps_web_and_cloud_adapters_distinct() {
        let mut state = crate::AppState::test_default_with_fixture_provider();
        state.channels.whatsapp_web_enabled = true;
        state.channels.whatsapp_cloud_enabled = true;

        assert_eq!(
            resolve_whatsapp_delivery_route(&state, &json!({"adapter": "whatsapp_web"})),
            crate::WhatsappDeliveryRoute::WebBridge
        );
        assert_eq!(
            resolve_whatsapp_delivery_route(&state, &json!({"adapter": "whatsapp_cloud"})),
            crate::WhatsappDeliveryRoute::Cloud
        );
    }

    #[test]
    fn whatsapp_delivery_falls_back_to_the_only_enabled_adapter() {
        let mut state = crate::AppState::test_default_with_fixture_provider();
        state.channels.whatsapp_web_enabled = true;
        state.channels.whatsapp_cloud_enabled = false;

        assert_eq!(
            resolve_whatsapp_delivery_route(&state, &json!({})),
            crate::WhatsappDeliveryRoute::WebBridge
        );
    }

    #[test]
    fn wechat_delivery_uses_raw_reply_target_not_scoped_conversation_id() {
        let payload = json!({
            "context_token": "stale-token",
            "external_chat_id": "wechat-scope-v1:opaque",
            "channel_ingress": {
                "context_token": "pinned-token",
                "reply_target": {"kind": "user", "external_id": "raw-peer"}
            }
        });

        assert_eq!(
            ingress_reply_target_from_payload(&payload).as_deref(),
            Some("raw-peer")
        );
        assert_eq!(
            ingress_context_token_from_payload(&payload),
            Some("pinned-token")
        );
    }
}
