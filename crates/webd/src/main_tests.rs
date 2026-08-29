use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use tokio::net::TcpListener;
use tower::ServiceExt;

use super::{
    build_outgoing_headers, build_webd_router, clear_login_failures,
    cors_allow_origin_from_headers, is_internal_upstream_path, login_client_ip,
    login_locked_response, login_retry_after, normalize_loopback_upstream, proxy_inner,
    record_login_failure, request_uses_https, session_cookie_value,
    uses_long_running_upstream_wait, valid_login_input, webd_error_response, with_cors, AppState,
    LoginAttemptKey,
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
        session_store_path: std::path::PathBuf::new(),
        sessions: Arc::new(Mutex::new(HashMap::new())),
        login_failure_limit: failure_limit,
        login_lockout_secs: lockout_secs,
        login_attempts: Arc::new(Mutex::new(HashMap::new())),
    }
}

#[tokio::test]
async fn webd_serves_ui_assets_without_forwarding_them_to_clawd() {
    let root = std::env::temp_dir().join(format!("agent-runtime-webd-ui-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("create UI fixture");
    std::fs::write(root.join("index.html"), "<html>webd-ui-marker</html>")
        .expect("write UI fixture");
    let app = build_webd_router(login_test_state(6, 900), root.clone());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("serve UI");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store, max-age=0")
    );
    assert_eq!(
        response
            .headers()
            .get(header::X_FRAME_OPTIONS)
            .and_then(|value| value.to_str().ok()),
        Some("DENY")
    );
    assert!(response.headers().contains_key("content-security-policy"));
    let body = to_bytes(response.into_body(), 4096)
        .await
        .expect("read UI response");
    assert!(String::from_utf8_lossy(&body).contains("webd-ui-marker"));

    let mut api_request = Request::builder()
        .uri("/v1/health")
        .body(Body::empty())
        .expect("API request");
    api_request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(SocketAddr::from((
            [127, 0, 0, 1],
            41008,
        ))));
    let api_response = app.oneshot(api_request).await.expect("proxy API");
    assert_eq!(api_response.status(), StatusCode::BAD_GATEWAY);
    let api_body = to_bytes(api_response.into_body(), 4096)
        .await
        .expect("read API response");
    assert!(!String::from_utf8_lossy(&api_body).contains("webd-ui-marker"));

    std::fs::remove_dir_all(root).expect("remove UI fixture");
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
    incoming.insert("x-agent-key", HeaderValue::from_static("client-key"));
    incoming.insert("x-agent-runtime-client", HeaderValue::from_static("ui"));

    let outgoing = build_outgoing_headers(
        &incoming,
        "127.0.0.1:8080",
        SocketAddr::from(([127, 0, 0, 1], 41000)),
        true,
        Some("session-admin-key"),
    );

    assert_eq!(
        outgoing
            .get("x-agent-key")
            .and_then(|value| value.to_str().ok()),
        Some("session-admin-key")
    );
    assert_eq!(
        outgoing
            .get("x-agent-runtime-client")
            .and_then(|value| value.to_str().ok()),
        Some("ui")
    );
}

#[test]
fn key_mode_forwards_client_key_and_ui_origin_without_web_session() {
    let mut incoming = HeaderMap::new();
    incoming.insert("x-agent-key", HeaderValue::from_static("admin-key"));
    incoming.insert("x-agent-runtime-client", HeaderValue::from_static("ui"));

    let outgoing = build_outgoing_headers(
        &incoming,
        "127.0.0.1:8080",
        SocketAddr::from(([127, 0, 0, 1], 41001)),
        false,
        None,
    );

    assert_eq!(
        outgoing
            .get("x-agent-key")
            .and_then(|value| value.to_str().ok()),
        Some("admin-key")
    );
    assert_eq!(
        outgoing
            .get("x-agent-runtime-client")
            .and_then(|value| value.to_str().ok()),
        Some("ui")
    );
}

#[test]
fn trusted_loopback_proxy_preserves_external_protocol_and_forwarding_chain() {
    let mut incoming = HeaderMap::new();
    incoming.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.44"));
    incoming.insert("x-forwarded-proto", HeaderValue::from_static("https"));

    let outgoing = build_outgoing_headers(
        &incoming,
        "127.0.0.1:8787",
        SocketAddr::from(([127, 0, 0, 1], 41006)),
        true,
        None,
    );

    assert_eq!(
        outgoing
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok()),
        Some("198.51.100.44, 127.0.0.1")
    );
    assert_eq!(
        outgoing
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok()),
        Some("https")
    );
}

#[test]
fn direct_clients_cannot_spoof_forwarded_identity_or_https() {
    let mut incoming = HeaderMap::new();
    incoming.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.44"));
    incoming.insert("x-forwarded-proto", HeaderValue::from_static("https"));

    let outgoing = build_outgoing_headers(
        &incoming,
        "127.0.0.1:8787",
        SocketAddr::from(([203, 0, 113, 9], 41007)),
        true,
        None,
    );

    assert_eq!(
        outgoing
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok()),
        Some("203.0.113.9")
    );
    assert_eq!(
        outgoing
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok()),
        Some("http")
    );
}

