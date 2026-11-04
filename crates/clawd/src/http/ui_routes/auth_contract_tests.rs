use claw_core::types::AuthIdentity;

use super::*;

#[test]
fn ui_auth_failures_expose_stable_machine_codes() {
    let (status, Json(response)) = ui_auth_error("auth_key_required");
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.error.as_deref(), Some("auth_key_required"));
    assert_eq!(
        response
            .data
            .as_ref()
            .and_then(|value| value.get("error_code"))
            .and_then(Value::as_str),
        Some("auth_key_required")
    );

    let (status, Json(response)) = ui_auth_code_error::<AuthIdentity>("auth_key_invalid");
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.error.as_deref(), Some("auth_key_invalid"));
    assert!(response.data.is_none());
}

const FIXTURE_KEY: &str = "fixture-auth-contract-admin";

fn auth_state() -> AppState {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_test_auth_identity(FIXTURE_KEY, "admin");
    state
}

fn headers_for(key: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        axum::http::HeaderValue::from_static(key),
    );
    headers
}

fn set_auth_database_available(state: &AppState, available: bool) {
    let sql = if available {
        "ALTER TABLE fixture_unavailable_auth_keys RENAME TO auth_keys"
    } else {
        "ALTER TABLE auth_keys RENAME TO fixture_unavailable_auth_keys"
    };
    state.core.db.get().unwrap().execute(sql, []).unwrap();
}

fn assert_lookup_failure<T: serde::Serialize>(response: (StatusCode, Json<ApiResponse<T>>)) {
    let (status, Json(response)) = response;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({"ok": false, "data": null, "error": "auth_lookup_failed"}),
    );
}

#[test]
fn ui_auth_guards_do_not_expose_database_errors_or_misclassify_them_as_invalid_keys() {
    let state = auth_state();
    set_auth_database_available(&state, false);
    let headers = headers_for(FIXTURE_KEY);
    assert_lookup_failure(require_ui_identity(&state, &headers).err().unwrap());
    assert_lookup_failure(require_ui_admin(&state, &headers).err().unwrap());
}

#[tokio::test]
async fn ui_auth_verification_and_identity_routes_keep_internal_errors_private() {
    let state = auth_state();
    set_auth_database_available(&state, false);
    assert_lookup_failure(
        verify_ui_key(
            State(state.clone()),
            Json(UiKeyVerifyRequest {
                user_key: FIXTURE_KEY.into(),
            }),
        )
        .await,
    );
    assert_lookup_failure(auth_me(State(state), headers_for(FIXTURE_KEY)).await);
}

#[tokio::test]
async fn ui_auth_missing_and_empty_keys_do_not_need_an_auth_database() {
    let state = AppState::test_default_with_fixture_provider();
    let (status, Json(response)) = require_ui_identity(&state, &HeaderMap::new())
        .err()
        .unwrap();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.error.as_deref(), Some("auth_key_required"));
    let (status, Json(response)) = verify_ui_key(
        State(state),
        Json(UiKeyVerifyRequest {
            user_key: "  ".into(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.error.as_deref(), Some("auth_key_invalid"));
}

#[tokio::test]
async fn ui_auth_invalid_keys_and_non_admin_roles_keep_their_existing_statuses() {
    let state = auth_state();
    state.seed_test_auth_identity("fixture-auth-contract-user", "user");
    let (status, Json(response)) =
        require_ui_identity(&state, &headers_for("fixture-auth-contract-unknown"))
            .err()
            .unwrap();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.error.as_deref(), Some("auth_key_invalid"));
    let (status, Json(response)) =
        require_ui_admin(&state, &headers_for("fixture-auth-contract-user"))
            .err()
            .unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(response.error.as_deref(), Some("admin_required"));
    let (status, Json(response)) = verify_ui_key(
        State(state),
        Json(UiKeyVerifyRequest {
            user_key: "fixture-auth-contract-unknown".into(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.error.as_deref(), Some("auth_key_invalid"));
}

#[tokio::test]
async fn ui_auth_database_recovery_preserves_the_same_identity_and_key() {
    let state = auth_state();
    let (status, Json(before)) = auth_me(State(state.clone()), headers_for(FIXTURE_KEY)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(before.ok);
    set_auth_database_available(&state, false);
    assert_lookup_failure(auth_me(State(state.clone()), headers_for(FIXTURE_KEY)).await);
    set_auth_database_available(&state, true);
    let (status, Json(after)) = auth_me(State(state), headers_for(FIXTURE_KEY)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}
