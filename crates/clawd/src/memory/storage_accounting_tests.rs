use super::observed_memory_bytes;
use crate::memory::retention::{automatic_generation_allowed, refresh_storage_pressure};
use claw_core::config::MemoryConfig;
use rusqlite::Connection;

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(crate::INIT_SQL).unwrap();
    crate::db_init::ensure_schedule_schema(&db).unwrap();
    crate::db_init::ensure_memory_schema(&db).unwrap();
    crate::db_init::ensure_channel_schema(&db).unwrap();
    crate::repo::auth::ensure_key_auth_schema(&db).unwrap();
    crate::memory::scope::ensure_memory_scope_schema(&db).unwrap();
    crate::memory::jobs::ensure_memory_job_schema(&db).unwrap();
    crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
    db
}

#[test]
fn unrelated_journal_growth_does_not_block_memory_generation() {
    let db = fixture();
    let baseline = observed_memory_bytes(&db).unwrap();
    let config = MemoryConfig {
        storage_soft_limit_bytes: baseline.saturating_mul(4).max(4096),
        ..MemoryConfig::default()
    };
    db.execute_batch(
        "CREATE TABLE task_payload_fixture(payload BLOB);
         INSERT INTO task_payload_fixture VALUES (zeroblob(4194304));",
    )
    .unwrap();
    db.execute("UPDATE memory_storage_pressure SET state='explicit_write_blocked', observed_bytes=1073741824 WHERE singleton_id=1", []).unwrap();
    let file_bytes: u64 = db
        .query_row(
            "SELECT page_count * page_size FROM pragma_page_count, pragma_page_size",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(file_bytes > config.storage_soft_limit_bytes);
    let (bytes, state) = refresh_storage_pressure(&db, &config, 2_000_000_000).unwrap();
    assert_eq!(bytes, baseline);
    assert_eq!(state, "normal");
    assert!(automatic_generation_allowed(&db, "fixture-principal").unwrap());
}

#[test]
fn owned_payload_blocks_and_deletion_recovers_without_vacuum() {
    let db = fixture();
    let baseline = observed_memory_bytes(&db).unwrap();
    let config = MemoryConfig {
        storage_soft_limit_bytes: baseline.saturating_mul(4).max(4096),
        ..MemoryConfig::default()
    };
    db.execute_batch(
        "CREATE TABLE memory_payload_fixture(payload BLOB);
         INSERT INTO memory_payload_fixture VALUES (zeroblob(1048576));",
    )
    .unwrap();
    let (bytes, state) = refresh_storage_pressure(&db, &config, 2_000_000_000).unwrap();
    assert_eq!(bytes, baseline + 1_048_576);
    assert_eq!(state, "explicit_write_blocked");
    assert!(!automatic_generation_allowed(&db, "fixture-principal").unwrap());
    db.execute("DELETE FROM memory_payload_fixture", [])
        .unwrap();
    let free_pages: u64 = db
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .unwrap();
    assert!(free_pages > 0);
    let (bytes, state) = refresh_storage_pressure(&db, &config, 2_000_000_001).unwrap();
    assert_eq!(bytes, baseline);
    assert_eq!(state, "normal");
    assert!(automatic_generation_allowed(&db, "fixture-principal").unwrap());
}

#[test]
fn payload_accounting_handles_utf8_nul_blob_and_quoted_identifiers() {
    let db = fixture();
    let baseline = observed_memory_bytes(&db).unwrap();
    db.execute_batch(
        "CREATE TABLE \"memory_payload_\"\"quoted\"(\"content\"\"text\" TEXT, data BLOB, n INTEGER, absent TEXT);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO \"memory_payload_\"\"quoted\" VALUES (?1, ?2, 3, NULL)",
        rusqlite::params!["中\0🙂", vec![0_u8, 1, 2]],
    )
    .unwrap();
    assert_eq!(observed_memory_bytes(&db).unwrap() - baseline, 8 + 3 + 8);
}

#[test]
fn pressure_record_is_not_charged_to_its_own_limit() {
    let db = fixture();
    let config = MemoryConfig {
        storage_soft_limit_bytes: u64::MAX / 2,
        ..Default::default()
    };
    let first = refresh_storage_pressure(&db, &config, 2_000_000_000)
        .unwrap()
        .0;
    let second = refresh_storage_pressure(&db, &config, 2_000_000_001)
        .unwrap()
        .0;
    assert_eq!(first, second);
}
