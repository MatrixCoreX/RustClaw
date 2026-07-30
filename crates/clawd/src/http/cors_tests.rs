use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::routing::post;
use axum::Router;
use tower::ServiceExt;

#[tokio::test]
async fn task_submit_preflight_accepts_ui_client_header() {
    let app = Router::new()
        .route("/v1/tasks", post(|| async { StatusCode::NO_CONTENT }))
        .layer(super::api_cors_layer());
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/v1/tasks")
                .header(header::ORIGIN, "http://127.0.0.1:3000")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(
                    header::ACCESS_CONTROL_REQUEST_HEADERS,
                    "content-type,x-agent-client,x-agent-key",
                )
                .body(Body::empty())
                .expect("build CORS preflight request"),
        )
        .await
        .expect("execute CORS preflight");

    assert_eq!(response.status(), StatusCode::OK);
    let allowed = response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
        .and_then(|value| value.to_str().ok())
        .expect("access-control-allow-headers");
    for expected in ["content-type", "x-agent-client", "x-agent-key"] {
        assert!(
            allowed.split(',').any(|value| value.trim() == expected),
            "missing {expected} in {allowed}"
        );
    }
}
