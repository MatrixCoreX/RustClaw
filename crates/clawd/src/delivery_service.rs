use anyhow::{anyhow, Context};
use claw_core::channel_delivery::{
    ChannelConversationWindow, ChannelConversationWindowState, ChannelDeliveryEnvelope,
    ChannelDeliveryReceipt, ChannelDeliverySource, ChannelDeliveryStatus, ChannelTextFormat,
    ChannelTextSegment, CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION, CHANNEL_DELIVERY_SCHEMA_VERSION,
};
use claw_core::channel_ingress::{
    default_adapter_for_channel, default_reply_target, ChannelReplyTarget,
};
use claw_core::types::ChannelKind;
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use crate::repo::ClaimChannelDeliveryDispatchOutcome;
use crate::{AppState, ClaimedTask, RuntimeChannel};

const DELIVERY_DISPATCH_LEASE_SECONDS: u64 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelDeliveryServiceStatus {
    Accepted,
    Delivered,
    Read,
    Failed,
    InProgress,
    QueryRequired,
}

#[derive(Debug, Clone)]
pub(crate) struct ChannelDeliveryServiceResult {
    pub(crate) status: ChannelDeliveryServiceStatus,
    pub(crate) receipt: Option<ChannelDeliveryReceipt>,
    pub(crate) error_text: Option<String>,
}

impl ChannelDeliveryServiceResult {
    pub(crate) fn accepted(&self) -> bool {
        matches!(
            self.status,
            ChannelDeliveryServiceStatus::Accepted
                | ChannelDeliveryServiceStatus::Delivered
                | ChannelDeliveryServiceStatus::Read
        )
    }

    pub(crate) fn delivered(&self) -> bool {
        matches!(
            self.status,
            ChannelDeliveryServiceStatus::Delivered | ChannelDeliveryServiceStatus::Read
        )
    }

    pub(crate) fn status_token(&self) -> &'static str {
        match self.status {
            ChannelDeliveryServiceStatus::Accepted => "accepted",
            ChannelDeliveryServiceStatus::Delivered => "delivered",
            ChannelDeliveryServiceStatus::Read => "read",
            ChannelDeliveryServiceStatus::Failed => "failed",
            ChannelDeliveryServiceStatus::InProgress => "in_progress",
            ChannelDeliveryServiceStatus::QueryRequired => "query_required",
        }
    }
}

