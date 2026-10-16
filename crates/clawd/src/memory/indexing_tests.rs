use claw_core::config::MemoryConfig;
use rusqlite::{params, Connection};

use super::{ensure_retrieval_schema, rebuild_retrieval_index};
use crate::memory::facts::{upsert_memory_fact_card, MemoryFactUpsert};

fn setup_db() -> Connection {
    let db = Connection::open_in_memory().expect("open memory db");
    db.execute_batch(crate::INIT_SQL).expect("init base schema");
    crate::db_init::ensure_memory_schema(&db).expect("ensure memory schema");
    crate::repo::auth::ensure_key_auth_schema(&db).expect("ensure key auth schema");
    ensure_retrieval_schema(&db).expect("ensure retrieval schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES ('user:test', 'user', 1, '1')",
        [],
    )
    .expect("insert test auth key");
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("principal schema");
    crate::repo::ensure_principal_ownership_schema(&db).expect("ownership schema");
    crate::memory::scope::ensure_memory_scope_schema(&db).expect("scope schema");
    db
}

#[test]
fn retrieval_schema_defaults_to_the_single_current_embedding_profile() {
    let db = Connection::open_in_memory().expect("open memory db");
    ensure_retrieval_schema(&db).expect("ensure retrieval schema");
    db.execute(
        "INSERT INTO memory_retrieval_index (
            source_kind, user_id, chat_id, memory_kind, search_text
         ) VALUES ('memory', 1, 1, 'episodic_event', 'schema default probe')",
        [],
    )
    .expect("insert row using schema defaults");

    let actual: (String, usize, String) = db
        .query_row(
            "SELECT embedding_model, embedding_dims, embedding_version
             FROM memory_retrieval_index",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("embedding defaults");
    assert_eq!(
        actual,
        (
            claw_core::retrieval_embedding::LOCAL_HASH_MODEL_ID.to_string(),
            claw_core::retrieval_embedding::LOCAL_HASH_DIMS,
            claw_core::retrieval_embedding::LOCAL_HASH_VERSION.to_string(),
        )
    );
}

#[test]
fn rebuild_retrieval_index_restores_memory_preference_and_fact_rows() {
    let db = setup_db();
    db.execute(
        "INSERT INTO memories
         (user_id, chat_id, user_key, channel, role, content, created_at, created_at_ts, memory_type, salience, is_instructional, safety_flag)
         VALUES (?1, ?2, ?3, 'ui', 'user', '用户正在测试记忆索引重建', '1000', ?4, 'generic', 0.8, 0, 'normal')",
        params![7, 11, "user:test", 1000_i64],
    )
    .expect("insert memory");
    db.execute(
        "INSERT INTO user_preferences
         (user_id, chat_id, user_key, pref_key, pref_value, confidence, source, updated_at, updated_at_ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            7,
            11,
            "user:test",
            "response_language",
            "zh-CN",
            0.96_f32,
            "memory_extract",
            "1000",
            1000_i64,
        ],
    )
    .expect("insert preference");
    let source_ids = [1_i64];
    let fact = MemoryFactUpsert::from_long_term_summary(
        "user_profile",
        "response_language",
        "zh-CN",
        "以后默认用中文回复",
        0.96,
        "long_term_summary:42",
        &source_ids,
        "explicit durable preference",
        Some("user_profile:response_language"),
    );
    upsert_memory_fact_card(&db, 7, 11, "user:test", &fact, 1001).expect("upsert fact");

    db.execute("DELETE FROM memory_retrieval_index", [])
        .expect("clear index");
    rebuild_retrieval_index(&db, &MemoryConfig::default()).expect("rebuild retrieval index");

    let memory_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_kind = ?1",
            [crate::memory::RETRIEVAL_SOURCE_MEMORY],
            |row| row.get(0),
        )
        .expect("memory rows");
    let preference_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_kind = ?1",
            [crate::memory::RETRIEVAL_SOURCE_PREFERENCE],
            |row| row.get(0),
        )
        .expect("preference rows");
    let fact_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_kind = ?1",
            [crate::memory::RETRIEVAL_SOURCE_MEMORY_FACT],
            |row| row.get(0),
        )
        .expect("fact rows");
    let embedding_version: String = db
        .query_row(
            "SELECT embedding_version FROM memory_retrieval_index WHERE source_kind = ?1 LIMIT 1",
            [crate::memory::RETRIEVAL_SOURCE_PREFERENCE],
            |row| row.get(0),
        )
        .expect("embedding version");

    assert!(memory_rows >= 1);
    assert_eq!(preference_rows, 1);
    assert_eq!(fact_rows, 1);
    assert_eq!(
        embedding_version,
        crate::memory::embedding::local_hash_embedding_spec().version
    );
}

