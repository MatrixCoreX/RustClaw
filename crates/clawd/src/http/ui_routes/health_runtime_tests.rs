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
        std::env::temp_dir().join(format!("agent-health-runtime-{}", uuid::Uuid::new_v4())),
    );
    fs::create_dir_all(&root.0).unwrap();
    state.skill_rt.workspace_root = root.0.clone();
    state.channels.whatsapp_web_enabled = false;
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
    state.seed_test_auth_identity("fixture-health-admin", "admin");
    let held = state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let started = Instant::now();
    let ((status, Json(response)), timer_delay) =
        tokio::join!(health(State(state), auth_headers(key)), async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            started.elapsed()
        },);
    release.join().unwrap();
    assert_eq!(status, expected_status);
    assert_eq!(response.ok, expected_status == StatusCode::OK);
    if status == StatusCode::OK {
        assert_eq!(response.data.unwrap().queue_length, 0);
    } else {
        assert!(response.data.is_none());
        assert_eq!(response.error.as_deref(), Some("auth_key_invalid"));
    }
    eprintln!(
        "health_pool_hold_ms=500 timer_delay_ms={}",
        timer_delay.as_millis()
    );
    assert!(timer_delay < Duration::from_millis(400), "{timer_delay:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn health_pool_wait_does_not_block_the_async_executor() {
    observe_pool_wait("fixture-health-admin", StatusCode::OK).await;
}

#[tokio::test(flavor = "current_thread")]
async fn health_invalid_key_pool_wait_does_not_block_the_async_executor() {
    observe_pool_wait("fixture-health-unknown", StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn health_missing_key_is_rejected_without_database_access() {
    let (state, _root) = fixture();
    let _held = state.core.db.get().unwrap();
    let (status, Json(response)) = tokio::time::timeout(
        Duration::from_secs(2),
        health(State(state), HeaderMap::new()),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!response.ok);
    assert!(response.data.is_none());
    assert_eq!(response.error.as_deref(), Some("auth_key_required"));
}

fn seed_task(state: &AppState, task_id: &str, user_id: i64, age: i64, active: bool) {
    state.seed_ask_task_row(task_id, user_id, 1, "{}");
    if age > 0 {
        let now = current_unix_ts();
        state.core.db.get().unwrap().execute(
            "UPDATE tasks SET status='running', lease_owner='fixture', lease_expires_at=?2, created_at=?3 WHERE task_id=?1",
            rusqlite::params![task_id, if active {now+120} else {now-1}, (now-age).to_string()],
        ).unwrap();
    }
}

#[tokio::test]
async fn health_task_counts_preserve_user_scope_and_live_lease_filtering() {
    let (state, _root) = fixture();
    for (key, role) in [
        ("fixture-admin", "admin"),
        ("fixture-a", "user"),
        ("fixture-b", "user"),
    ] {
        state.seed_test_auth_identity(key, role);
    }
    let a = resolve_auth_identity_by_key(&state, "fixture-a")
        .unwrap()
        .unwrap()
        .user_id;
    let b = resolve_auth_identity_by_key(&state, "fixture-b")
        .unwrap()
        .unwrap()
        .user_id;
    for id in ["queued-a1", "queued-a2"] {
        seed_task(&state, id, a, 0, false);
    }
    seed_task(&state, "queued-b", b, 0, false);
    seed_task(&state, "active-a", a, 30, true);
    seed_task(&state, "active-b", b, 60, true);
    seed_task(&state, "expired-b", b, 3600, false);
    for (key, queued, running, oldest) in [
        ("fixture-admin", 3, 2, 60),
        ("fixture-a", 2, 1, 30),
        ("fixture-b", 1, 1, 60),
    ] {
        let (status, Json(response)) = health(State(state.clone()), auth_headers(key)).await;
        assert_eq!(status, StatusCode::OK);
        let data = response.data.unwrap();
        assert_eq!(data.queue_length, queued);
        assert_eq!(data.running_length, running);
        assert!((oldest..oldest + 15).contains(&data.running_oldest_age_seconds));
        assert_eq!(data.user_count, 3);
        assert_eq!(data.bound_channel_count, 0);
        assert!(data.bound_channels.is_empty());
    }
}

#[tokio::test]
async fn health_rechecks_authentication_and_counts_on_every_request() {
    let (state, _root) = fixture();
    state.seed_test_auth_identity("fixture-fresh", "user");
    let identity = resolve_auth_identity_by_key(&state, "fixture-fresh")
        .unwrap()
        .unwrap();
    seed_task(&state, "fresh-task", identity.user_id, 0, false);
    let (status, Json(response)) =
        health(State(state.clone()), auth_headers("fixture-fresh")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response.data.unwrap().queue_length, 1);
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET status='succeeded' WHERE task_id='fresh-task'",
            [],
        )
        .unwrap();
    let (_, Json(response)) = health(State(state.clone()), auth_headers("fixture-fresh")).await;
    assert_eq!(response.data.unwrap().queue_length, 0);
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE auth_keys SET enabled=0 WHERE user_key='fixture-fresh'",
            [],
        )
        .unwrap();
    let (status, Json(response)) = health(State(state), auth_headers("fixture-fresh")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!response.ok);
    assert!(response.data.is_none());
    assert_eq!(response.error.as_deref(), Some("auth_key_invalid"));
}

