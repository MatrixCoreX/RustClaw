use std::io::SeekFrom;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
    ETAG, RANGE,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use claw_core::types::{ApiResponse, TaskQueryResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::repo::{check_task_view_access, get_task_query_record, TaskViewerAccessError};
use crate::task_artifacts::{self, TaskArtifactManifest};
use crate::AppState;

#[derive(Debug, Serialize)]
pub(crate) struct TaskArtifactList {
    schema_version: u32,
    task_id: String,
    artifacts: Vec<TaskArtifactManifest>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct TaskArtifactContentQuery {
    disposition: Option<String>,
}

pub(crate) async fn list_task_artifacts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(task_id): Path<Uuid>,
) -> Response {
    let task = match visible_task(&state, &headers, task_id) {
        Ok(task) => task,
        Err(response) => return response,
    };
    let artifacts = task_artifacts::manifests_from_result(task.result_json.as_ref());
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(TaskArtifactList {
                schema_version: task_artifacts::TASK_ARTIFACT_SCHEMA_VERSION,
                task_id: task_id.to_string(),
                artifacts,
            }),
            error: None,
        }),
    )
        .into_response()
}

pub(crate) async fn get_task_artifact_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((task_id, artifact_id)): Path<(Uuid, String)>,
    Query(query): Query<TaskArtifactContentQuery>,
) -> Response {
    serve_task_artifact(state, headers, task_id, artifact_id, query, true).await
}

pub(crate) async fn head_task_artifact_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((task_id, artifact_id)): Path<(Uuid, String)>,
    Query(query): Query<TaskArtifactContentQuery>,
) -> Response {
    serve_task_artifact(state, headers, task_id, artifact_id, query, false).await
}

async fn serve_task_artifact(
    state: AppState,
    headers: HeaderMap,
    task_id: Uuid,
    artifact_id: String,
    query: TaskArtifactContentQuery,
    include_body: bool,
) -> Response {
    let task = match visible_task(&state, &headers, task_id) {
        Ok(task) => task,
        Err(response) => return response,
    };
    let Some(manifest) =
        task_artifacts::manifest_by_id(task.result_json.as_ref(), artifact_id.trim())
    else {
        return api_error(StatusCode::NOT_FOUND, "task_artifact_not_found");
    };
    let Some(path) = task_artifacts::validated_delivery_artifact_path(
        &state.skill_rt.workspace_root,
        &task_id.to_string(),
        &manifest,
    ) else {
        return api_error(StatusCode::GONE, "task_artifact_unavailable");
    };
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(error) => {
            tracing::warn!(
                "task artifact open failed task_id={} artifact_id={} error={}",
                task_id,
                manifest.id,
                error
            );
            return api_error(StatusCode::GONE, "task_artifact_unavailable");
        }
    };
    let total_bytes = match file.metadata().await {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        _ => return api_error(StatusCode::GONE, "task_artifact_unavailable"),
    };
    let range = match requested_byte_range(headers.get(RANGE), total_bytes) {
        Ok(range) => range,
        Err(()) => return range_not_satisfiable(total_bytes),
    };
    let (status, start, end) = match range {
        Some((start, end)) => (StatusCode::PARTIAL_CONTENT, start, end),
        None if total_bytes > 0 => (StatusCode::OK, 0, total_bytes - 1),
        None => (StatusCode::OK, 0, 0),
    };
    let content_length = if total_bytes == 0 {
        0
    } else {
        end.saturating_sub(start).saturating_add(1)
    };
    if start > 0 && file.seek(SeekFrom::Start(start)).await.is_err() {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "task_artifact_seek_failed",
        );
    }

    let inline = query
        .disposition
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("inline"))
        && task_artifacts::inline_preview_allowed(&manifest.mime_type);
    let mut builder = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, manifest.mime_type.as_str())
        .header(CONTENT_LENGTH, content_length.to_string())
        .header(ACCEPT_RANGES, "bytes")
        .header(CACHE_CONTROL, "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header(
            CONTENT_DISPOSITION,
            content_disposition(&manifest.filename, inline),
        )
        .header(ETAG, format!("\"{}\"", manifest.sha256));
    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(CONTENT_RANGE, format!("bytes {start}-{end}/{total_bytes}"));
    }
    let body = if include_body && content_length > 0 {
        Body::from_stream(ReaderStream::new(file.take(content_length)))
    } else {
        Body::empty()
    };
    builder.body(body).unwrap_or_else(|error| {
        tracing::error!("task artifact response build failed: {error}");
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "task_artifact_response_failed",
        )
    })
}

