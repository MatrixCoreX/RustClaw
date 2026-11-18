use super::{automatic_generation_availability, GenerationAvailability, StoragePressure};
use crate::memory::capacity_test_fixtures::{pressure, Fixture};

#[test]
fn every_known_pressure_state_has_a_typed_read_only_generation_decision() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    for (state, expected) in [
        ("normal", GenerationAvailability::Available),
        ("derived_cleanup", GenerationAvailability::Available),
        ("backfill_paused", GenerationAvailability::Available),
        (
            "automatic_generation_paused",
            GenerationAvailability::StoragePaused,
        ),
        (
            "explicit_write_blocked",
            GenerationAvailability::StoragePaused,
        ),
    ] {
        pressure(&db, state);
        let before = db.total_changes();
        assert_eq!(
            automatic_generation_availability(&db, &fixture.principal).unwrap(),
            expected
        );
        assert_eq!(
            db.total_changes(),
            before,
            "capacity reads cannot repair state"
        );
    }
}

#[test]
fn missing_and_unknown_pressure_have_distinct_machine_status_without_repair() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "PRAGMA ignore_check_constraints = ON;
        UPDATE memory_storage_pressure SET state = 'future_state';",
    )
    .unwrap();
    assert_eq!(
        StoragePressure::load(&db).unwrap(),
        StoragePressure::Unknown
    );
    assert_eq!(
        automatic_generation_availability(&db, &fixture.principal).unwrap(),
        GenerationAvailability::StorageStateUnknown
    );
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    let before = db.total_changes();
    assert_eq!(
        StoragePressure::load(&db).unwrap(),
        StoragePressure::Missing
    );
    assert_eq!(
        automatic_generation_availability(&db, &fixture.principal).unwrap(),
        GenerationAvailability::StorageStateMissing
    );
    assert_eq!(db.total_changes(), before);
}

#[test]
fn each_exhausted_source_quota_has_its_own_machine_status() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    for (field, expected) in [
        ("max_rows", GenerationAvailability::PrincipalRowsExhausted),
        ("max_bytes", GenerationAvailability::PrincipalBytesExhausted),
        (
            "max_background_cost_microunits",
            GenerationAvailability::PrincipalBackgroundCostExhausted,
        ),
    ] {
        db.execute(
            "UPDATE memory_principal_quotas SET max_rows = 10, max_bytes = 10,
            max_background_cost_microunits = 10",
            [],
        )
        .unwrap();
        db.execute(
            &format!("UPDATE memory_principal_quotas SET {field} = 0"),
            [],
        )
        .unwrap();
        assert_eq!(
            automatic_generation_availability(&db, &fixture.principal).unwrap(),
            expected
        );
    }
}

#[test]
fn negative_quota_usage_or_limits_never_authorize_background_work() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    for field in [
        "used_rows",
        "max_rows",
        "used_bytes",
        "max_bytes",
        "used_background_cost_microunits",
        "max_background_cost_microunits",
    ] {
        db.execute(
            "UPDATE memory_principal_quotas SET used_rows = 0, used_bytes = 0,
            used_background_cost_microunits = 0, max_rows = 10, max_bytes = 10,
            max_background_cost_microunits = 10",
            [],
        )
        .unwrap();
        db.execute(
            &format!("UPDATE memory_principal_quotas SET {field} = -1"),
            [],
        )
        .unwrap();
        assert_eq!(
            automatic_generation_availability(&db, &fixture.principal).unwrap(),
            GenerationAvailability::PrincipalQuotaInvalid
        );
    }
}

#[test]
fn a_missing_principal_quota_cannot_be_created_by_availability_reads() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    db.execute(
        "DELETE FROM memory_principal_quotas WHERE principal_id = ?1",
        [&fixture.principal],
    )
    .unwrap();
    let before = db.total_changes();
    assert_eq!(
        automatic_generation_availability(&db, &fixture.principal).unwrap(),
        GenerationAvailability::PrincipalQuotaMissing
    );
    assert_eq!(db.total_changes(), before);
}

#[test]
fn noninteger_quota_values_are_structured_unavailability_not_worker_query_errors() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    for field in [
        "used_rows",
        "max_rows",
        "used_bytes",
        "max_bytes",
        "used_background_cost_microunits",
        "max_background_cost_microunits",
    ] {
        db.execute(
            "UPDATE memory_principal_quotas SET used_rows = 0, used_bytes = 0,
            used_background_cost_microunits = 0, max_rows = 10, max_bytes = 10,
            max_background_cost_microunits = 10",
            [],
        )
        .unwrap();
        db.execute(
            &format!("UPDATE memory_principal_quotas SET {field} = 'invalid'"),
            [],
        )
        .unwrap();
        assert_eq!(
            automatic_generation_availability(&db, &fixture.principal).unwrap(),
            GenerationAvailability::PrincipalQuotaInvalid
        );
    }
}

#[test]
fn an_invalid_storage_value_type_is_unknown_not_available_or_an_untyped_error() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "PRAGMA ignore_check_constraints = ON;
        UPDATE memory_storage_pressure SET state = X'FF';",
    )
    .unwrap();
    let before = db.total_changes();
    assert_eq!(
        automatic_generation_availability(&db, &fixture.principal).unwrap(),
        GenerationAvailability::StorageStateUnknown
    );
    assert_eq!(db.total_changes(), before);
}
