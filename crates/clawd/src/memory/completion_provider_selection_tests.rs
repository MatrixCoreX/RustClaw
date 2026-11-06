use crate::memory::completion_test_fixtures::GenerationFixture;

async fn assert_cold_vendor_job(kind: &str) {
    let fixture = GenerationFixture::for_vendor_kind(kind).await;
    let (ran, calls) = fixture.run_cold_first_request().await.unwrap();
    assert!(ran);
    assert_eq!(
        fixture.status(),
        "completed",
        "normal vendor {kind} selection must survive a cold worker"
    );
    assert_eq!(
        calls, 1,
        "the fresh worker must record its one real provider call"
    );
    assert_eq!(fixture.provider_request_count(), 1);
    assert_eq!(
        fixture.scalar("SELECT used_background_cost_microunits FROM memory_principal_quotas"),
        1000
    );
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        0
    );
    if kind == "extract" {
        assert_eq!(fixture.scalar("SELECT COUNT(*) FROM user_preferences"), 1);
    } else {
        assert_eq!(fixture.scalar("SELECT COUNT(*) FROM long_term_memories"), 1);
        assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memory_facts"), 1);
    }
    assert!(
        !fixture.run_reopened().await.unwrap(),
        "completed work must not replay on a second worker"
    );
    assert_eq!(fixture.provider_request_count(), 1);
}

#[tokio::test]
async fn normal_vendor_extract_job_resolves_after_cold_worker_restart() {
    assert_cold_vendor_job("extract").await;
}

#[tokio::test]
async fn normal_vendor_consolidation_job_resolves_after_cold_worker_restart() {
    assert_cold_vendor_job("consolidate").await;
}
