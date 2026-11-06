use super::{automatic_generation_availability, GenerationAvailability};
use crate::memory::capacity_test_fixtures::Fixture;

fn availability(db: &rusqlite::Connection, principal: &str) -> GenerationAvailability {
    let before = db.total_changes();
    let actual = automatic_generation_availability(db, principal)
        .expect("capacity type handling must return a structured decision");
    assert_eq!(
        db.total_changes(),
        before,
        "capacity decisions are read-only"
    );
    actual
}

#[test]
fn invalid_pending_cost_types_are_unavailable_without_sqlite_numeric_coercion() {
    let fixture = Fixture::memory();
    fixture.seed_job("pending", "running");
    let db = fixture.state().core.db.get().unwrap();
    // Simulate persisted corruption; normal writes remain protected by 029's CHECK.
    db.execute_batch("PRAGMA ignore_check_constraints = ON")
        .unwrap();
    for value in ["'invalid'", "1.5", "X'FF'"] {
        db.execute_batch(&format!(
            "UPDATE memory_jobs SET background_cost_pending_microunits = {value}"
        ))
        .unwrap();
        assert_eq!(
            availability(&db, &fixture.principal),
            GenerationAvailability::PrincipalQuotaInvalid
        );
    }
}

#[test]
fn overflowing_pending_cost_sum_is_unavailable_not_a_worker_query_error() {
    let fixture = Fixture::memory();
    fixture.seed_job("first", "running");
    fixture.seed_job("second", "running");
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "UPDATE memory_principal_quotas SET max_background_cost_microunits = 9223372036854775807;
         UPDATE memory_jobs SET background_cost_pending_microunits = 9223372036854775807;",
    )
    .unwrap();
    assert_eq!(
        availability(&db, &fixture.principal),
        GenerationAvailability::PrincipalQuotaInvalid
    );
}

#[test]
fn spent_plus_pending_cost_overflow_is_unavailable_without_float_promotion() {
    let fixture = Fixture::memory();
    fixture.seed_job("pending", "running");
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "UPDATE memory_principal_quotas SET used_background_cost_microunits = 9223372036854775806,
            max_background_cost_microunits = 9223372036854775807;
         UPDATE memory_jobs SET background_cost_pending_microunits = 2;",
    )
    .unwrap();
    assert_eq!(
        availability(&db, &fixture.principal),
        GenerationAvailability::PrincipalQuotaInvalid
    );
}

#[test]
fn valid_pending_cost_is_exact_and_cannot_read_another_principals_malformed_cost() {
    let fixture = Fixture::memory();
    fixture.seed_job("own", "running");
    fixture.seed_job("foreign", "running");
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "PRAGMA ignore_check_constraints = ON;
         UPDATE memory_principal_quotas SET used_background_cost_microunits = 100,
            max_background_cost_microunits = 1000;
         UPDATE memory_jobs SET principal_id = 'another-owner',
            background_cost_pending_microunits = 'invalid' WHERE job_id = 'foreign';
         UPDATE memory_jobs SET background_cost_pending_microunits = 899 WHERE job_id = 'own';",
    )
    .unwrap();
    assert_eq!(
        availability(&db, &fixture.principal),
        GenerationAvailability::Available
    );
    db.execute_batch(
        "UPDATE memory_jobs SET background_cost_pending_microunits = 900 WHERE job_id = 'own';",
    )
    .unwrap();
    assert_eq!(
        availability(&db, &fixture.principal),
        GenerationAvailability::PrincipalBackgroundCostExhausted
    );
}