fn visible_task(
    state: &AppState,
    headers: &HeaderMap,
    task_id: Uuid,
) -> Result<TaskQueryResponse, Response> {
    let Some((task, task_user_key, channel)) = (match get_task_query_record(state, task_id) {
        Ok(record) => record,
        Err(error) => {
            tracing::error!("task artifact task lookup failed: {error}");
            return Err(api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_artifact_store_error",
            ));
        }
    }) else {
        return Err(api_error(StatusCode::NOT_FOUND, "task_not_found"));
    };
    let provided_key = headers
        .get("x-rustclaw-key")
        .and_then(|value| value.to_str().ok());
    match check_task_view_access(state, task_user_key.as_deref(), &channel, provided_key) {
        Ok(()) => Ok(task),
        Err(TaskViewerAccessError::AuthLookup(error)) => {
            tracing::error!("task artifact auth lookup failed: {error}");
            Err(api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_artifact_auth_lookup_failed",
            ))
        }
        Err(TaskViewerAccessError::TaskOwnerMismatch) => {
            Err(api_error(StatusCode::UNAUTHORIZED, "task_owner_mismatch"))
        }
        Err(TaskViewerAccessError::InvalidUserKey) => {
            Err(api_error(StatusCode::UNAUTHORIZED, "auth_key_invalid"))
        }
    }
}

fn requested_byte_range(
    range: Option<&HeaderValue>,
    total_bytes: u64,
) -> Result<Option<(u64, u64)>, ()> {
    let Some(range) = range else {
        return Ok(None);
    };
    if total_bytes == 0 {
        return Err(());
    }
    let value = range.to_str().map_err(|_| ())?.trim();
    let value = value.strip_prefix("bytes=").ok_or(())?;
    if value.contains(',') {
        return Err(());
    }
    let (start, end) = value.split_once('-').ok_or(())?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        let start = total_bytes.saturating_sub(suffix.min(total_bytes));
        return Ok(Some((start, total_bytes - 1)));
    }
    let start = start.parse::<u64>().map_err(|_| ())?;
    if start >= total_bytes {
        return Err(());
    }
    let end = if end.is_empty() {
        total_bytes - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(total_bytes - 1)
    };
    if end < start {
        return Err(());
    }
    Ok(Some((start, end)))
}

fn content_disposition(filename: &str, inline: bool) -> String {
    let disposition = if inline { "inline" } else { "attachment" };
    let fallback = filename
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!(
        "{disposition}; filename=\"{}\"; filename*=UTF-8''{}",
        fallback.trim_matches('.'),
        percent_encode_utf8(filename)
    )
}

fn percent_encode_utf8(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b'-') {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn range_not_satisfiable(total_bytes: u64) -> Response {
    Response::builder()
        .status(StatusCode::RANGE_NOT_SATISFIABLE)
        .header(CONTENT_RANGE, format!("bytes */{total_bytes}"))
        .header(ACCEPT_RANGES, "bytes")
        .body(Body::empty())
        .unwrap_or_else(|_| api_error(StatusCode::RANGE_NOT_SATISFIABLE, "invalid_range"))
}

fn api_error(status: StatusCode, error: &'static str) -> Response {
    (
        status,
        Json(ApiResponse::<Value> {
            ok: false,
            data: None,
            error: Some(error.to_string()),
        }),
    )
        .into_response()
}

#[cfg(test)]
#[path = "task_artifacts_tests.rs"]
mod tests;
