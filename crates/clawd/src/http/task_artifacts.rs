use std::io::SeekFrom;
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::time::Duration;

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
use tokio::process::Command;
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
    preview: Option<String>,
}

const VIDEO_POSTER_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_VIDEO_POSTER_BYTES: usize = 8 * 1024 * 1024;
const VIDEO_BROWSER_PREVIEW_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_VIDEO_BROWSER_PREVIEW_BYTES: u64 = 256 * 1024 * 1024;

fn mime_has_top_level(mime_type: &str, expected: &str) -> bool {
    mime_type
        .split(';')
        .next()
        .and_then(|value| value.trim().split_once('/'))
        .is_some_and(|(top_level, _)| top_level.eq_ignore_ascii_case(expected))
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
    if query
        .preview
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("poster"))
    {
        return serve_video_poster(&path, &manifest, include_body).await;
    }
    if query
        .preview
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("browser"))
    {
        return serve_browser_video_preview(&path, &manifest, include_body).await;
    }
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

async fn serve_browser_video_preview(
    path: &FsPath,
    manifest: &TaskArtifactManifest,
    include_body: bool,
) -> Response {
    if !mime_has_top_level(&manifest.mime_type, "video") {
        return api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "task_artifact_browser_video_unsupported",
        );
    }
    let preview_path = match load_or_generate_browser_video(path, manifest).await {
        Ok(path) => path,
        Err(BrowserVideoError::ToolUnavailable) => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "task_artifact_browser_video_tool_unavailable",
            )
        }
        Err(BrowserVideoError::Timeout) => {
            return api_error(
                StatusCode::GATEWAY_TIMEOUT,
                "task_artifact_browser_video_timeout",
            )
        }
        Err(BrowserVideoError::TranscodeFailed | BrowserVideoError::InvalidOutput) => {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "task_artifact_browser_video_failed",
            )
        }
    };
    let file = match tokio::fs::File::open(&preview_path).await {
        Ok(file) => file,
        Err(error) => {
            tracing::warn!(
                "task artifact browser preview open failed path={} error={}",
                preview_path.display(),
                error
            );
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_artifact_browser_video_unavailable",
            );
        }
    };
    let content_length = match file.metadata().await {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        _ => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_artifact_browser_video_unavailable",
            )
        }
    };
    let filename = format!(
        "{}-browser-preview.mp4",
        manifest
            .filename
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(&manifest.filename)
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "video/mp4")
        .header(CONTENT_LENGTH, content_length.to_string())
        .header(CACHE_CONTROL, "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header(CONTENT_DISPOSITION, content_disposition(&filename, true))
        .header(ETAG, format!("\"{}-browser-v1\"", manifest.sha256))
        .body(if include_body {
            Body::from_stream(ReaderStream::new(file))
        } else {
            Body::empty()
        })
        .unwrap_or_else(|error| {
            tracing::error!("task artifact browser preview response build failed: {error}");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_artifact_browser_video_response_failed",
            )
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserVideoError {
    ToolUnavailable,
    Timeout,
    TranscodeFailed,
    InvalidOutput,
}

async fn load_or_generate_browser_video(
    path: &FsPath,
    manifest: &TaskArtifactManifest,
) -> Result<PathBuf, BrowserVideoError> {
    let cache_path =
        browser_video_cache_path(path, &manifest.sha256).ok_or(BrowserVideoError::InvalidOutput)?;
    if valid_browser_video_file(&cache_path).await {
        return Ok(cache_path);
    }

    let temp_path = cache_path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = generate_browser_video(path, &temp_path).await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(error);
    }
    if !valid_browser_video_file(&temp_path).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(BrowserVideoError::InvalidOutput);
    }
    if let Err(error) = tokio::fs::rename(&temp_path, &cache_path).await {
        tracing::warn!(
            "task artifact browser preview cache publish failed path={} error={}",
            cache_path.display(),
            error
        );
        let _ = tokio::fs::remove_file(&temp_path).await;
        if valid_browser_video_file(&cache_path).await {
            return Ok(cache_path);
        }
        return Err(BrowserVideoError::InvalidOutput);
    }
    Ok(cache_path)
}

async fn generate_browser_video(
    source_path: &FsPath,
    output_path: &FsPath,
) -> Result<(), BrowserVideoError> {
    let mut command = Command::new("ffmpeg");
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .args(["-hide_banner", "-loglevel", "error", "-threads", "1", "-i"])
        .arg(source_path)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-sn",
            "-dn",
            "-vf",
            "scale=w='min(1280,iw)':h=-2,pad=ceil(iw/2)*2:ceil(ih/2)*2",
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
            "-y",
        ])
        .arg(output_path);
    let output = match tokio::time::timeout(VIDEO_BROWSER_PREVIEW_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(BrowserVideoError::ToolUnavailable)
        }
        Ok(Err(error)) => {
            tracing::warn!("task artifact browser preview ffmpeg start failed: {error}");
            return Err(BrowserVideoError::TranscodeFailed);
        }
        Err(_) => return Err(BrowserVideoError::Timeout),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            "task artifact browser preview ffmpeg failed status={} error={}",
            output.status,
            stderr.chars().take(300).collect::<String>()
        );
        return Err(BrowserVideoError::TranscodeFailed);
    }
    Ok(())
}

