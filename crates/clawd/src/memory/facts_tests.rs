use super::{
    ensure_memory_fact_schema, expire_due_memory_facts, upsert_memory_fact_card, MemoryFactUpsert,
};
use rusqlite::Connection;

fn setup_db() -> Connection {
    let db = Connection::open_in_memory().expect("open memory db");
    ensure_memory_fact_schema(&db).expect("ensure fact schema");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("ensure retrieval schema");
    db
}

#[test]
fn memory_fact_card_supersedes_active_conflict_group() {
    let db = setup_db();
    let source_ids = [100_i64];
    let first = MemoryFactUpsert::from_long_term_summary(
        "user_profile",
        "response_language",
        "zh-CN",
        "User prefers Chinese replies.",
        0.96,
        "long_term_summary:100",
        &source_ids,
        "explicit preference",
        Some("user_profile:response_language"),
    );
    let first_id = upsert_memory_fact_card(&db, 7, 11, "user:test", &first, 1000)
        .expect("upsert first")
        .expect("first id");

    let second_source_ids = [101_i64];
    let second = MemoryFactUpsert::from_long_term_summary(
        "user_profile",
        "response_language",
        "en",
        "User prefers English replies.",
        0.97,
        "long_term_summary:101",
        &second_source_ids,
        "newer explicit preference",
        Some("user_profile:response_language"),
    );
    let second_id = upsert_memory_fact_card(&db, 7, 11, "user:test", &second, 1010)
        .expect("upsert second")
        .expect("second id");

    let first_status: String = db
        .query_row(
            "SELECT status FROM memory_facts WHERE id = ?1",
            [first_id],
            |row| row.get(0),
        )
        .expect("first status");
    let second_status: String = db
        .query_row(
            "SELECT status FROM memory_facts WHERE id = ?1",
            [second_id],
            |row| row.get(0),
        )
        .expect("second status");
    assert_eq!(first_status, crate::memory::MEMORY_FACT_STATUS_SUPERSEDED);
    assert_eq!(second_status, crate::memory::MEMORY_FACT_STATUS_ACTIVE);

    let old_index_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE search_text LIKE '%Chinese replies%'",
            [],
            |row| row.get(0),
        )
        .expect("old index count");
    let new_index_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE search_text LIKE '%English replies%'",
            [],
            |row| row.get(0),
        )
        .expect("new index count");
    assert_eq!(old_index_rows, 0);
    assert_eq!(new_index_rows, 1);
}

#[test]
fn memory_fact_card_expiry_removes_retrieval_row() {
    let db = setup_db();
    let source_ids = [200_i64];
    let mut fact = MemoryFactUpsert::from_long_term_summary(
        "user_profile",
        "temporary_channel",
        "telegram",
        "User temporarily prefers Telegram for notifications.",
        0.9,
        "long_term_summary:200",
        &source_ids,
        "explicit temporary preference",
        Some("user_profile:temporary_channel"),
    );
    fact.expires_at_ts = Some(1200);
    let fact_id = upsert_memory_fact_card(&db, 7, 11, "user:test", &fact, 1000)
        .expect("upsert expiring fact")
        .expect("fact id");

    let expired = expire_due_memory_facts(&db, 1300).expect("expire facts");
    assert_eq!(expired, 1);

    let status: String = db
        .query_row(
            "SELECT status FROM memory_facts WHERE id = ?1",
            [fact_id],
            |row| row.get(0),
        )
        .expect("fact status");
    let index_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_kind = ?1",
            [crate::memory::RETRIEVAL_SOURCE_MEMORY_FACT],
            |row| row.get(0),
        )
        .expect("index count");
    assert_eq!(status, crate::memory::MEMORY_FACT_STATUS_EXPIRED);
    assert_eq!(index_rows, 0);
}

#[test]
fn memory_fact_card_rejects_deictic_identifier_mapping() {
    let db = setup_db();
    let source_ids = [300_i64];
    let fact = MemoryFactUpsert::from_long_term_summary(
        "project_facts",
        "service_alias",
        "clawd",
        "项目别名：'那个服务' 代指 'clawd'",
        0.95,
        "long_term_summary:300",
        &source_ids,
        "deictic alias mapping should stay session-scoped",
        Some("project_facts:service_alias"),
    );
    let inserted = upsert_memory_fact_card(&db, 7, 11, "user:test", &fact, 1000)
        .expect("upsert should not fail");
    assert_eq!(inserted, None);

    let fact_rows: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_facts", [], |row| row.get(0))
        .expect("fact count");
    let index_rows: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_retrieval_index", [], |row| {
            row.get(0)
        })
        .expect("index count");
    assert_eq!(fact_rows, 0);
    assert_eq!(index_rows, 0);
}

#[test]
fn memory_fact_card_rejects_plain_locator_alias_mapping() {
    let db = setup_db();
    let source_ids = [301_i64];
    let fact = MemoryFactUpsert::from_long_term_summary(
        "project_facts",
        "config_alias",
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/configs/app_config.toml",
        "那个配置文件 maps to /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/configs/app_config.toml",
        0.95,
        "long_term_summary:301",
        &source_ids,
        "plain locator alias mapping should stay session-scoped",
        Some("project_facts:config_alias"),
    );
    let inserted = upsert_memory_fact_card(&db, 7, 11, "user:test", &fact, 1000)
        .expect("upsert should not fail");
    assert_eq!(inserted, None);

    let fact_rows: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_facts", [], |row| row.get(0))
        .expect("fact count");
    let index_rows: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_retrieval_index", [], |row| {
            row.get(0)
        })
        .expect("index count");
    assert_eq!(fact_rows, 0);
    assert_eq!(index_rows, 0);
}
