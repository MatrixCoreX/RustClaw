use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

#[path = "channel_provider_error_support.rs"]
mod support;

use support::*;

pub const CHANNEL_PROVIDER_ERROR_SCHEMA_VERSION: u16 = 1;
pub const CHANNEL_PROVIDER_ERROR_PREFIX: &str = "__CHANNEL_PROVIDER_ERROR_V1__:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelProviderFailureClass {
    Authentication,
    PermissionDenied,
    RecipientBlocked,
    TargetNotFound,
    RateLimited,
    PayloadRejected,
    ProviderUnavailable,
    Transport,
    InvalidResponse,
    Unknown,
}

impl ChannelProviderFailureClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::PermissionDenied => "permission_denied",
            Self::RecipientBlocked => "recipient_blocked",
            Self::TargetNotFound => "target_not_found",
            Self::RateLimited => "rate_limited",
            Self::PayloadRejected => "payload_rejected",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::Transport => "transport",
            Self::InvalidResponse => "invalid_response",
            Self::Unknown => "unknown",
        }
    }

    pub const fn error_code(self) -> &'static str {
        match self {
            Self::Authentication => "channel.provider.authentication",
            Self::PermissionDenied => "channel.provider.permission_denied",
            Self::RecipientBlocked => "channel.provider.recipient_blocked",
            Self::TargetNotFound => "channel.provider.target_not_found",
            Self::RateLimited => "channel.provider.rate_limited",
            Self::PayloadRejected => "channel.provider.payload_rejected",
            Self::ProviderUnavailable => "channel.provider.unavailable",
            Self::Transport => "channel.provider.transport",
            Self::InvalidResponse => "channel.provider.invalid_response",
            Self::Unknown => "channel.provider.unknown",
        }
    }

    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Authentication => "channel.error.provider_authentication",
            Self::PermissionDenied => "channel.error.provider_permission_denied",
            Self::RecipientBlocked => "channel.error.provider_recipient_blocked",
            Self::TargetNotFound => "channel.error.provider_target_not_found",
            Self::RateLimited => "channel.error.provider_rate_limited",
            Self::PayloadRejected => "channel.error.provider_payload_rejected",
            Self::ProviderUnavailable | Self::Transport | Self::Unknown => {
                "channel.error.provider_unavailable"
            }
            Self::InvalidResponse => "channel.error.provider_invalid_response",
        }
    }

    pub const fn retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::ProviderUnavailable | Self::Transport
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelProviderTransportKind {
    Timeout,
    Connect,
    Request,
    Body,
    Decode,
    Unknown,
}

impl ChannelProviderTransportKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Connect => "connect",
            Self::Request => "request",
            Self::Body => "body",
            Self::Decode => "decode",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelProviderError {
    pub schema_version: u16,
    pub source_adapter: String,
    pub operation: String,
    pub failure_class: ChannelProviderFailureClass,
    pub error_code: String,
    pub message_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
    pub retryable: bool,
    pub diagnostic_id: String,
}

impl ChannelProviderError {
    pub fn from_http_response(
        source_adapter: &str,
        operation: &str,
        status_code: u16,
        response_body: &str,
    ) -> Self {
        let failure_class = classify_http_status(status_code);
        let provider_error_code = extract_provider_error_code(response_body);
        let retry_after_seconds = extract_retry_after_seconds(response_body)
            .filter(|_| failure_class == ChannelProviderFailureClass::RateLimited);
        let diagnostic_id = diagnostic_id(
            source_adapter,
            operation,
            Some(status_code),
            response_body.as_bytes(),
        );
        let error = Self::new(
            source_adapter,
            operation,
            failure_class,
            Some(status_code),
            provider_error_code,
            retry_after_seconds,
            diagnostic_id,
        );
        log_redacted_provider_failure(&error, Some(response_body), None);
        error
    }

