use crate::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const ADMIN: &str = "fixture-runtime-reload-admin";
const USER: &str = "fixture-runtime-reload-user";

fn state() -> AppState {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_test_auth_identity(ADMIN, "admin");
    state.seed_test_auth_identity(USER, "user");
    state
}

fn headers(key: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        key.parse().unwrap(),
    );
    headers
}

fn assert_failure(
    response: (StatusCode, Json<ApiResponse<serde_json::Value>>),
    expected_status: StatusCode,
    expected_code: &str,
) {
    let (status, Json(body)) = response;
    assert_eq!(status, expected_status);
    let data = match expected_code {
        "auth_key_required" | "auth_key_invalid" => serde_json::json!({
            "owner_layer": "ui_auth", "error_code": expected_code,
            "status_code": expected_code,
            "message_key": format!("clawd.ui.auth.{expected_code}"),
        }),
        "admin_required" => serde_json::json!({
            "error_code": "admin_required", "message_key": "clawd.ui.auth.admin_required",
        }),
        _ => serde_json::Value::Null,
    };
    assert_eq!(
        serde_json::to_value(body).unwrap(),
        serde_json::json!({"ok": false, "data": data, "error": expected_code})
    );
}

#[tokio::test(flavor = "current_thread")]
async fn admin_reload_rejects_missing_invalid_and_non_admin_keys_before_work() {
    let state = state();
    let before = state.get_skill_views_snapshot();
    for (request_headers, status, code) in [
        (
            HeaderMap::new(),
            StatusCode::UNAUTHORIZED,
            "auth_key_required",
        ),
        (
            headers("fixture-unknown-reload-key"),
            StatusCode::UNAUTHORIZED,
            "auth_key_invalid",
        ),
        (headers(USER), StatusCode::FORBIDDEN, "admin_required"),
    ] {
        let response = skill_reload_request(state.clone(), request_headers, |_| {
            panic!("unauthorized callers must never prepare or publish runtime views")
        })
        .await;
        assert_failure(response, status, code);
        assert!(Arc::ptr_eq(&before, &state.get_skill_views_snapshot()));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn admin_reload_auth_lookup_failure_is_private_and_does_not_run() {
    let state = state();
    state
        .core
        .db
        .get()
        .unwrap()
        .execute_batch("ALTER TABLE auth_keys RENAME TO fixture_unavailable_reload_auth")
        .unwrap();
    assert_failure(
        skill_reload_request(state, headers(ADMIN), |_| panic!("auth lookup failed")).await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "auth_lookup_failed",
    );
}

#[tokio::test(flavor = "current_thread")]
async fn admin_reload_runs_once_off_the_async_executor_and_preserves_success_shape() {
    let async_thread = std::thread::current().id();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let expected = serde_json::json!({
        "registry_entries": 4, "execution_skills_count": 2, "planner_visible_count": 1
    });
    let result = expected.clone();
    let (status, Json(body)) = skill_reload_request(state(), headers(ADMIN), move |_| {
        assert_ne!(std::thread::current().id(), async_thread);
        observed.fetch_add(1, Ordering::SeqCst);
        Ok(result)
    })
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::to_value(body).unwrap(),
        serde_json::json!({"ok": true, "data": expected, "error": null})
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn admin_reload_real_config_failure_preserves_the_verified_view_and_hides_details() {
    let mut state = state();
    // This exclusively named absent workspace cannot touch real configuration.
    state.skill_rt.workspace_root = std::env::temp_dir().join(format!(
        "agent-runtime-absent-reload-{}",
        uuid::Uuid::new_v4()
    ));
    state.reload_ctx.config_path_for_reload = "configs/private-fixture.toml".into();
    let before = state.get_skill_views_snapshot();
    assert_failure(
        reload_skills_handler(State(state.clone()), headers(ADMIN)).await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "skill_runtime_reload_failed",
    );
    assert!(Arc::ptr_eq(&before, &state.get_skill_views_snapshot()));
}

#[tokio::test(flavor = "current_thread")]
async fn admin_reload_worker_panic_is_private_and_does_not_break_the_next_request() {
    let state = state();
    assert_failure(
        skill_reload_request(state.clone(), headers(ADMIN), |_| {
            panic!("private fixture provider-key=not-a-real-key path=/private/config")
        })
        .await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "skill_runtime_reload_worker_failed",
    );
    let (status, Json(body)) = skill_reload_request(state, headers(ADMIN), |_| {
        Ok(serde_json::json!({"recovered": true}))
    })
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.data, Some(serde_json::json!({"recovered": true})));
}

#[tokio::test(flavor = "current_thread")]
async fn admin_reload_does_not_cache_authorization_between_requests() {
    let state = state();
    let (status, _) = skill_reload_request(state.clone(), headers(ADMIN), |_| {
        Ok(serde_json::json!({"first": true}))
    })
    .await;
    assert_eq!(status, StatusCode::OK);
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("UPDATE auth_keys SET enabled=0 WHERE user_key=?1", [ADMIN])
        .unwrap();
    assert_failure(
        skill_reload_request(state, headers(ADMIN), |_| {
            panic!("revoked key must not run")
        })
        .await,
        StatusCode::UNAUTHORIZED,
        "auth_key_invalid",
    );
}
