use std::net::SocketAddr;

use axum::http::{HeaderMap, HeaderValue};

use super::build_outgoing_headers;

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
