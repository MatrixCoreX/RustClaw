use claw_core::types::AuthIdentity;
use rusqlite::Connection;

use super::{ensure_memory_scope_schema, resolve_conversation_scope, resolve_principal_scope};

fn setup_db() -> (Connection, AuthIdentity) {
    let db = Connection::open_in_memory().expect("scope fixture db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    crate::db_init::ensure_schedule_schema(&db).expect("schedule schema");
    crate::db_init::ensure_memory_schema(&db).expect("memory schema");
    crate::db_init::ensure_channel_schema(&db).expect("channel schema");
    crate::repo::auth::ensure_key_auth_schema(&db).expect("auth schema");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES ('scope-key', 'user', 1, '1')",
        [],
    )
    .expect("auth key");
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("principal backfill");
    crate::repo::ensure_principal_ownership_schema(&db).expect("ownership backfill");
    let principal_id: String = db
        .query_row(
            "SELECT principal_id FROM auth_keys WHERE user_key = 'scope-key'",
            [],
            |row| row.get(0),
        )
        .expect("principal id");
    (
        db,
        AuthIdentity {
            user_key: "scope-key".to_string(),
            principal_id,
            role: "user".to_string(),
            user_id: 9,
            chat_id: 11,
        },
    )
}

#[test]
fn legacy_memory_rows_receive_opaque_ids_and_proven_principal_scope() {
    let (db, identity) = setup_db();
    db.execute(
        "INSERT INTO memories (
            user_id, chat_id, user_key, principal_id, role, content, created_at
         ) VALUES (9, 11, 'scope-key', ?1, 'user', 'legacy recent', '1')",
        [&identity.principal_id],
    )
    .expect("legacy memory");
    db.execute(
        "INSERT INTO memory_facts (
            user_id, chat_id, user_key, principal_id, scope_kind, namespace, fact_text
         ) VALUES (9, 11, 'scope-key', ?1, 'user', 'project_facts', 'legacy fact')",
        [&identity.principal_id],
    )
    .expect("legacy project namespace fact");

    ensure_memory_scope_schema(&db).expect("first scope migration");
    let first_id: String = db
        .query_row("SELECT memory_id FROM memories LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("memory id");
    ensure_memory_scope_schema(&db).expect("repeat scope migration");
    let repeated_id: String = db
        .query_row("SELECT memory_id FROM memories LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("repeated memory id");
    assert_eq!(first_id, repeated_id);
    assert!(first_id.starts_with("memory_"));
    let (scope_kind, scope_ref, inferred): (String, String, i64) = db
        .query_row(
            "SELECT scope_kind, scope_ref, legacy_scope_inferred
             FROM memory_facts LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("fact scope");
    assert_eq!(scope_kind, "principal");
    assert_eq!(scope_ref, identity.principal_id);
    assert_eq!(inferred, 0);
}

#[test]
fn conversation_scope_is_stable_per_principal_and_never_uses_chat_id_alone() {
    let (db, identity) = setup_db();
    ensure_memory_scope_schema(&db).expect("scope migration");
    let first =
        resolve_conversation_scope(&identity, "conversation-a").expect("first conversation scope");
    let repeated = resolve_conversation_scope(&identity, "conversation-a")
        .expect("repeated conversation scope");
    let other_conversation =
        resolve_conversation_scope(&identity, "conversation-b").expect("other conversation scope");
    let mut other_identity = identity.clone();
    other_identity.principal_id = "principal-other".to_string();
    let other_principal = resolve_conversation_scope(&other_identity, "conversation-a")
        .expect("other principal scope");
    assert_eq!(first, repeated);
    assert_ne!(first.scope_ref, other_conversation.scope_ref);
    assert_ne!(first.scope_ref, other_principal.scope_ref);
    assert_eq!(
        resolve_principal_scope(&identity).scope_ref,
        identity.principal_id
    );
    assert!(first.scope_ref.starts_with("conversation_"));
    assert_eq!(first.scope_ref.len(), "conversation_".len() + 64);
    assert_ne!(first.scope_ref, identity.chat_id.to_string());
    assert_ne!(
        first.scope_ref,
        format!("{}:{}", identity.principal_id, identity.chat_id)
    );

    let migration_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM runtime_schema_migrations
             WHERE migration_id = '010_memory_scope_contract_v1'",
            [],
            |row| row.get(0),
        )
        .expect("migration count");
    assert_eq!(migration_count, 1);
}

#[test]
fn read_only_interruption_leaves_no_partial_scope_migration_and_retry_succeeds() {
    let (db, _identity) = setup_db();
    db.pragma_update(None, "query_only", true)
        .expect("enable query-only interruption");
    let error = ensure_memory_scope_schema(&db).expect_err("query-only migration must fail");
    assert!(!error.to_string().is_empty());
    let recorded: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM runtime_schema_migrations
             WHERE migration_id = '010_memory_scope_contract_v1'",
            [],
            |row| row.get(0),
        )
        .expect("migration record after interruption");
    assert_eq!(recorded, 0);
    let scope_columns: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = 'scope_ref'",
            [],
            |row| row.get(0),
        )
        .expect("scope columns after interruption");
    assert_eq!(scope_columns, 0);

    db.pragma_update(None, "query_only", false)
        .expect("disable query-only interruption");
    ensure_memory_scope_schema(&db).expect("retry scope migration");
    let recorded: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM runtime_schema_migrations
             WHERE migration_id = '010_memory_scope_contract_v1'",
            [],
            |row| row.get(0),
        )
        .expect("migration record after retry");
    assert_eq!(recorded, 1);
}
