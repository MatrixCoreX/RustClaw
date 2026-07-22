use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::routing::post;
use axum::Router;
use tokio::net::TcpListener;

use super::{build_outgoing_headers, proxy_inner, uses_long_running_upstream_wait, AppState};

#[test]
fn web_session_key_overrides_client_key_and_preserves_ui_origin() {
    let mut incoming = HeaderMap::new();
    incoming.insert("x-rustclaw-key", HeaderValue::from_static("client-key"));
    incoming.insert("x-rustclaw-client", HeaderValue::from_static("ui"));

    let outgoing = build_outgoing_headers(
        &incoming,
        "127.0.0.1:8080",
        SocketAddr::from(([127, 0, 0, 1], 41000)),
        true,
        Some("session-admin-key"),
    );

    assert_eq!(
        outgoing
            .get("x-rustclaw-key")
            .and_then(|value| value.to_str().ok()),
        Some("session-admin-key")
    );
    assert_eq!(
        outgoing
            .get("x-rustclaw-client")
            .and_then(|value| value.to_str().ok()),
        Some("ui")
    );
}

#[test]
fn key_mode_forwards_client_key_and_ui_origin_without_web_session() {
    let mut incoming = HeaderMap::new();
    incoming.insert("x-rustclaw-key", HeaderValue::from_static("admin-key"));
    incoming.insert("x-rustclaw-client", HeaderValue::from_static("ui"));

    let outgoing = build_outgoing_headers(
        &incoming,
        "127.0.0.1:8080",
        SocketAddr::from(([127, 0, 0, 1], 41001)),
        false,
        None,
    );

    assert_eq!(
        outgoing
            .get("x-rustclaw-key")
            .and_then(|value| value.to_str().ok()),
        Some("admin-key")
    );
    assert_eq!(
        outgoing
            .get("x-rustclaw-client")
            .and_then(|value| value.to_str().ok()),
        Some("ui")
    );
}

#[test]
fn skill_store_install_uses_long_running_upstream_wait() {
    assert!(uses_long_running_upstream_wait(
        &Method::POST,
        "/v1/skills/store/install"
    ));
    assert!(uses_long_running_upstream_wait(
        &Method::POST,
        "/v1/skills/store/install?source=ui"
    ));
    assert!(!uses_long_running_upstream_wait(
        &Method::GET,
        "/v1/skills/store/install"
    ));
    assert!(!uses_long_running_upstream_wait(
        &Method::POST,
        "/v1/skills/store/remove"
    ));
    assert!(!uses_long_running_upstream_wait(&Method::POST, "/v1/tasks"));
}

async fn delayed_upstream_response() -> &'static str {
    tokio::time::sleep(Duration::from_millis(80)).await;
    "ok"
}

#[tokio::test]
async fn install_wait_outlives_normal_proxy_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed upstream");
    let addr = listener
        .local_addr()
        .expect("read delayed upstream address");
    let upstream = Router::new()
        .route("/v1/skills/store/install", post(delayed_upstream_response))
        .route("/v1/skills/store/remove", post(delayed_upstream_response));
    let upstream_task = tokio::spawn(async move {
        axum::serve(listener, upstream)
            .await
            .expect("serve delayed upstream");
    });

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_millis(20))
        .build()
        .expect("build short-deadline client");
    let long_running_client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(1))
        .build()
        .expect("build long-running client");
    let state = AppState {
        upstream: format!("http://{addr}"),
        client,
        long_running_client,
        forward_x_forwarded: false,
        max_incoming_body_bytes: 1024,
        cookie_name: "test-session".to_string(),
        session_ttl_secs: 60,
        sessions: Arc::new(Mutex::new(HashMap::new())),
    };
    let client_addr = SocketAddr::from(([127, 0, 0, 1], 41002));

    let install = Request::builder()
        .method(Method::POST)
        .uri("/v1/skills/store/install")
        .body(Body::empty())
        .expect("build install request");
    let install_response = proxy_inner(state.clone(), client_addr, install).await;
    assert_eq!(install_response.status(), StatusCode::OK);
    let body = to_bytes(install_response.into_body(), 32)
        .await
        .expect("read install response");
    assert_eq!(&body[..], b"ok");

    let remove = Request::builder()
        .method(Method::POST)
        .uri("/v1/skills/store/remove")
        .body(Body::empty())
        .expect("build remove request");
    let remove_response = proxy_inner(state, client_addr, remove).await;
    assert_eq!(remove_response.status(), StatusCode::BAD_GATEWAY);

    upstream_task.abort();
}
