use super::*;
use crate::memory::embedding_jobs as runtime;

#[path = "embedding_snapshot_test_fixtures.rs"]
mod support;

#[tokio::test]
async fn unknown_response_expiry_allows_explicit_rebuild_without_provider_replay() {
    let (fixture, _credential, server) = support::unknown_response().await;
    assert!(
        !runtime::run_one_embedding_batch(fixture.state(), "snapshot-recovery")
            .await
            .unwrap()
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status='failed'
        AND retryable=0 AND error_code='memory_embedding_response_outcome_unknown' AND attempt=1"
        ),
        1
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_vector_snapshots WHERE state='building'"
        ),
        0
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_vector_snapshots WHERE state='corrupt'"
        ),
        1
    );
    let settings = crate::memory::settings::resolve_principal_memory_settings(
        &db,
        &fixture.principal,
        fixture.state().policy.memory.long_term_enabled,
    )
    .unwrap();
    let (_, generation, count) = runtime::enqueue_reindex(
        &db,
        &fixture.state().policy.memory,
        &fixture.principal,
        &settings.policy_digest,
        true,
    )
    .expect("a failed unknown outcome must not permanently lock explicit rebuild");
    assert_eq!((generation, count), (3, 1));
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_vector_rows WHERE status='active'"
        ),
        1
    );
    assert_eq!(
        scalar(&db, "SELECT COUNT(*) FROM memory_embedding_circuits"),
        0
    );
}

#[tokio::test]
async fn revoked_queued_build_is_retired_by_paused_maintenance_without_outbound_work() {
    let server = LoopbackEmbedding::start().await;
    let (fixture, _credential) = remote_fixture(&server.endpoint).await;
    {
        let db = fixture.state().core.db.get().unwrap();
        runtime::revoke_remote_profiles_for_principal(&db, &fixture.principal).unwrap();
        pressure(&db, "explicit_write_blocked");
    }
    assert!(
        !runtime::run_one_embedding_batch(fixture.state(), "cancel-maintenance")
            .await
            .unwrap()
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status='cancelled'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_vector_snapshots WHERE state='corrupt'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_vector_snapshots WHERE state='building'"
        ),
        0
    );
    assert_eq!(server.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_vector_rows WHERE status='active'"
        ),
        1
    );
    assert_eq!(
        scalar(&db, "SELECT COUNT(*) FROM memory_embedding_circuits"),
        0
    );
}

#[tokio::test]
async fn terminal_build_cleanup_preserves_active_and_other_owner_profile_generation_snapshots() {
    let fixture = Fixture::memory();
    let profile = crate::memory::vector_store::LOCAL_PROFILE_ID;
    let other = crate::repo::auth::create_auth_key(fixture.state(), "user").unwrap();
    let other = crate::resolve_auth_identity_by_key(fixture.state(), &other)
        .unwrap()
        .unwrap()
        .principal_id;
    {
        let db = fixture.state().core.db.get().unwrap();
        support::snapshot(&db, &fixture.principal, profile, 2, "building");
        support::terminal_job(&db, &fixture.principal, profile, 2, "failed");
        support::snapshot(&db, &fixture.principal, profile, 1, "active");
        support::terminal_job(&db, &fixture.principal, profile, 1, "failed");
        support::snapshot(&db, &other, profile, 2, "building");
        support::snapshot(&db, &fixture.principal, "another-profile", 2, "building");
        support::snapshot(&db, &fixture.principal, profile, 3, "building");
        pressure(&db, "explicit_write_blocked");
    }
    assert!(
        !runtime::run_one_embedding_batch(fixture.state(), "orphan-build-maintenance")
            .await
            .unwrap()
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(support::state(&db, &other, profile, 2), "building");
    assert_eq!(
        support::state(&db, &fixture.principal, "another-profile", 2),
        "building"
    );
    assert_eq!(
        support::state(&db, &fixture.principal, profile, 3),
        "building"
    );
    assert_eq!(
        support::state(&db, &fixture.principal, profile, 1),
        "active"
    );
    assert_eq!(
        support::state(&db, &fixture.principal, profile, 2),
        "corrupt"
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status='failed'"
        ),
        2
    );
}

#[tokio::test]
async fn healthy_paused_build_poll_stays_read_only_under_an_independent_writer() {
    let fixture = Fixture::file();
    fixture.seed_embedding();
    {
        let db = fixture.state().core.db.get().unwrap();
        runtime::enqueue_reindex(
            &db,
            &fixture.state().policy.memory,
            &fixture.principal,
            "fixture-consent",
            true,
        )
        .unwrap();
        pressure(&db, "explicit_write_blocked");
    }
    let writer = fixture.competing_writer();
    assert!(
        !runtime::run_one_embedding_batch(fixture.state(), "read-only-poll")
            .await
            .unwrap()
    );
    writer.execute_batch("ROLLBACK").unwrap();
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_vector_snapshots WHERE state='building'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_embedding_jobs WHERE status='queued' AND attempt=0"
        ),
        2
    );
    assert_eq!(scalar(&db, "SELECT COUNT(*) FROM memory_vector_rows"), 0);
}
