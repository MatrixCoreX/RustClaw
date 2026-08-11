use anyhow::{anyhow, Context};
use claw_core::channel_delivery::{
    ChannelConversationWindow, ChannelConversationWindowState, ChannelDeliveryEnvelope,
    ChannelDeliveryPartReceipt, ChannelDeliveryReceipt, ChannelDeliverySource,
    ChannelDeliveryStatus, ChannelTaskDeliveryContent, ChannelTaskDeliveryResponse,
    ChannelTaskDeliveryStatus, ChannelTextFormat, ChannelTextSegment,
    CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION, CHANNEL_DELIVERY_SCHEMA_VERSION,
    CHANNEL_TASK_DELIVERY_RESPONSE_SCHEMA_VERSION,
};
use claw_core::channel_ingress::{
    default_adapter_for_channel, default_reply_target, ChannelReplyTarget,
};
use claw_core::channel_open_platform::{scoped_open_platform_receipt_key, OpenPlatformRegion};
use claw_core::channel_provider_error::ChannelProviderError;
use claw_core::types::ChannelKind;
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use crate::repo::ClaimChannelDeliveryDispatchOutcome;
use crate::{AppState, ClaimedTask, RuntimeChannel};

const DELIVERY_DISPATCH_LEASE_SECONDS: u64 = 120;
const DELIVERY_DISPATCH_HEARTBEAT_SECONDS: u64 = 30;

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
    pub(crate) error_code: Option<String>,
    pub(crate) message_key: Option<String>,
    pub(crate) retryable: bool,
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

    pub(crate) fn into_task_response(self) -> ChannelTaskDeliveryResponse {
        ChannelTaskDeliveryResponse {
            schema_version: CHANNEL_TASK_DELIVERY_RESPONSE_SCHEMA_VERSION,
            status: match self.status {
                ChannelDeliveryServiceStatus::Accepted => ChannelTaskDeliveryStatus::Accepted,
                ChannelDeliveryServiceStatus::Delivered => ChannelTaskDeliveryStatus::Delivered,
                ChannelDeliveryServiceStatus::Read => ChannelTaskDeliveryStatus::Read,
                ChannelDeliveryServiceStatus::Failed => ChannelTaskDeliveryStatus::Failed,
                ChannelDeliveryServiceStatus::InProgress => ChannelTaskDeliveryStatus::InProgress,
                ChannelDeliveryServiceStatus::QueryRequired => {
                    ChannelTaskDeliveryStatus::QueryRequired
                }
            },
            accepted: self.accepted(),
            delivered: self.delivered(),
            receipt: self.receipt,
            error_code: self.error_code,
            message_key: self.message_key,
            retryable: self.retryable,
        }
    }
}

pub(crate) fn build_scheduled_delivery_envelope(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    text: &str,
) -> anyhow::Result<ChannelDeliveryEnvelope> {
    build_delivery_envelope(
        state,
        task,
        payload,
        text,
        ChannelDeliverySource::ScheduledTask,
        "schedule-terminal",
        None,
    )
}

pub(crate) fn build_proactive_notice_envelope(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    idempotency_suffix: &str,
    notice: claw_core::channel_notice::ChannelNotice,
) -> anyhow::Result<ChannelDeliveryEnvelope> {
    build_delivery_envelope(
        state,
        task,
        payload,
        "",
        ChannelDeliverySource::ProactiveNotice,
        idempotency_suffix,
        Some(notice),
    )
}

pub(crate) fn build_daemon_delivery_envelope(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    text: &str,
    source: ChannelDeliverySource,
    content: ChannelTaskDeliveryContent,
    notice: Option<claw_core::channel_notice::ChannelNotice>,
) -> anyhow::Result<ChannelDeliveryEnvelope> {
    if !matches!(
        source,
        ChannelDeliverySource::ImmediateDaemon | ChannelDeliverySource::BackgroundCompletion
    ) {
        return Err(anyhow!("channel_task_delivery_request_source_invalid"));
    }
    let idempotency_suffix = match content {
        ChannelTaskDeliveryContent::Full => "terminal",
        ChannelTaskDeliveryContent::TextOnly => "terminal-text",
        ChannelTaskDeliveryContent::MediaOnly => "terminal-media",
    };
    build_delivery_envelope(
        state,
        task,
        payload,
        text,
        source,
        idempotency_suffix,
        notice,
    )
}

