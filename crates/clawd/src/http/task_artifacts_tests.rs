use axum::body::to_bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use serde_json::json;

use super::*;

#[test]
fn parses_single_open_closed_and_suffix_ranges() {
    assert_eq!(requested_byte_range(None, 100), Ok(None));
    assert_eq!(
        requested_byte_range(Some(&HeaderValue::from_static("bytes=10-19")), 100),
        Ok(Some((10, 19)))
    );
    assert_eq!(
        requested_byte_range(Some(&HeaderValue::from_static("bytes=90-")), 100),
        Ok(Some((90, 99)))
    );
    assert_eq!(
        requested_byte_range(Some(&HeaderValue::from_static("bytes=-8")), 100),
        Ok(Some((92, 99)))
    );
}

#[test]
fn rejects_multiple_and_unsatisfiable_ranges() {
    assert_eq!(
        requested_byte_range(Some(&HeaderValue::from_static("bytes=0-1,4-5")), 100),
        Err(())
    );
    assert_eq!(
        requested_byte_range(Some(&HeaderValue::from_static("bytes=100-")), 100),
        Err(())
    );
    assert_eq!(
        requested_byte_range(Some(&HeaderValue::from_static("bytes=8-2")), 100),
        Err(())
    );
}

#[test]
fn content_disposition_keeps_unicode_only_in_rfc5987_value() {
    let header = content_disposition("月报 2026.pdf", false);
    assert!(header.starts_with("attachment; filename=\"___2026.pdf\""));
    assert!(header.contains("filename*=UTF-8''%E6%9C%88%E6%8A%A5%202026.pdf"));
}

#[test]
fn video_poster_validation_accepts_only_bounded_complete_jpeg_data() {
    assert!(valid_video_poster(&[0xff, 0xd8, 0xff, 0xd9]));
    assert!(!valid_video_poster(&[0xff, 0xd8, 0xff, 0x00]));
    assert!(!valid_video_poster(&[0x89, b'P', b'N', b'G']));

    let path = std::path::Path::new("/tmp/task-artifact/video.mp4");
    assert_eq!(
        video_poster_cache_path(path, "abc"),
        Some(std::path::PathBuf::from(
            "/tmp/task-artifact/.video-poster-v1-abc.jpg"
        ))
    );
}

#[test]
fn browser_video_cache_is_separate_from_the_original_download() {
    let path = std::path::Path::new("/tmp/task-artifact/video.mp4");
    assert_eq!(
        browser_video_cache_path(path, "abc"),
        Some(std::path::PathBuf::from(
            "/tmp/task-artifact/.video-browser-v1-abc.mp4"
        ))
    );
    assert_ne!(browser_video_cache_path(path, "abc").as_deref(), Some(path));
}

#[test]
fn video_mime_top_level_is_case_insensitive_and_ignores_parameters() {
    assert!(mime_has_top_level(
        " VIDEO/mp4; codecs=\"avc1.42E01E\" ",
        "video"
    ));
    assert!(!mime_has_top_level("application/video", "video"));
    assert!(!mime_has_top_level("video", "video"));
}

#[tokio::test]
async fn video_poster_rejects_non_video_artifacts_before_running_ffmpeg() {
    let manifest = TaskArtifactManifest {
        schema_version: 1,
        id: "artifact-1".to_string(),
        artifact_ref: String::new(),
        filename: "report.txt".to_string(),
        kind: "file".to_string(),
        mime_type: "text/plain".to_string(),
        size_bytes: 4,
        sha256: "a".repeat(64),
        download_url: "/v1/tasks/task-1/artifacts/artifact-1/content".to_string(),
        preview_url: None,
    };
    let response = serve_video_poster(std::path::Path::new("/not-used"), &manifest, true).await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn browser_video_rejects_non_video_artifacts_before_running_ffmpeg() {
    let manifest = TaskArtifactManifest {
        schema_version: 1,
        id: "artifact-1".to_string(),
        artifact_ref: String::new(),
        filename: "report.txt".to_string(),
        kind: "file".to_string(),
        mime_type: "text/plain".to_string(),
        size_bytes: 4,
        sha256: "a".repeat(64),
        download_url: "/v1/tasks/task-1/artifacts/artifact-1/content".to_string(),
        preview_url: None,
    };
    let response =
        serve_browser_video_preview(std::path::Path::new("/not-used"), &manifest, true).await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn endpoint_enforces_task_ownership_and_streams_requested_range() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-task-artifact-http-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    let task_id = uuid::Uuid::new_v4();
    let manifest = TaskArtifactManifest {
        schema_version: 1,
        id: "artifact-1".to_string(),
        artifact_ref: String::new(),
        filename: "report.txt".to_string(),
        kind: "file".to_string(),
        mime_type: "text/plain; charset=utf-8".to_string(),
        size_bytes: 10,
        sha256: "a".repeat(64),
        download_url: format!("/v1/tasks/{task_id}/artifacts/artifact-1/content"),
        preview_url: None,
    };
    let path = task_artifacts::delivery_artifact_path(
        &root,
        &task_id.to_string(),
        &manifest.id,
        &manifest.filename,
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"0123456789").unwrap();
    state
        .core
        .db
        .get()
        .unwrap()
        .execute_batch(
            "CREATE TABLE tasks (
                task_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                result_json TEXT,
                error_text TEXT,
                user_key TEXT,
                channel TEXT NOT NULL,
                updated_at TEXT,
                lease_owner TEXT,
                lease_expires_at INTEGER NOT NULL DEFAULT 0,
                claim_attempt INTEGER NOT NULL DEFAULT 0,
                claimed_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE auth_keys (
                user_key TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                last_used_at TEXT
            );",
        )
        .unwrap();
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "INSERT INTO tasks (
                task_id, status, payload_json, result_json, user_key, channel, updated_at
             ) VALUES (?1, 'succeeded', '{}', ?2, 'owner-key', 'ui', '1')",
            rusqlite::params![
                task_id.to_string(),
                json!({"text": "ready", "artifacts": [manifest]}).to_string()
            ],
        )
        .unwrap();
    state.seed_test_auth_identity("owner-key", "user");

    let mut wrong_headers = HeaderMap::new();
    wrong_headers.insert("x-agent-key", HeaderValue::from_static("wrong-key"));
    let denied = list_task_artifacts(State(state.clone()), wrong_headers, Path(task_id)).await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    let mut headers = HeaderMap::new();
    headers.insert("x-agent-key", HeaderValue::from_static("owner-key"));
    headers.insert(RANGE, HeaderValue::from_static("bytes=2-5"));
    let response = get_task_artifact_content(
        State(state),
        headers,
        Path((task_id, "artifact-1".to_string())),
        Query(TaskArtifactContentQuery::default()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()[CONTENT_RANGE], "bytes 2-5/10");
    let body = to_bytes(response.into_body(), 16).await.unwrap();
    assert_eq!(&body[..], b"2345");
    std::fs::remove_dir_all(root).unwrap();
}