fn browser_video_cache_path(path: &FsPath, sha256: &str) -> Option<PathBuf> {
    path.parent()
        .map(|parent| parent.join(format!(".video-browser-v1-{sha256}.mp4")))
}

async fn valid_browser_video_file(path: &FsPath) -> bool {
    let Ok(metadata) = tokio::fs::metadata(path).await else {
        return false;
    };
    if !metadata.is_file()
        || metadata.len() < 12
        || metadata.len() > MAX_VIDEO_BROWSER_PREVIEW_BYTES
    {
        return false;
    }
    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return false;
    };
    let mut header = [0_u8; 12];
    file.read_exact(&mut header).await.is_ok() && &header[4..8] == b"ftyp"
}

async fn serve_video_poster(
    path: &FsPath,
    manifest: &TaskArtifactManifest,
    include_body: bool,
) -> Response {
    if !mime_has_top_level(&manifest.mime_type, "video") {
        return api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "task_artifact_video_poster_unsupported",
        );
    }
    let poster = match load_or_generate_video_poster(path, manifest).await {
        Ok(poster) => poster,
        Err(VideoPosterError::ToolUnavailable) => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "task_artifact_video_poster_tool_unavailable",
            )
        }
        Err(VideoPosterError::Timeout) => {
            return api_error(
                StatusCode::GATEWAY_TIMEOUT,
                "task_artifact_video_poster_timeout",
            )
        }
        Err(VideoPosterError::DecodeFailed | VideoPosterError::InvalidOutput) => {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "task_artifact_video_poster_failed",
            )
        }
    };
    let filename = format!(
        "{}-preview.jpg",
        manifest
            .filename
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(&manifest.filename)
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "image/jpeg")
        .header(CONTENT_LENGTH, poster.len().to_string())
        .header(CACHE_CONTROL, "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header(CONTENT_DISPOSITION, content_disposition(&filename, true))
        .header(ETAG, format!("\"{}-poster-v1\"", manifest.sha256))
        .body(if include_body {
            Body::from(poster)
        } else {
            Body::empty()
        })
        .unwrap_or_else(|error| {
            tracing::error!("task artifact poster response build failed: {error}");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task_artifact_video_poster_response_failed",
            )
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VideoPosterError {
    ToolUnavailable,
    Timeout,
    DecodeFailed,
    InvalidOutput,
}

async fn load_or_generate_video_poster(
    path: &FsPath,
    manifest: &TaskArtifactManifest,
) -> Result<Vec<u8>, VideoPosterError> {
    let cache_path =
        video_poster_cache_path(path, &manifest.sha256).ok_or(VideoPosterError::InvalidOutput)?;
    if let Ok(cached) = tokio::fs::read(&cache_path).await {
        if valid_video_poster(&cached) {
            return Ok(cached);
        }
    }

    let poster = generate_video_poster(path).await?;
    let temp_path = cache_path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    if tokio::fs::write(&temp_path, &poster).await.is_ok() {
        if let Err(error) = tokio::fs::rename(&temp_path, &cache_path).await {
            tracing::warn!(
                "task artifact poster cache publish failed path={} error={}",
                cache_path.display(),
                error
            );
            let _ = tokio::fs::remove_file(temp_path).await;
        }
    }
    Ok(poster)
}

async fn generate_video_poster(path: &FsPath) -> Result<Vec<u8>, VideoPosterError> {
    let mut command = Command::new("ffmpeg");
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "1",
            "-ss",
            "0.05",
            "-i",
        ])
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-frames:v",
            "1",
            "-an",
            "-sn",
            "-dn",
            "-vf",
            "scale=1280:1280:force_original_aspect_ratio=decrease",
            "-q:v",
            "4",
            "-f",
            "image2pipe",
            "-vcodec",
            "mjpeg",
            "pipe:1",
        ]);
    let output = match tokio::time::timeout(VIDEO_POSTER_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(VideoPosterError::ToolUnavailable)
        }
        Ok(Err(error)) => {
            tracing::warn!("task artifact poster ffmpeg start failed: {error}");
            return Err(VideoPosterError::DecodeFailed);
        }
        Err(_) => return Err(VideoPosterError::Timeout),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            "task artifact poster ffmpeg failed status={} error={}",
            output.status,
            stderr.chars().take(300).collect::<String>()
        );
        return Err(VideoPosterError::DecodeFailed);
    }
    if !valid_video_poster(&output.stdout) {
        return Err(VideoPosterError::InvalidOutput);
    }
    Ok(output.stdout)
}

fn video_poster_cache_path(path: &FsPath, sha256: &str) -> Option<PathBuf> {
    path.parent()
        .map(|parent| parent.join(format!(".video-poster-v1-{sha256}.jpg")))
}

fn valid_video_poster(bytes: &[u8]) -> bool {
    bytes.len() >= 4
        && bytes.len() <= MAX_VIDEO_POSTER_BYTES
        && bytes.starts_with(&[0xff, 0xd8, 0xff])
        && bytes.ends_with(&[0xff, 0xd9])
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
    let provided_key = crate::auth_key_from_headers(headers);
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
