use crate::memory::completion_test_fixtures::GenerationFixture;

#[tokio::test]
async fn local_resource_wait_does_not_exhaust_provider_attempts_and_recovers_when_capacity_returns()
{
    let fixture = GenerationFixture::new().await;
    let mut held = Vec::new();
    let mut refused = false;
    for index in 0..64 {
        let task = crate::ClaimedTask {
            task_id: format!("resource-holder-{index}"),
            ..fixture.task.clone()
        };
        match crate::llm_gateway::acquire_llm_resource_lease(fixture.fixture.state(), &task) {
            Ok(lease) => held.push(lease),
            Err(_) => {
                refused = true;
                break;
            }
        }
    }
    assert!(
        refused && !held.is_empty(),
        "fixture must hold the actual broker capacity"
    );
    for _ in 0..fixture
        .fixture
        .state()
        .policy
        .memory
        .background_max_attempts
        .max(1)
        + 2
    {
        fixture.sql("UPDATE memory_jobs SET not_before_ts = 1 WHERE job_kind = 'extract';");
        assert!(fixture.run().await.unwrap());
        assert_eq!(fixture.status(), "retry_wait");
        assert_eq!(
            fixture.scalar("SELECT attempt FROM memory_jobs WHERE job_kind = 'extract'"),
            0
        );
        fixture.assert_no_provider_call();
    }
    drop(held);
    fixture.sql("UPDATE memory_jobs SET not_before_ts = 1 WHERE job_kind = 'extract';");
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.status(), "completed");
    fixture.assert_one_provider_call();
}

#[tokio::test]
async fn real_workers_complete_both_generation_kinds_with_one_request_and_no_staging_leak() {
    for kind in ["extract", "consolidate"] {
        let fixture = GenerationFixture::for_kind(kind).await;
        assert!(fixture.run().await.unwrap());
        assert_eq!(fixture.status(), "completed", "{kind}");
        fixture.assert_one_provider_call();
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
    }
}

#[tokio::test]
async fn persisted_provider_response_recovers_failed_canonical_write_without_a_second_request() {
    let fixture = GenerationFixture::new().await;
    fixture.sql(
        "CREATE TRIGGER fixture_preference_failure BEFORE INSERT ON user_preferences
        BEGIN SELECT RAISE(ABORT, 'fixture_preference_failure'); END;",
    );
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.status(), "retry_wait");
    fixture.assert_one_provider_call();
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM user_preferences"), 0);
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        1
    );
    fixture.sql(
        "DROP TRIGGER fixture_preference_failure;
        UPDATE memory_jobs SET not_before_ts = 1 WHERE status = 'retry_wait';",
    );
    assert!(fixture.run_reopened().await.unwrap());
    assert_eq!(fixture.status(), "completed");
    fixture.assert_one_provider_call();
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM user_preferences WHERE pref_key = 'response_format' AND pref_value = 'markdown'"), 1);
    assert_eq!(
        fixture.scalar("SELECT row_revision FROM user_preferences"),
        1
    );
    assert_eq!(
        fixture.scalar("SELECT used_background_cost_microunits FROM memory_principal_quotas"),
        1000
    );
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        0
    );
}

#[tokio::test]
async fn actual_canonical_commit_survives_terminal_write_failure_and_capacity_pause() {
    let fixture = GenerationFixture::new().await;
    fixture.sql(
        "CREATE TRIGGER fixture_terminal_failure BEFORE UPDATE OF status ON memory_jobs
        WHEN NEW.status = 'completed' BEGIN SELECT RAISE(ABORT, 'fixture_terminal_failure'); END;",
    );
    assert!(fixture.run().await.is_err());
    assert_eq!(fixture.status(), "running");
    fixture.assert_one_provider_call();
    assert_eq!(
        fixture.scalar("SELECT row_revision FROM user_preferences"),
        1
    );
    assert_eq!(
        fixture.scalar(
            "SELECT background_cost_pending_microunits FROM memory_jobs WHERE job_kind = 'extract'"
        ),
        1000
    );
    fixture.sql(
        "DROP TRIGGER fixture_terminal_failure;
        UPDATE memory_jobs SET lease_expires_at_ts = 1 WHERE job_kind = 'extract';
        UPDATE memory_storage_pressure SET state = 'explicit_write_blocked';
        UPDATE memory_principal_quotas SET max_background_cost_microunits = 0;",
    );
    assert!(fixture.run_reopened().await.unwrap());
    assert_eq!(fixture.status(), "completed");
    fixture.assert_one_provider_call();
    assert_eq!(
        fixture.scalar("SELECT row_revision FROM user_preferences"),
        1
    );
    assert_eq!(
        fixture.scalar("SELECT used_background_cost_microunits FROM memory_principal_quotas"),
        1000
    );
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        0
    );
}

