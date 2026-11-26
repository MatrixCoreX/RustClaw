use super::*;
use crate::memory::completion_test_fixtures::GenerationFixture;

#[tokio::test]
async fn unchanged_turn_reuses_summary_after_worker_reopen_without_a_second_model_call() {
    let fixture = GenerationFixture::with_summary_review(false).await;
    // Put consolidation first in claim order; it must wait for extraction.
    fixture.sql("UPDATE memory_jobs SET not_before_ts=0 WHERE job_kind='consolidate'");
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.status(), "completed");
    fixture.assert_one_provider_call();
    assert!(fixture.run_reopened().await.unwrap());
    fixture.assert_one_provider_call();
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE status='completed'"),
        2
    );
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE json_extract(checkpoint_json,'$.generation_review.summary_reused')=1"),1);
    assert_eq!(
        fixture.scalar("SELECT used_background_cost_microunits FROM memory_principal_quotas"),
        1000
    );
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        0
    );
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memories"), 2);
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_source_events"),
        2
    );
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM long_term_memories"), 0);
}

#[tokio::test]
async fn model_selected_new_information_runs_normal_consolidation() {
    let fixture = GenerationFixture::with_summary_review(true).await;
    assert!(fixture.run().await.unwrap());
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.provider_request_count(), 2);
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE status='completed'"),
        2
    );
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM long_term_memories"), 1);
}

#[tokio::test]
async fn changed_summary_invalidates_an_earlier_no_update_decision() {
    let fixture = GenerationFixture::with_summary_review(false).await;
    assert!(fixture.run().await.unwrap());
    crate::memory::upsert_long_term_summary(
        fixture.fixture.state(),
        fixture.task.user_id,
        fixture.task.chat_id,
        fixture.task.user_key.as_deref(),
        "A concurrent project decision.",
        0,
    )
    .unwrap();
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.provider_request_count(), 2);
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE json_extract(checkpoint_json,'$.generation_review.summary_reused')=1"),0);
}

#[tokio::test]
async fn changed_source_bytes_invalidate_the_review_even_with_unchanged_metadata() {
    let fixture = GenerationFixture::with_summary_review(false).await;
    assert!(fixture.run().await.unwrap());
    fixture.sql("UPDATE memories SET content=content||' changed' WHERE role='user'");
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.provider_request_count(), 2);
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE json_extract(checkpoint_json,'$.generation_review.summary_reused')=1"),0);
}

#[tokio::test]
async fn missing_decision_or_failed_extraction_falls_back_to_normal_consolidation() {
    for mutation in [
        "UPDATE memory_jobs SET checkpoint_json=json_remove(checkpoint_json,'$.generation_review.summary_update_needed') WHERE job_kind='extract'",
        "UPDATE memory_jobs SET status='failed' WHERE job_kind='extract'",
    ] {
        let fixture=GenerationFixture::with_summary_review(false).await;
        assert!(fixture.run().await.unwrap());
        fixture.sql(mutation);
        assert!(fixture.run().await.unwrap());
        assert_eq!(fixture.provider_request_count(),2);
        assert_eq!(fixture.scalar("SELECT COUNT(*) FROM long_term_memories"),1);
    }
}

#[tokio::test]
async fn snapshot_scope_and_owner_mismatches_never_pair_or_block_unrelated_work() {
    for column in [
        "principal_id",
        "scope_ref",
        "source_digest",
        "settings_revision",
        "policy_digest",
        "provider_name",
        "provider_type",
        "model_name",
        "model_capability_digest",
    ] {
        let fixture = GenerationFixture::with_summary_review(false).await;
        let db = fixture.fixture.state().core.db.get().unwrap();
        let consolidate: String = db
            .query_row(
                "SELECT job_id FROM memory_jobs WHERE job_kind='consolidate'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(waiting_for_extraction(&db, &consolidate).unwrap());
        // Column names come only from the closed fixture list above.
        db.execute(
            &format!(
                "UPDATE memory_jobs SET {column}={column}||'-changed' WHERE job_kind='consolidate'"
            ),
            [],
        )
        .unwrap();
        assert!(
            paired_extraction(&db, &consolidate).unwrap().is_none(),
            "{column}"
        );
        assert!(
            !waiting_for_extraction(&db, &consolidate).unwrap(),
            "{column}"
        );
        fixture.assert_no_provider_call();
    }
}

#[tokio::test]
async fn cancelled_extraction_does_not_leave_consolidation_waiting() {
    let fixture = GenerationFixture::with_summary_review(false).await;
    let db = fixture.fixture.state().core.db.get().unwrap();
    let consolidate: String = db
        .query_row(
            "SELECT job_id FROM memory_jobs WHERE job_kind='consolidate'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute(
        "UPDATE memory_jobs SET cancel_requested=1 WHERE job_kind='extract'",
        [],
    )
    .unwrap();
    assert!(!waiting_for_extraction(&db, &consolidate).unwrap());
}

#[tokio::test]
async fn persisted_review_response_survives_summary_change_and_failed_metadata_commit() {
    let fixture = GenerationFixture::with_summary_review(false).await;
    fixture.sql("CREATE TRIGGER fixture_review_commit_failure BEFORE UPDATE OF checkpoint_json ON memory_jobs
        WHEN NEW.job_kind='extract' AND json_type(NEW.checkpoint_json,'$.generation_review.summary_update_needed') IS NOT NULL
        BEGIN SELECT RAISE(ABORT,'fixture_review_commit_failure'); END;");
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.status(), "retry_wait");
    fixture.assert_one_provider_call();
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        1
    );
    crate::memory::upsert_long_term_summary(
        fixture.fixture.state(),
        fixture.task.user_id,
        fixture.task.chat_id,
        fixture.task.user_key.as_deref(),
        "A concurrently refreshed summary.",
        0,
    )
    .unwrap();
    fixture.sql(
        "DROP TRIGGER fixture_review_commit_failure;
        UPDATE memory_jobs SET not_before_ts=0 WHERE status='retry_wait';",
    );
    assert!(fixture.run_reopened().await.unwrap());
    assert_eq!(fixture.status(), "completed");
    assert!(fixture.run_reopened().await.unwrap());
    fixture.assert_one_provider_call();
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        0
    );
}