fn build_delivery_envelope(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    text: &str,
    source: ChannelDeliverySource,
    idempotency_suffix: &str,
    notice: Option<claw_core::channel_notice::ChannelNotice>,
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
    let conversation_window = if channel == ChannelKind::Whatsapp && adapter == "whatsapp_cloud" {
        let external_user_id = external_user_id
            .or(external_chat_id)
            .ok_or_else(|| anyhow!("whatsapp_cloud_conversation_identity_missing"))?;
        crate::repo::whatsapp_cloud_conversation_window(
            &state.core.db,
            &state.channels.whatsapp_phone_number_id,
            external_user_id,
            crate::now_ts_u64(),
        )?
    } else {
        ChannelConversationWindow {
            state: if context_token.is_some() {
                ChannelConversationWindowState::Open
            } else {
                ChannelConversationWindowState::Unknown
            },
            expires_at_ts: None,
            context_token,
        }
    };
    let delivery_id = format!("delivery:{}:{idempotency_suffix}", task.task_id);
    let base_idempotency_key = format!("{}:{idempotency_suffix}", task.task_id);
    let idempotency_key = match channel {
        ChannelKind::Feishu => {
            scoped_open_platform_receipt_key(OpenPlatformRegion::Feishu, &base_idempotency_key)
        }
        ChannelKind::Lark => {
            scoped_open_platform_receipt_key(OpenPlatformRegion::Lark, &base_idempotency_key)
        }
        _ => base_idempotency_key,
    };
    let rendered_notice_text = notice.as_ref().map(|notice| {
        let vars = notice
            .params
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        claw_core::channel_i18n::common_text_with_vars_for_locale(
            &locale,
            &notice.message_key,
            &vars,
        )
    });
    let delivery_text = if text.trim().is_empty() {
        rendered_notice_text.unwrap_or_default()
    } else {
        text.to_string()
    };
    let envelope = ChannelDeliveryEnvelope {
        schema_version: CHANNEL_DELIVERY_SCHEMA_VERSION,
        delivery_id,
        task_id: Some(task.task_id.clone()),
        source,
        channel,
        adapter,
        reply_target,
        locale,
        conversation_window,
        idempotency_key,
        text_segments: vec![ChannelTextSegment {
            text: delivery_text,
            format: ChannelTextFormat::Plain,
        }],
        artifacts: Vec::new(),
        previews: Vec::new(),
        notice,
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
                error_code: None,
                message_key: None,
                retryable: false,
            });
        }
        ClaimChannelDeliveryDispatchOutcome::QueryRequired => {
            return Ok(ChannelDeliveryServiceResult {
                status: ChannelDeliveryServiceStatus::QueryRequired,
                receipt: None,
                error_code: None,
                message_key: None,
                retryable: false,
            });
        }
    };

    let text = envelope
        .text_segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let (send_result, observed_provider_message_ids) =
        crate::channel_send::capture_channel_send_progress(send_with_dispatch_lease(
            state,
            &envelope.idempotency_key,
            &lease_token,
            crate::worker::send_task_channel_message(
                state,
                task,
                payload,
                &text,
                &envelope.delivery_id,
                &envelope.conversation_window,
                envelope.source,
            ),
        ))
        .await;
    let now = crate::now_ts_u64();
    let receipt = match send_result {
        Ok(mut outcome) => {
            for provider_message_id in observed_provider_message_ids {
                if !outcome.provider_message_ids.contains(&provider_message_id) {
                    outcome.provider_message_ids.push(provider_message_id);
                }
            }
            accepted_delivery_receipt(envelope, outcome, now)
        }
        Err(error_text) => {
            let (error_code, message_key, diagnostic_id, provider_error_code, retryable) =
                delivery_failure_fields(&error_text);
            warn!(
                event = "channel_delivery_failure",
                delivery_id = %envelope.delivery_id,
                adapter = %envelope.adapter,
                error_code = %error_code,
                message_key = %message_key,
                retryable,
                diagnostic_id = %diagnostic_id,
                "channel delivery failed"
            );
            let partial = !observed_provider_message_ids.is_empty();
            let mut parts = observed_provider_message_ids
                .iter()
                .enumerate()
                .map(
                    |(part_index, provider_message_id)| ChannelDeliveryPartReceipt {
                        part_index: part_index as u32,
                        status: ChannelDeliveryStatus::Accepted,
                        provider_message_id: Some(provider_message_id.clone()),
                        error_code: None,
                    },
                )
                .collect::<Vec<_>>();
            if partial {
                parts.push(ChannelDeliveryPartReceipt {
                    part_index: parts.len() as u32,
                    status: ChannelDeliveryStatus::Failed,
                    provider_message_id: None,
                    error_code: Some(error_code.clone()),
                });
            }
            ChannelDeliveryReceipt {
                schema_version: CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION,
                delivery_id: envelope.delivery_id.clone(),
                idempotency_key: envelope.idempotency_key.clone(),
                channel: envelope.channel,
                adapter: envelope.adapter.clone(),
                status: if partial {
                    ChannelDeliveryStatus::Partial
                } else {
                    ChannelDeliveryStatus::Failed
                },
                provider_message_ids: observed_provider_message_ids,
                parts,
                error_code: Some(error_code),
                message_key: Some(message_key),
                diagnostic_id: Some(diagnostic_id),
                provider_error_code,
                // Replaying a multi-part send from its beginning would duplicate the
                // provider-accepted prefix. Keep the exact partial receipt for an
                // operator/part-aware retry instead of doing an unsafe whole-send retry.
                retryable: retryable && !partial,
                updated_at_ts: now,
            }
        }
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
        error_code: receipt.error_code.clone(),
        message_key: receipt.message_key.clone(),
        retryable: receipt.retryable,
        receipt: Some(receipt),
    })
}

