use rusqlite::params;

use crate::memory::capacity_race_test_fixtures::{
    another_principal, copy_queued_job_for, pause_inside_claim,
};
use crate::memory::capacity_test_fixtures::{job_status, Fixture};

#[test]
fn generation_claim_rechecks_capacity_inside_its_actual_writer_transaction() {
    let fixture = Fixture::file();
    fixture.seed_job("queued", "queued");
    let claimed = pause_inside_claim(&fixture, |state| {
        super::claim_next_job(state, "transaction-race-worker")
    })
    .unwrap();
    assert!(
        claimed.is_none(),
        "a preflight before the writer gate is insufficient"
    );
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(job_status(&db, "queued"), ("queued".to_string(), 0));
}

#[test]
fn quota_blocked_oldest_principal_does_not_starve_another_eligible_principal() {
    let fixture = Fixture::memory();
    let other = another_principal(&fixture);
    fixture.seed_job("quota-blocked", "queued");
    {
        let db = fixture.state().core.db.get().unwrap();
        copy_queued_job_for(&db, "quota-blocked", "eligible", &other);
        db.execute(
            "UPDATE memory_principal_quotas SET max_rows = 0, revision = 41
            WHERE principal_id = ?1",
            [&fixture.principal],
        )
        .unwrap();
    }
    let claimed = super::claim_next_job(fixture.state(), "fair-worker")
        .unwrap()
        .unwrap();
    assert_eq!(claimed.job_id, "eligible");
    {
        let db = fixture.state().core.db.get().unwrap();
        assert_eq!(job_status(&db, "quota-blocked"), ("queued".to_string(), 0));
        db.execute(
            "UPDATE memory_principal_quotas SET max_rows = ?2, revision = 42
            WHERE principal_id = ?1",
            params![
                fixture.principal,
                fixture.state().policy.memory.max_rows.max(1) as i64
            ],
        )
        .unwrap();
    }
    assert_eq!(
        super::claim_next_job(fixture.state(), "recovered-worker")
            .unwrap()
            .unwrap()
            .job_id,
        "quota-blocked"
    );
}

#[tokio::test]
async fn one_principals_unsettled_cost_does_not_block_another_durable_completion() {
    let fixture = Fixture::memory();
    let other = another_principal(&fixture);
    fixture.seed_job("accounting-a", "running");
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute("UPDATE memory_jobs SET checkpoint_json = '{\"schema_version\":1,\"phase\":\"commit_verified\"}',
            progress_current = 2, progress_total = 2 WHERE job_id = 'accounting-a'", []).unwrap();
        copy_queued_job_for(&db, "accounting-a", "accounting-b", &other);
        db.execute_batch("CREATE TRIGGER fixture_only_a_cost_failure
            BEFORE UPDATE OF used_background_cost_microunits ON memory_principal_quotas
            WHEN OLD.principal_id = (SELECT principal_id FROM memory_jobs WHERE job_id = 'accounting-a')
            BEGIN SELECT RAISE(ABORT, 'fixture_only_a_cost_failure'); END;").unwrap();
    }
    let _failed_or_deferred =
        super::complete_job(fixture.state(), "accounting-a", "capacity-worker");
    super::run_one_memory_job(fixture.state(), "independent-worker")
        .await
        .unwrap();
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(job_status(&db, "accounting-a").0, "completed");
    assert_eq!(job_status(&db, "accounting-b").0, "completed");
    let other_cost: i64 = db
        .query_row(
            "SELECT used_background_cost_microunits
        FROM memory_principal_quotas WHERE principal_id = ?1",
            [other],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(other_cost, 1000);
}
