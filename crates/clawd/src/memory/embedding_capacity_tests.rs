use crate::memory::capacity_test_fixtures::{pressure, scalar, Fixture};

#[tokio::test]
async fn capacity_pause_prevents_embedding_execution_and_preserves_the_outbox() {
    for state in ["automatic_generation_paused", "explicit_write_blocked"] {
        let fixture = Fixture::memory();
        fixture.seed_embedding();
        {
            let db = fixture.state().core.db.get().unwrap();
            pressure(&db, state);
        }
        let ran = super::run_one_embedding_batch(fixture.state(), "capacity-worker")
            .await
            .unwrap();
        let db = fixture.state().core.db.get().unwrap();
        assert_eq!(
            (
                ran,
                scalar(
                    &db,
                    "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status = 'queued'"
                ),
                scalar(&db, "SELECT COUNT(*) FROM memory_vector_rows")
            ),
            (false, 1, 0),
            "{state}"
        );
    }
}

#[test]
fn paused_embedding_claim_does_not_spend_an_attempt_or_consume_a_lease() {
    let fixture = Fixture::memory();
    fixture.seed_embedding();
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "explicit_write_blocked");
    }
    assert!(
        super::claim_embedding_batch(fixture.state(), "capacity-worker")
            .unwrap()
            .is_empty()
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(&db, "SELECT SUM(attempt) FROM memory_embedding_jobs"),
        0
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_embedding_jobs WHERE lease_owner IS NOT NULL"
        ),
        0
    );
}

#[test]
fn embedding_claim_rechecks_pressure_after_its_readiness_probe() {
    let fixture = Fixture::memory();
    fixture.seed_embedding();
    {
        let db = fixture.state().core.db.get().unwrap();
        assert!(super::embedding_batch_needs_write(&db, crate::now_ts_u64() as i64).unwrap());
        pressure(&db, "automatic_generation_paused");
    }
    assert!(
        super::claim_embedding_batch(fixture.state(), "capacity-worker")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_missing_runtime_pressure_row_cannot_authorize_embedding_work() {
    let fixture = Fixture::memory();
    fixture.seed_embedding();
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute("DELETE FROM memory_storage_pressure", [])
            .unwrap();
    }
    assert!(
        super::claim_embedding_batch(fixture.state(), "capacity-worker")
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn backfill_pressure_pauses_reindex_but_keeps_the_build_queued_for_recovery() {
    let fixture = Fixture::memory();
    fixture.seed_embedding();
    assert!(
        super::run_one_embedding_batch(fixture.state(), "initial-worker")
            .await
            .unwrap()
    );
    {
        let db = fixture.state().core.db.get().unwrap();
        let (_, generation, rows) = super::enqueue_reindex(
            &db,
            &fixture.state().policy.memory,
            &fixture.principal,
            "sha256:local-policy",
            false,
        )
        .unwrap();
        assert_eq!((generation, rows), (2, 1));
        pressure(&db, "backfill_paused");
    }
    assert!(
        super::claim_embedding_batch(fixture.state(), "capacity-worker")
            .unwrap()
            .is_empty()
    );
    {
        let db = fixture.state().core.db.get().unwrap();
        assert_eq!(
            scalar(
                &db,
                "SELECT COUNT(*) FROM memory_vector_snapshots WHERE state = 'building'"
            ),
            1
        );
        assert_eq!(
            scalar(
                &db,
                "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status = 'queued'"
            ),
            1
        );
        pressure(&db, "normal");
    }
    assert!(
        super::run_one_embedding_batch(fixture.state(), "recovered-worker")
            .await
            .unwrap()
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(scalar(&db, "SELECT COUNT(*) FROM memory_vector_snapshots WHERE state = 'active' AND generation = 2"), 1);
}

#[tokio::test]
async fn backfill_pressure_does_not_disable_incremental_indexing_in_the_active_generation() {
    let fixture = Fixture::memory();
    fixture.seed_embedding();
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "backfill_paused");
    }
    assert!(
        super::run_one_embedding_batch(fixture.state(), "incremental-worker")
            .await
            .unwrap()
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_vector_rows WHERE status = 'active'"
        ),
        1
    );
}

#[test]
fn paused_embedding_poll_is_read_only_while_another_connection_holds_the_writer_lock() {
    let fixture = Fixture::file();
    fixture.seed_embedding();
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "automatic_generation_paused");
    }
    let writer = fixture.competing_writer();
    let result = super::claim_embedding_batch(fixture.state(), "capacity-worker");
    writer.execute_batch("ROLLBACK").unwrap();
    assert!(result
        .expect("capacity-paused poll must not need a writer")
        .is_empty());
}
