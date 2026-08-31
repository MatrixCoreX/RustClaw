use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use claw_core::channel_event_admission::{
    verify_admission_request_signature, ChannelEventClaimRequest, ChannelEventClaimResponse,
    ChannelEventClaimStatus, ChannelEventFinishRequest, ChannelEventFinishResponse,
    ChannelEventFinishStatus, CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
    CHANNEL_EVENT_ADMISSION_SIGNATURE_HEADER, CHANNEL_EVENT_ADMISSION_SIGNATURE_TOLERANCE_SECS,
    CHANNEL_EVENT_ADMISSION_TIMESTAMP_HEADER,
};
use claw_core::types::{ApiResponse, ChannelKind};

use crate::repo::{
    ChannelEventAdmissionError, ClaimChannelEventOutcome, FinishChannelEventOutcome,
};
use crate::AppState;

pub(crate) async fn claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<ApiResponse<ChannelEventClaimResponse>>) {
    let request = match serde_json::from_slice::<ChannelEventClaimRequest>(&body) {
        Ok(request) if request.validate().is_ok() => request,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "channel_event_admission_request_invalid",
            )
        }
    };
    if !authenticate(
        &state,
        &headers,
        &body,
        request.channel,
        &request.account_id,
    ) {
        return error(
            StatusCode::UNAUTHORIZED,
            "channel_event_admission_signature_invalid",
        );
    }
    match crate::repo::claim_channel_event(&state.core.db, &request) {
        Ok(ClaimChannelEventOutcome::Acquired {
            lease_token,
            lease_expires_at_ts,
        }) => success(ChannelEventClaimResponse {
            schema_version: CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
            status: ChannelEventClaimStatus::Acquired,
            lease_token: Some(lease_token),
            lease_expires_at_ts: Some(lease_expires_at_ts),
        }),
        Ok(ClaimChannelEventOutcome::InProgress {
            lease_expires_at_ts,
        }) => success(ChannelEventClaimResponse {
            schema_version: CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
            status: ChannelEventClaimStatus::InProgress,
            lease_token: None,
            lease_expires_at_ts: Some(lease_expires_at_ts),
        }),
        Ok(ClaimChannelEventOutcome::Completed) => success(ChannelEventClaimResponse {
            schema_version: CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
            status: ChannelEventClaimStatus::Completed,
            lease_token: None,
            lease_expires_at_ts: None,
        }),
        Err(error_value) => admission_error(error_value),
    }
}

pub(crate) async fn finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<ApiResponse<ChannelEventFinishResponse>>) {
    let request = match serde_json::from_slice::<ChannelEventFinishRequest>(&body) {
        Ok(request) if request.validate().is_ok() => request,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "channel_event_admission_request_invalid",
            )
        }
    };
    if !authenticate(
        &state,
        &headers,
        &body,
        request.channel,
        &request.account_id,
    ) {
        return error(
            StatusCode::UNAUTHORIZED,
            "channel_event_admission_signature_invalid",
        );
    }
    match crate::repo::finish_channel_event(&state.core.db, &request) {
        Ok(outcome) => success(ChannelEventFinishResponse {
            schema_version: CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
            status: match outcome {
                FinishChannelEventOutcome::Completed => ChannelEventFinishStatus::Completed,
                FinishChannelEventOutcome::Released => ChannelEventFinishStatus::Released,
                FinishChannelEventOutcome::AlreadyCompleted => {
                    ChannelEventFinishStatus::AlreadyCompleted
                }
            },
        }),
        Err(error_value) => admission_error(error_value),
    }
}

fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    body: &[u8],
    channel: ChannelKind,
    account_id: &str,
) -> bool {
    let timestamp = headers
        .get(CHANNEL_EVENT_ADMISSION_TIMESTAMP_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let signature = headers
        .get(CHANNEL_EVENT_ADMISSION_SIGNATURE_HEADER)
        .and_then(|value| value.to_str().ok());
    let (Some(timestamp), Some(signature), Some(secret)) = (
        timestamp,
        signature,
        admission_secret(state, channel, account_id),
    ) else {
        return false;
    };
    if !timestamp_is_fresh(crate::now_ts_u64(), timestamp) {
        return false;
    }
    verify_admission_request_signature(&secret, timestamp, body, signature)
}

fn timestamp_is_fresh(now: u64, timestamp: u64) -> bool {
    timestamp != 0 && now.abs_diff(timestamp) <= CHANNEL_EVENT_ADMISSION_SIGNATURE_TOLERANCE_SECS
}

fn admission_secret(state: &AppState, channel: ChannelKind, account_id: &str) -> Option<String> {
    let account_id = account_id.trim();
    match channel {
        ChannelKind::Telegram => state
            .channels
            .telegram_bot_tokens
            .get(account_id)
            .map(String::as_str)
            .map(str::trim)
            .filter(|secret| !secret.is_empty())
            .map(str::to_string),
        ChannelKind::Whatsapp => (state.channels.whatsapp_phone_number_id.trim() == account_id)
            .then_some(state.channels.whatsapp_app_secret.trim())
            .filter(|secret| !secret.is_empty())
            .map(str::to_string),
        ChannelKind::Feishu => state
            .channels
            .feishu_send_config
            .as_ref()
            .filter(|config| config.app_id == account_id)
            .map(|config| config.app_secret.trim())
            .filter(|secret| !secret.is_empty())
            .map(str::to_string),
        ChannelKind::Lark => state
            .channels
            .lark_send_config
            .as_ref()
            .filter(|config| config.app_id == account_id)
            .map(|config| config.app_secret.trim())
            .filter(|secret| !secret.is_empty())
            .map(str::to_string),
        ChannelKind::Wechat => crate::channel_send::resolve_wechat_send_config(state)
            .map(|config| config.bot_token.trim().to_string())
            .filter(|secret| !secret.is_empty()),
        ChannelKind::Ui => None,
    }
}

fn admission_error<T: serde::Serialize>(
    error_value: ChannelEventAdmissionError,
) -> (StatusCode, Json<ApiResponse<T>>) {
    let status = match error_value {
        ChannelEventAdmissionError::InvalidRequest => StatusCode::BAD_REQUEST,
        ChannelEventAdmissionError::ReceiptNotFound => StatusCode::NOT_FOUND,
        ChannelEventAdmissionError::PayloadConflict
        | ChannelEventAdmissionError::NonceConflict
        | ChannelEventAdmissionError::LeaseMismatch
        | ChannelEventAdmissionError::LeaseExpired => StatusCode::CONFLICT,
        ChannelEventAdmissionError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error(status, error_value.to_string())
}

fn success<T: serde::Serialize>(data: T) -> (StatusCode, Json<ApiResponse<T>>) {
    crate::api_ok(data)
}

fn error<T: serde::Serialize>(
    status: StatusCode,
    error_code: impl Into<String>,
) -> (StatusCode, Json<ApiResponse<T>>) {
    crate::api_err(status, error_code)
}

#[cfg(test)]
#[path = "channel_event_admission_tests.rs"]
mod tests;
