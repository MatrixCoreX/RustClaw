use std::time::Duration;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::Response;
use futures_util::StreamExt;

use super::errors;

pub(super) async fn read(
    request: Request,
    limit: usize,
    seconds: u64,
) -> Result<Vec<u8>, Response> {
    if limit == 0 || seconds == 0 {
        return Err(errors::reject(
            StatusCode::SERVICE_UNAVAILABLE,
            "memory_import_ingress_unavailable",
        ));
    }
    if let Some(length) = request.headers().get(axum::http::header::CONTENT_LENGTH) {
        let length = length
            .to_str()
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| {
                errors::reject(StatusCode::BAD_REQUEST, "memory_import_request_invalid")
            })?;
        if length > limit as u64 {
            return Err(errors::reject(
                StatusCode::PAYLOAD_TOO_LARGE,
                "memory_import_upload_chunk_too_large",
            ));
        }
    }
    let receive = async move {
        let mut stream = request.into_body().into_data_stream();
        let mut bytes = Vec::new();
        while let Some(frame) = stream.next().await {
            let frame = frame.map_err(|_| {
                errors::reject(StatusCode::BAD_REQUEST, "memory_import_body_read_failed")
            })?;
            if bytes
                .len()
                .checked_add(frame.len())
                .map_or(true, |total| total > limit)
            {
                return Err(errors::reject(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "memory_import_upload_chunk_too_large",
                ));
            }
            bytes.try_reserve(frame.len()).map_err(|_| {
                errors::reject(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "memory_import_buffer_unavailable",
                )
            })?;
            bytes.extend_from_slice(&frame);
        }
        Ok(bytes)
    };
    // Only inbound byte receipt has this existing network deadline. Parsing and
    // whole confirmation run independently and are never killed by this timer.
    tokio::time::timeout(Duration::from_secs(seconds), receive)
        .await
        .map_err(|_| {
            errors::reject(
                StatusCode::REQUEST_TIMEOUT,
                "memory_import_body_read_timeout",
            )
        })?
}