    pub fn from_transport(
        source_adapter: &str,
        operation: &str,
        kind: ChannelProviderTransportKind,
        diagnostic_material: &str,
    ) -> Self {
        let diagnostic_id = diagnostic_id(
            source_adapter,
            operation,
            None,
            diagnostic_material.as_bytes(),
        );
        let error = Self::new(
            source_adapter,
            operation,
            ChannelProviderFailureClass::Transport,
            None,
            None,
            None,
            diagnostic_id,
        );
        log_redacted_provider_failure(&error, None, Some(kind.as_str()));
        error
    }

    pub fn invalid_response(
        source_adapter: &str,
        operation: &str,
        diagnostic_material: &str,
    ) -> Self {
        let diagnostic_id = diagnostic_id(
            source_adapter,
            operation,
            None,
            diagnostic_material.as_bytes(),
        );
        let error = Self::new(
            source_adapter,
            operation,
            ChannelProviderFailureClass::InvalidResponse,
            None,
            None,
            None,
            diagnostic_id,
        );
        log_redacted_provider_failure(&error, None, Some("invalid_response"));
        error
    }

    pub fn decode(value: &str) -> Option<Self> {
        let encoded = value.trim().strip_prefix(CHANNEL_PROVIDER_ERROR_PREFIX)?;
        let decoded: Self = serde_json::from_str(encoded).ok()?;
        decoded.is_valid().then_some(decoded)
    }

    pub fn from_machine_failure(
        source_adapter: &str,
        operation: &str,
        failure_class: ChannelProviderFailureClass,
        status_code: Option<u16>,
        provider_error_code: Option<&str>,
        retry_after_seconds: Option<u64>,
        diagnostic_material: &str,
    ) -> Self {
        let provider_error_code = provider_error_code
            .filter(|value| is_provider_code(value))
            .map(str::to_string);
        let retry_after_seconds = retry_after_seconds
            .filter(|seconds| *seconds > 0 && *seconds <= 86_400)
            .filter(|_| failure_class == ChannelProviderFailureClass::RateLimited);
        let diagnostic_id = diagnostic_id(
            source_adapter,
            operation,
            status_code,
            diagnostic_material.as_bytes(),
        );
        let error = Self::new(
            source_adapter,
            operation,
            failure_class,
            status_code,
            provider_error_code,
            retry_after_seconds,
            diagnostic_id,
        );
        log_redacted_provider_failure(&error, None, Some("typed_machine_failure"));
        error
    }

    pub fn is_valid(&self) -> bool {
        self.schema_version == CHANNEL_PROVIDER_ERROR_SCHEMA_VERSION
            && is_machine_token(&self.source_adapter, false)
            && is_machine_token(&self.operation, false)
            && self.error_code == self.failure_class.error_code()
            && self.message_key == self.failure_class.message_key()
            && self.retryable == self.failure_class.retryable()
            && is_diagnostic_id(&self.diagnostic_id)
            && self
                .provider_error_code
                .as_deref()
                .is_none_or(is_provider_code)
            && self.retry_after_seconds.is_none_or(|seconds| {
                self.failure_class == ChannelProviderFailureClass::RateLimited
                    && seconds > 0
                    && seconds <= 86_400
            })
    }

    fn new(
        source_adapter: &str,
        operation: &str,
        failure_class: ChannelProviderFailureClass,
        status_code: Option<u16>,
        provider_error_code: Option<String>,
        retry_after_seconds: Option<u64>,
        diagnostic_id: String,
    ) -> Self {
        Self {
            schema_version: CHANNEL_PROVIDER_ERROR_SCHEMA_VERSION,
            source_adapter: normalized_machine_token(source_adapter, "unknown_adapter"),
            operation: normalized_machine_token(operation, "unknown_operation"),
            failure_class,
            error_code: failure_class.error_code().to_string(),
            message_key: failure_class.message_key().to_string(),
            status_code,
            provider_error_code,
            retry_after_seconds,
            retryable: failure_class.retryable(),
            diagnostic_id,
        }
    }
}

impl fmt::Display for ChannelProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let encoded = serde_json::to_string(self).map_err(|_| fmt::Error)?;
        write!(formatter, "{CHANNEL_PROVIDER_ERROR_PREFIX}{encoded}")
    }
}

impl Error for ChannelProviderError {}

#[cfg(test)]
#[path = "channel_provider_error_tests.rs"]
mod tests;
