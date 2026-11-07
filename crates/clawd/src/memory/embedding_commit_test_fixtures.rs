use super::*;

pub(super) struct FaultRun {
    pub(super) fixture: Fixture,
    pub(super) server: LoopbackEmbedding,
    _credential: Credential,
}

pub(super) async fn publication_fault() -> FaultRun {
    let mut server = LoopbackEmbedding::start().await;
    let (fixture, credential) = remote_fixture(&server.endpoint).await;
    fixture
        .state()
        .core
        .db
        .get()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER publication_fault BEFORE UPDATE OF status ON memory_embedding_jobs
         WHEN NEW.status='completed' AND NEW.profile_id IN (
           SELECT profile_id FROM memory_embedding_profiles WHERE provider_kind='remote_http')
         BEGIN SELECT RAISE(ABORT,'publication_ack_failed'); END;",
        )
        .unwrap();
    let state = fixture.state().clone();
    let worker =
        tokio::spawn(
            async move { runtime::run_one_embedding_batch(&state, "commit-worker").await },
        );
    server.wait_for_request().await;
    server.release();
    assert!(
        worker.await.unwrap().is_err(),
        "actual acknowledgement trigger must fail"
    );
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    FaultRun {
        fixture,
        server,
        _credential: credential,
    }
}

pub(super) fn remote_count(fixture: &Fixture, predicate: &str) -> i64 {
    let db = fixture.state().core.db.get().unwrap();
    scalar(&db,&format!("SELECT COUNT(*) FROM memory_embedding_jobs
        WHERE profile_id IN (SELECT profile_id FROM memory_embedding_profiles WHERE provider_kind='remote_http') AND ({predicate})"))
}

pub(super) fn active_remote(fixture: &Fixture) -> i64 {
    scalar(&fixture.state().core.db.get().unwrap(),
        "SELECT COUNT(*) FROM memory_vector_rows WHERE status='active'
         AND profile_id IN (SELECT profile_id FROM memory_embedding_profiles WHERE provider_kind='remote_http')")
}

pub(super) enum Change {
    ExpireLease,
    ChangeConsent,
    ChangeCanonical,
}

pub(super) async fn changed_inflight(change: Change) {
    let mut server = LoopbackEmbedding::start().await;
    let (fixture, _credential) = remote_fixture(&server.endpoint).await;
    let state = fixture.state().clone();
    let worker =
        tokio::spawn(
            async move { runtime::run_one_embedding_batch(&state, "commit-worker").await },
        );
    server.wait_for_request().await;
    {
        let db = fixture.state().core.db.get().unwrap();
        match change {
            Change::ExpireLease => {
                assert_eq!(
                    db.execute(
                        "UPDATE memory_embedding_jobs SET lease_expires_at_ts=?1
                    WHERE status='running' AND lease_owner='commit-worker'",
                        [crate::now_ts_u64() as i64 - 1]
                    )
                    .unwrap(),
                    1
                );
            }
            Change::ChangeCanonical => {
                assert_eq!(
                    db.execute(
                        "UPDATE user_preferences SET pref_value='new canonical content'
                    WHERE principal_id=?1",
                        [&fixture.principal]
                    )
                    .unwrap(),
                    1
                );
            }
            Change::ChangeConsent => {
                // Bypass no cancellation helper: the writer must recheck the
                // actual effective settings even if a cancel signal is absent.
                assert_eq!(db.execute("UPDATE memory_runtime_settings SET external_context_policy='exclude',revision=revision+1
                    WHERE principal_id=?1",[&fixture.principal]).unwrap(),1);
            }
        }
    }
    server.release();
    assert!(worker.await.unwrap().unwrap());
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    assert_eq!(active_remote(&fixture), 0);
    assert_eq!(
        remote_count(
            &fixture,
            "json_extract(checkpoint_json,'$.phase')='vector_committed'"
        ),
        0
    );
    assert_eq!(
        scalar(
            &fixture.state().core.db.get().unwrap(),
            "SELECT COUNT(*) FROM memory_embedding_circuits"
        ),
        0
    );
}
