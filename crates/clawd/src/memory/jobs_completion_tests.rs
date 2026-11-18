use crate::memory::capacity_test_fixtures::{job_status, pressure, scalar, Fixture};

fn committed_job() -> Fixture {
    let fixture = Fixture::memory();
    fixture.seed_job("committed", "running");
    let db = fixture.state().core.db.get().unwrap();
    db.execute("UPDATE memory_jobs SET checkpoint_json = '{\"schema_version\":1,\"phase\":\"commit_verified\"}',
        progress_current = 2, progress_total = 2 WHERE job_id = 'committed'", []).unwrap();
    db.execute(
        "UPDATE memory_principal_quotas SET used_background_cost_microunits = 7,
        max_background_cost_microunits = 50000, revision = 17",
        [],
    )
    .unwrap();
    drop(db);
    fixture
}

fn fail_accounting(db: &rusqlite::Connection) {
    db.execute_batch(
        "CREATE TRIGGER fixture_cost_failure BEFORE UPDATE OF used_background_cost_microunits
        ON memory_principal_quotas BEGIN SELECT RAISE(ABORT, 'fixture_cost_failure'); END;",
    )
    .unwrap();
}

#[test]
fn completed_generation_result_is_not_failed_just_because_cost_write_fails() {
    let fixture = committed_job();
    {
        let db = fixture.state().core.db.get().unwrap();
        fail_accounting(&db);
    }
    let result = super::complete_job(fixture.state(), "committed", "capacity-worker");
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(job_status(&db, "committed"), ("completed".to_string(), 1));
    assert!(
        result.is_ok(),
        "durable completion must expose deferred accounting, not execution failure"
    );
}

#[tokio::test]
async fn failed_completion_cost_is_eventually_settled_once_without_replaying_provider_work() {
    let fixture = committed_job();
    {
        let db = fixture.state().core.db.get().unwrap();
        fail_accounting(&db);
    }
    let _failed_or_deferred = super::complete_job(fixture.state(), "committed", "capacity-worker");
    {
        let db = fixture.state().core.db.get().unwrap();
        assert_eq!(job_status(&db, "committed").0, "completed");
        db.execute_batch("DROP TRIGGER fixture_cost_failure")
            .unwrap();
        pressure(&db, "explicit_write_blocked");
    }
    for _ in 0..3 {
        super::run_one_memory_job(fixture.state(), "reconcile-worker")
            .await
            .unwrap();
    }
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT used_background_cost_microunits FROM memory_principal_quotas"
        ),
        1007
    );
    assert_eq!(
        scalar(&db, "SELECT revision FROM memory_principal_quotas"),
        17
    );
    assert_eq!(job_status(&db, "committed"), ("completed".to_string(), 1));
    let checkpoint: String = db
        .query_row("SELECT checkpoint_json FROM memory_jobs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&checkpoint).unwrap()["phase"],
        "commit_verified"
    );
}

#[test]
fn repeated_completion_cannot_double_charge_successful_work() {
    let fixture = committed_job();
    super::complete_job(fixture.state(), "committed", "capacity-worker").unwrap();
    let _already_terminal = super::complete_job(fixture.state(), "committed", "capacity-worker");
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT used_background_cost_microunits FROM memory_principal_quotas"
        ),
        1007
    );
}

#[test]
fn rejected_terminal_write_does_not_charge_or_erase_the_committed_checkpoint() {
    let fixture = committed_job();
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute_batch(
            "CREATE TRIGGER fixture_terminal_failure BEFORE UPDATE OF status ON memory_jobs
            BEGIN SELECT RAISE(ABORT, 'fixture_terminal_failure'); END;",
        )
        .unwrap();
    }
    assert!(super::complete_job(fixture.state(), "committed", "capacity-worker").is_err());
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(job_status(&db, "committed"), ("running".to_string(), 1));
    assert_eq!(
        scalar(
            &db,
            "SELECT used_background_cost_microunits FROM memory_principal_quotas"
        ),
        7
    );
}

#[tokio::test]
async fn durable_committed_checkpoint_survives_completion_failure_without_provider_reexecution() {
    let fixture = committed_job();
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute_batch(
            "CREATE TRIGGER fixture_terminal_failure BEFORE UPDATE OF status ON memory_jobs
            BEGIN SELECT RAISE(ABORT, 'fixture_terminal_failure'); END;",
        )
        .unwrap();
    }
    assert!(super::complete_job(fixture.state(), "committed", "capacity-worker").is_err());
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute_batch(
            "DROP TRIGGER fixture_terminal_failure;
            UPDATE memory_jobs SET lease_expires_at_ts = 1;",
        )
        .unwrap();
    }
    // No source task/provider snapshot exists in this fixture: only the
    // already-durable checkpoint may finish, never a second model execution.
    super::run_one_memory_job(fixture.state(), "recovery-worker")
        .await
        .unwrap();
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(job_status(&db, "committed"), ("completed".to_string(), 2));
    assert_eq!(
        scalar(
            &db,
            "SELECT used_background_cost_microunits FROM memory_principal_quotas"
        ),
        1007
    );
}
