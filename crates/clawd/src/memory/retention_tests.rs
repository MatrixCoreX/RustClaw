use rusqlite::{params, Connection};

use super::*;

fn setup_db() -> Connection {
    let db = Connection::open_in_memory().expect("fixture db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    crate::db_init::ensure_schedule_schema(&db).expect("schedule schema");
    crate::db_init::ensure_memory_schema(&db).expect("memory schema");
    crate::db_init::ensure_channel_schema(&db).expect("channel schema");
    crate::repo::auth::ensure_key_auth_schema(&db).expect("auth schema");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
    db
}

#[test]
fn row_quota_is_applied_per_principal_without_cross_principal_eviction() {
    let db = setup_db();
    super::super::scope::ensure_memory_scope_schema(&db).expect("scope schema");
    super::super::jobs::ensure_memory_job_schema(&db).expect("jobs schema");
    let now = 2_000_000_000_i64;
    for (principal, user_id, count) in [
        ("principal-heavy", 10_i64, 5_i64),
        ("principal-light", 20_i64, 2_i64),
    ] {
        for sequence in 0..count {
            db.execute(
                "INSERT INTO memories(
                    memory_id, user_id, chat_id, user_key, principal_id, scope_kind,
                    scope_ref, channel, role, content, created_at, created_at_ts
                 ) VALUES (?1, ?2, 1, ?3, ?3, 'principal', ?3, 'ui', 'user',
                           ?4, ?5, ?6)",
                params![
                    format!("memory-{principal}-{sequence}"),
                    user_id,
                    principal,
                    format!("content-{sequence}"),
                    now.to_string(),
                    now + sequence,
                ],
            )
            .expect("insert source row");
        }
    }
    let mut config = MemoryConfig::default();
    config.max_rows = 2;
    config.retention_days = 3650;
    config.storage_soft_limit_bytes = u64::MAX / 2;
    let report = cleanup_memory_data(&db, &config, now + 10).expect("cleanup");
    assert_eq!(report.deleted_source_rows, 3);
    for principal in ["principal-heavy", "principal-light"] {
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE principal_id = ?1",
                [principal],
                |row| row.get(0),
            )
            .expect("count principal rows");
        assert_eq!(count, 2, "{principal}");
    }
}

#[test]
fn pressure_state_machine_is_monotonic_by_threshold() {
    assert_eq!(pressure_state(74, 100), "normal");
    assert_eq!(pressure_state(75, 100), "derived_cleanup");
    assert_eq!(pressure_state(85, 100), "backfill_paused");
    assert_eq!(pressure_state(95, 100), "automatic_generation_paused");
    assert_eq!(pressure_state(100, 100), "explicit_write_blocked");
}