pub(crate) fn build_scheduled_delivery_envelope(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    text: &str,
) -> anyhow::Result<ChannelDeliveryEnvelope> {
    let runtime_channel = crate::worker::runtime_channel_from_payload(state, payload);
    let channel = channel_kind(runtime_channel);
    let ingress = payload.get("channel_ingress");
    let external_user_id = ingress
        .and_then(|value| value.get("external_user_id"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("external_user_id").and_then(Value::as_str));
    let external_chat_id = ingress
        .and_then(|value| value.get("external_chat_id"))
        .and_then(Value::as_str)
        .or_else(|| task.external_chat_id.as_deref())
        .or_else(|| payload.get("external_chat_id").and_then(Value::as_str));
    let reply_target = ingress
        .and_then(|value| value.get("reply_target"))
        .cloned()
        .map(serde_json::from_value::<ChannelReplyTarget>)
        .transpose()
        .context("channel_delivery_reply_target_parse_failed")?
        .or_else(|| default_reply_target(channel, external_user_id, external_chat_id))
        .ok_or_else(|| anyhow!("channel_delivery_reply_target_missing"))?;
    let adapter = ingress
        .and_then(|value| value.get("adapter"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("adapter").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_adapter_for_channel(channel))
        .to_string();
    let locale = ingress
        .and_then(|value| value.get("locale"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("locale").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("und")
        .to_string();
    let context_token = ingress
        .and_then(|value| value.get("context_token"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("context_token").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let delivery_id = format!("delivery:{}:schedule-terminal", task.task_id);
    let idempotency_key = format!("{}:schedule-terminal", task.task_id);
    let envelope = ChannelDeliveryEnvelope {
        schema_version: CHANNEL_DELIVERY_SCHEMA_VERSION,
        delivery_id,
        task_id: Some(task.task_id.clone()),
        source: ChannelDeliverySource::ScheduledTask,
        channel,
        adapter,
        reply_target,
        locale,
        conversation_window: ChannelConversationWindow {
            state: if context_token.is_some() {
                ChannelConversationWindowState::Open
            } else {
                ChannelConversationWindowState::Unknown
            },
            expires_at_ts: None,
            context_token,
        },
        idempotency_key,
        text_segments: vec![ChannelTextSegment {
            text: text.to_string(),
            format: ChannelTextFormat::Plain,
        }],
        artifacts: Vec::new(),
        previews: Vec::new(),
        notice: None,
    };
    envelope
        .validate()
        .map_err(|err| anyhow!(err.to_string()))?;
    Ok(envelope)
}

pub(crate) async fn deliver_task_envelope(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    envelope: &ChannelDeliveryEnvelope,
) -> anyhow::Result<ChannelDeliveryServiceResult> {
    envelope
        .validate()
        .map_err(|err| anyhow!(err.to_string()))?;
    let claim = crate::repo::claim_channel_delivery_dispatch(
        &state.core.db,
        envelope,
        DELIVERY_DISPATCH_LEASE_SECONDS,
    )?;
    let lease_token = match claim {
        ClaimChannelDeliveryDispatchOutcome::Acquired { lease_token } => lease_token,
        ClaimChannelDeliveryDispatchOutcome::ExistingReceipt(receipt) => {
            return Ok(result_from_existing_receipt(receipt));
        }
        ClaimChannelDeliveryDispatchOutcome::InProgress => {
            return Ok(ChannelDeliveryServiceResult {
                status: ChannelDeliveryServiceStatus::InProgress,
                receipt: None,
                error_text: None,
            });
        }
        ClaimChannelDeliveryDispatchOutcome::QueryRequired => {
            return Ok(ChannelDeliveryServiceResult {
                status: ChannelDeliveryServiceStatus::QueryRequired,
                receipt: None,
                error_text: None,
            });
        }
    };

    let text = envelope
        .text_segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let send_result = crate::worker::send_task_channel_message(state, task, payload, &text).await;
    let now = crate::now_ts_u64();
    let (receipt, error_text) = match send_result {
        Ok(()) => (
            ChannelDeliveryReceipt {
                schema_version: CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION,
                delivery_id: envelope.delivery_id.clone(),
                idempotency_key: envelope.idempotency_key.clone(),
                channel: envelope.channel,
                adapter: envelope.adapter.clone(),
                status: ChannelDeliveryStatus::Accepted,
                provider_message_ids: Vec::new(),
                parts: Vec::new(),
                error_code: None,
                diagnostic_id: None,
                retryable: false,
                updated_at_ts: now,
            },
            None,
        ),
        Err(error_text) => (
            ChannelDeliveryReceipt {
                schema_version: CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION,
                delivery_id: envelope.delivery_id.clone(),
                idempotency_key: envelope.idempotency_key.clone(),
                channel: envelope.channel,
                adapter: envelope.adapter.clone(),
                status: ChannelDeliveryStatus::Failed,
                provider_message_ids: Vec::new(),
                parts: Vec::new(),
                error_code: Some("channel.send_failed".to_string()),
                diagnostic_id: Some(format!("delivery:{}", Uuid::new_v4().simple())),
                retryable: false,
                updated_at_ts: now,
            },
            Some(error_text),
        ),
    };
    crate::repo::record_channel_delivery_receipt(&state.core.db, &receipt)?;
    if let Err(err) = crate::repo::complete_channel_delivery_dispatch(
        &state.core.db,
        &envelope.idempotency_key,
        &lease_token,
    ) {
        warn!(
            "channel delivery dispatch completion failed delivery_id={} diagnostic={}",
            envelope.delivery_id, err
        );
    }
    Ok(ChannelDeliveryServiceResult {
        status: receipt_status(receipt.status),
        receipt: Some(receipt),
        error_text,
    })
}

fn result_from_existing_receipt(receipt: ChannelDeliveryReceipt) -> ChannelDeliveryServiceResult {
    ChannelDeliveryServiceResult {
        status: receipt_status(receipt.status),
        receipt: Some(receipt),
        error_text: None,
    }
}

fn receipt_status(status: ChannelDeliveryStatus) -> ChannelDeliveryServiceStatus {
    match status {
        ChannelDeliveryStatus::Accepted => ChannelDeliveryServiceStatus::Accepted,
        ChannelDeliveryStatus::Delivered => ChannelDeliveryServiceStatus::Delivered,
        ChannelDeliveryStatus::Read => ChannelDeliveryServiceStatus::Read,
        ChannelDeliveryStatus::Failed | ChannelDeliveryStatus::Partial => {
            ChannelDeliveryServiceStatus::Failed
        }
    }
}

fn channel_kind(channel: RuntimeChannel) -> ChannelKind {
    match channel {
        RuntimeChannel::Telegram => ChannelKind::Telegram,
        RuntimeChannel::Whatsapp => ChannelKind::Whatsapp,
        RuntimeChannel::Wechat => ChannelKind::Wechat,
        RuntimeChannel::Feishu => ChannelKind::Feishu,
        RuntimeChannel::Lark => ChannelKind::Lark,
    }
}

#[cfg(test)]
#[path = "delivery_service_tests.rs"]
mod tests;
