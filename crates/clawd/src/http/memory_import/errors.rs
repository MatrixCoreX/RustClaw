use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::Value;

use crate::memory::import_upload::{self as upload, Failure, FailureKind};
use crate::ApiResponse;

use super::super::writing;

fn status(kind: FailureKind) -> StatusCode {
    match kind {
        FailureKind::BadRequest => StatusCode::BAD_REQUEST,
        FailureKind::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        FailureKind::NotFound => StatusCode::NOT_FOUND,
        FailureKind::Conflict => StatusCode::CONFLICT,
        FailureKind::Unavailable | FailureKind::Busy => StatusCode::SERVICE_UNAVAILABLE,
        FailureKind::StorageFull => StatusCode::INSUFFICIENT_STORAGE,
        FailureKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
pub(super) fn reject(status: StatusCode, code: &'static str) -> Response {
    crate::api_err::<Value>(status, code).into_response()
}
pub(super) fn failure(error: anyhow::Error) -> Response {
    if let Some(reply) = writing::execution_error::<Value>(&error) {
        return reply.into_response();
    }
    let failure = upload::failure(&error).unwrap_or(Failure {
        kind: FailureKind::Internal,
        code: "memory_import_failed",
        retry_after_seconds: None,
    });
    tracing::warn!(error=%error,error_code=failure.code,"memory_import_request_failed");
    let mut reply = reject(status(failure.kind), failure.code);
    if let Some(seconds) = failure.retry_after_seconds {
        if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
            reply.headers_mut().insert(header::RETRY_AFTER, value);
        }
    }
    reply
}
pub(in crate::http::memory_routes) fn typed_upload_error<T: Serialize>(
    error: &anyhow::Error,
) -> Option<(StatusCode, Json<ApiResponse<T>>)> {
    if !error
        .chain()
        .any(|cause| cause.downcast_ref::<upload::UploadError>().is_some())
    {
        return None;
    }
    let failure = upload::failure(error)?;
    Some(crate::api_err(status(failure.kind), failure.code))
}
