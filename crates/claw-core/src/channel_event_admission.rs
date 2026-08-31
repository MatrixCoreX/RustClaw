use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::types::{ApiResponse, ChannelKind};

pub const CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION: u16 = 1;
pub const CHANNEL_EVENT_ADMISSION_TIMESTAMP_HEADER: &str = "x-channel-admission-timestamp";
pub const CHANNEL_EVENT_ADMISSION_SIGNATURE_HEADER: &str = "x-channel-admission-signature-256";
pub const CHANNEL_EVENT_ADMISSION_SIGNATURE_TOLERANCE_SECS: u64 = 60;
pub const CHANNEL_EVENT_ADMISSION_DEFAULT_LEASE_SECS: u64 = 300;
pub const CHANNEL_EVENT_ADMISSION_MIN_LEASE_SECS: u64 = 30;
pub const CHANNEL_EVENT_ADMISSION_MAX_LEASE_SECS: u64 = 900;
const CHANNEL_EVENT_ADMISSION_REQUEST_ATTEMPTS: u8 = 3;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelEventClaimRequest {
    pub schema_version: u16,
    pub channel: ChannelKind,
    pub account_id: String,
    pub provider_event_id: String,
    pub payload_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_timestamp: Option<u64>,
    pub lease_seconds: u64,
}

impl ChannelEventClaimRequest {
    pub fn new(
        channel: ChannelKind,
        account_id: impl Into<String>,
        provider_event_id: impl Into<String>,
        payload: &[u8],
    ) -> Self {
        Self {
            schema_version: CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
            channel,
            account_id: account_id.into(),
            provider_event_id: provider_event_id.into(),
            payload_sha256: sha256_hex(payload),
            provider_nonce: None,
            provider_timestamp: None,
            lease_seconds: CHANNEL_EVENT_ADMISSION_DEFAULT_LEASE_SECS,
        }
    }