#[test]
fn rebuild_retrieval_index_skips_safety_signal_memories() {
    let db = setup_db();
    db.execute(
        "INSERT INTO memories
         (user_id, chat_id, user_key, channel, role, content, created_at, created_at_ts, memory_type, salience, is_instructional, safety_flag)
         VALUES (?1, ?2, ?3, 'ui', 'user', 'policy-sensitive memory row', '1000', ?4, ?5, 0.2, 0, ?6)",
        params![
            7,
            11,
            "user:test",
            1000_i64,
            crate::memory::MEMORY_TYPE_SAFETY_SIGNAL,
            crate::memory::MEMORY_SAFETY_FLAG_INJECTION_LIKE
        ],
    )
    .expect("insert safety signal memory");
    let safety_memory_id = db.last_insert_rowid();

    rebuild_retrieval_index(&db, &MemoryConfig::default()).expect("rebuild retrieval index");

    let rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index
             WHERE source_kind = ?1 AND source_memory_id = ?2",
            params![crate::memory::RETRIEVAL_SOURCE_MEMORY, safety_memory_id],
            |row| row.get(0),
        )
        .expect("safety signal index rows");
    assert_eq!(rows, 0);
}

#[test]
fn rebuild_retrieval_index_skips_legacy_rows_without_a_principal_binding() {
    let db = setup_db();
    db.execute(
        "INSERT INTO user_preferences
         (user_id, chat_id, user_key, pref_key, pref_value, confidence, source,
          updated_at, updated_at_ts, scope_kind, scope_ref)
         VALUES (77, 88, 'anon:77:88', 'agent_display_name', 'Legacy', 0.9,
                 'legacy', '1000', 1000, 'principal', NULL)",
        [],
    )
    .expect("insert unowned legacy preference");

    rebuild_retrieval_index(&db, &MemoryConfig::default()).expect("rebuild retrieval index");

    let indexed: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_pref_key = 'agent_display_name'",
            [],
            |row| row.get(0),
        )
        .expect("count unowned index rows");
    assert_eq!(indexed, 0);
}

#[test]
fn failed_rebuild_preserves_the_previous_index_generation() {
    let db = setup_db();
    db.execute(
        "INSERT INTO memory_retrieval_index (
            source_kind, user_id, chat_id, user_key, memory_kind, search_text,
            topic_tags, vector_json, metadata_json, created_at_ts, updated_at_ts
         ) VALUES ('sentinel', 7, 11, 'user:test', 'semantic_fact',
                   'previous generation', '', '[]', '{}', 1, 1)",
        [],
    )
    .expect("insert previous generation sentinel");
    db.execute(
        "INSERT INTO user_preferences
         (user_id, chat_id, user_key, pref_key, pref_value, confidence, source,
          updated_at, updated_at_ts, scope_kind, scope_ref)
         VALUES (7, 11, 'user:test', 'invalid_scope', 'value', 0.9,
                 'test', '1000', 1000, 'conversation', NULL)",
        [],
    )
    .expect("insert invalid scoped preference");

    let error = rebuild_retrieval_index(&db, &MemoryConfig::default())
        .expect_err("invalid scoped source must fail closed");
    assert!(error
        .to_string()
        .contains("memory_retrieval_scope_ref_required"));

    let sentinel_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_kind = 'sentinel'",
            [],
            |row| row.get(0),
        )
        .expect("count preserved sentinel rows");
    assert_eq!(sentinel_rows, 1);
}
