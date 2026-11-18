use super::boundary_test_fixtures as setup;
use super::*;
use crate::memory::availability::{self, ByteMeasurement};
use crate::memory::growth_availability::{self, GrowthDecision, ProbePhase};

#[path = "ux_budget_test_fixtures.rs"]
mod support;

#[test]
fn unrelated_main_growth_can_pause_background_without_blocking_manual_content() {
    let mut fixture = support::fixture(false);
    let limit = {
        let db = fixture.state().core.db.get().unwrap();
        let page: u64 = db
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .unwrap();
        let limit = support::domain(&db, &fixture) + page * 32;
        db.execute_batch("CREATE TABLE unrelated_growth(payload BLOB)")
            .unwrap();
        db.execute(
            "INSERT INTO unrelated_growth VALUES(zeroblob(?1))",
            [i64::try_from(limit * 2).unwrap()],
        )
        .unwrap();
        limit
    };
    support::limit(&mut fixture, limit);
    let db = fixture.state().core.db.get().unwrap();
    let report = support::report(&db, &fixture).0;
    assert_eq!(
        report.capacity_decision,
        crate::memory::capacity::GenerationAvailability::StoragePaused
    );
    assert_eq!(report.manual_growth.decision, GrowthDecision::Available);
    assert_eq!(
        report.manual_growth.configured_limit_bytes,
        ByteMeasurement::Measured { bytes: limit }
    );
    let request = MemoryCorrectionRequest {
        expected_revision: 1,
        content: "Small explicit correction".into(),
    };
    assert_eq!(
        correct_memory(
            &db,
            &fixture.principal,
            &fixture.principal,
            "witness",
            &request,
            103
        )
        .unwrap()
        .status,
        "corrected"
    );
}

#[test]
fn source_quota_exhaustion_does_not_become_manual_domain_exhaustion() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute(
        "UPDATE memory_principal_quotas SET max_rows=0,max_bytes=0 WHERE principal_id=?1",
        [&fixture.principal],
    )
    .unwrap();
    let report = support::report(&db, &fixture).0;
    assert_eq!(
        report.capacity_decision,
        crate::memory::capacity::GenerationAvailability::PrincipalRowsExhausted
    );
    assert_eq!(report.manual_growth.decision, GrowthDecision::Available);
    assert_eq!(
        correct_memory(
            &db,
            &fixture.principal,
            &fixture.principal,
            "witness",
            &support::large_correction(),
            103
        )
        .unwrap()
        .status,
        "corrected"
    );
}

#[test]
fn missing_budget_is_read_only_unavailability_and_actual_growth_rejection() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute("DELETE FROM memory_storage_budget", []).unwrap();
    let before = support::state(&db);
    let changes = db.total_changes();
    db.pragma_update(None, "query_only", true).unwrap();
    let report = support::report(&db, &fixture).0;
    assert_eq!(
        report.manual_growth.decision,
        GrowthDecision::StoragePolicyMissing
    );
    assert!(matches!(
        report.manual_growth.configured_limit_bytes,
        ByteMeasurement::Unavailable { .. }
    ));
    assert_eq!(support::state(&db), before);
    assert_eq!(db.total_changes(), changes);
    db.pragma_update(None, "query_only", false).unwrap();
    let error = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "witness",
        &support::large_correction(),
        103,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_policy_missing");
    assert_eq!(support::state(&db), before);
}

#[test]
fn damaged_budget_has_typed_projection_and_is_not_normalized() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute(
        "UPDATE memory_storage_budget SET configured_limit_bytes='broken'",
        [],
    )
    .unwrap();
    assert_eq!(
        crate::memory::storage_budget::Snapshot::load(&db).unwrap_err(),
        crate::memory::storage_budget::ReadError::Invalid
    );
    let before = support::state(&db);
    db.pragma_update(None, "query_only", true).unwrap();
    assert_eq!(
        support::report(&db, &fixture).0.manual_growth.decision,
        GrowthDecision::StoragePolicyInvalid
    );
    assert_eq!(support::state(&db), before);
    db.pragma_update(None, "query_only", false).unwrap();
    assert_eq!(
        correct_memory(
            &db,
            &fixture.principal,
            &fixture.principal,
            "witness",
            &support::large_correction(),
            103
        )
        .unwrap_err()
        .to_string(),
        "memory_storage_policy_invalid"
    );
    assert_eq!(support::state(&db), before);
}

#[test]
fn actual_domain_limit_reports_blocked_without_disabling_recovery() {
    let mut fixture = support::fixture(false);
    support::hard(&mut fixture);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        support::report(&db, &fixture).0.manual_growth.decision,
        GrowthDecision::DomainBudgetReached
    );
    assert_eq!(
        correct_memory(
            &db,
            &fixture.principal,
            &fixture.principal,
            "witness",
            &support::large_correction(),
            103
        )
        .unwrap_err()
        .to_string(),
        "memory_storage_write_blocked"
    );
    assert_eq!(
        export_memory(&db, &fixture.principal, 103)
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(
        delete_memory_with_revision(
            &db,
            &fixture.principal,
            "witness",
            &MemoryMutationRequest {
                expected_revision: 1
            },
            104
        )
        .unwrap()
        .status,
        "deleted"
    );
}

#[test]
fn ordinary_projection_keeps_global_manual_limit_and_measurement_private() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let settings = crate::memory::settings::resolve_principal_memory_settings(
        &db,
        &fixture.principal,
        fixture.state().policy.memory.long_term_enabled,
    )
    .unwrap();
    let (report, path) = availability::observe_sqlite(
        &db,
        settings,
        false,
        fixture.state().policy.memory.storage_soft_limit_bytes,
        103,
    )
    .unwrap();
    assert_eq!(report.settings.target_principal_id, fixture.principal);
    assert_eq!(report.manual_growth.decision, GrowthDecision::Available);
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
fn invalid_state_or_policy_is_rejected_without_sampling_domains() {
    for variant in ["state", "policy"] {
        let fixture = support::fixture(false);
        let db = fixture.state().core.db.get().unwrap();
        let expected = if variant == "state" {
            db.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE memory_storage_pressure SET state='future_state'").unwrap();
            GrowthDecision::StorageStateUnknown
        } else {
            db.execute("DELETE FROM memory_storage_budget", []).unwrap();
            GrowthDecision::StoragePolicyMissing
        };
        let before = support::state(&db);
        let observation = growth_availability::probe(&db, ProbePhase::BeforeGrowth, || {
            panic!("invalid state must not scan btrees")
        });
        assert_eq!(observation.decision, expected);
        assert!(observation.admit().is_err());
        assert_eq!(support::state(&db), before);
    }
}

#[test]
fn exact_allocation_limit_distinguishes_completion_from_new_growth() {
    let mut fixture = support::fixture(false);
    support::hard(&mut fixture);
    let db = fixture.state().core.db.get().unwrap();
    let before = support::state(&db);
    let beginning = growth_availability::probe(&db, ProbePhase::BeforeGrowth, || {
        availability::memory_btrees(&db)
    });
    let completion = growth_availability::probe(&db, ProbePhase::AfterGrowth, || {
        availability::memory_btrees(&db)
    });
    assert_eq!(beginning.decision, GrowthDecision::DomainBudgetReached);
    assert_eq!(completion.decision, GrowthDecision::Available);
    assert!(completion.admit().is_ok());
    assert_eq!(support::state(&db), before);
}
