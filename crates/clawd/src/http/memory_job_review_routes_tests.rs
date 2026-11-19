use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use tower::ServiceExt;
#[tokio::test]
async fn review_routes_require_identity_and_never_expose_another_principals_jobs() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_test_auth_identity("review-user", "user");
    let app = router().with_state(state);
    let anonymous = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/memory/jobs/review")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/memory/jobs/review")
                .header("x-agent-key", "review-user")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert!(value["data"]["jobs"].as_array().unwrap().is_empty());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/memory/jobs/missing/disposition")
                .header("x-agent-key", "review-user")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"action":"archive","review_token":"a".repeat(64)})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}
