use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use super::build_ui_router;
use crate::AppState;

const TEST_KEY: &str = "rk-git-config-test";

async fn request(
    router: axum::Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-agent-key", TEST_KEY);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = router.oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn git_connection_and_credential_api_is_write_only_and_revision_guarded() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let workspace = std::env::temp_dir().join(format!(
        "agent-runtime-git-config-route-test-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let mut state = state;
    state.skill_rt.workspace_root = workspace.clone();
    state.seed_test_auth_identity(TEST_KEY, "admin");
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state);

    let (status, initial) = request(router.clone(), Method::GET, "/v1/git/connections", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initial["data"]["revision"], 0);

    let (status, saved) = request(
        router.clone(),
        Method::POST,
        "/v1/git/connections",
        Some(json!({
            "expected_revision": 0,
            "id": "primary",
            "allowed_owners": ["ExampleOwner"],
            "allowed_repositories": ["example-repository"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["data"]["revision"], 1);

    let secret = "fixture-github-token-never-return";
    let (status, credential) = request(
        router.clone(),
        Method::POST,
        "/v1/git/credentials/github_git_token",
        Some(json!({"value": secret})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(credential.to_string().contains("\"configured\":true"));
    assert!(!credential.to_string().contains(secret));

    let (status, conflict) = request(
        router.clone(),
        Method::POST,
        "/v1/git/connections",
        Some(json!({
            "expected_revision": 0,
            "id": "other",
            "allowed_owners": ["ExampleOwner"],
            "allowed_repositories": ["example-repository"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(conflict["error"], "git_connection_revision_conflict");

    let (status, removed) = request(
        router,
        Method::DELETE,
        "/v1/git/credentials/github_git_token",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!removed.to_string().contains(secret));
    let _ = std::fs::remove_dir_all(workspace);
}
