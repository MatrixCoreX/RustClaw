use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceExt;

#[tokio::test]
async fn clawd_does_not_accept_browser_cross_origin_preflight() {
    let app = Router::new().route("/v1/tasks", post(|| async { StatusCode::NO_CONTENT }));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/v1/tasks")
                .header(header::ORIGIN, "http://127.0.0.1:3000")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(
                    header::ACCESS_CONTROL_REQUEST_HEADERS,
                    "content-type,if-none-match,x-agent-client,x-agent-key",
                )
                .body(Body::empty())
                .expect("build CORS preflight request"),
        )
        .await
        .expect("execute CORS preflight");

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(!response
        .headers()
        .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
}

#[tokio::test]
async fn clawd_leaves_browser_response_policy_to_webd() {
    let app = Router::new().route(
        "/v1/candles",
        get(|| async { ([(header::ETAG, "\"candles-v1\"")], StatusCode::OK) }),
    );
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/candles")
                .header(header::ORIGIN, "http://127.0.0.1:3000")
                .body(Body::empty())
                .expect("build CORS cache request"),
        )
        .await
        .expect("execute CORS cache request");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok()),
        Some("\"candles-v1\"")
    );
    assert!(!response
        .headers()
        .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
}
