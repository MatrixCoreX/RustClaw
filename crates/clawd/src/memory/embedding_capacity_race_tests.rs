use crate::memory::capacity_race_test_fixtures::pause_inside_claim;
use crate::memory::capacity_test_fixtures::{pressure, scalar, Fixture};

#[test]
fn embedding_claim_rechecks_capacity_inside_its_actual_writer_transaction() {
    let fixture = Fixture::file();
    fixture.seed_embedding();
    let claimed = pause_inside_claim(&fixture, |state| {
        super::claim_embedding_batch(state, "transaction-race-worker")
    })
    .unwrap();
    assert!(
        claimed.is_empty(),
        "the actual transaction must recheck a pause committed after readiness"
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(&db, "SELECT SUM(attempt) FROM memory_embedding_jobs"),
        0
    );
}

#[tokio::test]
async fn paused_oldest_backfill_partition_does_not_starve_active_generation_work() {
    let fixture = Fixture::memory();
    fixture.seed_embedding();
    assert!(
        super::run_one_embedding_batch(fixture.state(), "initial-worker")
            .await
            .unwrap()
    );
    {
        let db = fixture.state().core.db.get().unwrap();
        super::enqueue_reindex(
            &db,
            &fixture.state().policy.memory,
            &fixture.principal,
            "sha256:explicit-reindex-policy",
            false,
        )
        .unwrap();
        crate::memory::indexing::index_preference_entries(
            &db,
            1,
            0,
            &fixture.key,
            &[(
                "extra-format".to_string(),
                "new incremental content".to_string(),
                1.0,
                "user".to_string(),
            )],
            crate::now_ts_u64() as i64,
        )
        .unwrap();
        db.execute_batch(
            "UPDATE memory_embedding_jobs SET not_before_ts = 1, created_at_ts = 1
                WHERE status = 'queued' AND profile_generation = 2;
            UPDATE memory_embedding_jobs SET not_before_ts = 2, created_at_ts = 2
                WHERE status = 'queued' AND profile_generation = 1;",
        )
        .unwrap();
        pressure(&db, "backfill_paused");
        assert_eq!(
            scalar(
                &db,
                "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status = 'queued'"
            ),
            2
        );
    }
    let claimed = super::claim_embedding_batch(fixture.state(), "fair-index-worker").unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].profile_generation, 1);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_embedding_jobs
        WHERE profile_generation = 2 AND status = 'queued' AND attempt = 0"
        ),
        1
    );
}
