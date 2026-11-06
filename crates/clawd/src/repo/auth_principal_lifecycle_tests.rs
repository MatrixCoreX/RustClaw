use super::*;
use rusqlite::types::Value;

const KEY: &str = "synthetic-lifecycle-key";

fn fixture() -> (Connection, String) {
    let db = setup_db();
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES (?1, 'user', 1, '1')",
        [KEY],
    )
    .unwrap();
    ensure_principal_identity_schema(&db).unwrap();
    let principal = principal_id_for_user_key(&db, KEY).unwrap().unwrap();
    (db, principal)
}

fn snapshot(db: &Connection) -> Vec<Vec<Vec<Value>>> {
    [
        "SELECT * FROM auth_keys ORDER BY user_key",
        "SELECT * FROM principals ORDER BY principal_id",
        "SELECT * FROM credential_bindings ORDER BY credential_digest",
    ]
    .into_iter()
    .map(|sql| {
        let mut statement = db.prepare(sql).unwrap();
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|index| row.get(index))
                    .collect::<rusqlite::Result<Vec<Value>>>()
            })
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    })
    .collect()
}

fn revoke(db: &Connection) {
    db.execute(
        "UPDATE credential_bindings SET status = 'revoked', revoked_at = 'revoked-sentinel'",
        [],
    )
    .unwrap();
}

#[test]
fn bootstrap_does_not_revive_a_rotated_credential_or_erase_its_revocation_time() {
    let (db, principal) = fixture();
    rotate_credential_binding(&db, &principal, KEY, "synthetic-next-key").unwrap();
    assert!(principal_id_for_user_key(&db, KEY).unwrap().is_none());
    let before = snapshot(&db);
    ensure_principal_identity_schema(&db).unwrap();
    assert!(principal_id_for_user_key(&db, KEY).unwrap().is_none());
    assert_eq!(snapshot(&db), before);
    assert_eq!(
        principal_id_for_user_key(&db, "synthetic-next-key").unwrap(),
        Some(principal)
    );
}

#[test]
fn bootstrap_of_an_initialized_identity_has_no_row_writes() {
    let (db, _) = fixture();
    let before: i64 = db
        .query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap();
    ensure_principal_identity_schema(&db).unwrap();
    let after: i64 = db
        .query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        after, before,
        "schema reads must not rewrite every identity"
    );
}

#[test]
fn bootstrap_preserves_explicit_principal_state_revision_and_timestamps() {
    let (db, principal) = fixture();
    db.execute(
        "UPDATE principals SET status = 'frozen', revision = 19,
         updated_at = 'principal-state-sentinel' WHERE principal_id = ?1",
        [&principal],
    )
    .unwrap();
    let before = snapshot(&db);
    ensure_principal_identity_schema(&db).unwrap();
    assert_eq!(snapshot(&db), before);
    assert!(principal_id_for_user_key(&db, KEY).unwrap().is_none());
}

#[test]
fn bootstrap_rejects_a_conflicting_binding_owner_without_mutating_any_identity() {
    let (db, _) = fixture();
    db.execute(
        "INSERT INTO principals (principal_id, role, status, revision, created_at, updated_at)
         VALUES ('other-principal', 'user', 'active', 3, '2', '2')",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE credential_bindings SET principal_id = 'other-principal'",
        [],
    )
    .unwrap();
    let before = snapshot(&db);
    ensure_principal_identity_schema(&db).expect_err("bootstrap cannot reassign a binding owner");
    assert_eq!(snapshot(&db), before);
}

#[test]
fn bootstrap_late_import_preserves_existing_revocation_and_canonical_ownership() {
    let (db, principal) = fixture();
    revoke(&db);
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES ('synthetic-late-key', 'user', 1, '2')",
        [],
    )
    .unwrap();
    ensure_principal_identity_schema(&db).unwrap();
    assert!(principal_id_for_user_key(&db, KEY).unwrap().is_none());
    let old_owner: String = db
        .query_row(
            "SELECT principal_id FROM auth_keys WHERE user_key = ?1",
            [KEY],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_owner, principal);
    let late = principal_id_for_user_key(&db, "synthetic-late-key")
        .unwrap()
        .unwrap();
    assert_ne!(late, principal);
    let before = snapshot(&db);
    ensure_principal_identity_schema(&db).unwrap();
    assert_eq!(snapshot(&db), before);
}

#[test]
fn bootstrap_recovers_missing_auth_owner_from_the_existing_binding_without_reassigning_it() {
    let (db, principal) = fixture();
    db.execute(
        "UPDATE auth_keys SET principal_id = NULL WHERE user_key = ?1",
        [KEY],
    )
    .unwrap();
    ensure_principal_identity_schema(&db).unwrap();
    assert_eq!(
        principal_id_for_user_key(&db, KEY).unwrap(),
        Some(principal.clone())
    );
    let owner: String = db
        .query_row(
            "SELECT principal_id FROM auth_keys WHERE user_key = ?1",
            [KEY],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(owner, principal);
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM principals", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn bootstrap_rejects_digest_drift_before_identity_writes() {
    let (db, _) = fixture();
    db.execute(
        "UPDATE runtime_schema_migrations SET schema_digest = 'sha256:wrong'",
        [],
    )
    .unwrap();
    let before = snapshot(&db);
    let error = ensure_principal_identity_schema(&db).unwrap_err();
    assert!(error
        .to_string()
        .contains("runtime_schema_migration_digest_mismatch"));
    assert_eq!(snapshot(&db), before);
}

#[test]
fn repeated_creation_cannot_revive_a_revoked_binding() {
    let (db, _) = fixture();
    revoke(&db);
    let before = snapshot(&db);
    create_principal_for_auth_key(&db, KEY, "user")
        .expect_err("revocation is not bootstrap authority");
    assert_eq!(snapshot(&db), before);
}

#[test]
fn repeated_creation_cannot_replace_frozen_merged_or_deleted_principals() {
    for status in ["frozen", "merged", "deleted"] {
        let (db, principal) = fixture();
        db.execute(
            "UPDATE principals SET status = ?2 WHERE principal_id = ?1",
            params![principal, status],
        )
        .unwrap();
        let before = snapshot(&db);
        create_principal_for_auth_key(&db, KEY, "user")
            .expect_err("inactive owner must not be replaced");
        assert_eq!(snapshot(&db), before, "{status}");
    }
}

#[test]
fn creation_requires_an_existing_auth_key_and_does_not_leave_orphan_grants() {
    let (db, _) = fixture();
    let before = snapshot(&db);
    create_principal_for_auth_key(&db, "synthetic-absent-key", "admin")
        .expect_err("missing auth key");
    assert_eq!(snapshot(&db), before);
}

#[test]
fn rotation_cannot_steal_another_principals_binding_or_revoke_the_source_on_failure() {
    let (db, principal) = fixture();
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES ('synthetic-other-key', 'user', 1, '2')",
        [],
    )
    .unwrap();
    ensure_principal_identity_schema(&db).unwrap();
    let before = snapshot(&db);
    rotate_credential_binding(&db, &principal, KEY, "synthetic-other-key")
        .expect_err("destination owner conflict");
    assert_eq!(snapshot(&db), before);
}
