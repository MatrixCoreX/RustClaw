use super::*;
use crate::memory::embedding_jobs as runtime;

#[path = "embedding_commit_test_fixtures.rs"]
mod support;

#[tokio::test]
async fn failed_ack_rolls_back_vector_and_recovers_response_without_another_provider_call() {
    let run = support::publication_fault().await;
    assert_eq!(support::active_remote(&run.fixture), 0);
    assert_eq!(
        support::remote_count(
            &run.fixture,
            "status='running' AND json_extract(checkpoint_json,'$.phase')='response_received'"
        ),
        1
    );
    {
        let db = run.fixture.state().core.db.get().unwrap();
        db.execute_batch("DROP TRIGGER publication_fault").unwrap();
        pressure(&db, "explicit_write_blocked");
        let profile: String = db
            .query_row(
                "SELECT profile_id FROM memory_embedding_jobs WHERE status='running'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        runtime::set_profile_paused(&db, &run.fixture.principal, &profile, true).unwrap();
    }
    assert!(
        runtime::run_one_embedding_batch(run.fixture.state(), "commit-worker")
            .await
            .unwrap()
    );
    assert_eq!(support::remote_count(&run.fixture,"status='completed' AND attempt=1 AND json_extract(checkpoint_json,'$.phase')='vector_committed'"),1);
    assert_eq!(support::active_remote(&run.fixture), 1);
    assert_eq!(run.server.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn received_response_is_discarded_if_consent_is_revoked_before_recovery() {
    let run = support::publication_fault().await;
    {
        let db = run.fixture.state().core.db.get().unwrap();
        db.execute_batch("DROP TRIGGER publication_fault").unwrap();
        runtime::revoke_remote_profiles_for_principal(&db, &run.fixture.principal).unwrap();
    }
    assert!(
        runtime::run_one_embedding_batch(run.fixture.state(), "commit-worker")
            .await
            .unwrap()
    );
    assert_eq!(support::active_remote(&run.fixture), 0);
    assert_eq!(
        support::remote_count(
            &run.fixture,
            "status='cancelled' AND json_extract(checkpoint_json,'$.phase')='response_discarded'"
        ),
        1
    );
    assert_eq!(run.server.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_live_replacement_lease_cannot_be_consumed_by_response_recovery() {
    let run = support::publication_fault().await;
    {
        let db = run.fixture.state().core.db.get().unwrap();
        db.execute_batch(
            "DROP TRIGGER publication_fault;
            UPDATE memory_embedding_jobs SET lease_owner='new-worker' WHERE status='running';",
        )
        .unwrap();
    }
    assert!(
        !runtime::run_one_embedding_batch(run.fixture.state(), "commit-worker")
            .await
            .unwrap()
    );
    assert_eq!(support::active_remote(&run.fixture), 0);
    assert_eq!(
        support::remote_count(
            &run.fixture,
            "status='running' AND lease_owner='new-worker'"
        ),
        1
    );
    assert!(
        runtime::run_one_embedding_batch(run.fixture.state(), "new-worker")
            .await
            .unwrap()
    );
    assert_eq!(support::active_remote(&run.fixture), 1);
    assert_eq!(run.server.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_unpersisted_provider_response_is_not_blindly_replayed_after_lease_expiry() {
    let mut server = LoopbackEmbedding::start().await;
    let (fixture, _credential) = remote_fixture(&server.endpoint).await;
    fixture
        .state()
        .core
        .db
        .get()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER receive_fault BEFORE UPDATE OF checkpoint_json ON memory_embedding_jobs
         WHEN json_extract(NEW.checkpoint_json,'$.phase')='response_received'
         BEGIN SELECT RAISE(ABORT,'response_persistence_failed'); END;",
        )
        .unwrap();
    let state = fixture.state().clone();
    let worker =
        tokio::spawn(
            async move { runtime::run_one_embedding_batch(&state, "commit-worker").await },
        );
    server.wait_for_request().await;
    server.release();
    assert!(worker.await.unwrap().is_err());
    assert_eq!(support::active_remote(&fixture), 0);
    assert_eq!(
        support::remote_count(
            &fixture,
            "status='running' AND json_extract(checkpoint_json,'$.phase')='dispatch_started'"
        ),
        1
    );
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute_batch("DROP TRIGGER receive_fault").unwrap();
        db.execute(
            "UPDATE memory_embedding_jobs SET lease_expires_at_ts=?1 WHERE status='running'",
            [crate::now_ts_u64() as i64 - 1],
        )
        .unwrap();
    }
    assert!(
        !runtime::run_one_embedding_batch(fixture.state(), "recovery-worker")
            .await
            .unwrap()
    );
    assert_eq!(support::remote_count(&fixture,"status='failed' AND retryable=0 AND error_code='memory_embedding_response_outcome_unknown' AND attempt=1"),1);
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn expired_inflight_lease_cannot_publish_a_vector_or_completion_receipt() {
    support::changed_inflight(support::Change::ExpireLease).await;
}

#[tokio::test]
async fn effective_consent_is_rechecked_without_relying_on_a_cancel_signal() {
    support::changed_inflight(support::Change::ChangeConsent).await;
}

#[tokio::test]
async fn canonical_content_is_rechecked_even_if_its_index_row_did_not_change() {
    support::changed_inflight(support::Change::ChangeCanonical).await;
}

#[tokio::test]
async fn one_failed_publication_does_not_roll_back_valid_batch_peers() {
    let fixture = Fixture::memory();
    fixture.seed_embedding();
    let first: i64;
    {
        let db = fixture.state().core.db.get().unwrap();
        first = db
            .query_row(
                "SELECT retrieval_id FROM memory_embedding_jobs",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let entries = [(
            "second-format".into(),
            "independent second content".into(),
            1.0,
            "user".into(),
        )];
        crate::memory::capacity_test_fixtures::persist_preference_sources(
            &db,
            1,
            0,
            &fixture.key,
            &entries,
            crate::now_ts_u64() as i64,
        );
        crate::memory::indexing::index_preference_entries(
            &db,
            1,
            0,
            &fixture.key,
            &entries,
            crate::now_ts_u64() as i64,
        )
        .unwrap();
        db.execute_batch(&format!(
            "CREATE TRIGGER peer_ack_fault BEFORE UPDATE OF status ON memory_embedding_jobs
            WHEN NEW.status='completed' AND NEW.retrieval_id={first}
            BEGIN SELECT RAISE(ABORT,'first_ack_failed'); END;"
        ))
        .unwrap();
    }
    assert!(
        runtime::run_one_embedding_batch(fixture.state(), "mixed-worker")
            .await
            .is_err()
    );
    {
        let db = fixture.state().core.db.get().unwrap();
        assert_eq!(
            scalar(
                &db,
                "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status='completed'"
            ),
            1
        );
        assert_eq!(
            scalar(
                &db,
                "SELECT COUNT(*) FROM memory_vector_rows WHERE status='active'"
            ),
            1
        );
        assert_eq!(scalar(&db,"SELECT COUNT(*) FROM memory_embedding_jobs WHERE json_extract(checkpoint_json,'$.phase')='response_received'"),1);
        db.execute_batch("DROP TRIGGER peer_ack_fault").unwrap();
    }
    assert!(
        runtime::run_one_embedding_batch(fixture.state(), "mixed-worker")
            .await
            .unwrap()
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status='completed' AND attempt=1"
        ),
        2
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_vector_rows WHERE status='active'"
        ),
        2
    );
}
