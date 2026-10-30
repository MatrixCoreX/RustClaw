use super::*;
use std::time::Instant;

struct Workspace(PathBuf);

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> (AppState, Workspace) {
    let mut state = AppState::test_default_with_fixture_provider();
    state.core.db = r2d2::Pool::builder()
        .max_size(1)
        .build(r2d2_sqlite::SqliteConnectionManager::memory())
        .unwrap();
    let root = Workspace(
        std::env::temp_dir().join(format!("agent-host-read-runtime-{}", uuid::Uuid::new_v4())),
    );
    fs::create_dir_all(&root.0).unwrap();
    state.skill_rt.workspace_root = root.0.clone();
    (state.with_seeded_db_schema(), root)
}

fn auth_headers(key: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        axum::http::HeaderValue::from_static(key),
    );
    headers
}

async fn observe_pool_wait(key: &'static str, expected_status: StatusCode) {
    let (state, _root) = fixture();
    state.seed_test_auth_identity("fixture-host-admin", "admin");
    let held = state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let started = Instant::now();
    let ((status, Json(response)), timer_delay) = tokio::join!(
        host_system_summary(State(state), auth_headers(key)),
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            started.elapsed()
        }
    );
    release.join().unwrap();
    assert_eq!(status, expected_status);
    assert_eq!(response.ok, expected_status == StatusCode::OK);
    if status == StatusCode::OK {
        let data = response.data.unwrap();
        assert_eq!(data["schema_version"], 4);
        assert_eq!(data["runtime_resources"]["waiting_tasks"], 0);
    } else {
        let (_, Json(expected)) = ui_auth_error("auth_key_invalid");
        assert_eq!(response.data, expected.data);
        assert_eq!(response.error, expected.error);
    }
    eprintln!(
        "host_summary_pool_hold_ms=500 timer_delay_ms={}",
        timer_delay.as_millis()
    );
    assert!(timer_delay < Duration::from_millis(400), "{timer_delay:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn host_summary_pool_wait_does_not_block_the_async_executor() {
    observe_pool_wait("fixture-host-admin", StatusCode::OK).await;
}

#[tokio::test(flavor = "current_thread")]
async fn host_summary_invalid_key_pool_wait_does_not_block_the_async_executor() {
    observe_pool_wait("fixture-host-unknown", StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn host_summary_missing_key_is_rejected_without_database_access() {
    let (state, _root) = fixture();
    let _held = state.core.db.get().unwrap();
    let started = Instant::now();
    let (status, Json(response)) = tokio::time::timeout(
        Duration::from_secs(2),
        host_system_summary(State(state), HeaderMap::new()),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!response.ok);
    let (_, Json(expected)) = ui_auth_error("auth_key_required");
    assert_eq!(response.data, expected.data);
    assert_eq!(response.error, expected.error);
    assert!(started.elapsed() < Duration::from_millis(400));
}

#[tokio::test]
async fn host_summary_auth_lookup_failure_does_not_return_host_data() {
    let (state, _root) = fixture();
    state.seed_test_auth_identity("fixture-host-admin", "admin");
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("DROP TABLE auth_keys", [])
        .unwrap();
    let (status, Json(response)) =
        host_system_summary(State(state), auth_headers("fixture-host-admin")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!response.ok);
    assert!(response.data.is_none());
    assert!(response.error.is_some());
}

#[tokio::test]
async fn host_summary_task_lookup_failure_preserves_unknown_instead_of_zero() {
    let (state, _root) = fixture();
    state.seed_test_auth_identity("fixture-host-admin", "admin");
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("DROP TABLE tasks", [])
        .unwrap();
    let (status, Json(response)) =
        host_system_summary(State(state), auth_headers("fixture-host-admin")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(response.ok);
    let data = response.data.unwrap();
    assert_eq!(data["schema_version"], 4);
    for field in [
        "waiting_tasks",
        "resource_waiting_tasks",
        "recent_waiting_reason_code",
    ] {
        assert!(data["runtime_resources"][field].is_null(), "{field}");
    }
    assert!(data["unavailable_fields"]
        .as_array()
        .unwrap()
        .contains(&json!({
            "field":"runtime_resources.waiting_tasks", "code":"task_waiting_summary_unavailable"
        })));
}

#[tokio::test]
async fn host_summary_rechecks_auth_and_persisted_server_wait_counts() {
    let (state, _root) = fixture();
    state.seed_test_auth_identity("fixture-host-admin", "admin");
    state.seed_test_auth_identity("fixture-host-user", "user");
    for (id, status, lifecycle_state, reason, updated) in [
        (
            "resource",
            "running",
            "waiting",
            "resource_admission_wait",
            "2",
        ),
        (
            "provider",
            "running",
            "background",
            "provider_blocker_wait_background",
            "3",
        ),
        ("finished", "failed", "waiting", "fixture_old_wait", "4"),
        ("active", "running", "running", "fixture_active", "5"),
    ] {
        state.seed_ask_task_row(id, 77, 1, "{}");
        state
            .core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE tasks SET status=?2, result_json=?3, updated_at=?4 WHERE task_id=?1",
                rusqlite::params![id, status, json!({"task_lifecycle":{
                "state":lifecycle_state, "resume_reason":reason, "waiting_reason_code":reason
            }}).to_string(), updated],
            )
            .unwrap();
    }
    let (status, Json(response)) =
        host_system_summary(State(state.clone()), auth_headers("fixture-host-admin")).await;
    assert_eq!(status, StatusCode::OK);
    let data = response.data.unwrap();
    assert_eq!(data["runtime_resources"]["waiting_tasks"], 2);
    assert_eq!(data["runtime_resources"]["resource_waiting_tasks"], 1);
    assert_eq!(
        data["runtime_resources"]["recent_waiting_reason_code"],
        "provider_blocker_wait_background"
    );
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET status='succeeded' WHERE task_id='provider'",
            [],
        )
        .unwrap();
    let (status, Json(response)) =
        host_system_summary(State(state.clone()), auth_headers("fixture-host-user")).await;
    assert_eq!(status, StatusCode::OK);
    let data = response.data.unwrap();
    assert_eq!(data["runtime_resources"]["waiting_tasks"], 1);
    assert_eq!(data["runtime_resources"]["resource_waiting_tasks"], 1);
    assert_eq!(
        data["runtime_resources"]["recent_waiting_reason_code"],
        "resource_admission_wait"
    );
    for field in [
        "cgroup_version",
        "process_memory_roles_current_bytes",
        "process_memory_roles_peak_bytes",
    ] {
        assert!(data["runtime_resources"][field].is_null(), "{field}");
    }
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE auth_keys SET enabled=0 WHERE user_key='fixture-host-user'",
            [],
        )
        .unwrap();
    let (status, Json(response)) =
        host_system_summary(State(state), auth_headers("fixture-host-user")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (_, Json(expected)) = ui_auth_error("auth_key_invalid");
    assert_eq!(response.data, expected.data);
    assert_eq!(response.error, expected.error);
}