    pub fn validate(&self) -> Result<(), ChannelEventAdmissionValidationError> {
        if self.schema_version != CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION {
            return Err(ChannelEventAdmissionValidationError::SchemaVersion);
        }
        validate_identifier(&self.account_id, 256)?;
        validate_identifier(&self.provider_event_id, 512)?;
        validate_sha256(&self.payload_sha256)?;
        if let Some(nonce) = self.provider_nonce.as_deref() {
            validate_identifier(nonce, 512)?;
        }
        if self.provider_timestamp == Some(0) {
            return Err(ChannelEventAdmissionValidationError::Timestamp);
        }
        if !(CHANNEL_EVENT_ADMISSION_MIN_LEASE_SECS..=CHANNEL_EVENT_ADMISSION_MAX_LEASE_SECS)
            .contains(&self.lease_seconds)
        {
            return Err(ChannelEventAdmissionValidationError::Lease);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelEventClaimStatus {
    Acquired,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelEventClaimResponse {
    pub schema_version: u16,
    pub status: ChannelEventClaimStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at_ts: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelEventFinishOutcome {
    Completed,
    RetryableFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelEventFinishRequest {
    pub schema_version: u16,
    pub channel: ChannelKind,
    pub account_id: String,
    pub provider_event_id: String,
    pub payload_sha256: String,
    pub lease_token: String,
    pub outcome: ChannelEventFinishOutcome,
}

impl ChannelEventFinishRequest {
    pub fn validate(&self) -> Result<(), ChannelEventAdmissionValidationError> {
        if self.schema_version != CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION {
            return Err(ChannelEventAdmissionValidationError::SchemaVersion);
        }
        validate_identifier(&self.account_id, 256)?;
        validate_identifier(&self.provider_event_id, 512)?;
        validate_sha256(&self.payload_sha256)?;
        validate_identifier(&self.lease_token, 128)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelEventFinishStatus {
    Completed,
    Released,
    AlreadyCompleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelEventFinishResponse {
    pub schema_version: u16,
    pub status: ChannelEventFinishStatus,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ChannelEventAdmissionValidationError {
    #[error("channel_event_admission_schema_version_invalid")]
    SchemaVersion,
    #[error("channel_event_admission_identifier_invalid")]
    Identifier,
    #[error("channel_event_admission_digest_invalid")]
    Digest,
    #[error("channel_event_admission_timestamp_invalid")]
    Timestamp,
    #[error("channel_event_admission_lease_invalid")]
    Lease,
}

#[derive(Debug, Error)]
pub enum ChannelEventAdmissionClientError {
    #[error("channel_event_admission_request_invalid")]
    InvalidRequest,
    #[error("channel_event_admission_request_failed")]
    Request,
    #[error("channel_event_admission_http_status_{0}")]
    HttpStatus(u16),
    #[error("channel_event_admission_response_invalid")]
    InvalidResponse,
    #[error("channel_event_admission_rejected")]
    Rejected,
}

pub async fn claim_channel_event(
    client: &reqwest::Client,
    base_url: &str,
    secret: &str,
    request: &ChannelEventClaimRequest,
) -> Result<ChannelEventClaimResponse, ChannelEventAdmissionClientError> {
    request
        .validate()
        .map_err(|_| ChannelEventAdmissionClientError::InvalidRequest)?;
    send_signed_request(
        client,
        base_url,
        "/v1/internal/channel-ingress/claim",
        secret,
        request,
    )
    .await
}

pub async fn finish_channel_event(
    client: &reqwest::Client,
    base_url: &str,
    secret: &str,
    request: &ChannelEventFinishRequest,
) -> Result<ChannelEventFinishResponse, ChannelEventAdmissionClientError> {
    request
        .validate()
        .map_err(|_| ChannelEventAdmissionClientError::InvalidRequest)?;
    send_signed_request(
        client,
        base_url,
        "/v1/internal/channel-ingress/finish",
        secret,
        request,
    )
    .await
}

async fn send_signed_request<T, R>(
    client: &reqwest::Client,
    base_url: &str,
    path: &str,
    secret: &str,
    request: &T,
) -> Result<R, ChannelEventAdmissionClientError>
where
    T: Serialize,
    R: for<'de> Deserialize<'de>,
{
    if secret.trim().is_empty() {
        return Err(ChannelEventAdmissionClientError::InvalidRequest);
    }
    let body = serde_json::to_vec(request)
        .map_err(|_| ChannelEventAdmissionClientError::InvalidRequest)?;
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let mut attempt = 0_u8;
    let response = loop {
        attempt = attempt.saturating_add(1);
        let timestamp = std::time::SystemTime::UNIX_EPOCH
            .elapsed()
            .map_err(|_| ChannelEventAdmissionClientError::Request)?
            .as_secs();
        let signature = sign_admission_request(secret, timestamp, &body)
            .map_err(|_| ChannelEventAdmissionClientError::InvalidRequest)?;
        match client
            .post(&url)
            .header("content-type", "application/json")
            .header(CHANNEL_EVENT_ADMISSION_TIMESTAMP_HEADER, timestamp)
            .header(CHANNEL_EVENT_ADMISSION_SIGNATURE_HEADER, signature)
            .body(body.clone())
            .send()
            .await
        {
            Ok(response)
                if admission_retryable_status(response.status().as_u16())
                    && attempt < CHANNEL_EVENT_ADMISSION_REQUEST_ATTEMPTS => {}
            Ok(response) => break response,
            Err(_) if attempt < CHANNEL_EVENT_ADMISSION_REQUEST_ATTEMPTS => {}
            Err(_) => return Err(ChannelEventAdmissionClientError::Request),
        }
        tokio::time::sleep(std::time::Duration::from_millis(100 * u64::from(attempt))).await;
    };
    if !response.status().is_success() {
        return Err(ChannelEventAdmissionClientError::HttpStatus(
            response.status().as_u16(),
        ));
    }
    let response = response
        .json::<ApiResponse<R>>()
        .await
        .map_err(|_| ChannelEventAdmissionClientError::InvalidResponse)?;
    if !response.ok {
        return Err(ChannelEventAdmissionClientError::Rejected);
    }
    response
        .data
        .ok_or(ChannelEventAdmissionClientError::InvalidResponse)
}

fn admission_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429) || status >= 500
}

pub fn sign_admission_request(
    secret: &str,
    timestamp: u64,
    body: &[u8],
) -> Result<String, ChannelEventAdmissionValidationError> {
    if secret.trim().is_empty() || timestamp == 0 {
        return Err(ChannelEventAdmissionValidationError::Timestamp);
    }
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| ChannelEventAdmissionValidationError::Identifier)?;
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b"\n");
    mac.update(body);
    Ok(format!(
        "sha256={}",
        hex_lower(&mac.finalize().into_bytes())
    ))
}

pub fn verify_admission_request_signature(
    secret: &str,
    timestamp: u64,
    body: &[u8],
    signature: &str,
) -> bool {
    let Some(signature) = signature.trim().strip_prefix("sha256=") else {
        return false;
    };
    let Ok(signature) = decode_hex_32(signature) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b"\n");
    mac.update(body);
    mac.verify_slice(&signature).is_ok()
}

pub fn sha256_hex(payload: &[u8]) -> String {
    hex_lower(&Sha256::digest(payload))
}

fn validate_identifier(
    value: &str,
    max_len: usize,
) -> Result<(), ChannelEventAdmissionValidationError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > max_len
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(ChannelEventAdmissionValidationError::Identifier);
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), ChannelEventAdmissionValidationError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ChannelEventAdmissionValidationError::Digest);
    }
    Ok(())
}

fn decode_hex_32(value: &str) -> Result<[u8; 32], ()> {
    if value.len() != 64 {
        return Err(());
    }
    let mut bytes = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(chunk).map_err(|_| ())?;
        bytes[index] = u8::from_str_radix(text, 16).map_err(|_| ())?;
    }
    Ok(bytes)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
#[path = "channel_event_admission_tests.rs"]
mod tests;
