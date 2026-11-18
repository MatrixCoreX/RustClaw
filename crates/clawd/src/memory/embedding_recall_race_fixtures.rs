use super::*;
use crate::memory::ux::{
    correct_memory, delete_memory_with_revision, record_feedback, MemoryCorrectionRequest,
    MemoryFeedbackKind, MemoryFeedbackRequest, MemoryMutationRequest,
};

#[derive(Clone, Copy)]
pub(super) enum Change {
    Suppress,
    Delete,
    Correct,
    RevokeConsent,
    ReplaceLease,
    ChangeProjection,
}

fn memory_id(fixture: &Fixture, key: &str) -> String {
    fixture
        .state()
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT memory_id FROM user_preferences WHERE user_key=?1",
            [key],
            |row| row.get(0),
        )
        .unwrap()
}

fn suppress(fixture: &Fixture, owner: &str, key: &str) {
    let id = memory_id(fixture, key);
    let db = fixture.state().core.db.get().unwrap();
    let result = record_feedback(
        &db,
        owner,
        &id,
        &MemoryFeedbackRequest {
            expected_revision: 1,
            feedback_kind: MemoryFeedbackKind::DoNotUse,
            retrieval_event_ref: None,
            corrected_content: None,
        },
        crate::now_ts_u64() as i64,
    )
    .unwrap();
    assert_eq!(result.status, "do_not_use");
}

pub(super) async fn rejected(change: Change) {
    let mut server = LoopbackEmbedding::start().await;
    let (fixture, _credential) = remote_fixture(&server.endpoint).await;
    let state = fixture.state().clone();
    let worker = tokio::spawn(async move {
        crate::memory::embedding_jobs::run_one_embedding_batch(&state, "recall-race-worker").await
    });
    server.wait_for_request().await;
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    let id = memory_id(&fixture, &fixture.key);
    let (profile, retrieval): (String, i64) = {
        let db = fixture.state().core.db.get().unwrap();
        db.query_row(
            "SELECT profile_id,retrieval_id FROM memory_embedding_jobs
             WHERE principal_id=?1 AND status='running' AND lease_owner='recall-race-worker'",
            [&fixture.principal],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    };
    {
        let db = fixture.state().core.db.get().unwrap();
        match change {
            Change::Suppress => {
                drop(db);
                suppress(&fixture, &fixture.principal, &fixture.key);
            }
            Change::Delete => {
                let result = delete_memory_with_revision(
                    &db,
                    &fixture.principal,
                    &id,
                    &MemoryMutationRequest {
                        expected_revision: 1,
                    },
                    crate::now_ts_u64() as i64,
                )
                .unwrap();
                assert_eq!(result.status, "deleted");
            }
            Change::Correct => {
                let result = correct_memory(
                    &db,
                    &fixture.principal,
                    &fixture.principal,
                    &id,
                    &MemoryCorrectionRequest {
                        expected_revision: 1,
                        content: "Updated while embedding".into(),
                    },
                    crate::now_ts_u64() as i64,
                )
                .unwrap();
                assert_eq!(result.status, "corrected");
            }
            Change::RevokeConsent => {
                assert_eq!(
                    crate::memory::embedding_jobs::revoke_remote_profiles_for_principal(
                        &db,
                        &fixture.principal,
                    )
                    .unwrap(),
                    (1, 0)
                );
            }
            Change::ReplaceLease => {
                assert_eq!(db.execute(
                    "UPDATE memory_embedding_jobs SET lease_owner='replacement-worker'
                     WHERE principal_id=?1 AND status='running' AND lease_owner='recall-race-worker'",
                    [&fixture.principal],
                ).unwrap(), 1);
            }
            Change::ChangeProjection => {
                assert_eq!(db.execute(
                    "UPDATE memory_retrieval_index SET search_text='Concurrent replacement projection'
                     WHERE id=?1 AND principal_id=?2",
                    rusqlite::params![retrieval, fixture.principal],
                ).unwrap(), 1);
            }
        }
    }
    server.release();
    // Discarding a stale response or reporting lease loss are both allowed;
    // neither outcome may publish a vector based on revoked/stale authority.
    let outcome = worker.await.expect("worker must not panic");
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    let db = fixture.state().core.db.get().unwrap();
    let active: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_vector_rows
             WHERE principal_id=?1 AND profile_id=?2 AND status='active'",
            rusqlite::params![fixture.principal, profile],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        active, 0,
        "late embedding response must not publish stale or revoked state; outcome={outcome:?}"
    );
    if matches!(change, Change::ReplaceLease) {
        assert_eq!(
            scalar(
                &db,
                "SELECT COUNT(*) FROM memory_embedding_jobs
                WHERE status='running' AND lease_owner='replacement-worker'"
            ),
            1,
            "the stale worker must not finish or consume its replacement's lease"
        );
    }
}

pub(super) async fn control(other_owner: bool) {
    let mut server = LoopbackEmbedding::start().await;
    let (fixture, _credential) = remote_fixture(&server.endpoint).await;
    let state = fixture.state().clone();
    let worker = tokio::spawn(async move {
        crate::memory::embedding_jobs::run_one_embedding_batch(&state, "recall-control-worker")
            .await
    });
    server.wait_for_request().await;
    if other_owner {
        let key = crate::repo::auth::create_auth_key(fixture.state(), "user").unwrap();
        let owner = crate::resolve_auth_identity_by_key(fixture.state(), &key)
            .unwrap()
            .unwrap()
            .principal_id;
        {
            let db = fixture.state().core.db.get().unwrap();
            let entries = [(
                "other-format".into(),
                "other content".into(),
                1.0,
                "user".into(),
            )];
            crate::memory::capacity_test_fixtures::persist_preference_sources(
                &db,
                1,
                0,
                &key,
                &entries,
                crate::now_ts_u64() as i64,
            );
            crate::memory::indexing::index_preference_entries(
                &db,
                1,
                0,
                &key,
                &entries,
                crate::now_ts_u64() as i64,
            )
            .unwrap();
        }
        suppress(&fixture, &owner, &key);
    }
    server.release();
    assert!(worker.await.unwrap().unwrap());
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    let db = fixture.state().core.db.get().unwrap();
    let actual: (i64, i64) = db.query_row(
        "SELECT
            (SELECT COUNT(*) FROM memory_vector_rows WHERE principal_id=?1 AND status='active'),
            (SELECT COUNT(*) FROM memory_embedding_jobs WHERE principal_id=?1 AND status='completed')",
        [&fixture.principal],
        |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    assert_eq!(
        actual,
        (2, 2),
        "valid source and lease must commit both local and remote embeddings exactly once"
    );
}
