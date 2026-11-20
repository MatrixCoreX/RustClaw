use crate::memory::completion_test_fixtures::GenerationFixture;
use crate::memory::ux::{check_memory_consistency, ensure_memory_ux_schema};

fn initialize_checker(fixture: &GenerationFixture) {
    let db = fixture.fixture.state().core.db.get().unwrap();
    ensure_memory_ux_schema(&db).unwrap();
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memories WHERE id > 76000"),
        2
    );
    assert_eq!(
        fixture.scalar("SELECT MAX(source_sequence) FROM memory_source_events"),
        2
    );
}

#[tokio::test]
async fn concurrent_sweep_preserves_running_and_queued_jobs_with_large_source_ids() {
    let fixture = GenerationFixture::with_held_summary_review().await;
    initialize_checker(&fixture);
    let inspect = async {
        fixture.wait_for_held_request().await;
        {
            let db = fixture.fixture.state().core.db.get().unwrap();
            for repair in [false, true] {
                let report = check_memory_consistency(
                    &db,
                    &fixture.fixture.principal,
                    repair,
                    crate::now_ts_u64() as i64,
                )
                .unwrap();
                assert_eq!(report.stale_jobs, 0);
                assert_eq!(report.repaired_rows, 0);
            }
        }
        assert_eq!(
            fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE cancel_requested=1"),
            0
        );
        assert_eq!(
            fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE status='running'"),
            1
        );
        assert_eq!(
            fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE status='queued'"),
            1
        );
        fixture.release_provider_response();
    };
    let (result, ()) = tokio::join!(fixture.run(), inspect);
    assert!(result.unwrap());
    assert_eq!(fixture.status(), "completed");
    assert!(fixture.run_reopened().await.unwrap());
    fixture.assert_one_provider_call();
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE status='completed'"),
        2
    );
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memories"), 2);
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_source_events"),
        2
    );
}

#[tokio::test]
async fn truly_missing_or_mismatched_sources_still_cancel_only_affected_jobs() {
    for mutation in [
        "DELETE FROM memory_source_events",
        "UPDATE memory_source_events SET principal_id='another-owner'",
        "UPDATE memory_source_events SET source_task_id='another-task'",
        "UPDATE memory_jobs SET source_event_start=80000,source_event_end=80001",
    ] {
        let fixture = GenerationFixture::with_held_summary_review().await;
        initialize_checker(&fixture);
        fixture.sql(mutation);
        let db = fixture.fixture.state().core.db.get().unwrap();
        let report = check_memory_consistency(
            &db,
            &fixture.fixture.principal,
            true,
            crate::now_ts_u64() as i64,
        )
        .unwrap();
        assert_eq!(report.stale_jobs, 2, "{mutation}");
        assert_eq!(report.repaired_rows, 2, "{mutation}");
        drop(db);
        assert_eq!(
            fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE cancel_requested=1"),
            2
        );
        assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memories"), 2);
        fixture.assert_no_provider_call();
    }
}
