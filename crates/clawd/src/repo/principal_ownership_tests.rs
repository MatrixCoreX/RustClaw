use rusqlite::Connection;

use super::ensure_principal_ownership_schema;

fn setup_db() -> Connection {
    let db = Connection::open_in_memory().expect("ownership fixture db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    crate::db_init::ensure_schedule_schema(&db).expect("schedule schema");
    crate::db_init::ensure_memory_schema(&db).expect("memory schema");
    crate::db_init::ensure_channel_schema(&db).expect("channel schema");
    crate::repo::auth::ensure_key_auth_schema(&db).expect("auth schema");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
    db
}

#[test]
fn ownership_migration_backfills_all_current_runtime_domains_and_is_idempotent() {
    let db = setup_db();
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES ('key-a', 'user', 1, '1')",
        [],
    )
    .expect("auth row");
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("identity migration");
    let principal_id: String = db
        .query_row(
            "SELECT principal_id FROM auth_keys WHERE user_key = 'key-a'",
            [],
            |row| row.get(0),
        )
        .expect("principal id");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, kind, payload_json, status, created_at, updated_at
         ) VALUES ('task-a', 1, 2, 'key-a', 'ask', '{}', 'queued', '1', '1')",
        [],
    )
    .expect("task row");
    db.execute(
        "INSERT INTO memories (
            user_id, chat_id, user_key, role, content, created_at
         ) VALUES (1, 2, 'key-a', 'user', 'fixture', '1')",
        [],
    )
    .expect("memory row");
    db.execute(
        "INSERT INTO conversation_metadata (
            owner_user_key, owner_user_id, conversation_id, title, created_at, updated_at
         ) VALUES ('key-a', 1, 'conversation-a', 'fixture', '1', '1')",
        [],
    )
    .expect("conversation metadata");

    ensure_principal_ownership_schema(&db).expect("first ownership migration");
    ensure_principal_ownership_schema(&db).expect("repeat ownership migration");

    for (table, column) in [
        ("tasks", "principal_id"),
        ("memories", "principal_id"),
        ("conversation_metadata", "owner_principal_id"),
    ] {
        let actual: String = db
            .query_row(
                &format!("SELECT {column} FROM {table} LIMIT 1"),
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|error| panic!("read {table}.{column}: {error}"));
        assert_eq!(actual, principal_id, "{table}.{column}");
    }
    let applied: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM runtime_schema_migrations
             WHERE migration_id = '009_principal_ownership_v1'",
            [],
            |row| row.get(0),
        )
        .expect("migration count");
    assert_eq!(applied, 1);
}

#[test]
fn ownership_migration_rejects_digest_drift_without_changing_rows() {
    let db = setup_db();
    db.execute(
        "INSERT INTO runtime_schema_migrations (migration_id, schema_digest, applied_at)
         VALUES ('009_principal_ownership_v1', 'sha256:wrong', '1')",
        [],
    )
    .expect("poison migration digest");
    let error = ensure_principal_ownership_schema(&db).expect_err("digest drift must fail");
    assert!(error
        .to_string()
        .contains("runtime_schema_migration_digest_mismatch"));
    let task_columns: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'principal_id'",
            [],
            |row| row.get(0),
        )
        .expect("task principal column count");
    assert_eq!(task_columns, 0);
}
