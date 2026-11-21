use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
    routing::post,
    Router,
};
use tower::ServiceExt;
fn multipart(size: usize) -> Vec<u8> {
    let mut body=b"--boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"large.txt\"\r\nContent-Type: text/plain\r\n\r\n".to_vec();
    body.extend(std::iter::repeat_n(b'x', size));
    body.extend_from_slice(b"\r\n--boundary--\r\n");
    body
}
#[tokio::test]
async fn real_multipart_supports_twenty_mib_and_rejects_oversize_and_replay_changes() {
    let root = tempfile::tempdir().unwrap();
    let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.skill_rt.workspace_root = root.path().into();
    state.seed_test_auth_identity("upload-owner", "user");
    let app = Router::new()
        .route(
            "/upload/:client_id",
            post(upload).layer(axum::extract::DefaultBodyLimit::max(61 * 1024 * 1024)),
        )
        .with_state(state);
    let send = |size| {
        Request::builder()
            .method("POST")
            .uri("/upload/stable-message")
            .header("x-agent-key", "upload-owner")
            .header("content-type", "multipart/form-data; boundary=boundary")
            .body(Body::from(multipart(size)))
            .unwrap()
    };
    let response = app.clone().oneshot(send(20 * 1024 * 1024)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    let path = body["data"][0]["path"].as_str().unwrap();
    assert_eq!(
        std::fs::metadata(root.path().join(path)).unwrap().len(),
        20 * 1024 * 1024
    );
    let response = app
        .clone()
        .oneshot(send(20 * 1024 * 1024 + 1))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let response = app.clone().oneshot(send(8)).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        std::fs::metadata(root.path().join(path)).unwrap().len(),
        20 * 1024 * 1024
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/upload/unauthorized")
                .header("content-type", "multipart/form-data; boundary=boundary")
                .body(Body::from(multipart(4)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
