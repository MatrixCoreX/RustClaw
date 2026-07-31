use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::warn;

use super::{ChannelProviderError, ChannelProviderFailureClass};

pub(super) fn classify_http_status(status_code: u16) -> ChannelProviderFailureClass {
    match status_code {
        401 => ChannelProviderFailureClass::Authentication,
        403 => ChannelProviderFailureClass::PermissionDenied,
        408 | 425 | 429 => ChannelProviderFailureClass::RateLimited,
        400 | 404 | 409 | 410 | 413 | 415 | 422 => ChannelProviderFailureClass::PayloadRejected,
        500..=599 => ChannelProviderFailureClass::ProviderUnavailable,
        _ => ChannelProviderFailureClass::Unknown,
    }
}

pub(super) fn extract_provider_error_code(response_body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(response_body).ok()?;
    [
        "/error/code",
        "/error/error_subcode",
        "/code",
        "/errcode",
        "/error_code",
    ]
    .iter()
    .find_map(|pointer| scalar_provider_code(value.pointer(pointer)?))
}

fn scalar_provider_code(value: &Value) -> Option<String> {
    let value = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => return None,
    };
    is_provider_code(&value).then_some(value)
}

pub(super) fn is_provider_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

pub(super) fn normalized_machine_token(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if is_machine_token(value, false) {
        value.to_string()
    } else {
        fallback.to_string()
    }
}

pub(super) fn is_machine_token(value: &str, require_namespace: bool) -> bool {
    if value.is_empty() || value.len() > 96 || (require_namespace && !value.contains('.')) {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || (index > 0 && matches!(byte, b'_' | b'-' | b'.'))
    })
}

pub(super) fn diagnostic_id(
    source_adapter: &str,
    operation: &str,
    status_code: Option<u16>,
    material: &[u8],
) -> String {
    let mut digest = Sha256::new();
    digest.update(source_adapter.as_bytes());
    digest.update([0]);
    digest.update(operation.as_bytes());
    digest.update([0]);
    digest.update(status_code.unwrap_or_default().to_be_bytes());
    digest.update([0]);
    digest.update(material);
    let hex = format!("{:x}", digest.finalize());
    format!("channel-provider:{}", &hex[..24])
}

pub(super) fn is_diagnostic_id(value: &str) -> bool {
    value.len() <= 128
        && value.starts_with("channel-provider:")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
}

fn body_digest(body: &str) -> String {
    format!("{:x}", Sha256::digest(body.as_bytes()))
}

pub(super) fn log_redacted_provider_failure(
    error: &ChannelProviderError,
    response_body: Option<&str>,
    transport_kind: Option<&str>,
) {
    let body_bytes = response_body.map(str::len).unwrap_or_default();
    let body_sha256 = response_body.map(body_digest).unwrap_or_default();
    warn!(
        event = "channel_provider_failure",
        schema_version = error.schema_version,
        source_adapter = %error.source_adapter,
        operation = %error.operation,
        failure_class = error.failure_class.as_str(),
        error_code = %error.error_code,
        message_key = %error.message_key,
        status_code = error.status_code.unwrap_or_default(),
        provider_error_code = error.provider_error_code.as_deref().unwrap_or("none"),
        retryable = error.retryable,
        diagnostic_id = %error.diagnostic_id,
        transport_kind = transport_kind.unwrap_or("none"),
        body_redacted = response_body.is_some(),
        body_bytes,
        body_sha256 = %body_sha256,
        "channel provider request failed"
    );
}
