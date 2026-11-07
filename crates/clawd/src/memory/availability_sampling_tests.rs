use super::*;
use crate::memory::capacity_test_fixtures::{persist_preference_sources, pressure, Fixture};
use std::cell::Cell;

fn fixture(file: bool) -> Fixture {
    let fixture = if file {
        Fixture::file()
    } else {
        Fixture::memory()
    };
    {
        let db = fixture.state().core.db.get().unwrap();
        crate::memory::ux::ensure_memory_ux_schema(&db).unwrap();
        crate::memory::retention::initialize_storage_pressure(
            &db,
            &fixture.state().policy.memory,
            100,
        )
        .unwrap();
    }
    fixture
}

fn observe(
    fixture: &Fixture,
    db: &Connection,
    admin: bool,
    scans: &Cell<usize>,
) -> (MemoryAvailability, Option<String>) {
    let settings = crate::memory::settings::resolve_principal_memory_settings(
        db,
        &fixture.principal,
        fixture.state().policy.memory.long_term_enabled,
    )
    .unwrap();
    let changes = db.total_changes();
    db.pragma_update(None, "query_only", true).unwrap();
    let report = observe_with_domain_sampler(
        db,
        settings,
        admin,
        fixture.state().policy.memory.storage_soft_limit_bytes,
        101,
        || {
            scans.set(scans.get() + 1);
            measurements::memory_btrees(db)
        },
    )
    .unwrap();
    assert_eq!(db.total_changes(), changes);
    assert_eq!(report.0.settings.target_principal_id, fixture.principal);
    db.pragma_update(None, "query_only", false).unwrap();
    report
}

fn ordinary_unavailable(fixture: &Fixture, expected: GrowthDecision) {
    let db = fixture.state().core.db.get().unwrap();
    let scans = Cell::new(0);
    let (report, path) = observe(fixture, &db, false, &scans);
    assert_eq!(report.manual_growth.decision, expected);
    assert_eq!(scans.get(), 0, "invalid state/policy needs no domain scan");
    assert_eq!(
        report.manual_growth.configured_limit_bytes,
        ByteMeasurement::AdminOnly
    );
    assert_eq!(
        report.measurements.memory_domain_assigned_btree_bytes,
        ByteMeasurement::AdminOnly
    );
    assert_eq!(path, None);
}

#[test]
fn missing_state_ordinary_read_skips_domain_scan_without_repair() {
    let fixture = fixture(false);
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute("DELETE FROM memory_storage_pressure", [])
            .unwrap();
    }
    ordinary_unavailable(&fixture, GrowthDecision::StorageStateMissing);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM memory_storage_pressure", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn unknown_state_ordinary_read_skips_domain_scan_without_normalizing() {
    let fixture = fixture(false);
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE memory_storage_pressure SET state='future_state'").unwrap();
    }
    ordinary_unavailable(&fixture, GrowthDecision::StorageStateUnknown);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        db.query_row("SELECT state FROM memory_storage_pressure", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "future_state"
    );
}

#[test]
fn missing_policy_ordinary_read_skips_domain_scan_without_inserting_defaults() {
    let fixture = fixture(false);
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute("DELETE FROM memory_storage_budget", []).unwrap();
    }
    ordinary_unavailable(&fixture, GrowthDecision::StoragePolicyMissing);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM memory_storage_budget", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn damaged_policy_ordinary_read_skips_domain_scan_without_rewriting_policy() {
    let fixture = fixture(false);
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute(
            "UPDATE memory_storage_budget SET configured_limit_bytes='invalid'",
            [],
        )
        .unwrap();
    }
    ordinary_unavailable(&fixture, GrowthDecision::StoragePolicyInvalid);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT configured_limit_bytes FROM memory_storage_budget",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "invalid"
    );
}

#[test]
fn unbound_explicit_stop_ordinary_read_skips_domain_scan() {
    let fixture = fixture(false);
    {
        let db = fixture.state().core.db.get().unwrap();
        pressure(&db, "explicit_write_blocked");
    }
    ordinary_unavailable(&fixture, GrowthDecision::ExplicitStopUnbound);
}

#[test]
fn healthy_ordinary_read_samples_once_without_exposing_global_bytes_or_path() {
    let fixture = fixture(true);
    let db = fixture.state().core.db.get().unwrap();
    let scans = Cell::new(0);
    let (report, path) = observe(&fixture, &db, false, &scans);
    assert_eq!(report.manual_growth.decision, GrowthDecision::Available);
    assert_eq!(scans.get(), 1);
    assert_eq!(
        report.manual_growth.configured_limit_bytes,
        ByteMeasurement::AdminOnly
    );
    assert_eq!(
        report.measurements.memory_domain_assigned_btree_bytes,
        ByteMeasurement::AdminOnly
    );
    assert_eq!(path, None);
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("fixture.sqlite"));
    assert!(!json.contains(&fixture.key));
}

