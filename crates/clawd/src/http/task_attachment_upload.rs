use crate::{
    ui_attachments::{self, UploadInput},
    AppState,
};
use axum::{
    extract::{Multipart, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::OnceLock;
use tokio::sync::Semaphore;

pub(crate) async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(client_id): Path<String>,
    multipart: Multipart,
) -> Response {
    let identity = match crate::require_auth_identity_for_api::<serde_json::Value>(&state, &headers)
    {
        Ok(identity) => identity,
        Err(response) => return response.into_response(),
    };
    if client_id.is_empty() || client_id.len() > 512 {
        return error(StatusCode::BAD_REQUEST, "ui_attachment_client_id_invalid");
    }
    static GATE: OnceLock<Semaphore> = OnceLock::new();
    let Ok(_permit) = GATE.get_or_init(|| Semaphore::new(1)).try_acquire() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "ui_attachment_upload_busy");
    };
    let root = &state.skill_rt.workspace_root;
    let available = fs2::available_space(root).unwrap_or(0);
    if available < 2 * 1024 * 1024 * 1024 + ui_attachments::MAX_UI_TOTAL_ATTACHMENT_BYTES as u64 {
        return error(
            StatusCode::INSUFFICIENT_STORAGE,
            "ui_attachment_storage_pressure",
        );
    }
    let inputs = match tokio::time::timeout(
        std::time::Duration::from_secs(120),
        read_multipart(multipart),
    )
    .await
    {
        Ok(Ok(inputs)) => inputs,
        Ok(Err(response)) => return response,
        Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "ui_attachment_upload_timeout"),
    };
    match ui_attachments::commit_upload_batch(
        root,
        identity.user_id,
        identity.chat_id,
        &client_id,
        inputs,
    ) {
        Ok(items) => Json(json!({"ok":true,"data":items,"error":null})).into_response(),
        Err(code) => error(
            if code == "ui_attachment_replay_conflict" {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            },
            &code,
        ),
    }
}
fn error(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({"ok":false,"data":null,"error":code}))).into_response()
}

async fn read_multipart(mut multipart: Multipart) -> Result<Vec<UploadInput>, Response> {
    let mut inputs = Vec::new();
    let mut total = 0usize;
    loop {
        let next = match multipart.next_field().await {
            Ok(next) => next,
            Err(_) => {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "ui_attachment_multipart_invalid",
                ))
            }
        };
        let Some(mut field) = next else {
            break;
        };
        if inputs.len() >= ui_attachments::MAX_UI_ATTACHMENTS {
            return Err(error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "ui_attachments_too_many",
            ));
        }
        let mut input = UploadInput {
            name: field.file_name().unwrap_or("attachment").to_owned(),
            mime_type: field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_owned(),
            kind: field.name().unwrap_or("file").to_owned(),
            bytes: Vec::new(),
        };
        loop {
            let chunk = match field.chunk().await {
                Ok(chunk) => chunk,
                Err(_) => {
                    return Err(error(
                        StatusCode::BAD_REQUEST,
                        "ui_attachment_stream_failed",
                    ))
                }
            };
            let Some(chunk) = chunk else {
                break;
            };
            total = total.saturating_add(chunk.len());
            if input.bytes.len().saturating_add(chunk.len())
                > ui_attachments::MAX_UI_ATTACHMENT_BYTES
            {
                return Err(error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "ui_attachment_too_large",
                ));
            }
            if total > ui_attachments::MAX_UI_TOTAL_ATTACHMENT_BYTES {
                return Err(error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "ui_attachments_total_too_large",
                ));
            }
            input.bytes.extend_from_slice(&chunk);
        }
        inputs.push(input);
    }
    Ok(inputs)
}

#[cfg(test)]
#[path = "task_attachment_upload_tests.rs"]
mod tests;
