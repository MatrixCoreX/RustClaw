use super::*;
use rusqlite::types::Value;

const KEY: &str = "synthetic-atomicity-key";
const NEXT: &str = "synthetic-atomicity-next";

fn fixture() -> (Connection, String) {
    let db = setup_db();
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at) VALUES (?1, 'user', 1, '1')",
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
        let mut stmt = db.prepare(sql).unwrap();
        let columns = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                (0..columns)
                    .map(|i| row.get(i))
                    .collect::<rusqlite::Result<Vec<Value>>>()
            })
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    })
    .collect()
}

fn fail_binding_insert(db: &Connection) {
    db.execute_batch(
        "CREATE TRIGGER fixture_binding_failure BEFORE INSERT ON credential_bindings
        BEGIN SELECT RAISE(ABORT, 'synthetic binding insertion failure'); END;",
    )
    .unwrap();
}

#[test]
fn standalone_rotation_rolls_back_old_revocation_when_new_binding_insert_fails() {
    let (db, principal) = fixture();
    fail_binding_insert(&db);
    let before = snapshot(&db);
    rotate_credential_binding(&db, &principal, KEY, NEXT).expect_err("injected binding failure");
    assert_eq!(snapshot(&db), before);
    assert_eq!(
        principal_id_for_user_key(&db, KEY).unwrap(),
        Some(principal)
    );
    assert!(principal_id_for_user_key(&db, NEXT).unwrap().is_none());
    assert!(db.is_autocommit());
}

#[test]
fn caller_owned_transaction_can_commit_rotation_without_an_implicit_inner_commit() {
    let (mut db, principal) = fixture();
    let tx = db.transaction().unwrap();
    tx.execute_batch("CREATE TABLE fixture_outer_work (value TEXT); INSERT INTO fixture_outer_work VALUES ('keep');").unwrap();
    rotate_credential_binding(&tx, &principal, KEY, NEXT).unwrap();
    assert!(!tx.is_autocommit());
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT value FROM fixture_outer_work", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
    assert!(principal_id_for_user_key(&db, KEY).unwrap().is_none());
    assert_eq!(
        principal_id_for_user_key(&db, NEXT).unwrap(),
        Some(principal)
    );
}

#[test]
fn propagated_rotation_failure_rolls_back_the_callers_transaction_and_its_outer_work() {
    let (mut db, principal) = fixture();
    db.execute_batch("CREATE TABLE fixture_outer_work (value TEXT);")
        .unwrap();
    fail_binding_insert(&db);
    let before = snapshot(&db);
    let result = (|| -> anyhow::Result<()> {
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO fixture_outer_work VALUES ('must roll back')",
            [],
        )?;
        rotate_credential_binding(&tx, &principal, KEY, NEXT)?;
        tx.commit()?;
        Ok(())
    })();
    assert!(result.is_err());
    assert_eq!(snapshot(&db), before);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM fixture_outer_work", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(db.is_autocommit());
}

#[test]
fn late_backfill_and_creation_failures_do_not_leave_partial_principals_or_owner_assignments() {
    let (db, _) = fixture();
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at) VALUES (?1, 'user', 1, '2')",
        [NEXT],
    )
    .unwrap();
    fail_binding_insert(&db);
    let before = snapshot(&db);
    ensure_principal_identity_schema(&db).expect_err("injected backfill failure");
    assert_eq!(snapshot(&db), before);
    create_principal_for_auth_key(&db, NEXT, "user").expect_err("injected creation failure");
    assert_eq!(snapshot(&db), before);
}

#[test]
fn ownership_digest_drift_fails_before_late_identity_backfill_and_preserves_all_rows() {
    let (db, _) = fixture();
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at) VALUES (?1, 'user', 1, '2')",
        [NEXT],
    )
    .unwrap();
    db.execute(
        "INSERT INTO runtime_schema_migrations (migration_id,schema_digest,applied_at)
        VALUES ('009_principal_ownership_v1','sha256:wrong','1')",
        [],
    )
    .unwrap();
    let before = snapshot(&db);
    let error =
        crate::repo::principal_ownership::ensure_principal_ownership_schema(&db).unwrap_err();
    assert!(error
        .to_string()
        .contains("runtime_schema_migration_digest_mismatch"));
    assert_eq!(snapshot(&db), before);
}

#[test]
fn bootstrap_admin_login_failure_rolls_back_new_auth_key_principal_and_binding() {
    let db = setup_db();
    ensure_principal_identity_schema(&db).unwrap();
    db.execute_batch(
        "CREATE TRIGGER fixture_web_login_failure BEFORE INSERT ON webd_login_accounts
        BEGIN SELECT RAISE(ABORT, 'synthetic web login insertion failure'); END;",
    )
    .unwrap();
    let before = snapshot(&db);
    crate::repo::auth::ensure_bootstrap_admin_key(&db).expect_err("injected web login failure");
    assert_eq!(snapshot(&db), before);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM webd_login_accounts", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn full_batch_owner_conflict_is_detected_before_any_pending_import_write() {
    let (mut db, _) = fixture();
    db.execute(
        "INSERT INTO auth_keys (user_key,role,enabled,created_at) VALUES (?1,'user',1,'0')",
        [NEXT],
    )
    .unwrap();
    db.execute(
        "INSERT INTO principals (principal_id,role,status,revision,created_at,updated_at)
        VALUES ('synthetic-other-owner','user','active',1,'1','1')",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE credential_bindings SET principal_id='synthetic-other-owner'",
        [],
    )
    .unwrap();
    let before = snapshot(&db);
    let tx = db.transaction().unwrap();
    ensure_principal_identity_schema(&tx).expect_err("later conflict must precede every write");
    assert_eq!(snapshot(&tx), before);
    tx.rollback().unwrap();
}

#[test]
fn normalized_credential_collision_cannot_implicitly_merge_or_move_principal_ownership() {
    let (db, _) = fixture();
    db.execute(
        "INSERT INTO auth_keys (user_key,role,enabled,created_at) VALUES (?1,'user',1,'0')",
        [format!(" {KEY} ")],
    )
    .unwrap();
    let before = snapshot(&db);
    let error = ensure_principal_identity_schema(&db).unwrap_err();
    assert!(error.to_string().contains("identity_credential_collision"));
    assert_eq!(snapshot(&db), before);
}