#[test]
fn healthy_admin_read_reuses_one_actual_sample_for_decision_and_diagnostics() {
    let fixture = fixture(true);
    let db = fixture.state().core.db.get().unwrap();
    let actual = measurements::memory_btrees(&db);
    assert!(matches!(actual, ByteMeasurement::Measured { bytes } if bytes > 0));
    let scans = Cell::new(0);
    let (report, path) = observe(&fixture, &db, true, &scans);
    assert_eq!(report.manual_growth.decision, GrowthDecision::Available);
    assert_eq!(scans.get(), 1);
    assert_eq!(
        report.measurements.memory_domain_assigned_btree_bytes,
        actual
    );
    assert!(path.unwrap().ends_with("fixture.sqlite"));
    let settings = crate::memory::settings::resolve_principal_memory_settings(
        &db,
        &fixture.principal,
        fixture.state().policy.memory.long_term_enabled,
    )
    .unwrap();
    let public = observe_sqlite(
        &db,
        settings,
        true,
        fixture.state().policy.memory.storage_soft_limit_bytes,
        101,
    )
    .unwrap()
    .0;
    assert_eq!(
        serde_json::to_value(public).unwrap(),
        serde_json::to_value(report).unwrap()
    );
}

#[test]
fn admin_missing_policy_still_samples_once_for_read_only_diagnostics() {
    let fixture = fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute("DELETE FROM memory_storage_budget", []).unwrap();
    let scans = Cell::new(0);
    let (report, _) = observe(&fixture, &db, true, &scans);
    assert_eq!(
        report.manual_growth.decision,
        GrowthDecision::StoragePolicyMissing
    );
    assert_eq!(scans.get(), 1);
    assert!(
        matches!(report.measurements.memory_domain_assigned_btree_bytes, ByteMeasurement::Measured { bytes } if bytes > 0)
    );
    assert!(matches!(
        report.manual_growth.configured_limit_bytes,
        ByteMeasurement::Unavailable { .. }
    ));
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM memory_storage_budget", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn a_later_observation_resamples_changed_storage_instead_of_reusing_old_bytes() {
    let fixture = fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let scans = Cell::new(0);
    let (before, _) = observe(&fixture, &db, true, &scans);
    assert_eq!(scans.get(), 1);
    persist_preference_sources(
        &db,
        1,
        0,
        &fixture.key,
        &[(
            "large-sampling-content".into(),
            "fresh domain content ".repeat(16384),
            1.0,
            "user".into(),
        )],
        102,
    );
    let (after, _) = observe(&fixture, &db, true, &scans);
    assert_eq!(scans.get(), 2);
    let bytes = |value: ByteMeasurement| match value {
        ByteMeasurement::Measured { bytes } => bytes,
        other => panic!("actual measurement required: {other:?}"),
    };
    assert!(
        bytes(after.measurements.memory_domain_assigned_btree_bytes)
            > bytes(before.measurements.memory_domain_assigned_btree_bytes)
    );
}

#[test]
fn failed_actual_measurement_is_shared_once_and_a_later_read_can_recover() {
    let fixture = fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch("CREATE TEMP TABLE dbstat(pgsize,name)")
        .unwrap();
    assert_eq!(
        measurements::memory_btrees(&db),
        ByteMeasurement::unavailable(MeasurementUnavailable::QueryFailed)
    );
    let scans = Cell::new(0);
    let (blocked, _) = observe(&fixture, &db, true, &scans);
    assert_eq!(
        blocked.manual_growth.decision,
        GrowthDecision::MeasurementUnavailable
    );
    assert_eq!(
        blocked.measurements.memory_domain_assigned_btree_bytes,
        ByteMeasurement::unavailable(MeasurementUnavailable::QueryFailed)
    );
    assert_eq!(scans.get(), 1, "diagnostics must not retry a failed sample");
    db.execute_batch("DROP TABLE temp.dbstat").unwrap();
    let (recovered, _) = observe(&fixture, &db, true, &scans);
    assert_eq!(recovered.manual_growth.decision, GrowthDecision::Available);
    assert!(
        matches!(recovered.measurements.memory_domain_assigned_btree_bytes, ByteMeasurement::Measured { bytes } if bytes > 0)
    );
    assert_eq!(scans.get(), 2);
}