#[tokio::test]
async fn health_auth_lookup_failure_never_returns_a_health_snapshot() {
    let (state, _root) = fixture();
    state.seed_test_auth_identity("fixture-broken-auth", "admin");
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("DROP TABLE auth_keys", [])
        .unwrap();
    let (status, Json(response)) = health(State(state), auth_headers("fixture-broken-auth")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!response.ok);
    assert!(response.data.is_none());
    assert!(response.error.is_some());
}

#[tokio::test]
async fn health_reads_fresh_bot_and_gateway_status_without_a_ttl_cache() {
    let (mut state, root) = fixture();
    state.seed_test_auth_identity("fixture-status", "admin");
    state.channels.telegram_configured_bot_names = Arc::new(vec!["fixture-bot".into()]);
    state.channels.whatsapp_cloud_enabled = true;
    let (_, Json(before)) = health(State(state.clone()), auth_headers("fixture-status")).await;
    assert_eq!(
        before.data.unwrap().telegram_bot_statuses[0].status,
        "missing"
    );
    let now = current_unix_ts();
    for (directory, value) in [
        (
            "telegram-bot-status",
            json!({"name":"fixture-bot","healthy":true,"status":"running","last_heartbeat_ts":now,"last_error":null}),
        ),
        (
            "gateway-instance-status",
            json!({"kind":"whatsapp_cloud","name":"primary","scope":"whatsapp_cloud:primary","healthy":true,"status":"running","last_heartbeat_ts":now,"last_error":null}),
        ),
    ] {
        let directory = root.0.join("run").join(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("fixture.json"), value.to_string()).unwrap();
    }
    let (status, Json(after)) = health(State(state), auth_headers("fixture-status")).await;
    assert_eq!(status, StatusCode::OK);
    let data = after.data.unwrap();
    assert_eq!(data.telegram_configured_bot_names, ["fixture-bot"]);
    assert_eq!(data.telegram_configured_bot_count, 1);
    let bot = &data.telegram_bot_statuses[0];
    assert!(bot.healthy);
    assert_eq!(bot.status, "running");
    assert_eq!(bot.last_heartbeat_ts, Some(now));
    let gateway = data
        .gateway_instance_statuses
        .iter()
        .find(|item| item.scope == "whatsapp_cloud:primary")
        .unwrap();
    assert!(gateway.healthy);
    assert_eq!(gateway.last_heartbeat_ts, Some(now));
    assert_eq!(gateway.status, "running");
}
