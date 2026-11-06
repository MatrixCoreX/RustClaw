use super::*;

const KEY: &str = "synthetic-channel-lifecycle";

fn fixture() -> AppState {
    let state = auth_lifecycle_test_state();
    state.seed_test_auth_identity(KEY, "user");
    let db = state.core.db.get().unwrap();
    db.execute(
        "INSERT INTO channel_bindings
         (channel, external_user_id, external_chat_id, user_key, principal_id, bound_at, updated_at)
         SELECT 'telegram', 'fixture-user', 'fixture-chat', user_key, principal_id, '1', '1'
         FROM auth_keys WHERE user_key = ?1",
        [KEY],
    )
    .unwrap();
    drop(db);
    state
}

fn resolve(state: &AppState, user: Option<&str>, chat: Option<&str>) -> Option<AuthIdentity> {
    resolve_channel_binding_identity(state, "telegram", user, chat).unwrap()
}

fn reject_revoked(user: Option<&str>, chat: Option<&str>) {
    let state = fixture();
    assert!(resolve(&state, user, chat).is_some());
    let db = state.core.db.get().unwrap();
    db.execute("UPDATE auth_keys SET last_used_at = NULL", [])
        .unwrap();
    db.execute(
        "UPDATE credential_bindings SET status = 'revoked', revoked_at = '2'",
        [],
    )
    .unwrap();
    drop(db);
    assert!(resolve(&state, user, chat).is_none());
    let used: Option<String> = state
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT last_used_at FROM auth_keys WHERE user_key = ?1",
            [KEY],
            |row| row.get(0),
        )
        .unwrap();
    assert!(used.is_none(), "denied identity must not be marked as used");
}

#[test]
fn channel_exact_match_observes_fresh_credential_revocation() {
    reject_revoked(Some("fixture-user"), Some("fixture-chat"));
}

#[test]
fn channel_chat_only_match_observes_fresh_credential_revocation() {
    reject_revoked(None, Some("fixture-chat"));
}

#[test]
fn channel_user_only_match_observes_fresh_credential_revocation() {
    reject_revoked(Some("fixture-user"), None);
}

#[test]
fn channel_user_fallback_observes_fresh_credential_revocation() {
    reject_revoked(Some("fixture-user"), Some("fixture-other-chat"));
}

#[test]
fn channel_identity_rejects_inactive_canonical_principals() {
    for status in ["frozen", "merged", "deleted"] {
        let state = fixture();
        state
            .core
            .db
            .get()
            .unwrap()
            .execute("UPDATE principals SET status = ?1", [status])
            .unwrap();
        assert!(
            resolve(&state, Some("fixture-user"), Some("fixture-chat")).is_none(),
            "{status}"
        );
    }
}

#[test]
fn channel_identity_rejects_conflicting_credential_ownership() {
    let state = fixture();
    state.seed_test_auth_identity("synthetic-other-channel-key", "user");
    let db = state.core.db.get().unwrap();
    db.execute(
        "UPDATE credential_bindings SET principal_id = (
             SELECT principal_id FROM auth_keys WHERE user_key = 'synthetic-other-channel-key'
         ) WHERE credential_digest = ?1",
        [auth_principal::credential_digest(KEY)],
    )
    .unwrap();
    drop(db);
    assert!(resolve(&state, Some("fixture-user"), Some("fixture-chat")).is_none());
}

#[test]
fn disabled_auth_key_still_blocks_channel_identity_and_bootstrap_does_not_enable_it() {
    let state = fixture();
    let db = state.core.db.get().unwrap();
    db.execute(
        "UPDATE auth_keys SET enabled = 0 WHERE user_key = ?1",
        [KEY],
    )
    .unwrap();
    ensure_principal_identity_schema(&db).unwrap();
    drop(db);
    assert!(resolve(&state, Some("fixture-user"), Some("fixture-chat")).is_none());
}

#[test]
fn canonical_channel_identity_survives_authorized_key_rotation_and_bootstrap() {
    let state = fixture();
    let before = resolve(&state, Some("fixture-user"), Some("fixture-chat")).unwrap();
    let mut db = state.core.db.get().unwrap();
    let rowid: i64 = db
        .query_row(
            "SELECT rowid FROM auth_keys WHERE user_key = ?1",
            [KEY],
            |row| row.get(0),
        )
        .unwrap();
    let tx = db.transaction().unwrap();
    rotate_auth_key_row(&tx, rowid, KEY, "synthetic-channel-rotated").unwrap();
    tx.commit().unwrap();
    ensure_principal_identity_schema(&db).unwrap();
    drop(db);
    assert!(resolve_auth_identity_by_key(&state, KEY).unwrap().is_none());
    let after = resolve(&state, Some("fixture-user"), Some("fixture-chat")).unwrap();
    assert_eq!(after.principal_id, before.principal_id);
    assert_eq!(after.user_id, before.user_id);
    assert_eq!(after.chat_id, before.chat_id);
    assert_eq!(after.user_key, "synthetic-channel-rotated");
}
