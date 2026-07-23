use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use tokio::net::TcpListener;

use super::{
    build_outgoing_headers, clear_login_failures, login_client_ip, login_locked_response,
    login_retry_after, proxy_inner, record_login_failure, uses_long_running_upstream_wait,
    AppState, LoginAttemptKey,
};

fn login_test_state(failure_limit: u32, lockout_secs: u64) -> AppState {
    AppState {
        upstream: "http://127.0.0.1:1".to_string(),
        client: reqwest::Client::new(),
        long_running_client: reqwest::Client::new(),
        forward_x_forwarded: false,
        max_incoming_body_bytes: 1024,
        cookie_name: "test-session".to_string(),
        session_ttl_secs: 60,
        sessions: Arc::new(Mutex::new(HashMap::new())),
        login_failure_limit: failure_limit,
        login_lockout_secs: lockout_secs,
        login_attempts: Arc::new(Mutex::new(HashMap::new())),
    }
}

fn login_attempt_key(ip: [u8; 4], username: &str) -> LoginAttemptKey {
    LoginAttemptKey {
        client_ip: IpAddr::from(ip),
        username: username.to_string(),
    }
}

#[test]
fn sixth_failed_login_locks_only_the_same_ip_and_username() {
    let state = login_test_state(6, 15 * 60);
    let key = login_attempt_key([198, 51, 100, 10], "alice");
    for now in 100..105 {
        assert_eq!(record_login_failure(&state, key.clone(), now), None);
    }
    assert_eq!(record_login_failure(&state, key.clone(), 105), Some(900));
    assert_eq!(login_retry_after(&state, &key, 106), Some(899));

    let other_ip = login_attempt_key([198, 51, 100, 11], "alice");
    let other_username = login_attempt_key([198, 51, 100, 10], "bob");
    assert_eq!(login_retry_after(&state, &other_ip, 106), None);
    assert_eq!(login_retry_after(&state, &other_username, 106), None);
}

#[test]
fn successful_login_clears_consecutive_failures() {
    let state = login_test_state(3, 900);
    let key = login_attempt_key([203, 0, 113, 20], "alice");
    assert_eq!(record_login_failure(&state, key.clone(), 200), None);
    assert_eq!(record_login_failure(&state, key.clone(), 201), None);

    clear_login_failures(&state, &key);

    assert_eq!(record_login_failure(&state, key.clone(), 202), None);
    assert_eq!(login_retry_after(&state, &key, 202), None);
    assert_eq!(
        state
            .login_attempts
            .lock()
            .expect("login attempts")
            .get(&key)
            .map(|entry| entry.consecutive_failures),
        Some(1)
    );
}

#[test]
fn login_lock_expires_after_configured_window() {
    let state = login_test_state(2, 900);
    let key = login_attempt_key([192, 0, 2, 30], "alice");
    assert_eq!(record_login_failure(&state, key.clone(), 300), None);
    assert_eq!(record_login_failure(&state, key.clone(), 301), Some(900));
    assert_eq!(login_retry_after(&state, &key, 1200), Some(1));
    assert_eq!(login_retry_after(&state, &key, 1201), None);
}

#[test]
fn forwarded_client_ip_is_trusted_only_from_a_loopback_proxy() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static("198.51.100.44, 127.0.0.1"),
    );
    let loopback_peer = SocketAddr::from(([127, 0, 0, 1], 41004));
    let external_peer = SocketAddr::from(([203, 0, 113, 9], 41005));

    assert_eq!(
        login_client_ip(&headers, loopback_peer, true),
        IpAddr::from([198, 51, 100, 44])
    );
    assert_eq!(
        login_client_ip(&headers, loopback_peer, false),
        loopback_peer.ip()
    );
    assert_eq!(
        login_client_ip(&headers, external_peer, true),
        external_peer.ip()
    );

    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_static("not-an-ip, 198.51.100.44"),
    );
    assert_eq!(
        login_client_ip(&headers, loopback_peer, true),
        loopback_peer.ip()
    );
}

#[tokio::test]
async fn locked_login_returns_structured_429_with_retry_after() {
    let response = login_locked_response(899, None);
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok()),
        Some("899")
    );
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("read locked response");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("parse locked response");
    assert_eq!(payload["error_code"], "login_temporarily_locked");
    assert_eq!(payload["data"]["retry_after_seconds"], 899);
}

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

#[test]
fn task_event_stream_uses_long_running_upstream_wait() {
    assert!(uses_long_running_upstream_wait(
        &Method::GET,
        "/v1/tasks/task-123/events"
    ));
    assert!(uses_long_running_upstream_wait(
        &Method::GET,
        "/v1/tasks/task-123/events?cursor=17"
    ));
    assert!(!uses_long_running_upstream_wait(
        &Method::POST,
        "/v1/tasks/task-123/events"
    ));
    assert!(!uses_long_running_upstream_wait(
        &Method::GET,
        "/v1/tasks/task-123"
    ));
    assert!(!uses_long_running_upstream_wait(
        &Method::GET,
        "/v1/tasks/nested/task/events"
    ));
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
        .route("/v1/skills/store/remove", post(delayed_upstream_response))
        .route("/v1/tasks/task-123/events", get(delayed_upstream_response));
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
        login_failure_limit: 6,
        login_lockout_secs: 900,
        login_attempts: Arc::new(Mutex::new(HashMap::new())),
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

#[tokio::test]
async fn task_event_stream_outlives_normal_proxy_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed upstream");
    let addr = listener
        .local_addr()
        .expect("read delayed upstream address");
    let upstream = Router::new().route("/v1/tasks/task-123/events", get(delayed_upstream_response));
    let upstream_task = tokio::spawn(async move {
        axum::serve(listener, upstream)
            .await
            .expect("serve delayed upstream");
    });

    let state = AppState {
        upstream: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_millis(20))
            .build()
            .expect("build short-deadline client"),
        long_running_client: reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(1))
            .build()
            .expect("build long-running client"),
        forward_x_forwarded: false,
        max_incoming_body_bytes: 1024,
        cookie_name: "test-session".to_string(),
        session_ttl_secs: 60,
        sessions: Arc::new(Mutex::new(HashMap::new())),
        login_failure_limit: 6,
        login_lockout_secs: 900,
        login_attempts: Arc::new(Mutex::new(HashMap::new())),
    };
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/tasks/task-123/events?cursor=0")
        .body(Body::empty())
        .expect("build task event request");

    let response = proxy_inner(state, SocketAddr::from(([127, 0, 0, 1], 41003)), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 32)
        .await
        .expect("read task event response");
    assert_eq!(&body[..], b"ok");

    upstream_task.abort();
}
