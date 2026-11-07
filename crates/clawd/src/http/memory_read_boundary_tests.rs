use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::time::Duration;
use tower::ServiceExt;

use super::memory_read_race_fixtures::Setup;
#[path = "memory_read_boundary_test_fixtures.rs"]
mod fixtures;

#[derive(Clone, Copy)]
enum Change {
    Key,
    Binding,
    Role,
    Reassign,
    OtherKey,
    None,
}

fn scenario(route: &str, change: Change, admin: bool) {
    let mut setup = Setup::new(admin);
    let key = setup.fixture.key.clone();
    let owner = setup.fixture.principal.clone();
    let identity = crate::resolve_auth_identity_by_key(setup.fixture.state(), &key)
        .unwrap()
        .unwrap();
    assert_eq!(identity.principal_id, owner);
    assert_eq!(identity.role, if admin { "admin" } else { "user" });
    let cross_owner = admin && route == "/memory/settings";
    let target = if cross_owner {
        setup.other_principal.clone()
    } else {
        owner.clone()
    };
    let uri = if cross_owner {
        format!("{route}?scope=admin&target_principal_id={target}")
    } else {
        route.to_string()
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let request = Request::builder().uri(uri)
            .header(claw_core::product_identity::AUTH_KEY_HEADER, &key)
            .body(Body::empty()).unwrap();
        setup.arm();
        let router = super::memory_routes::router().with_state(setup.fixture.state().clone());
        let pending = tokio::spawn(async move { router.oneshot(request).await });
        tokio::time::timeout(Duration::from_secs(30), setup.authenticated.take().unwrap())
            .await.expect("observe actual authentication check-in").unwrap();
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
        // The original request must not borrow authority from its surviving sibling.
        assert!(crate::repo::auth::require_active_principal_credential(&setup.direct, &owner).is_ok());
        let current = crate::repo::auth::require_request_identity(&setup.direct, &identity);
        match change {
            Change::Key | Change::Binding => assert_eq!(current.unwrap_err().to_string(), "request_credential_revoked"),
            Change::Role | Change::Reassign => assert_eq!(current.unwrap_err().to_string(), "request_identity_changed"),
            _ => current.unwrap(),
        }
        // These sources and policies did not exist in any earlier valid auth snapshot.
        fixtures::seed_late_state(&setup, &identity);
        let before = setup.version();
        setup.release.release();
        let response = pending.await.unwrap().unwrap();
        setup.assert_handshake_valid();
        assert_eq!(setup.version(), before, "a GET must not commit read repair");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let reply: serde_json::Value = serde_json::from_slice(&body).unwrap();
        match change {
            Change::None | Change::OtherKey => {
                assert_eq!(status, StatusCode::OK, "{route}: {reply}");
                assert_eq!(reply["ok"], true);
                fixtures::assert_normal_data(route, &reply["data"], &target);
            }
            _ => {
                let (expected, code) = match change {
                    Change::Key | Change::Binding => (StatusCode::UNAUTHORIZED, "request_credential_revoked"),
                    _ => (StatusCode::FORBIDDEN, "request_identity_changed"),
                };
                assert_eq!(status, expected, "{route} returned post-auth-change private state: {reply}");
                assert_eq!(reply["ok"], false);
                assert_eq!(reply["error"], code);
                assert!(reply["data"].is_null());
            }
        }
        for secret in [&key, &setup.sibling_key, &setup.other_key] {
            assert!(!reply.to_string().contains(secret));
        }
    });
}

macro_rules! read_contract {
    ($name:ident, $route:literal) => {
        mod $name {
            use super::*;
            #[test]
            fn credential_revoked() {
                scenario($route, Change::Key, false);
            }
            #[test]
            fn binding_revoked() {
                scenario($route, Change::Binding, false);
            }
            #[test]
            fn role_downgraded() {
                scenario($route, Change::Role, true);
            }
            #[test]
            fn principal_reassigned() {
                scenario($route, Change::Reassign, false);
            }
            #[test]
            fn unchanged_user_other_key_changed() {
                scenario($route, Change::OtherKey, false);
            }
            #[test]
            fn unchanged_admin() {
                scenario($route, Change::None, true);
            }
        }
    };
}

read_contract!(overview, "/memory");
read_contract!(settings, "/memory/settings");
read_contract!(preferences, "/memory/preferences");
read_contract!(facts, "/memory/facts");
read_contract!(recent, "/memory/recent");
read_contract!(items, "/memory/items");
read_contract!(export, "/memory/export");
read_contract!(markdown, "/memory/export/markdown");
read_contract!(clear_preview, "/memory/clear/preview");
read_contract!(vector_status, "/memory/vector/status");
read_contract!(remote_disclosure, "/memory/remote-disclosure");
