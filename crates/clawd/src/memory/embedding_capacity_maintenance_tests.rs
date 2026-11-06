use rusqlite::params;

use crate::memory::capacity_test_fixtures::{pressure, scalar, Fixture};
use crate::memory::vector_store::LOCAL_PROFILE_ID;

fn status(db: &rusqlite::Connection, id: &str) -> (String, i64) {
    db.query_row(
        "SELECT status, attempt FROM memory_embedding_jobs WHERE job_id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

fn seed_many(fixture: &Fixture) -> Vec<String> {
    let db = fixture.state().core.db.get().unwrap();
    let entries = (0..3)
        .map(|number| {
            (
                format!("fixture-{number}"),
                format!("fixture text {number}"),
                1.0,
                "user".to_string(),
            )
        })
        .collect::<Vec<_>>();
    crate::memory::indexing::index_preference_entries(
        &db,
        1,
        0,
        &fixture.key,
        &entries,
        crate::now_ts_u64() as i64,
    )
    .unwrap();
    let mut query = db
        .prepare("SELECT job_id FROM memory_embedding_jobs ORDER BY job_id")
        .unwrap();
    let rows = query
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 3);
    rows
}

#[test]
fn paused_embedding_maintenance_keeps_cancel_expiry_and_active_lease_semantics() {
    let fixture = Fixture::memory();
    let ids = seed_many(&fixture);
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "automatic_generation_paused");
        db.execute(
            "UPDATE memory_embedding_jobs SET cancel_requested = 1 WHERE job_id = ?1",
            [&ids[0]],
        )
        .unwrap();
        db.execute(
            "UPDATE memory_embedding_jobs SET status = 'running', lease_owner = 'expired-worker',
            lease_expires_at_ts = 1, attempt = 1 WHERE job_id = ?1",
            [&ids[1]],
        )
        .unwrap();
        db.execute(
            "UPDATE memory_embedding_jobs SET status = 'running', lease_owner = 'active-worker',
            lease_expires_at_ts = ?2, attempt = 1 WHERE job_id = ?1",
            params![ids[2], crate::now_ts_u64() as i64 + 300],
        )
        .unwrap();
    }
    let claimed = super::claim_embedding_batch(fixture.state(), "capacity-worker").unwrap();
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(status(&db, &ids[0]), ("cancelled".to_string(), 0));
    assert_eq!(status(&db, &ids[1]), ("retry_wait".to_string(), 1));
    assert_eq!(status(&db, &ids[2]), ("running".to_string(), 1));
    assert!(claimed.is_empty());
}

#[test]
fn expired_cancelled_embedding_lease_is_finalized_without_a_surviving_worker() {
    let fixture = Fixture::memory();
    fixture.seed_embedding();
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "explicit_write_blocked");
        db.execute(
            "UPDATE memory_embedding_jobs SET status = 'running', lease_owner = 'gone-worker',
            lease_expires_at_ts = 1, cancel_requested = 1, attempt = 1",
            [],
        )
        .unwrap();
    }
    assert!(
        super::claim_embedding_batch(fixture.state(), "capacity-worker")
            .unwrap()
            .is_empty()
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(scalar(&db, "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status = 'cancelled' AND lease_owner IS NULL"), 1);
}

#[test]
fn capacity_recovery_cannot_clear_an_independent_user_pause_or_provider_circuit() {
    let fixture = Fixture::memory();
    fixture.seed_embedding();
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "automatic_generation_paused");
        db.execute(
            "INSERT INTO memory_embedding_controls(principal_id, profile_id, state, updated_at_ts)
            VALUES (?1, ?2, 'paused', 123)",
            params![fixture.principal, LOCAL_PROFILE_ID],
        )
        .unwrap();
        db.execute("INSERT INTO memory_embedding_circuits(principal_id, profile_id, failure_count,
            open_until_ts, last_error_code, updated_at_ts) VALUES (?1, ?2, 4, ?3, 'fixture_rate_limit', 123)",
            params![fixture.principal, LOCAL_PROFILE_ID, crate::now_ts_u64() as i64 + 300]).unwrap();
    }
    assert!(
        super::claim_embedding_batch(fixture.state(), "capacity-worker")
            .unwrap()
            .is_empty()
    );
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "normal");
    }
    assert!(
        super::claim_embedding_batch(fixture.state(), "capacity-worker")
            .unwrap()
            .is_empty()
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(scalar(&db, "SELECT COUNT(*) FROM memory_embedding_controls WHERE state = 'paused' AND updated_at_ts = 123"), 1);
    assert_eq!(scalar(&db, "SELECT failure_count FROM memory_embedding_circuits WHERE last_error_code = 'fixture_rate_limit'"), 4);
    assert_eq!(
        scalar(&db, "SELECT SUM(attempt) FROM memory_embedding_jobs"),
        0
    );
}
