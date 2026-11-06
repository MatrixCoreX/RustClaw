use rusqlite::Connection;

use super::{GenerationLease, JobPhase};
use crate::memory::capacity::{automatic_generation_availability, GenerationAvailability};
use crate::memory::capacity_test_fixtures::{scalar, Fixture};

fn committed() -> Fixture {
    let fixture = Fixture::memory();
    fixture.seed_job("committed", "running");
    fixture.state().core.db.get().unwrap().execute(
        "UPDATE memory_jobs SET checkpoint_json = '{\"schema_version\":1,\"phase\":\"commit_verified\"}'", []
    ).unwrap();
    fixture
}

#[test]
fn checkpoint_phases_are_versioned_machine_state_not_user_visible_prose() {
    assert_eq!(super::phase("{}"), JobPhase::Unstarted);
    for (token, expected) in [
        ("source_range_verified", JobPhase::SourceVerified),
        ("resource_wait", JobPhase::ResourceWait),
        ("provider_request_started", JobPhase::ProviderStarted),
        ("provider_response_persisted", JobPhase::ResponsePersisted),
        ("commit_verified", JobPhase::Committed),
    ] {
        let value =
            serde_json::json!({"schema_version":1,"phase":token,"untrusted_text":"ignored"});
        assert_eq!(super::phase(&value.to_string()), expected);
    }
    for value in [
        "null",
        "[]",
        "not-json",
        "{\"phase\":\"commit_verified\"}",
        "{\"schema_version\":2,\"phase\":\"commit_verified\"}",
        "{\"schema_version\":1,\"phase\":\"future_phase\"}",
    ] {
        assert_eq!(super::phase(value), JobPhase::Invalid);
    }
}

#[test]
fn upgrade_is_transactional_idempotent_and_never_rebills_historical_terminal_jobs() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE runtime_schema_migrations(
        migration_id TEXT PRIMARY KEY, schema_digest TEXT NOT NULL, applied_at TEXT NOT NULL);",
    )
    .unwrap();
    db.execute_batch(include_str!(
        "../../../../migrations/011_durable_memory_jobs.sql"
    ))
    .unwrap();
    db.execute_batch(
        "INSERT INTO memory_jobs(job_id, job_kind, principal_id, scope_kind,
        scope_ref, source_digest, eligibility_json, settings_revision, policy_digest,
        provider_name, provider_type, model_name, model_capability_digest, status,
        not_before_ts, created_at_ts, updated_at_ts) VALUES ('legacy','extract','owner',
        'principal','owner','digest','{}',1,'policy','provider','type','model','capability',
        'completed',0,0,0);",
    )
    .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    super::ensure_schema(&tx).unwrap();
    tx.commit().unwrap();
    let before = db.total_changes();
    super::ensure_schema(&db).unwrap();
    assert_eq!(db.total_changes(), before);
    assert_eq!(
        scalar(
            &db,
            "SELECT SUM(background_cost_pending_microunits) FROM memory_jobs"
        ),
        0
    );
    assert_eq!(
        scalar(&db, "SELECT COUNT(*) FROM memory_generation_outputs"),
        0
    );
    db.execute(
        "UPDATE runtime_schema_migrations SET schema_digest = 'changed' WHERE migration_id = ?1",
        [super::MIGRATION_ID],
    )
    .unwrap();
    let before = db.total_changes();
    assert!(super::ensure_schema(&db).is_err());
    assert_eq!(db.total_changes(), before);
}

#[test]
fn settlement_marker_failure_rolls_back_the_quota_increment_and_can_settle_once_later() {
    let fixture = committed();
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute_batch(
            "CREATE TRIGGER fixture_settlement_failure
            BEFORE UPDATE OF background_cost_pending_microunits ON memory_jobs
            WHEN NEW.background_cost_pending_microunits = 0
            BEGIN SELECT RAISE(ABORT, 'fixture_settlement_failure'); END;",
        )
        .unwrap();
    }
    super::complete(fixture.state(), "committed", "capacity-worker").unwrap();
    {
        let db = fixture.state().core.db.get().unwrap();
        assert_eq!(
            scalar(
                &db,
                "SELECT used_background_cost_microunits FROM memory_principal_quotas"
            ),
            0
        );
        assert_eq!(
            scalar(
                &db,
                "SELECT background_cost_pending_microunits FROM memory_jobs"
            ),
            1000
        );
        db.execute_batch("DROP TRIGGER fixture_settlement_failure")
            .unwrap();
    }
    assert!(super::settle_due(fixture.state()).unwrap());
    assert!(!super::settle_due(fixture.state()).unwrap());
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT used_background_cost_microunits FROM memory_principal_quotas"
        ),
        1000
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT background_cost_pending_microunits FROM memory_jobs"
        ),
        0
    );
}