async fn send_with_dispatch_lease<F>(
    state: &AppState,
    idempotency_key: &str,
    lease_token: &str,
    send_future: F,
) -> Result<crate::channel_send::ChannelSendOutcome, String>
where
    F: std::future::Future<Output = Result<crate::channel_send::ChannelSendOutcome, String>>,
{
    let heartbeat_period = std::time::Duration::from_secs(DELIVERY_DISPATCH_HEARTBEAT_SECONDS);
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + heartbeat_period,
        heartbeat_period,
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tokio::pin!(send_future);

    loop {
        tokio::select! {
            send_result = &mut send_future => return send_result,
            _ = heartbeat.tick() => {
                match crate::repo::renew_channel_delivery_dispatch(
                    &state.core.db,
                    idempotency_key,
                    lease_token,
                    DELIVERY_DISPATCH_LEASE_SECONDS,
                ) {
                    Ok(true) => {}
                    Ok(false) => {
                        warn!(
                            event = "channel_delivery_dispatch_lease_lost",
                            idempotency_key = %idempotency_key,
                            "channel delivery dispatch lease ownership was lost"
                        );
                        return Err("channel_delivery_dispatch_lease_lost".to_string());
                    }
                    Err(error) => {
                        warn!(
                            event = "channel_delivery_dispatch_lease_renewal_failed",
                            idempotency_key = %idempotency_key,
                            diagnostic = %error,
                            "channel delivery dispatch lease renewal failed"
                        );
                        return Err("channel_delivery_dispatch_lease_renewal_failed".to_string());
                    }
                }
            }
        }
    }
}

fn accepted_delivery_receipt(
    envelope: &ChannelDeliveryEnvelope,
    outcome: crate::channel_send::ChannelSendOutcome,
    now: u64,
) -> ChannelDeliveryReceipt {
    let parts = outcome
        .provider_message_ids
        .iter()
        .enumerate()
        .map(
            |(part_index, provider_message_id)| ChannelDeliveryPartReceipt {
                part_index: part_index as u32,
                status: ChannelDeliveryStatus::Accepted,
                provider_message_id: Some(provider_message_id.clone()),
                error_code: None,
            },
        )
        .collect();
    ChannelDeliveryReceipt {
        schema_version: CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION,
        delivery_id: envelope.delivery_id.clone(),
        idempotency_key: envelope.idempotency_key.clone(),
        channel: envelope.channel,
        adapter: envelope.adapter.clone(),
        status: ChannelDeliveryStatus::Accepted,
        provider_message_ids: outcome.provider_message_ids,
        parts,
        error_code: None,
        message_key: None,
        diagnostic_id: None,
        provider_error_code: None,
        retryable: false,
        updated_at_ts: now,
    }
}

fn delivery_failure_fields(error_text: &str) -> (String, String, String, Option<String>, bool) {
    let provider_error = ChannelProviderError::decode(error_text);
    let error_code = provider_error
        .as_ref()
        .map(|error| error.error_code.clone())
        .unwrap_or_else(|| "channel.delivery.failed".to_string());
    let message_key = provider_error
        .as_ref()
        .map(|error| error.message_key.clone())
        .unwrap_or_else(|| "channel.error.delivery_failed".to_string());
    let diagnostic_id = provider_error
        .as_ref()
        .map(|error| error.diagnostic_id.clone())
        .unwrap_or_else(|| format!("delivery:{}", Uuid::new_v4().simple()));
    let retryable = provider_error.as_ref().is_some_and(|error| error.retryable);
    let provider_error_code = provider_error.and_then(|error| error.provider_error_code);
    (
        error_code,
        message_key,
        diagnostic_id,
        provider_error_code,
        retryable,
    )
}

fn result_from_existing_receipt(receipt: ChannelDeliveryReceipt) -> ChannelDeliveryServiceResult {
    ChannelDeliveryServiceResult {
        status: receipt_status(receipt.status),
        error_code: receipt.error_code.clone(),
        message_key: receipt.message_key.clone(),
        retryable: receipt.retryable,
        receipt: Some(receipt),
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
