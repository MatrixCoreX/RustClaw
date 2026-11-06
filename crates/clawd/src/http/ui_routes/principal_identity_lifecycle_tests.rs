use super::model_settings_integrity_tests::support::{Fixture, ADMIN, USER};
use super::*;
use axum::http::Method;

async fn identity(fixture: &Fixture, key: &str) -> (StatusCode, Value) {
    fixture.api(key, Method::GET, "/v1/auth/me", None).await
}

fn deny(response: &(StatusCode, Value)) {
    assert_eq!(response.0, StatusCode::UNAUTHORIZED);
    assert_eq!(response.1["error"], "auth_key_invalid");
    assert_eq!(response.1["ok"], false);
    assert!(response.1.get("principal_id").is_none());
}

#[tokio::test]
async fn ui_identity_observes_revocation_immediately_without_a_cached_authorization() {
    let fixture = Fixture::new();
    assert_eq!(identity(&fixture, ADMIN).await.0, StatusCode::OK);
    fixture
        .sql("UPDATE credential_bindings SET status='revoked', revoked_at='revocation-sentinel'");
    let response = identity(&fixture, ADMIN).await;
    deny(&response);
    fixture.assert_private(&response.1);
    let verified = fixture
        .api(
            "",
            Method::POST,
            "/v1/auth/ui-key/verify",
            Some(json!({"user_key":ADMIN})),
        )
        .await;
    assert_eq!(verified.0, StatusCode::UNAUTHORIZED);
    let db = fixture.state.core.db.get().unwrap();
    let count: i64 = db.query_row("SELECT COUNT(*) FROM credential_bindings WHERE status='revoked' AND revoked_at='revocation-sentinel'", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 2, "authentication reads must not repair revocation");
}

#[tokio::test]
async fn ui_identity_rejects_frozen_merged_and_deleted_owners() {
    for status in ["frozen", "merged", "deleted"] {
        let fixture = Fixture::new();
        fixture
            .state
            .core
            .db
            .get()
            .unwrap()
            .execute("UPDATE principals SET status = ?1", [status])
            .unwrap();
        let before = fixture.files();
        for key in [ADMIN, USER] {
            let response = identity(&fixture, key).await;
            deny(&response);
            fixture.assert_private(&response.1);
        }
        assert_eq!(fixture.files(), before);
    }
}

#[tokio::test]
async fn ui_identity_rejects_a_binding_owned_by_another_principal() {
    let fixture = Fixture::new();
    fixture.sql("UPDATE credential_bindings SET principal_id = (SELECT principal_id FROM auth_keys WHERE user_key='fixture-settings-user') WHERE principal_id = (SELECT principal_id FROM auth_keys WHERE user_key='fixture-settings-admin')");
    deny(&identity(&fixture, ADMIN).await);
    assert_eq!(identity(&fixture, USER).await.0, StatusCode::OK);
}

#[tokio::test]
async fn ui_identity_missing_binding_evidence_is_not_recreated_by_authentication_reads() {
    let fixture = Fixture::new();
    fixture.sql("DELETE FROM credential_bindings");
    deny(&identity(&fixture, ADMIN).await);
    let count: i64 = fixture
        .state
        .core
        .db
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM credential_bindings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn ui_identity_binding_database_failure_is_private_and_not_an_invalid_key() {
    let fixture = Fixture::new();
    fixture.sql("ALTER TABLE credential_bindings RENAME TO fixture_unavailable_bindings");
    let response = identity(&fixture, ADMIN).await;
    assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response.1,
        json!({"ok":false,"data":null,"error":"auth_lookup_failed"})
    );
    fixture.assert_private(&response.1);
    assert!(!response.1.to_string().contains("credential_bindings"));
}

#[tokio::test]
async fn ui_identity_disabled_keys_remain_rejected_and_enabled_control_is_unchanged() {
    let fixture = Fixture::new();
    fixture.sql("UPDATE auth_keys SET enabled = 0 WHERE role='user'");
    deny(&identity(&fixture, USER).await);
    let active = identity(&fixture, ADMIN).await;
    assert_eq!(active.0, StatusCode::OK);
    assert_eq!(active.1["data"]["role"], "admin");
}

#[tokio::test]
async fn password_login_cannot_authenticate_a_revoked_binding() {
    let fixture = Fixture::new();
    {
        let db = fixture.state.core.db.get().unwrap();
        crate::repo::auth::upsert_webd_login_account(
            &db,
            "fixture-login",
            "synthetic-password-1234",
            USER,
        )
        .unwrap();
    }
    let login = || json!({"username":"fixture-login", "password":"synthetic-password-1234"});
    let accepted = fixture
        .api(
            "",
            Method::POST,
            "/v1/internal/webd/verify-login",
            Some(login()),
        )
        .await;
    assert_eq!(accepted.0, StatusCode::OK);
    fixture.sql("UPDATE credential_bindings SET status='revoked', revoked_at='2'");
    let rejected = fixture
        .api(
            "",
            Method::POST,
            "/v1/internal/webd/verify-login",
            Some(login()),
        )
        .await;
    assert_eq!(rejected.0, StatusCode::UNAUTHORIZED);
    assert_eq!(rejected.1["error"], "invalid_credentials");
    fixture.assert_private(&rejected.1);
    assert!(!rejected.1.to_string().contains(USER));
}

#[tokio::test]
async fn password_login_cannot_authenticate_a_frozen_principal() {
    let fixture = Fixture::new();
    {
        let db = fixture.state.core.db.get().unwrap();
        crate::repo::auth::upsert_webd_login_account(
            &db,
            "fixture-login",
            "synthetic-password-1234",
            USER,
        )
        .unwrap();
    }
    fixture.sql("UPDATE principals SET status='frozen'");
    let rejected = fixture
        .api(
            "",
            Method::POST,
            "/v1/internal/webd/verify-login",
            Some(json!({"username":"fixture-login", "password":"synthetic-password-1234"})),
        )
        .await;
    assert_eq!(rejected.0, StatusCode::UNAUTHORIZED);
    assert_eq!(rejected.1["error"], "invalid_credentials");
    fixture.assert_private(&rejected.1);
}
