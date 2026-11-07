use crate::memory::capacity_test_fixtures::{persist_preference_sources, Fixture};

const WORKER: &str = "lease-renewal-worker";

fn claimed_batch() -> (Fixture, Vec<super::EmbeddingJob>) {
    let mut fixture = Fixture::memory();
    fixture.state_mut().policy.memory.background_lease_seconds = 300;
    fixture.seed_embedding();
    {
        let db = fixture.state().core.db.get().unwrap();
        let entries = [(
            "detail".to_string(),
            "lease renewal fixture".to_string(),
            1.0_f32,
            "user".to_string(),
        )];
        let now = crate::now_ts_u64() as i64;
        persist_preference_sources(&db, 1, 0, &fixture.key, &entries, now);
        crate::memory::indexing::index_preference_entries(
            &db,
            1,
            0,
            &fixture.key,
            &entries,
            now,
        )
        .unwrap();
    }
    let jobs = super::claim_embedding_batch(fixture.state(), WORKER).unwrap();
    assert_eq!(jobs.len(), 2);
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute(
            "UPDATE memory_embedding_jobs SET lease_expires_at_ts=?1,
             checkpoint_json=json_set(checkpoint_json,'$.phase','dispatch_started')
             WHERE lease_owner=?2",
            rusqlite::params![crate::now_ts_u64() as i64 + 60, WORKER],
        )
        .unwrap();
    }
    (fixture, jobs)
}

fn lease_snapshot(fixture: &Fixture, job_id: Option<&str>) -> String {
    fixture
        .state()
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT json_group_array(json_array(job_id,status,attempt,lease_owner,
                 lease_expires_at_ts,cancel_requested,checkpoint_json,updated_at_ts))
             FROM (SELECT * FROM memory_embedding_jobs
                   WHERE ?1 IS NULL OR job_id=?1 ORDER BY job_id)",
            [job_id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn all_cancelled_items_stop_waiting_without_rewriting_dispatch_or_lease() {
    let (fixture, jobs) = claimed_batch();
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute("UPDATE memory_embedding_jobs SET cancel_requested=1", [])
            .unwrap();
    }
    let before = lease_snapshot(&fixture, None);
    assert!(!super::renew_embedding_leases(fixture.state(), WORKER, &jobs).unwrap());
    assert_eq!(lease_snapshot(&fixture, None), before);
}

#[test]
fn a_valid_peer_keeps_the_batch_alive_without_renewing_cancelled_items() {
    let (fixture, jobs) = claimed_batch();
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute(
            "UPDATE memory_embedding_jobs SET cancel_requested=1 WHERE job_id=?1",
            [&jobs[0].job_id],
        )
        .unwrap();
    }
    let cancelled = lease_snapshot(&fixture, Some(&jobs[0].job_id));
    let now = crate::now_ts_u64() as i64;
    assert!(super::renew_embedding_leases(fixture.state(), WORKER, &jobs).unwrap());
    assert_eq!(lease_snapshot(&fixture, Some(&jobs[0].job_id)), cancelled);
    let db = fixture.state().core.db.get().unwrap();
    let expires: i64 = db
        .query_row(
            "SELECT lease_expires_at_ts FROM memory_embedding_jobs WHERE job_id=?1",
            [&jobs[1].job_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(expires >= now + 300);
}

#[test]
fn expired_replaced_or_completed_leases_do_not_keep_a_request_alive() {
    for change in [
        "UPDATE memory_embedding_jobs SET lease_expires_at_ts=0",
        "UPDATE memory_embedding_jobs SET lease_owner='replacement-worker'",
        "UPDATE memory_embedding_jobs SET status='completed'",
    ] {
        let (fixture, jobs) = claimed_batch();
        fixture
            .state()
            .core
            .db
            .get()
            .unwrap()
            .execute(change, [])
            .unwrap();
        let before = lease_snapshot(&fixture, None);
        assert!(!super::renew_embedding_leases(fixture.state(), WORKER, &jobs).unwrap());
        assert_eq!(lease_snapshot(&fixture, None), before, "{change}");
    }
}

#[test]
fn every_healthy_item_is_renewed_without_consuming_another_attempt() {
    let (fixture, jobs) = claimed_batch();
    let now = crate::now_ts_u64() as i64;
    assert!(super::renew_embedding_leases(fixture.state(), WORKER, &jobs).unwrap());
    let db = fixture.state().core.db.get().unwrap();
    let renewed: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_embedding_jobs
             WHERE status='running' AND lease_owner=?1 AND cancel_requested=0
               AND lease_expires_at_ts>=?2 AND attempt=1
               AND json_extract(checkpoint_json,'$.phase')='dispatch_started'",
            rusqlite::params![WORKER, now + 300],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(renewed, 2);
}
