use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CHANNEL_NOTICE_SCHEMA_VERSION: u16 = 1;
pub const CHANNEL_NOTICE_SAFE_GENERIC_MESSAGE_KEY: &str = "common.safe_generic_error";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelNoticeSeverity {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelNoticeActionKind {
    Retry,
    Rebind,
    OpenSettings,
    InspectStatus,
    ContactSupport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelNoticeNextAction {
    pub kind: ChannelNoticeActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_key: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelNotice {
    pub schema_version: u16,
    pub notice_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub message_key: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
    pub severity: ChannelNoticeSeverity,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next_actions: Vec<ChannelNoticeNextAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_id: Option<String>,
}

impl ChannelNotice {
    pub fn status(
        notice_code: impl Into<String>,
        message_key: impl Into<String>,
        severity: ChannelNoticeSeverity,
    ) -> Self {
        Self {
            schema_version: CHANNEL_NOTICE_SCHEMA_VERSION,
            notice_code: notice_code.into(),
            error_code: None,
            message_key: message_key.into(),
            params: BTreeMap::new(),
            severity,
            retryable: false,
            next_actions: Vec::new(),
            diagnostic_id: None,
        }
    }

    pub fn error(
        notice_code: impl Into<String>,
        error_code: impl Into<String>,
        message_key: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            schema_version: CHANNEL_NOTICE_SCHEMA_VERSION,
            notice_code: notice_code.into(),
            error_code: Some(error_code.into()),
            message_key: message_key.into(),
            params: BTreeMap::new(),
            severity: ChannelNoticeSeverity::Error,
            retryable,
            next_actions: Vec::new(),
            diagnostic_id: None,
        }
    }

    pub fn validate(&self) -> Result<(), ChannelNoticeValidationError> {
        if self.schema_version != CHANNEL_NOTICE_SCHEMA_VERSION {
            return Err(ChannelNoticeValidationError::UnsupportedSchemaVersion);
        }
        if !is_machine_token(&self.notice_code, true) {
            return Err(ChannelNoticeValidationError::InvalidNoticeCode);
        }
        if self
            .error_code
            .as_deref()
            .is_some_and(|value| !is_machine_token(value, true))
        {
            return Err(ChannelNoticeValidationError::InvalidErrorCode);
        }
        if !is_machine_token(&self.message_key, true) {
            return Err(ChannelNoticeValidationError::InvalidMessageKey);
        }
        validate_params(&self.params)?;
        if self.next_actions.len() > 8 {
            return Err(ChannelNoticeValidationError::TooManyNextActions);
        }
        for action in &self.next_actions {
            if action
                .message_key
                .as_deref()
                .is_some_and(|value| !is_machine_token(value, true))
            {
                return Err(ChannelNoticeValidationError::InvalidActionMessageKey);
            }
            validate_params(&action.params)?;
        }
        if self.diagnostic_id.as_deref().is_some_and(|value| {
            value.is_empty()
                || value.len() > 128
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
        }) {
            return Err(ChannelNoticeValidationError::InvalidDiagnosticId);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ChannelNoticeValidationError {
    #[error("channel_notice_schema_version_unsupported")]
    UnsupportedSchemaVersion,
    #[error("channel_notice_notice_code_invalid")]
    InvalidNoticeCode,
    #[error("channel_notice_error_code_invalid")]
    InvalidErrorCode,
    #[error("channel_notice_message_key_invalid")]
    InvalidMessageKey,
    #[error("channel_notice_param_invalid")]
    InvalidParam,
    #[error("channel_notice_param_value_too_long")]
    ParamValueTooLong,
    #[error("channel_notice_next_actions_too_many")]
    TooManyNextActions,
    #[error("channel_notice_action_message_key_invalid")]
    InvalidActionMessageKey,
    #[error("channel_notice_diagnostic_id_invalid")]
    InvalidDiagnosticId,
}

fn validate_params(params: &BTreeMap<String, String>) -> Result<(), ChannelNoticeValidationError> {
    if params.len() > 32 {
        return Err(ChannelNoticeValidationError::InvalidParam);
    }
    for (name, value) in params {
        if !is_machine_token(name, false) {
            return Err(ChannelNoticeValidationError::InvalidParam);
        }
        if value.len() > 4096 {
            return Err(ChannelNoticeValidationError::ParamValueTooLong);
        }
    }
    Ok(())
}

fn is_machine_token(value: &str, require_namespace: bool) -> bool {
    if value.is_empty() || value.len() > 160 || (require_namespace && !value.contains('.')) {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || (index > 0 && matches!(byte, b'_' | b'-' | b'.'))
    })
}

#[cfg(test)]
#[path = "channel_notice_tests.rs"]
mod tests;
