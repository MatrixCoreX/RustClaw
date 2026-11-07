use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::time::Duration;
use tower::ServiceExt;

#[path = "memory_project_read_test_fixtures.rs"]
mod fixtures;
#[path = "memory_project_lookup_tests.rs"]
mod lookup;

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
    let mut fixture = fixtures::ProjectFixture::new(admin);
    let original_project = fixture.initialize();
    let key = fixture.setup.fixture.key.clone();
    let owner = fixture.setup.fixture.principal.clone();
    let identity = crate::resolve_auth_identity_by_key(fixture.setup.fixture.state(), &key)
        .unwrap()
        .unwrap();
    assert_eq!(identity.principal_id, owner);
    assert_eq!(identity.role, if admin { "admin" } else { "user" });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let request = Request::builder().uri("/memory/projects/current")
            .header(claw_core::product_identity::AUTH_KEY_HEADER, &key)
            .body(Body::empty()).unwrap();
        fixture.setup.arm();
        let router = super::memory_routes::router().with_state(fixture.setup.fixture.state().clone());
        let pending = tokio::spawn(async move { router.oneshot(request).await });
        tokio::time::timeout(Duration::from_secs(30), fixture.setup.authenticated.take().unwrap())
            .await.expect("observe real project authentication check-in").unwrap();
        match change {
            Change::Key => { assert_eq!(fixture.setup.direct.execute("UPDATE auth_keys SET enabled=0 WHERE user_key=?1", [&key]).unwrap(), 1); }
            Change::Binding => { assert_eq!(fixture.setup.direct.execute("UPDATE credential_bindings SET status='revoked' WHERE credential_digest=?1", [crate::repo::auth::credential_digest(&key)]).unwrap(), 1); }
            Change::Role => { assert_eq!(fixture.setup.direct.execute("UPDATE auth_keys SET role='user' WHERE user_key=?1", [&key]).unwrap(), 1); }
            Change::Reassign => {
                let transaction = fixture.setup.direct.unchecked_transaction().unwrap();
                assert_eq!(transaction.execute("UPDATE auth_keys SET principal_id=?1 WHERE user_key=?2", rusqlite::params![fixture.setup.other_principal, key]).unwrap(), 1);
                assert_eq!(transaction.execute("UPDATE credential_bindings SET principal_id=?1 WHERE credential_digest=?2", rusqlite::params![fixture.setup.other_principal, crate::repo::auth::credential_digest(&key)]).unwrap(), 1);
                transaction.commit().unwrap();
            }
            Change::OtherKey => { assert_eq!(fixture.setup.direct.execute("UPDATE auth_keys SET enabled=0 WHERE user_key=?1", [&fixture.setup.other_key]).unwrap(), 1); }
            Change::None => {}
        }
        assert!(crate::repo::auth::require_active_principal_credential(&fixture.setup.direct, &owner).is_ok());
        let checked = crate::repo::auth::require_request_identity(&fixture.setup.direct, &identity);
        match change {
            Change::Key | Change::Binding => assert_eq!(checked.unwrap_err().to_string(), "request_credential_revoked"),
            Change::Role | Change::Reassign => assert_eq!(checked.unwrap_err().to_string(), "request_identity_changed"),
            _ => checked.unwrap(),
        }
        let project = fixture.late_mapping();
        assert_ne!(project, original_project, "mapping is created only after auth changed");
        let before = fixture.setup.version();
        fixture.setup.release.release();
        let response = pending.await.unwrap().unwrap();
        fixture.setup.assert_handshake_valid();
        assert_eq!(fixture.setup.version(), before, "project GET must not commit read repair");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let reply: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        match change {
            Change::None | Change::OtherKey => fixture.assert_scope(status, &reply, &project),
            _ => {
                let (expected, code) = match change {
                    Change::Key | Change::Binding => (StatusCode::UNAUTHORIZED, "request_credential_revoked"),
                    _ => (StatusCode::FORBIDDEN, "request_identity_changed"),
                };
                assert_eq!(status, expected, "project returned a mapping created after auth changed; expected_scope_ref={project}; reply={reply}");
                assert_eq!(reply["ok"], false);
                assert_eq!(reply["error"], code);
                assert!(reply["data"].is_null());
            }
        }
        fixture.assert_no_private_metadata(&reply);
    });
}

#[test]
fn credential_revoked_before_project_read() {
    scenario(Change::Key, false);
}
#[test]
fn binding_revoked_before_project_read() {
    scenario(Change::Binding, false);
}
#[test]
fn role_downgraded_before_project_read() {
    scenario(Change::Role, true);
}
#[test]
fn principal_reassigned_before_project_read() {
    scenario(Change::Reassign, false);
}
#[test]
fn unchanged_user_reads_late_alias_after_another_key_changes() {
    scenario(Change::OtherKey, false);
}
#[test]
fn unchanged_admin_reads_late_alias_without_repair() {
    scenario(Change::None, true);
}
