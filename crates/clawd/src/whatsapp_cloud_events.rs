use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tracing::{info, warn};

use crate::AppState;

type HmacSha256 = Hmac<Sha256>;

pub(crate) async fn handle_whatsapp_cloud_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if verify_signature(
        &state.channels.whatsapp_app_secret,
        &headers,
        "x-hub-signature-256",
        &body,
    )
    .is_err()
    {
        return (StatusCode::UNAUTHORIZED, "whatsapp_cloud_signature_invalid").into_response();
    }
    let payload = match serde_json::from_slice::<
        claw_core::channel_whatsapp_cloud::WhatsappWebhookPayload,
    >(&body)
    {
        Ok(payload) => payload,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "whatsapp_cloud_event_invalid").into_response();
        }
    };
    for status in payload.statuses() {
        let Some(delivery_status) = status.delivery_status() else {
            continue;
        };
        let Some(event_at_ts) = status.timestamp else {
            warn!(
                event = "whatsapp_cloud_status_timestamp_missing",
                provider_message_id = %status.id,
                "whatsapp_cloud_delivery_status_ignored"
            );
            continue;
        };
        match crate::repo::record_whatsapp_cloud_provider_status(
            &state.core.db,
            &status.id,
            delivery_status,
            event_at_ts,
            status.provider_error_code().as_deref(),
        ) {
            Ok(crate::repo::RecordWhatsappProviderStatusOutcome::UnknownMessage) => {
                info!(
                    event = "whatsapp_cloud_status_unknown_message",
                    provider_message_id = %status.id,
                    "whatsapp_cloud_delivery_status_unknown_message"
                );
            }
            Ok(_) => {}
            Err(error) => {
                warn!(
                    event = "whatsapp_cloud_status_record_failed",
                    provider_message_id = %status.id,
                    diagnostic = %error,
                    "whatsapp_cloud_delivery_status_record_failed"
                );
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "whatsapp_cloud_event_record_failed",
                )
                    .into_response();
            }
        }
    }
    (StatusCode::OK, "ok").into_response()
}

pub(crate) async fn handle_whatsapp_cloud_accepted(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if verify_signature(
        &state.channels.whatsapp_app_secret,
        &headers,
        "x-channel-event-signature-256",
        &body,
    )
    .is_err()
    {
        return (StatusCode::UNAUTHORIZED, "whatsapp_cloud_signature_invalid").into_response();
    }
    let event = match serde_json::from_slice::<
        claw_core::channel_whatsapp_cloud::WhatsappAcceptedDeliveryEvent,
    >(&body)
    {
        Ok(event) if event.validate() => event,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "whatsapp_cloud_accepted_event_invalid",
            )
                .into_response();
        }
    };
    let receipt = claw_core::channel_delivery::ChannelDeliveryReceipt {
        schema_version: claw_core::channel_delivery::CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION,
        delivery_id: event.delivery_id(),
        idempotency_key: event.idempotency_key(),
        channel: claw_core::types::ChannelKind::Whatsapp,
        adapter: "whatsapp_cloud".to_string(),
        status: claw_core::channel_delivery::ChannelDeliveryStatus::Accepted,
        provider_message_ids: event.provider_message_ids,
        parts: Vec::new(),
        error_code: None,
        message_key: None,
        diagnostic_id: None,
        provider_error_code: None,
        retryable: false,
        updated_at_ts: event.accepted_at_ts,
    };
    match crate::repo::record_channel_delivery_receipt(&state.core.db, &receipt) {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(error) => {
            warn!(
                event = "whatsapp_cloud_accepted_event_record_failed",
                diagnostic = %error,
                "whatsapp_cloud_accepted_event_record_failed"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "whatsapp_cloud_accepted_event_record_failed",
            )
                .into_response()
        }
    }
}

fn verify_signature(
    app_secret: &str,
    headers: &HeaderMap,
    header_name: &'static str,
    body: &[u8],
) -> Result<(), ()> {
    if app_secret.trim().is_empty() {
        return Err(());
    }
    let provided = headers
        .get(header_name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("sha256="))
        .and_then(|value| hex::decode(value).ok())
        .ok_or(())?;
    let mut mac = HmacSha256::new_from_slice(app_secret.as_bytes()).map_err(|_| ())?;
    mac.update(body);
    mac.verify_slice(&provided).map_err(|_| ())
}

#[cfg(test)]
#[path = "whatsapp_cloud_events_tests.rs"]
mod tests;
