use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::channel_ingress::ChannelReplyTarget;
use crate::channel_notice::ChannelNotice;
use crate::types::ChannelKind;

pub const CHANNEL_DELIVERY_SCHEMA_VERSION: u16 = 1;
pub const CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION: u16 = 1;
pub const CHANNEL_TASK_DELIVERY_REQUEST_SCHEMA_VERSION: u16 = 1;
pub const CHANNEL_TASK_DELIVERY_RESPONSE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelDeliverySource {
    ImmediateDaemon,
    BackgroundCompletion,
    ScheduledTask,
    ProactiveNotice,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelTaskDeliveryContent {
    #[default]
    Full,
    TextOnly,
    MediaOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelTaskDeliveryRequest {
    pub schema_version: u16,
    pub source: ChannelDeliverySource,
    #[serde(default)]
    pub content: ChannelTaskDeliveryContent,
}

impl ChannelTaskDeliveryRequest {
    pub fn daemon(source: ChannelDeliverySource) -> Self {
        Self::daemon_with_content(source, ChannelTaskDeliveryContent::Full)
    }

    pub fn daemon_with_content(
        source: ChannelDeliverySource,
        content: ChannelTaskDeliveryContent,
    ) -> Self {
        Self {
            schema_version: CHANNEL_TASK_DELIVERY_REQUEST_SCHEMA_VERSION,
            source,
            content,
        }
    }

    pub fn validate(&self) -> Result<(), ChannelTaskDeliveryRequestValidationError> {
        if self.schema_version != CHANNEL_TASK_DELIVERY_REQUEST_SCHEMA_VERSION {
            return Err(ChannelTaskDeliveryRequestValidationError::UnsupportedSchemaVersion);
        }
        if !matches!(
            self.source,
            ChannelDeliverySource::ImmediateDaemon | ChannelDeliverySource::BackgroundCompletion
        ) {
            return Err(ChannelTaskDeliveryRequestValidationError::InvalidSource);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelTaskDeliveryStatus {
    Accepted,
    Delivered,
    Read,
    Failed,
    InProgress,
    QueryRequired,
    NotRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelTaskDeliveryResponse {
    pub schema_version: u16,
    pub status: ChannelTaskDeliveryStatus,
    pub accepted: bool,
    pub delivered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<ChannelDeliveryReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_key: Option<String>,
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ChannelTaskDeliveryRequestValidationError {
    #[error("channel_task_delivery_request_schema_version_unsupported")]
    UnsupportedSchemaVersion,
    #[error("channel_task_delivery_request_source_invalid")]
    InvalidSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelDeliveryHistoryDisposition {
    AssistantResult,
    TransportOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelTextFormat {
    Plain,
    Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelTextSegment {
    pub text: String,
    pub format: ChannelTextFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelArtifactKind {
    Image,
    Video,
    Audio,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelArtifactRef {
    pub artifact_ref: String,
    pub kind: ChannelArtifactKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelArtifactPreview {
    pub artifact_ref: String,
    pub preview_artifact_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelConversationWindowState {
    Open,
    Closed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelConversationWindow {
    pub state: ChannelConversationWindowState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ts: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDeliveryEnvelope {
    pub schema_version: u16,
    pub delivery_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub source: ChannelDeliverySource,
    pub channel: ChannelKind,
    pub adapter: String,
    pub reply_target: ChannelReplyTarget,
    pub locale: String,
    pub conversation_window: ChannelConversationWindow,
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_segments: Vec<ChannelTextSegment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ChannelArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previews: Vec<ChannelArtifactPreview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<ChannelNotice>,
}

impl ChannelDeliveryEnvelope {
    pub fn history_disposition(&self) -> ChannelDeliveryHistoryDisposition {
        if self.notice.is_some() || self.source == ChannelDeliverySource::ProactiveNotice {
            ChannelDeliveryHistoryDisposition::TransportOnly
        } else {
            ChannelDeliveryHistoryDisposition::AssistantResult
        }
    }

    pub fn validate(&self) -> Result<(), ChannelDeliveryValidationError> {
        if self.schema_version != CHANNEL_DELIVERY_SCHEMA_VERSION {
            return Err(ChannelDeliveryValidationError::UnsupportedSchemaVersion);
        }
        if !is_identifier(&self.delivery_id, 160)
            || !is_identifier(&self.idempotency_key, 200)
            || !is_machine_name(&self.adapter, 96)
        {
            return Err(ChannelDeliveryValidationError::InvalidIdentifier);
        }
        if self.reply_target.external_id.trim().is_empty()
            || self.reply_target.external_id.len() > 512
        {
            return Err(ChannelDeliveryValidationError::InvalidReplyTarget);
        }
        if self.locale.trim().is_empty() || self.locale.len() > 32 {
            return Err(ChannelDeliveryValidationError::InvalidLocale);
        }
        if self.text_segments.is_empty() && self.artifacts.is_empty() && self.notice.is_none() {
            return Err(ChannelDeliveryValidationError::EmptyDelivery);
        }
        if self.text_segments.len() > 128 || self.artifacts.len() > 64 || self.previews.len() > 64 {
            return Err(ChannelDeliveryValidationError::TooManyParts);
        }
        if self
            .text_segments
            .iter()
            .any(|segment| segment.text.is_empty() || segment.text.len() > 262_144)
        {
            return Err(ChannelDeliveryValidationError::InvalidTextSegment);
        }
        for artifact in &self.artifacts {
            validate_artifact_ref(&artifact.artifact_ref)?;
            if artifact
                .mime_type
                .as_deref()
                .is_some_and(|value| !is_mime_type(value))
                || artifact
                    .display_name
                    .as_deref()
                    .is_some_and(|value| value.is_empty() || value.len() > 512)
            {
                return Err(ChannelDeliveryValidationError::InvalidArtifact);
            }
        }
        for preview in &self.previews {
            validate_artifact_ref(&preview.artifact_ref)?;
            validate_artifact_ref(&preview.preview_artifact_ref)?;
            if preview.preview_artifact_ref == preview.artifact_ref
                || !self
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.artifact_ref == preview.artifact_ref)
                || preview
                    .mime_type
                    .as_deref()
                    .is_some_and(|value| !is_mime_type(value))
            {
                return Err(ChannelDeliveryValidationError::InvalidPreview);
            }
        }
        if self
            .conversation_window
            .context_token
            .as_deref()
            .is_some_and(|value| value.is_empty() || value.len() > 4096)
        {
            return Err(ChannelDeliveryValidationError::InvalidConversationWindow);
        }
        if matches!(
            self.conversation_window.state,
            ChannelConversationWindowState::Closed
        ) && self.conversation_window.context_token.is_some()
        {
            return Err(ChannelDeliveryValidationError::InvalidConversationWindow);
        }
        if let Some(notice) = &self.notice {
            notice
                .validate()
                .map_err(|_| ChannelDeliveryValidationError::InvalidNotice)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelDeliveryStatus {
    Accepted,
    Delivered,
    Read,
    Failed,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDeliveryPartReceipt {
    pub part_index: u32,
    pub status: ChannelDeliveryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDeliveryReceipt {
    pub schema_version: u16,
    pub delivery_id: String,
    pub idempotency_key: String,
    pub channel: ChannelKind,
    pub adapter: String,
    pub status: ChannelDeliveryStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_message_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<ChannelDeliveryPartReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_error_code: Option<String>,
    pub retryable: bool,
    pub updated_at_ts: u64,
}

impl ChannelDeliveryReceipt {
    pub fn history_disposition(&self) -> ChannelDeliveryHistoryDisposition {
        ChannelDeliveryHistoryDisposition::TransportOnly
    }

    pub fn validate(&self) -> Result<(), ChannelDeliveryValidationError> {
        if self.schema_version != CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION {
            return Err(ChannelDeliveryValidationError::UnsupportedReceiptSchemaVersion);
        }
        if !is_identifier(&self.delivery_id, 160)
            || !is_identifier(&self.idempotency_key, 200)
            || !is_machine_name(&self.adapter, 96)
        {
            return Err(ChannelDeliveryValidationError::InvalidIdentifier);
        }
        if self.provider_message_ids.len() > 128 || self.parts.len() > 128 {
            return Err(ChannelDeliveryValidationError::TooManyParts);
        }
        if self
            .provider_message_ids
            .iter()
            .any(|value| value.is_empty() || value.len() > 512)
        {
            return Err(ChannelDeliveryValidationError::InvalidProviderMessageId);
        }
        if self.parts.iter().enumerate().any(|(index, part)| {
            part.part_index as usize != index
                || part
                    .provider_message_id
                    .as_deref()
                    .is_some_and(|value| value.is_empty() || value.len() > 512)
                || part
                    .error_code
                    .as_deref()
                    .is_some_and(|value| !is_machine_name(value, 160))
        }) {
            return Err(ChannelDeliveryValidationError::InvalidPartReceipt);
        }
        if self
            .error_code
            .as_deref()
            .is_some_and(|value| !is_machine_name(value, 160))
            || self
                .message_key
                .as_deref()
                .is_some_and(|value| !is_machine_name(value, 160))
            || self.diagnostic_id.as_deref().is_some_and(|value| {
                value.is_empty()
                    || value.len() > 128
                    || !value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':')
                    })
            })
            || self.provider_error_code.as_deref().is_some_and(|value| {
                value.is_empty()
                    || value.len() > 128
                    || !value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
                    })
            })
        {
            return Err(ChannelDeliveryValidationError::InvalidReceiptError);
        }
        match self.status {
            ChannelDeliveryStatus::Accepted => {
                if self.error_code.is_some()
                    || self.message_key.is_some()
                    || self.provider_error_code.is_some()
                    || self.retryable
                {
                    return Err(ChannelDeliveryValidationError::InvalidReceiptState);
                }
            }
            ChannelDeliveryStatus::Delivered | ChannelDeliveryStatus::Read => {
                if self.provider_message_ids.is_empty()
                    || self.error_code.is_some()
                    || self.message_key.is_some()
                    || self.provider_error_code.is_some()
                    || self.retryable
                {
                    return Err(ChannelDeliveryValidationError::InvalidReceiptState);
                }
            }
            ChannelDeliveryStatus::Failed => {
                if self.error_code.is_none() || self.diagnostic_id.is_none() {
                    return Err(ChannelDeliveryValidationError::InvalidReceiptState);
                }
            }
            ChannelDeliveryStatus::Partial => {
                let has_success = self.parts.iter().any(|part| {
                    matches!(
                        part.status,
                        ChannelDeliveryStatus::Accepted
                            | ChannelDeliveryStatus::Delivered
                            | ChannelDeliveryStatus::Read
                    )
                });
                let has_failure = self
                    .parts
                    .iter()
                    .any(|part| matches!(part.status, ChannelDeliveryStatus::Failed));
                if !has_success
                    || !has_failure
                    || self.error_code.is_none()
                    || self.diagnostic_id.is_none()
                {
                    return Err(ChannelDeliveryValidationError::InvalidReceiptState);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelDeliveryRetryDecision {
    SendNew,
    QueryProviderReceipt,
    AlreadyDelivered,
    RetryFailedParts,
    StopTerminalFailure,
}

pub fn delivery_retry_decision(
    receipt: Option<&ChannelDeliveryReceipt>,
) -> ChannelDeliveryRetryDecision {
    let Some(receipt) = receipt else {
        return ChannelDeliveryRetryDecision::SendNew;
    };
    match receipt.status {
        ChannelDeliveryStatus::Accepted => ChannelDeliveryRetryDecision::QueryProviderReceipt,
        ChannelDeliveryStatus::Delivered | ChannelDeliveryStatus::Read => {
            ChannelDeliveryRetryDecision::AlreadyDelivered
        }
        ChannelDeliveryStatus::Failed | ChannelDeliveryStatus::Partial if receipt.retryable => {
            ChannelDeliveryRetryDecision::RetryFailedParts
        }
        ChannelDeliveryStatus::Failed | ChannelDeliveryStatus::Partial => {
            ChannelDeliveryRetryDecision::StopTerminalFailure
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ChannelDeliveryValidationError {
    #[error("channel_delivery_schema_version_unsupported")]
    UnsupportedSchemaVersion,
    #[error("channel_delivery_receipt_schema_version_unsupported")]
    UnsupportedReceiptSchemaVersion,
    #[error("channel_delivery_identifier_invalid")]
    InvalidIdentifier,
    #[error("channel_delivery_reply_target_invalid")]
    InvalidReplyTarget,
    #[error("channel_delivery_locale_invalid")]
    InvalidLocale,
    #[error("channel_delivery_empty")]
    EmptyDelivery,
    #[error("channel_delivery_too_many_parts")]
    TooManyParts,
    #[error("channel_delivery_text_segment_invalid")]
    InvalidTextSegment,
    #[error("channel_delivery_artifact_invalid")]
    InvalidArtifact,
    #[error("channel_delivery_preview_invalid")]
    InvalidPreview,
    #[error("channel_delivery_conversation_window_invalid")]
    InvalidConversationWindow,
    #[error("channel_delivery_notice_invalid")]
    InvalidNotice,
    #[error("channel_delivery_provider_message_id_invalid")]
    InvalidProviderMessageId,
    #[error("channel_delivery_part_receipt_invalid")]
    InvalidPartReceipt,
    #[error("channel_delivery_receipt_error_invalid")]
    InvalidReceiptError,
    #[error("channel_delivery_receipt_state_invalid")]
    InvalidReceiptState,
}

fn validate_artifact_ref(value: &str) -> Result<(), ChannelDeliveryValidationError> {
    if !value.starts_with("artifact:") || !is_identifier(value, 240) {
        return Err(ChannelDeliveryValidationError::InvalidArtifact);
    }
    Ok(())
}

fn is_identifier(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.' | b'/')
        })
}

fn is_machine_name(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

fn is_mime_type(value: &str) -> bool {
    let Some((kind, subtype)) = value.split_once('/') else {
        return false;
    };
    !kind.is_empty()
        && !subtype.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+' | b'/')
        })
}

#[cfg(test)]
#[path = "channel_delivery_tests.rs"]
mod tests;
