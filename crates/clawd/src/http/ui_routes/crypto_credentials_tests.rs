use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use super::build_ui_router;
use crate::AppState;

const TEST_KEY: &str = "rk-crypto-api-test";

async fn call_api(
    router: axum::Router,
    method: Method,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri("/v1/auth/crypto-credentials")
        .header("x-rustclaw-key", TEST_KEY);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = router
        .oneshot(builder.body(body).expect("crypto API request"))
        .await
        .expect("crypto API response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("crypto API response body");
    (
        status,
        serde_json::from_slice(&bytes).expect("crypto API response JSON"),
    )
}

#[tokio::test]
async fn credential_api_uses_crypto_owned_storage_and_redacts_secrets() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state
        .core
        .db
        .get()
        .expect("main db")
        .execute(
            "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
             VALUES (?1, 'admin', 1, '1', NULL)",
            rusqlite::params![TEST_KEY],
        )
        .expect("seed auth key");
    let router = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state.clone());

    let (status, upserted) = call_api(
        router.clone(),
        Method::POST,
        Some(serde_json::json!({
            "exchange": "okx",
            "api_key": "fixture-api-key",
            "api_secret": "fixture-api-secret",
            "passphrase": "fixture-passphrase"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(upserted["data"]["exchange"], "okx");
    assert_ne!(upserted["data"]["api_key_masked"], "fixture-api-key");
    assert!(!upserted.to_string().contains("fixture-api-secret"));
    assert!(!upserted.to_string().contains("fixture-passphrase"));

    let (status, listed) = call_api(router, Method::GET, None).await;
    assert_eq!(status, StatusCode::OK);
    let okx = listed["data"]
        .as_array()
        .expect("credential status array")
        .iter()
        .find(|status| status["exchange"] == "okx")
        .expect("OKX credential status");
    assert_eq!(okx["configured"], true);
    assert!(!listed.to_string().contains("fixture-api-secret"));

    let private_context = crate::repo::crypto_credential_context_for_user_key(&state, TEST_KEY)
        .expect("crypto-owned credential context");
    assert_eq!(private_context["okx"]["api_secret"], "fixture-api-secret");
    let main = state.core.db.get().expect("main db");
    let main_table_count: i64 = main
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table' AND name='exchange_api_credentials'",
            [],
            |row| row.get(0),
        )
        .expect("main crypto table count");
    assert_eq!(main_table_count, 0);
}
