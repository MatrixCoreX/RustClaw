use super::*;
use crate::db_init;

#[test]
fn crypto_migration_is_verified_and_drops_the_legacy_table() {
    let main = db_init::test_pool();
    let crypto = db_init::test_pool();
    {
        let db = main.get().expect("main");
        db.execute_batch(
            "CREATE TABLE exchange_api_credentials (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_key TEXT NOT NULL,
                exchange TEXT NOT NULL,
                api_key TEXT NOT NULL,
                api_secret TEXT NOT NULL,
                passphrase TEXT,
                enabled INTEGER NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(user_key, exchange)
            );",
        )
        .expect("legacy schema");
        db.execute(
            "INSERT INTO exchange_api_credentials
             (user_key, exchange, api_key, api_secret, passphrase, enabled, updated_at)
             VALUES ('rk-user', 'okx', 'key', 'secret', 'phrase', 1, '2026-07-24')",
            [],
        )
        .expect("legacy row");
    }
    {
        let db = crypto.get().expect("crypto");
        crate::skill_storage::schema::ensure_crypto_schema(&db).expect("crypto schema");
    }
    migrate_legacy_crypto(&main, &crypto).expect("migrate");
    migrate_legacy_crypto(&main, &crypto).expect("idempotent second start");
    let main_db = main.get().expect("main");
    assert!(!table_exists(&main_db, "exchange_api_credentials").expect("table check"));
    let crypto_db = crypto.get().expect("crypto");
    let count: i64 = crypto_db
        .query_row("SELECT COUNT(*) FROM exchange_api_credentials", [], |row| {
            row.get(0)
        })
        .expect("credential count");
    assert_eq!(count, 1);
}

#[test]
fn kb_migration_moves_only_kb_rows_out_of_the_main_index() {
    let main = db_init::test_pool();
    let kb = db_init::test_pool();
    {
        let db = main.get().expect("main");
        crate::memory::indexing::ensure_retrieval_schema(&db).expect("main schema");
        db.execute(
            "INSERT INTO memory_retrieval_index (
                source_kind, source_ref, user_id, chat_id, user_key, memory_kind,
                search_text, topic_tags, vector_json, metadata_json, salience,
                success_state, tool_or_skill_name, created_at_ts, updated_at_ts
             ) VALUES (
                'kb_doc', 'kb:rk-user:docs:chunk-1', 0, 0, 'rk-user',
                'knowledge_doc', 'isolated knowledge', 'isolated knowledge',
                '[]', '{}', 0.78, 'succeeded', 'kb', 1, 1
             )",
            [],
        )
        .expect("KB row");
        db.execute(
            "INSERT INTO memory_retrieval_index (
                source_kind, user_id, chat_id, user_key, memory_kind,
                search_text, topic_tags, vector_json, metadata_json, salience,
                success_state, created_at_ts, updated_at_ts
             ) VALUES (
                'memory', 1, 1, 'rk-user', 'episodic_event',
                'runtime memory', 'runtime memory', '[]', '{}', 0.5,
                'neutral', 1, 1
             )",
            [],
        )
        .expect("runtime row");
    }
    {
        let db = kb.get().expect("kb");
        crate::skill_storage::schema::ensure_kb_schema(&db).expect("KB schema");
    }
    migrate_legacy_kb_rows(&main, &kb).expect("migrate");
    migrate_legacy_kb_rows(&main, &kb).expect("idempotent second start");
    let main_db = main.get().expect("main");
    let main_kb_count: i64 = main_db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_kind = 'kb_doc'",
            [],
            |row| row.get(0),
        )
        .expect("main KB count");
    let runtime_count: i64 = main_db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_kind = 'memory'",
            [],
            |row| row.get(0),
        )
        .expect("runtime row count");
    assert_eq!(main_kb_count, 0);
    assert_eq!(runtime_count, 1);
    let kb_db = kb.get().expect("kb");
    let kb_count: i64 = kb_db
        .query_row("SELECT COUNT(*) FROM memory_retrieval_index", [], |row| {
            row.get(0)
        })
        .expect("KB count");
    assert_eq!(kb_count, 1);
}
