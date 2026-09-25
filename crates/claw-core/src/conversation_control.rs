use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::conversation_input::ConversationInputScopeRef;
use crate::product_identity::AUTH_KEY_HEADER;
use crate::types::ApiResponse;

pub const CONVERSATION_CONTROL_SCHEMA_VERSION: u32 = 1;
pub const CANCEL_REQUESTED_MESSAGE_KEY: &str = "channel.control.cancel_requested";
pub const CANCEL_NO_ACTIVE_MESSAGE_KEY: &str = "channel.control.cancel_no_active";
pub const CANCEL_FAILED_MESSAGE_KEY: &str = "channel.control.cancel_failed";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelCurrentConversationTaskRequest {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    pub client_request_id: String,
    pub scope: ConversationInputScopeRef,
    #[serde(default)]
    pub expected_task_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelCurrentConversationTaskStatus {
    CancelRequested,
    NoActiveTask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelCurrentConversationTaskReceipt {
    pub schema_version: u32,
    pub status: CancelCurrentConversationTaskStatus,
    #[serde(default)]
    pub task_id: Option<Uuid>,
    pub canceled: u64,
}

impl CancelCurrentConversationTaskRequest {
    pub fn validate(&self) -> bool {
        self.schema_version == CONVERSATION_CONTROL_SCHEMA_VERSION
            && valid_machine_token(&self.client_request_id, 512)
            && self.scope.validate()
    }
}

#[derive(Debug, Error)]
pub enum ConversationControlClientError {
    #[error("conversation_control_request_invalid")]
    InvalidRequest,
    #[error("conversation_control_request_failed")]
    Request,
    #[error("conversation_control_http_status_{0}")]
    HttpStatus(u16),
    #[error("conversation_control_response_invalid")]
    InvalidResponse,
    #[error("conversation_control_rejected")]
    Rejected,
}

pub async fn cancel_current_conversation_task(
    client: &reqwest::Client,
    base_url: &str,
    auth_key: &str,
    request: &CancelCurrentConversationTaskRequest,
) -> Result<CancelCurrentConversationTaskReceipt, ConversationControlClientError> {
    if auth_key.trim().is_empty() || !request.validate() {
        return Err(ConversationControlClientError::InvalidRequest);
    }
    let response = client
        .post(format!(
            "{}/v1/conversation-inputs/cancel-current",
            base_url.trim_end_matches('/')
        ))
        .header(AUTH_KEY_HEADER, auth_key.trim())
        .json(request)
        .send()
        .await
        .map_err(|_| ConversationControlClientError::Request)?;
    let status = response.status();
    if !status.is_success() {
        return Err(ConversationControlClientError::HttpStatus(status.as_u16()));
    }
    let body = response
        .json::<ApiResponse<CancelCurrentConversationTaskReceipt>>()
        .await
        .map_err(|_| ConversationControlClientError::InvalidResponse)?;
    if !body.ok {
        return Err(ConversationControlClientError::Rejected);
    }
    body.data
        .ok_or(ConversationControlClientError::InvalidResponse)
}

pub fn cancel_receipt_message_key(receipt: &CancelCurrentConversationTaskReceipt) -> &'static str {
    match receipt.status {
        CancelCurrentConversationTaskStatus::CancelRequested => CANCEL_REQUESTED_MESSAGE_KEY,
        CancelCurrentConversationTaskStatus::NoActiveTask => CANCEL_NO_ACTIVE_MESSAGE_KEY,
    }
}

/// Parses the optional machine task reference accepted after `/cancel`.
///
/// `Some(None)` is an argument-free current-task cancellation, `Some(Some(_))`
/// is an exact current-task guard, and `None` means the argument is not part of
/// the control protocol and must not be widened into an unscoped cancellation.
pub fn parse_cancel_expected_task_id(tail: &str) -> Option<Option<Uuid>> {
    let tail = tail.trim();
    if tail.is_empty() {
        return Some(None);
    }
    Uuid::parse_str(tail).ok().map(Some)
}

fn valid_machine_token(value: &str, max_chars: usize) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.chars().count() <= max_chars
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '@'))
}

const fn schema_version() -> u32 {
    CONVERSATION_CONTROL_SCHEMA_VERSION
}

#[cfg(test)]
#[path = "conversation_control_tests.rs"]
mod tests;
