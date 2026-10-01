use std::path::PathBuf;

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::{header, Request, StatusCode};
use axum::routing::post;
use axum::Router;
use tower::ServiceExt;

use super::{sanitize_upload_relative_path, stream_multipart_field_to_file};

async fn store_uploaded_files(State(root): State<PathBuf>, mut multipart: Multipart) -> StatusCode {
    let mut total_bytes = 0usize;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => return StatusCode::NO_CONTENT,
            Err(_) => return StatusCode::BAD_REQUEST,
        };
        let Some(relative_path) = field.file_name().and_then(sanitize_upload_relative_path) else {
            continue;
        };
        let target = root.join(relative_path);
        if let Err(error) = stream_multipart_field_to_file(field, &target, &mut total_bytes).await {
            return error.status;
        }
    }
}

fn multipart_body(boundary: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, bytes) in files {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

#[tokio::test]
async fn multipart_skill_upload_streams_beyond_axum_default_body_limit() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-skill-upload-stream-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("create upload fixture root");
    let app = Router::new()
        .route(
            "/upload",
            post(store_uploaded_files).layer(DefaultBodyLimit::disable()),
        )
        .with_state(root.clone());
    let payload = vec![0x5au8; 3 * 1024 * 1024];
    let boundary = "agent-runtime-upload-boundary";
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(multipart_body(
                    boundary,
                    &[("bundle/payload.bin", payload.as_slice())],
                )))
                .expect("build multipart upload request"),
        )
        .await
        .expect("run multipart upload route");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        std::fs::metadata(root.join("bundle/payload.bin"))
            .expect("read uploaded file metadata")
            .len(),
        payload.len() as u64
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn multipart_skill_upload_rejects_duplicate_paths_without_overwrite() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-skill-upload-duplicate-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("create upload fixture root");
    let app = Router::new()
        .route(
            "/upload",
            post(store_uploaded_files).layer(DefaultBodyLimit::disable()),
        )
        .with_state(root.clone());
    let boundary = "agent-runtime-upload-duplicate-boundary";
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/upload")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(multipart_body(
                    boundary,
                    &[
                        ("bundle/file.txt", b"first"),
                        ("bundle/file.txt", b"second"),
                    ],
                )))
                .expect("build duplicate multipart request"),
        )
        .await
        .expect("run duplicate multipart route");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        std::fs::read(root.join("bundle/file.txt")).expect("read first uploaded file"),
        Bytes::from_static(b"first")
    );
    let _ = std::fs::remove_dir_all(root);
}
