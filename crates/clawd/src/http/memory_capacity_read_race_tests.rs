use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::time::Duration;
use tower::ServiceExt;

use super::memory_read_race_fixtures as fixtures;

#[derive(Clone, Copy)]
enum Change {
    Key,
    Binding,
    Role,
    Reassign,
    OtherKey,
    None,
}

fn scenario(change: Change, admin: bool) {
    let mut setup = fixtures::Setup::new(admin);
    let key = setup.fixture.key.clone();
    let owner = setup.fixture.principal.clone();
    let identity = crate::resolve_auth_identity_by_key(setup.fixture.state(), &key)
        .unwrap()
        .unwrap();
    assert_eq!(identity.principal_id, owner);
    assert_eq!(identity.role, if admin { "admin" } else { "user" });
    assert_eq!(
        setup
            .direct
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE principal_id=?1",
                [&owner],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let request = Request::builder().uri("/memory/availability")
            .header(claw_core::product_identity::AUTH_KEY_HEADER, &key).body(Body::empty()).unwrap();
        setup.arm();
        let router = super::memory_routes::router().with_state(setup.fixture.state().clone());
        let pending = tokio::spawn(async move { router.oneshot(request).await });
        tokio::time::timeout(Duration::from_secs(30), setup.authenticated.take().unwrap()).await
            .expect("observe the real authentication connection check-in").unwrap();
        match change {
            Change::Key => { assert_eq!(setup.direct.execute("UPDATE auth_keys SET enabled=0 WHERE user_key=?1", [&key]).unwrap(), 1); }
            Change::Binding => { assert_eq!(setup.direct.execute("UPDATE credential_bindings SET status='revoked' WHERE credential_digest=?1", [crate::repo::auth::credential_digest(&key)]).unwrap(), 1); }
            Change::Role => { assert_eq!(setup.direct.execute("UPDATE auth_keys SET role='user' WHERE user_key=?1", [&key]).unwrap(), 1); }
            Change::Reassign => {
                let transaction = setup.direct.unchecked_transaction().unwrap();
                assert_eq!(transaction.execute("UPDATE auth_keys SET principal_id=?1 WHERE user_key=?2", rusqlite::params![setup.other_principal, key]).unwrap(), 1);
                assert_eq!(transaction.execute("UPDATE credential_bindings SET principal_id=?1 WHERE credential_digest=?2", rusqlite::params![setup.other_principal, crate::repo::auth::credential_digest(&key)]).unwrap(), 1);
                transaction.commit().unwrap();
            }
            Change::OtherKey => { assert_eq!(setup.direct.execute("UPDATE auth_keys SET enabled=0 WHERE user_key=?1", [&setup.other_key]).unwrap(), 1); }
            Change::None => {}
        }
        // Another surviving key is not authority for this original request.
        assert!(crate::repo::auth::require_active_principal_credential(&setup.direct, &owner).is_ok());
        let current = crate::repo::auth::require_request_identity(&setup.direct, &identity);
        match change {
            Change::Key | Change::Binding => assert_eq!(current.unwrap_err().to_string(), "request_credential_revoked"),
            Change::Role | Change::Reassign => assert_eq!(current.unwrap_err().to_string(), "request_identity_changed"),
            _ => current.unwrap(),
        }
        let bytes = setup.late_private_content();
        if admin {
            setup.direct.execute_batch("CREATE TABLE post_auth_private_history(payload BLOB); INSERT INTO post_auth_private_history VALUES(zeroblob(262144));").unwrap();
        }
        let before = setup.version();
        setup.release.release();
        let response = pending.await.unwrap().unwrap();
        setup.assert_handshake_valid();
        assert_eq!(setup.version(), before, "the HTTP observation cannot commit any read repair");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let reply: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(setup.direct.query_row("SELECT LENGTH(CAST(content AS BLOB)) FROM memories WHERE memory_id='post-auth-private'", [], |r| r.get::<_, usize>(0)).unwrap(), bytes);
        match change {
            Change::None | Change::OtherKey => {
                assert_eq!(status, StatusCode::OK, "{reply}");
                assert_eq!(reply["ok"], true);
                assert_eq!(reply["data"]["settings"]["target_principal_id"], owner);
                assert_eq!(reply["data"]["measurements"]["own_source_transcript_payload_bytes"]["bytes"], bytes.to_string());
                assert_eq!(reply["data"]["measurements"]["main_sqlite_allocated_page_bytes"]["status"], if admin { "measured" } else { "admin_only" });
            }
            _ => {
                let (expected, code) = match change {
                    Change::Key | Change::Binding => (StatusCode::UNAUTHORIZED, "request_credential_revoked"),
                    _ => (StatusCode::FORBIDDEN, "request_identity_changed"),
                };
                assert_eq!(status, expected, "a stale authenticated identity returned post-change state: {reply}");
                assert_eq!(reply["ok"], false);
                assert_eq!(reply["error"], code);
                assert!(reply["data"].is_null());
            }
        }
        for secret in [&key, &setup.sibling_key, &setup.other_key, "post-auth-private", "post_auth_private_history"] {
            assert!(!reply.to_string().contains(secret));
        }
    });
}

#[test]
fn credential_revoked_after_authentication_does_not_return_later_private_state() {
    scenario(Change::Key, false);
}
#[test]
fn binding_revoked_after_authentication_does_not_return_later_private_state() {
    scenario(Change::Binding, false);
}
#[test]
fn role_downgraded_after_authentication_does_not_return_later_admin_metrics() {
    scenario(Change::Role, true);
}
#[test]
fn principal_reassigned_after_authentication_does_not_return_previous_owner_state() {
    scenario(Change::Reassign, false);
}
#[test]
fn unchanged_user_still_reads_own_state_after_another_credential_changes() {
    scenario(Change::OtherKey, false);
}
#[test]
fn unchanged_admin_still_reads_current_global_diagnostics_without_mutating_storage() {
    scenario(Change::None, true);
}