#[test]
fn deferred_completed_cost_still_reserves_the_principals_generation_budget() {
    let fixture = committed();
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute_batch(
            "UPDATE memory_principal_quotas SET max_background_cost_microunits = 1000;
            CREATE TRIGGER fixture_cost_failure BEFORE UPDATE OF used_background_cost_microunits
            ON memory_principal_quotas BEGIN SELECT RAISE(ABORT, 'fixture_cost_failure'); END;",
        )
        .unwrap();
    }
    super::complete(fixture.state(), "committed", "capacity-worker").unwrap();
    {
        let db = fixture.state().core.db.get().unwrap();
        assert_eq!(
            automatic_generation_availability(&db, &fixture.principal).unwrap(),
            GenerationAvailability::PrincipalBackgroundCostExhausted
        );
        db.execute_batch("DROP TRIGGER fixture_cost_failure")
            .unwrap();
    }
    super::settle_due(fixture.state()).unwrap();
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        automatic_generation_availability(&db, &fixture.principal).unwrap(),
        GenerationAvailability::PrincipalBackgroundCostExhausted
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT used_background_cost_microunits FROM memory_principal_quotas"
        ),
        1000
    );
}

#[tokio::test]
async fn expired_cancelled_committed_lease_finishes_and_accounts_without_a_provider() {
    let fixture = committed();
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute_batch(
            "UPDATE memory_jobs SET cancel_requested = 1, lease_expires_at_ts = 1;
            UPDATE memory_storage_pressure SET state = 'explicit_write_blocked';",
        )
        .unwrap();
    }
    for _ in 0..3 {
        crate::memory::jobs::run_one_memory_job(fixture.state(), "recovery-worker")
            .await
            .unwrap();
    }
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_jobs WHERE status = 'cancelled' AND attempt = 1"
        ),
        1
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT used_background_cost_microunits FROM memory_principal_quotas"
        ),
        1000
    );
}

#[test]
fn a_stale_generation_lease_rolls_back_canonical_writes_and_cannot_mark_a_result() {
    let fixture = committed();
    let db = fixture.state().core.db.get().unwrap();
    {
        let tx = db.unchecked_transaction().unwrap();
        crate::memory::upsert_user_preferences(
            fixture.state(),
            &tx,
            1,
            0,
            &fixture.key,
            &[(
                "response_format".into(),
                "markdown".into(),
                1.0,
                "fixture".into(),
            )],
            "1",
            1,
        )
        .unwrap();
        assert!(super::mark_committed(
            &tx,
            GenerationLease {
                job_id: "committed",
                worker_id: "stale-worker",
                principal_id: &fixture.principal
            }
        )
        .is_err());
    }
    assert_eq!(scalar(&db, "SELECT COUNT(*) FROM user_preferences"), 0);
    assert_eq!(
        scalar(
            &db,
            "SELECT background_cost_pending_microunits FROM memory_jobs"
        ),
        0
    );
}

#[test]
fn overflowing_cost_is_deferred_without_promoting_the_counter_to_a_float_or_losing_completion() {
    let fixture = committed();
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute(
            "UPDATE memory_principal_quotas SET used_background_cost_microunits = ?1",
            [i64::MAX - 500],
        )
        .unwrap();
    }
    super::complete(fixture.state(), "committed", "capacity-worker").unwrap();
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT used_background_cost_microunits FROM memory_principal_quotas"
        ),
        i64::MAX - 500
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT background_cost_pending_microunits FROM memory_jobs"
        ),
        1000
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT COUNT(*) FROM memory_jobs WHERE status = 'completed'"
        ),
        1
    );
    assert_eq!(
        automatic_generation_availability(&db, &fixture.principal).unwrap(),
        GenerationAvailability::PrincipalQuotaInvalid
    );
}
