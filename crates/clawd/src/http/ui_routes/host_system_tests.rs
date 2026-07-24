use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use super::{
    build_ui_router, parse_linux_meminfo, parse_linux_os_release, parse_linux_uptime,
    parse_macos_available_memory, parse_macos_boot_time, parse_macos_system_version, HostCapacity,
};
use crate::AppState;

#[test]
fn linux_fixture_parses_distribution_memory_and_uptime() {
    let release = r#"
NAME="Ubuntu"
VERSION="24.04.2 LTS (Noble Numbat)"
PRETTY_NAME="Ubuntu 24.04.2 LTS"
VERSION_ID="24.04"
"#;
    let (name, version) = parse_linux_os_release(release);
    assert_eq!(name.as_deref(), Some("Ubuntu"));
    assert_eq!(version.as_deref(), Some("24.04.2 LTS (Noble Numbat)"));

    let (total, available) =
        parse_linux_meminfo("MemTotal:       8042000 kB\nMemAvailable:   4012000 kB\n");
    assert_eq!(total, Some(8_042_000 * 1024));
    assert_eq!(available, Some(4_012_000 * 1024));
    assert_eq!(parse_linux_uptime("12345.67 901.00\n"), Some(12_345));
}

#[test]
fn macos_fixture_parses_version_memory_and_boot_time() {
    let plist = r#"
<dict>
  <key>ProductName</key><string>macOS</string>
  <key>ProductUserVisibleVersion</key><string>15.5</string>
</dict>
"#;
    let (name, version) = parse_macos_system_version(plist);
    assert_eq!(name.as_deref(), Some("macOS"));
    assert_eq!(version.as_deref(), Some("15.5"));
    let vm_stat = "Pages free: 100.\nPages inactive: 200.\nPages speculative: 50.\n";
    assert_eq!(
        parse_macos_available_memory(vm_stat, 4096),
        Some(350 * 4096)
    );
    assert_eq!(
        parse_macos_boot_time("{ sec = 1750000000, usec = 0 }"),
        Some(1_750_000_000)
    );
}

#[test]
fn partial_capacity_is_serialized_without_inventing_values() {
    let value = serde_json::to_value(HostCapacity::new(Some(1024), None))
        .expect("serialize partial capacity");
    assert_eq!(value["total_bytes"], 1024);
    assert!(value["available_bytes"].is_null());
    assert!(value["available_ratio"].is_null());
}

#[tokio::test]
async fn host_summary_endpoint_requires_ui_authentication() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/v1/system/host-summary")
                .body(Body::empty())
                .expect("host summary request"),
        )
        .await
        .expect("host summary response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn authenticated_host_summary_is_versioned_bounded_and_secret_free() {
    const KEY: &str = "rk-host-summary-test";
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state
        .core
        .db
        .get()
        .expect("main db")
        .execute(
            "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
             VALUES (?1, 'admin', 1, '1', NULL)",
            rusqlite::params![KEY],
        )
        .expect("seed auth key");
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/v1/system/host-summary")
                .header("x-rustclaw-key", KEY)
                .body(Body::empty())
                .expect("host summary request"),
        )
        .await
        .expect("host summary response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("bounded host summary body");
    let value: Value = serde_json::from_slice(&body).expect("host summary JSON");
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["schema_version"], 1);
    assert!(value["data"]["architecture"].is_string());
    assert!(value["data"]["os"]["family"].is_string());
    assert!(value["data"]["memory"]["total_bytes"].is_number());
    assert!(value["data"]["storage"]["total_bytes"].is_number());
    let encoded = String::from_utf8(body.to_vec()).expect("UTF-8 response");
    assert!(!encoded.contains(KEY));
    assert!(!encoded.contains("workspace_root"));
    assert!(!encoded.contains("environment"));
}
