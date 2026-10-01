use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use super::{
    build_ui_router, host_task_waiting_summary, parse_linux_os_release, parse_linux_uptime,
    parse_macos_boot_time, parse_macos_system_version, HostCapacity,
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

#[test]
fn waiting_summary_uses_persisted_task_lifecycle_not_broker_attempts() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().expect("task db");
    for (task_id, lifecycle, updated_at) in [
        (
            "task-resource-waiting",
            serde_json::json!({
                "task_lifecycle": {
                    "state": "waiting",
                    "resume_reason": "resource_admission_wait",
                    "waiting_reason_code": "resource_admission_wait"
                }
            }),
            "2",
        ),
        (
            "task-provider-waiting",
            serde_json::json!({
                "task_lifecycle": {
                    "state": "background",
                    "resume_reason": "provider_blocker_wait_background",
                    "waiting_reason_code": "provider_blocker_wait_background"
                }
            }),
            "3",
        ),
    ] {
        db.execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, channel, kind, payload_json, status,
                result_json, created_at, updated_at, lease_expires_at, claim_attempt
             ) VALUES (?1, 1, 1, 'ui', 'ask', '{}', 'running', ?2, '1', ?3, 0, 0)",
            rusqlite::params![task_id, lifecycle.to_string(), updated_at],
        )
        .expect("insert waiting task");
    }
    drop(db);

    let summary = host_task_waiting_summary(&state).expect("waiting summary");

    assert_eq!(summary.waiting_tasks, 2);
    assert_eq!(summary.resource_waiting_tasks, 1);
    assert_eq!(
        summary.recent_waiting_reason_code.as_deref(),
        Some("provider_blocker_wait_background")
    );
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
        .skill_rt
        .skill_concurrency_gates
        .resource_broker()
        .record_runtime_process_memory(
            crate::runtime_process_memory::RuntimeProcessMemorySample {
                measurement: "fixture".to_string(),
                process_count: 2,
                resident_and_swap_bytes: 1_024,
                roles: std::collections::BTreeMap::from([
                    ("core".to_string(), 768),
                    ("web_gateway".to_string(), 256),
                ]),
            },
            Some(16_384),
        );
    state.seed_test_auth_identity(KEY, "admin");
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/v1/system/host-summary")
                .header("x-agent-key", KEY)
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
    assert_eq!(value["data"]["schema_version"], 4);
    assert!(value["data"]["architecture"].is_string());
    assert!(value["data"]["os"]["family"].is_string());
    assert!(value["data"]["memory"]["total_bytes"].is_number());
    assert!(value["data"]["storage"]["total_bytes"].is_number());
    assert!(value["data"]["runtime_resources"]["pressure_state"].is_string());
    assert!(value["data"]["runtime_resources"]["active_leases"].is_number());
    assert!(value["data"]["runtime_resources"]["waiting_tasks"].is_number());
    assert_eq!(
        value["data"]["runtime_resources"]["process_memory_current_bytes"],
        1_024
    );
    assert_eq!(
        value["data"]["runtime_resources"]["process_memory_roles_current_bytes"]["core"],
        768
    );
    let encoded = String::from_utf8(body.to_vec()).expect("UTF-8 response");
    assert!(!encoded.contains(KEY));
    assert!(!encoded.contains("workspace_root"));
    assert!(!encoded.contains("environment"));
}

#[tokio::test]
async fn ordinary_user_host_summary_hides_cgroup_detail() {
    const KEY: &str = "rk-host-summary-user-test";
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_test_auth_identity(KEY, "user");
    let response = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state)
        .oneshot(
            Request::builder()
                .uri("/v1/system/host-summary")
                .header("x-agent-key", KEY)
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
    assert!(value["data"]["runtime_resources"]["cgroup_version"].is_null());
    assert!(value["data"]["runtime_resources"]["process_memory_roles_current_bytes"].is_null());
    assert!(value["data"]["runtime_resources"]["process_memory_roles_peak_bytes"].is_null());
}