#[tokio::test]
async fn lost_response_persistence_is_uncertain_not_permission_to_replay_the_provider() {
    let fixture = GenerationFixture::new().await;
    fixture.sql(
        "CREATE TRIGGER fixture_response_failure BEFORE INSERT ON memory_generation_outputs
        BEGIN SELECT RAISE(ABORT, 'fixture_response_failure'); END;",
    );
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.status(), "failed");
    fixture.assert_one_provider_call();
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM user_preferences"), 0);
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        0
    );
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memory_jobs
        WHERE job_kind = 'extract' AND error_code = 'memory_job_provider_result_unknown' AND retryable = 0"), 1);
    fixture.sql("DROP TRIGGER fixture_response_failure;");
    for _ in 0..3 {
        assert!(!fixture.run().await.unwrap());
    }
    fixture.assert_one_provider_call();
    assert_eq!(fixture.status(), "failed");
}

#[tokio::test]
async fn cancelling_after_actual_commit_preserves_the_result_and_charges_completed_work_once() {
    let fixture = GenerationFixture::new().await;
    fixture.sql(
        "CREATE TRIGGER fixture_cancel_after_commit AFTER UPDATE OF checkpoint_json ON memory_jobs
        WHEN json_extract(NEW.checkpoint_json, '$.phase') = 'commit_verified'
        BEGIN UPDATE memory_jobs SET cancel_requested = 1 WHERE job_id = NEW.job_id; END;",
    );
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.status(), "cancelled");
    fixture.assert_one_provider_call();
    assert_eq!(
        fixture.scalar("SELECT row_revision FROM user_preferences"),
        1
    );
    assert_eq!(
        fixture.scalar("SELECT used_background_cost_microunits FROM memory_principal_quotas"),
        1000
    );
    for _ in 0..3 {
        fixture.run().await.unwrap();
    }
    fixture.assert_one_provider_call();
    assert_eq!(
        fixture.scalar("SELECT used_background_cost_microunits FROM memory_principal_quotas"),
        1000
    );
}

#[tokio::test]
async fn summary_and_fact_failure_roll_back_together_then_resume_the_saved_response() {
    let fixture = GenerationFixture::for_kind("consolidate").await;
    fixture.sql(
        "CREATE TRIGGER fixture_fact_failure BEFORE INSERT ON memory_facts
        BEGIN SELECT RAISE(ABORT, 'fixture_fact_failure'); END;",
    );
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.status(), "retry_wait");
    fixture.assert_one_provider_call();
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM long_term_memories"), 0);
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memory_facts"), 0);
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        1
    );
    fixture.sql(
        "DROP TRIGGER fixture_fact_failure;
        UPDATE memory_jobs SET not_before_ts = 1 WHERE status = 'retry_wait';",
    );
    assert!(fixture.run_reopened().await.unwrap());
    assert_eq!(fixture.status(), "completed");
    fixture.assert_one_provider_call();
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM long_term_memories"), 1);
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memory_facts"), 1);
    assert_eq!(
        fixture.scalar("SELECT used_background_cost_microunits FROM memory_principal_quotas"),
        1000
    );
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        0
    );
}

#[tokio::test]
async fn explicit_memory_clear_discards_staged_responses_and_cannot_resurrect_deleted_sources() {
    use crate::memory::ux::{clear_preview, clear_with_mode, MemoryBulkClearRequest};
    let fixture = GenerationFixture::new().await;
    fixture.sql(
        "CREATE TRIGGER fixture_preference_failure BEFORE INSERT ON user_preferences
        BEGIN SELECT RAISE(ABORT, 'fixture_preference_failure'); END;",
    );
    assert!(fixture.run().await.unwrap());
    fixture.assert_one_provider_call();
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        1
    );
    fixture.sql("DROP TRIGGER fixture_preference_failure;");
    {
        let db = fixture.fixture.state().core.db.get().unwrap();
        let principal = &fixture.fixture.principal;
        let preview = clear_preview(&db, principal, "transcript_and_derived").unwrap();
        clear_with_mode(
            &db,
            principal,
            &MemoryBulkClearRequest {
                mode: "transcript_and_derived".into(),
                expected_transcript_rows: preview.transcript_rows,
                expected_derived_rows: preview.derived_rows,
            },
            crate::now_ts_u64() as i64,
        )
        .unwrap();
    }
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        0
    );
    for _ in 0..3 {
        fixture.run().await.unwrap();
    }
    fixture.assert_one_provider_call();
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memories"), 0);
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM user_preferences"), 0);
    assert_eq!(fixture.status(), "cancelled");
}
