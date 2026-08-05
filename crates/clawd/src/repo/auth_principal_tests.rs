use rusqlite::{params, Connection};

use super::{
    create_principal_for_auth_key, ensure_principal_identity_schema, principal_id_for_user_key,
    rotate_credential_binding,
};

fn setup_db() -> Connection {
    let db = Connection::open_in_memory().expect("principal fixture db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("auth schema");
    db.execute_batch(crate::WEBD_LOGIN_SQL)
        .expect("web login schema");
    db
}

#[test]
fn principal_backfill_is_idempotent_and_credential_digest_only() {
    let db = setup_db();
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES (?1, 'user', 1, '1')",
        ["synthetic-key-a"],
    )
    .expect("insert auth key");

    ensure_principal_identity_schema(&db).expect("first principal migration");
    let first = principal_id_for_user_key(&db, "synthetic-key-a")
        .expect("resolve principal")
        .expect("principal id");
    ensure_principal_identity_schema(&db).expect("repeat principal migration");
    let second = principal_id_for_user_key(&db, "synthetic-key-a")
        .expect("resolve principal again")
        .expect("principal id again");
    let raw_key_bindings: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM credential_bindings WHERE credential_digest = ?1",
            ["synthetic-key-a"],
            |row| row.get(0),
        )
        .expect("raw key binding count");

    assert_eq!(first, second);
    assert_eq!(raw_key_bindings, 0);
}

#[test]
fn rotation_preserves_principal_and_revokes_old_digest() {
    let db = setup_db();
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES (?1, 'user', 1, '1')",
        ["synthetic-key-old"],
    )
    .expect("insert auth key");
    let principal_id =
        create_principal_for_auth_key(&db, "synthetic-key-old", "user").expect("create principal");
    rotate_credential_binding(&db, &principal_id, "synthetic-key-old", "synthetic-key-new")
        .expect("rotate binding");

    assert!(principal_id_for_user_key(&db, "synthetic-key-old")
        .expect("resolve old")
        .is_none());
    assert_eq!(
        principal_id_for_user_key(&db, "synthetic-key-new")
            .expect("resolve new")
            .as_deref(),
        Some(principal_id.as_str())
    );
    let revoked: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM credential_bindings
             WHERE principal_id = ?1 AND status = 'revoked'",
            params![principal_id],
            |row| row.get(0),
        )
        .expect("revoked binding count");
    assert_eq!(revoked, 1);
}
