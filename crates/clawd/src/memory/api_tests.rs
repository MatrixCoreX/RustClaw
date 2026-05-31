use std::fs;

use rusqlite::{params, Connection};

use super::{
    clear_memory_scope, delete_memory_object, expire_memory_object, list_facts, list_preferences,
    memory_overview, update_memory_settings_file, MemoryClearScope, MemoryCounts,
    MemorySettingsRequest,
};
use crate::memory::facts::{upsert_memory_fact_card, MemoryFactUpsert};

fn setup_db() -> Connection {
    let db = Connection::open_in_memory().expect("open memory db");
    db.execute_batch(crate::INIT_SQL).expect("init base schema");
    crate::db_init::ensure_memory_schema(&db).expect("ensure memory schema");
    crate::repo::auth::ensure_key_auth_schema(&db).expect("ensure key auth schema");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("ensure retrieval schema");
    db
}

#[test]
fn memory_api_lists_preferences_and_facts() {
    let db = setup_db();
    crate::memory::indexing::index_preference_entries(
        &db,
        7,
        11,
        "user:test",
        &[(
            "response_language".to_string(),
            "zh-CN".to_string(),
            0.96,
            "memory_extract".to_string(),
        )],
        1000,
    )
    .expect("index preference");
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
    let source_ids = [42_i64];
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

    let prefs = list_preferences(&db, 7, 11, "user:test").expect("list preferences");
    let facts = list_facts(&db, 7, "user:test").expect("list facts");
    let overview = memory_overview(&db, 7, 11, "user:test", true, true).expect("memory overview");

    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0].id, "preference:1");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].id, "fact:1");
    assert_eq!(facts[0].reason, "explicit durable preference");
    assert_eq!(
        overview.counts,
        MemoryCounts {
            recent: 0,
            preferences: 1,
            facts_active: 1,
            facts_total: 1,
            long_term_summaries: 0,
        }
    );
}

#[test]
fn memory_api_delete_fact_marks_deleted_and_removes_index() {
    let db = setup_db();
    let source_ids = [42_i64];
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

    let deleted = delete_memory_object(&db, 7, 11, "user:test", "fact:1", 1100)
        .expect("delete fact")
        .expect("deleted fact");

    let status: String = db
        .query_row("SELECT status FROM memory_facts WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("fact status");
    let index_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_ref = 'memory_fact:1'",
            [],
            |row| row.get(0),
        )
        .expect("index rows");
    assert_eq!(deleted.kind, "fact");
    assert_eq!(status, crate::memory::MEMORY_FACT_STATUS_DELETED);
    assert_eq!(index_rows, 0);
}

#[test]
fn memory_api_expire_fact_marks_expired_and_removes_index() {
    let db = setup_db();
    let source_ids = [42_i64];
    let fact = MemoryFactUpsert::from_long_term_summary(
        "user_profile",
        "timezone",
        "Asia/Shanghai",
        "用户常用时区是 Asia/Shanghai",
        0.91,
        "long_term_summary:42",
        &source_ids,
        "stable profile fact",
        Some("user_profile:timezone"),
    );
    upsert_memory_fact_card(&db, 7, 11, "user:test", &fact, 1001).expect("upsert fact");

    let expired = expire_memory_object(&db, 7, 11, "user:test", "fact:1", 1200)
        .expect("expire fact")
        .expect("expired fact");

    let (status, expires_at_ts): (String, i64) = db
        .query_row(
            "SELECT status, COALESCE(expires_at_ts, 0) FROM memory_facts WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("fact status");
    let index_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_ref = 'memory_fact:1'",
            [],
            |row| row.get(0),
        )
        .expect("index rows");
    assert_eq!(expired.kind, "fact");
    assert_eq!(expired.expired, true);
    assert_eq!(status, crate::memory::MEMORY_FACT_STATUS_EXPIRED);
    assert_eq!(expires_at_ts, 1200);
    assert_eq!(index_rows, 0);
}

#[test]
fn memory_api_clear_all_removes_scoped_records_and_indexes() {
    let db = setup_db();
    db.execute(
        "INSERT INTO memories
         (user_id, chat_id, user_key, channel, role, content, created_at, created_at_ts, memory_type, salience, is_instructional, safety_flag)
         VALUES (?1, ?2, ?3, 'ui', 'user', '记住我的测试上下文', '1000', ?4, 'generic', 0.8, 0, 'normal')",
        params![7, 11, "user:test", 1000_i64],
    )
    .expect("insert memory");
    let memory_id = db.last_insert_rowid();
    crate::memory::indexing::index_memory_row(
        &db,
        7,
        11,
        "user:test",
        memory_id,
        "user",
        "记住我的测试上下文",
        "generic",
        0.8,
        false,
        1000,
    )
    .expect("index memory");
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
    crate::memory::indexing::index_preference_entries(
        &db,
        7,
        11,
        "user:test",
        &[(
            "response_language".to_string(),
            "zh-CN".to_string(),
            0.96,
            "memory_extract".to_string(),
        )],
        1000,
    )
    .expect("index preference");
    let source_ids = [memory_id];
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

    let cleared = clear_memory_scope(&db, 7, 11, "user:test", MemoryClearScope::All, 1300)
        .expect("clear all");

    let recent_count: i64 = db
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("recent count");
    let pref_count: i64 = db
        .query_row("SELECT COUNT(*) FROM user_preferences", [], |row| {
            row.get(0)
        })
        .expect("preference count");
    let fact_status: String = db
        .query_row("SELECT status FROM memory_facts WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("fact status");
    let index_count: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_retrieval_index", [], |row| {
            row.get(0)
        })
        .expect("index count");

    assert_eq!(cleared.scope, "all");
    assert_eq!(cleared.recent_deleted, 1);
    assert_eq!(cleared.preferences_deleted, 1);
    assert_eq!(cleared.facts_deleted, 1);
    assert_eq!(recent_count, 0);
    assert_eq!(pref_count, 0);
    assert_eq!(fact_status, crate::memory::MEMORY_FACT_STATUS_DELETED);
    assert_eq!(index_count, 0);
}

#[test]
fn memory_api_settings_updates_bool_without_duplicate_keys() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-memory-api-test-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    fs::create_dir_all(root.join("configs")).expect("create configs dir");
    fs::write(
        root.join("configs/memory.toml"),
        "# memory settings\nlong_term_enabled = true\nembedding_dims = 24\n",
    )
    .expect("write memory config");

    let changed = update_memory_settings_file(
        &root,
        &MemorySettingsRequest {
            long_term_enabled: Some(false),
        },
    )
    .expect("update setting");
    let unchanged = update_memory_settings_file(
        &root,
        &MemorySettingsRequest {
            long_term_enabled: Some(false),
        },
    )
    .expect("update same setting");
    let raw = fs::read_to_string(root.join("configs/memory.toml")).expect("read config");
    let long_term_lines = raw
        .lines()
        .filter(|line| line.trim_start().starts_with("long_term_enabled"))
        .count();

    assert_eq!(changed.long_term_enabled, false);
    assert_eq!(changed.restart_required, true);
    assert_eq!(unchanged.long_term_enabled, false);
    assert_eq!(unchanged.restart_required, false);
    assert_eq!(long_term_lines, 1);
    assert!(raw.contains("long_term_enabled = false"));

    let _ = fs::remove_dir_all(root);
}
