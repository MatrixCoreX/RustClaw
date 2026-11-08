use claw_core::config::MemoryConfig;
use rusqlite::params;

const NOW: i64 = 2_000_000_000;

fn cleanup(days: u64) -> [Vec<i64>; 3] {
    let db = super::setup_db();
    crate::memory::scope::ensure_memory_scope_schema(&db).unwrap();
    for (index, at) in [0, NOW - 86_401, NOW - 86_400, NOW].into_iter().enumerate() {
        let id = format!("retention-duration-{index}");
        db.execute(
            "INSERT INTO memories(memory_id,user_id,chat_id,user_key,principal_id,
             scope_kind,scope_ref,channel,role,content,created_at,created_at_ts)
             VALUES (?1,1,?2,'duration-owner','duration-owner','principal',
             'duration-owner','ui','user','retained source',?3,?4)",
            params![id, index as i64, at.to_string(), at],
        )
        .unwrap();
        db.execute(
            "INSERT INTO long_term_memories(user_id,chat_id,user_key,principal_id,
             summary,created_at,updated_at,created_at_ts,updated_at_ts)
             VALUES (1,?1,'duration-owner','duration-owner','retained summary',?2,?2,?3,?3)",
            params![index as i64, at.to_string(), at],
        )
        .unwrap();
        db.execute(
            "INSERT INTO memory_raw_candidates(candidate_id,generation_job_id,principal_id,
             scope_kind,scope_ref,candidate_kind,content_json,content_digest,trust_tier,
             sensitivity,status,created_at_ts) VALUES (?1,'duration-job','duration-owner',
             'principal','duration-owner','fact','{}',?1,'user','normal','pending',?2)",
            params![id, at],
        )
        .unwrap();
    }
    let mut config = MemoryConfig::default();
    config.retention_days = days;
    config.long_term_retention_days = days;
    config.raw_candidate_retention_days = days;
    config.max_rows = 100;
    config.long_term_max_rows = 100;
    config.raw_candidate_max_rows_per_principal = 100;
    config.storage_soft_limit_bytes = u64::MAX / 2;
    super::super::cleanup_memory_data(&db, &config, NOW).unwrap();
    [
        "SELECT created_at_ts FROM memories ORDER BY created_at_ts",
        "SELECT updated_at_ts FROM long_term_memories ORDER BY updated_at_ts",
        "SELECT created_at_ts FROM memory_raw_candidates ORDER BY created_at_ts",
    ]
    .map(|sql| {
        db.prepare(sql)
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<i64>>>()
            .unwrap()
    })
}

#[test]
fn all_memory_retention_kinds_preserve_fresh_rows_for_large_durations() {
    for days in [i64::MAX as u64, u64::MAX] {
        for rows in cleanup(days) {
            assert_eq!(rows, [0, NOW - 86_401, NOW - 86_400, NOW], "days={days}");
        }
    }
}

#[test]
fn ordinary_and_zero_durations_keep_the_strict_cutoff_boundary() {
    for (days, expected) in [
        (0, vec![NOW]),
        (1, vec![NOW - 86_400, NOW]),
        (3650, vec![NOW - 86_401, NOW - 86_400, NOW]),
    ] {
        for rows in cleanup(days) {
            assert_eq!(rows, expected, "days={days}");
        }
    }
}