#[test]
fn review_flag_is_typed_and_optional_for_older_provider_responses() {
    let parsed =
        crate::memory::intent::parse_memory_intent_schema(r#"{"memory_actions":[]}"#).unwrap();
    assert_eq!(parsed.summary_update_needed, None);
    assert!(crate::memory::intent::parse_memory_intent_schema(
        r#"{"memory_actions":[],"summary_update_needed":"false"}"#
    )
    .is_err());
}

#[tokio::test]
async fn cancelling_the_sibling_cannot_change_a_persisted_extraction_prompt() {
    let fixture = GenerationFixture::with_summary_review(false).await;
    fixture.sql("CREATE TRIGGER fixture_review_context_failure BEFORE UPDATE OF checkpoint_json ON memory_jobs
        WHEN NEW.job_kind='extract' AND json_type(NEW.checkpoint_json,'$.generation_review.summary_update_needed') IS NOT NULL
        BEGIN SELECT RAISE(ABORT,'fixture_review_context_failure'); END;");
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.status(), "retry_wait");
    fixture.assert_one_provider_call();
    fixture.sql(
        "DROP TRIGGER fixture_review_context_failure;
        UPDATE memory_jobs SET status='cancelled',cancel_requested=1 WHERE job_kind='consolidate';
        UPDATE memory_jobs SET not_before_ts=0 WHERE job_kind='extract';",
    );
    assert!(fixture.run_reopened().await.unwrap());
    assert_eq!(fixture.status(), "completed");
    fixture.assert_one_provider_call();
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM memory_generation_outputs"),
        0
    );
}

#[tokio::test]
async fn forced_consolidation_does_not_pair_or_reuse_a_summary() {
    let fixture = GenerationFixture::with_forced_summary_review().await;
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE json_type(checkpoint_json,'$.generation_review') IS NOT NULL"),0);
    assert!(fixture.run().await.unwrap());
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.provider_request_count(), 2);
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM long_term_memories"), 1);
}

#[tokio::test]
async fn truncated_or_invalid_generation_retries_without_committing_a_partial_summary() {
    let fixture = GenerationFixture::for_kind("consolidate").await;
    fixture.queue_response("length", r#"{"summary":"partial","fact_candidates":[]}"#);
    fixture.queue_response("stop", r#"{"summary":"short","fact_candidates":[{"fact_value":{}}]}"#);
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.status(), "completed");
    assert_eq!(fixture.provider_request_count(), 3);
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM long_term_memories WHERE summary IN ('partial','short')"),0);
}

#[tokio::test]
async fn repeated_length_terminates_without_committing_even_a_parseable_prefix() {
    let fixture = GenerationFixture::for_kind("consolidate").await;
    for _ in 0..3 { fixture.queue_response("length", r#"{"summary":"partial","fact_candidates":[]}"#); }
    assert!(fixture.run().await.unwrap());
    assert_eq!(fixture.status(),"failed");
    assert_eq!(fixture.provider_request_count(),3);
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM memory_jobs WHERE error_code='memory_job_output_limit_exhausted'"),1);
    assert_eq!(fixture.scalar("SELECT COUNT(*) FROM long_term_memories WHERE summary='partial'"),0);
}
