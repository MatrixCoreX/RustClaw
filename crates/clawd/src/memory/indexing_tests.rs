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
    db
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
    rebuild_retrieval_index(&db, &MemoryConfig::default(), &std::env::temp_dir())
        .expect("rebuild retrieval index");

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

    rebuild_retrieval_index(&db, &MemoryConfig::default(), &std::env::temp_dir())
        .expect("rebuild retrieval index");

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