#[test]
fn credentialed_cors_is_limited_to_the_request_hostname() {
    let mut headers = HeaderMap::new();
    headers.insert(header::HOST, HeaderValue::from_static("localhost:8788"));
    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://localhost:8788"),
    );
    assert_eq!(
        cors_allow_origin_from_headers(&headers)
            .and_then(|value| value.to_str().ok().map(str::to_string)),
        Some("http://localhost:8788".to_string())
    );

    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("http://localhost:3000"),
    );
    assert!(cors_allow_origin_from_headers(&headers).is_none());

    headers.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://untrusted.example"),
    );
    assert!(cors_allow_origin_from_headers(&headers).is_none());
}

#[tokio::test]
async fn proxy_rejects_a_present_cross_origin_before_contacting_upstream() {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/v1/tasks")
        .header(header::HOST, "localhost:8788")
        .header(header::ORIGIN, "https://untrusted.example")
        .body(Body::empty())
        .expect("request");
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(SocketAddr::from((
            [127, 0, 0, 1],
            41009,
        ))));

    let response = proxy_inner(
        login_test_state(6, 900),
        SocketAddr::from(([127, 0, 0, 1], 41009)),
        request,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), 2048)
        .await
        .expect("read forbidden response");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("JSON response");
    assert_eq!(payload["data"]["error_code"], "webd_origin_not_allowed");
}

#[test]
fn credentialed_cors_preserves_upstream_vary_dimensions() {
    let mut response = Response::builder()
        .header(header::VARY, "Accept-Encoding")
        .body(Body::empty())
        .expect("build response");
    let origin = HeaderValue::from_static("https://agent-runtime.example");

    response = with_cors(response, Some(&origin));

    let vary = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>();
    assert_eq!(vary, vec!["Accept-Encoding", "Origin"]);
    assert_eq!(
        response
            .headers()
            .get(header::X_FRAME_OPTIONS)
            .and_then(|value| value.to_str().ok()),
        Some("DENY")
    );
    assert!(response.headers().contains_key("content-security-policy"));
}

#[test]
fn webd_upstream_is_restricted_to_explicit_loopback_http() {
    assert_eq!(
        normalize_loopback_upstream("http://127.0.0.1:8787/").unwrap(),
        "http://127.0.0.1:8787"
    );
    assert_eq!(
        normalize_loopback_upstream("http://[::1]:8787").unwrap(),
        "http://[::1]:8787"
    );
    for invalid in [
        "https://127.0.0.1:8787",
        "http://localhost:8787",
        "http://192.168.1.10:8787",
        "http://127.0.0.1",
        "http://127.0.0.1:8787/v1",
        "http://user@127.0.0.1:8787",
    ] {
        assert!(normalize_loopback_upstream(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn internal_clawd_routes_are_never_exposed_by_webd() {
    assert!(is_internal_upstream_path("/v1/internal"));
    assert!(is_internal_upstream_path("/v1/internal/webd/verify-login"));
    assert!(is_internal_upstream_path("/v1/internal/skills/admit"));
    assert!(!is_internal_upstream_path("/v1/tasks"));
    assert!(!is_internal_upstream_path("/v1/internalized"));
}

#[test]
fn login_inputs_are_bounded_before_hash_verification_or_lockout_tracking() {
    assert!(valid_login_input("admin", "a sufficiently long password"));
    assert!(!valid_login_input("", "password"));
    assert!(!valid_login_input("admin", ""));
    assert!(!valid_login_input(&"u".repeat(129), "password"));
    assert!(!valid_login_input("admin", &"p".repeat(1025)));
}

#[tokio::test]
async fn proxy_failures_use_a_structured_json_envelope() {
    let response = webd_error_response(StatusCode::BAD_GATEWAY, "webd_upstream_unavailable", None);
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("read proxy error response");
    let payload: serde_json::Value =
        serde_json::from_slice(&body).expect("parse proxy error response");
    assert_eq!(payload["error"], "webd_upstream_unavailable");
    assert_eq!(payload["data"]["error_code"], "webd_upstream_unavailable");
}

#[test]
fn https_sessions_use_secure_http_only_cookies() {
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
    assert!(request_uses_https(
        &headers,
        SocketAddr::from(([127, 0, 0, 1], 41008)),
        true,
    ));
    let cookie = session_cookie_value("webd_sid", "session-id", 60, true);
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(cookie.contains("; Secure"));
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

#[test]
fn task_artifact_delivery_uses_streaming_upstream_for_webd_and_nginx_paths() {
    let path = "/v1/tasks/task-123/artifacts/artifact-9/content";
    assert!(uses_long_running_upstream_wait(&Method::GET, path));
    assert!(uses_long_running_upstream_wait(
        &Method::GET,
        &format!("{path}?disposition=inline")
    ));
    assert!(uses_long_running_upstream_wait(&Method::HEAD, path));
    assert!(!uses_long_running_upstream_wait(&Method::POST, path));
    assert!(!uses_long_running_upstream_wait(
        &Method::GET,
        "/v1/tasks/task-123/artifacts"
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
        session_store_path: std::path::PathBuf::new(),
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
        session_store_path: std::path::PathBuf::new(),
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
