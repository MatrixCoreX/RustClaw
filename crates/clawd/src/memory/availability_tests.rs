use super::*;
use crate::memory::capacity_test_fixtures::{pressure, Fixture};

#[path = "availability_test_fixtures.rs"]
mod fixtures;
use fixtures::{measured, observe, recent};

#[test]
fn capacity_observation_is_query_only_and_does_not_repair_missing_rows() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    db.execute(
        "DELETE FROM memory_principal_quotas WHERE principal_id=?1",
        [&fixture.principal],
    )
    .unwrap();
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    let before = db.total_changes();
    db.pragma_update(None, "query_only", true).unwrap();
    let report = observe(&fixture, &db, false);
    assert_eq!(
        report.capacity_decision,
        GenerationAvailability::StorageStateMissing
    );
    assert_eq!(
        report.quota.decision,
        GenerationAvailability::PrincipalQuotaMissing
    );
    assert!(report.quota.counters.is_none());
    assert_eq!(report.pressure.observed_at_ts, None);
    assert_eq!(db.total_changes(), before);
    db.pragma_update(None, "query_only", false).unwrap();
}

#[test]
fn source_payload_counts_utf8_and_nul_without_other_principals_or_derived_data() {
    let fixture = Fixture::memory();
    let key = crate::repo::auth::create_auth_key(fixture.state(), "user").unwrap();
    let other = crate::resolve_auth_identity_by_key(fixture.state(), &key)
        .unwrap()
        .unwrap();
    let db = fixture.state().core.db.get().unwrap();
    recent(&db, &fixture, "own", "中\0文🐼");
    fixtures::derived_fact(&db, &fixture);
    // Distinct owner rows in the same database, not a second isolated database.
    fixtures::recent_as(
        &db,
        &key,
        &other.principal_id,
        "other",
        &"foreign".repeat(100),
    );
    let report = observe(&fixture, &db, false);
    assert_eq!(
        measured(&report.measurements.own_source_transcript_payload_bytes),
        "中\0文🐼".len() as u64
    );
    assert_eq!(
        report.quota.scope,
        "source_transcript_content_rows_and_bytes"
    );
    assert_eq!(
        report.quota.counters.unwrap().used_bytes,
        0,
        "a read cannot rewrite recorded quota usage to the live payload count"
    );
}

#[test]
fn ordinary_observation_redacts_all_global_metrics_and_database_path() {
    let fixture = Fixture::file();
    let db = fixture.state().core.db.get().unwrap();
    let settings =
        crate::memory::settings::resolve_principal_memory_settings(&db, &fixture.principal, true)
            .unwrap();
    let (report, path) = observe_sqlite(&db, settings, false, 123, 10).unwrap();
    assert!(path.is_none());
    let json = serde_json::to_value(report).unwrap();
    for field in [
        "main_sqlite_allocated_page_bytes",
        "memory_domain_assigned_btree_bytes",
        "sqlite_freelist_page_bytes",
        "database_path_filesystem_available_bytes",
    ] {
        assert_eq!(
            json["measurements"][field],
            serde_json::json!({"status":"admin_only"})
        );
    }
    assert_eq!(json["pressure"]["observed_bytes"]["status"], "admin_only");
    assert_eq!(
        json["pressure"]["configured_soft_limit_bytes"]["status"],
        "admin_only"
    );
    let text = json.to_string();
    assert!(!text.contains("fixture.sqlite"));
    assert!(!text.contains(&fixture.key));
}

#[test]
fn whole_sqlite_growth_is_not_reported_as_memory_domain_growth() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    let before = observe(&fixture, &db, true);
    db.execute_batch("CREATE TABLE unrelated_history(payload BLOB)")
        .unwrap();
    db.execute("INSERT INTO unrelated_history VALUES(zeroblob(262144))", [])
        .unwrap();
    let after = observe(&fixture, &db, true);
    assert!(
        measured(&after.measurements.main_sqlite_allocated_page_bytes)
            > measured(&before.measurements.main_sqlite_allocated_page_bytes)
    );
    assert_eq!(
        after.measurements.memory_domain_assigned_btree_bytes,
        before.measurements.memory_domain_assigned_btree_bytes
    );
    assert_eq!(
        after.measurements.own_source_transcript_payload_bytes,
        before.measurements.own_source_transcript_payload_bytes
    );
    assert_eq!(
        after.pressure.observed_bytes, before.pressure.observed_bytes,
        "a diagnostic read cannot refresh the producer observation"
    );
}

#[test]
fn btree_measurement_includes_memory_indexes_but_not_freelist_or_host_space() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    let before = observe(&fixture, &db, true);
    recent(&db, &fixture, "large", &"data".repeat(16384));
    let after = observe(&fixture, &db, true);
    assert!(
        measured(&after.measurements.memory_domain_assigned_btree_bytes)
            > measured(&before.measurements.memory_domain_assigned_btree_bytes)
    );
    assert!(
        measured(&after.measurements.main_sqlite_allocated_page_bytes)
            >= measured(&after.measurements.memory_domain_assigned_btree_bytes)
    );
    assert_eq!(
        after.measurements.database_path_filesystem_available_bytes,
        ByteMeasurement::unavailable(MeasurementUnavailable::NotSampled)
    );
    assert_eq!(
        after.pressure.measurement_scope,
        "main_sqlite_allocated_pages_not_memory_domain_or_host_free_space"
    );
}

#[test]
fn requested_generation_is_distinct_from_current_capacity_pause() {
    let mut fixture = Fixture::memory();
    fixture.state_mut().policy.memory.long_term_enabled = true;
    let identity = crate::resolve_auth_identity_by_key(fixture.state(), &fixture.key)
        .unwrap()
        .unwrap();
    let db = fixture.state().core.db.get().unwrap();
    let request = serde_json::from_value(serde_json::json!({
        "scope":"principal", "expected_revision":0, "generate_mode":"enabled"
    }))
    .unwrap();
    crate::memory::settings::update_memory_settings(&db, &identity, &request, true).unwrap();
    pressure(&db, "automatic_generation_paused");
    let report = observe(&fixture, &db, false);
    assert!(report.settings.generate_memory);
    assert_eq!(
        report.capacity_decision,
        GenerationAvailability::StoragePaused
    );
    assert_eq!(report.quota.decision, GenerationAvailability::Available);
}

#[test]
fn capacity_view_keeps_unknown_pressure_and_invalid_quota_distinct_without_writes() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE memory_storage_pressure SET state='future_state'; UPDATE memory_principal_quotas SET max_bytes='broken'").unwrap();
    let before = db.total_changes();
    let report = observe(&fixture, &db, false);
    assert_eq!(report.pressure.state, StoragePressure::Unknown);
    assert_eq!(
        report.capacity_decision,
        GenerationAvailability::StorageStateUnknown
    );
    assert_eq!(
        report.quota.decision,
        GenerationAvailability::PrincipalQuotaInvalid
    );
    assert!(report.quota.counters.is_none());
    assert_eq!(db.total_changes(), before);
}

#[test]
fn recorded_quota_cost_includes_pending_logical_work_not_provider_billing() {
    let fixture = Fixture::memory();
    fixture.seed_job("pending", "queued");
    let db = fixture.state().core.db.get().unwrap();
    db.execute(
        "UPDATE memory_jobs SET background_cost_pending_microunits=11",
        [],
    )
    .unwrap();
    db.execute("UPDATE memory_principal_quotas SET used_background_cost_microunits=7,max_background_cost_microunits=18", []).unwrap();
    let report = observe(&fixture, &db, false);
    assert_eq!(
        report
            .quota
            .counters
            .unwrap()
            .spent_and_pending_logical_microunits,
        18
    );
    assert_eq!(
        report.quota.decision,
        GenerationAvailability::PrincipalBackgroundCostExhausted
    );
}

#[test]
fn zero_or_corrupt_pressure_metadata_is_not_an_observed_timestamp_or_zero_bytes() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE memory_storage_pressure SET observed_bytes='invalid',revision=-1,updated_at_ts=0").unwrap();
    let report = observe(&fixture, &db, true);
    assert_eq!(report.pressure.observed_at_ts, None);
    assert_eq!(report.pressure.revision, None);
    assert_eq!(
        report.pressure.observed_bytes,
        ByteMeasurement::unavailable(MeasurementUnavailable::InvalidValue)
    );
}

#[test]
fn observation_succeeds_during_an_independent_writer_reservation() {
    let fixture = Fixture::file();
    let writer = fixture.competing_writer();
    let db = fixture.state().core.db.get().unwrap();
    let before = db.total_changes();
    let report = observe(&fixture, &db, true);
    assert_eq!(report.capacity_decision, GenerationAvailability::Available);
    assert_eq!(db.total_changes(), before);
    writer.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn filesystem_measurement_uses_the_actual_file_and_has_no_root_fallback() {
    let fixture = Fixture::file();
    let db = fixture.state().core.db.get().unwrap();
    let path = measurements::database_path(&db).unwrap();
    assert!(measured(&filesystem_available(Some(&path))) > 0);
    assert_eq!(
        filesystem_available(Some("")),
        ByteMeasurement::unavailable(MeasurementUnavailable::InMemoryDatabase)
    );
    assert_eq!(
        filesystem_available(None),
        ByteMeasurement::unavailable(MeasurementUnavailable::QueryFailed)
    );
    let missing = format!("{path}/not-a-directory");
    assert_eq!(
        filesystem_available(Some(&missing)),
        ByteMeasurement::unavailable(MeasurementUnavailable::FilesystemUnavailable)
    );
}

#[test]
fn missing_migration_marker_is_not_repaired_by_capacity_observation() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    let settings =
        crate::memory::settings::resolve_principal_memory_settings(&db, &fixture.principal, true)
            .unwrap();
    db.execute("DELETE FROM runtime_schema_migrations", [])
        .unwrap();
    let before = db.total_changes();
    assert!(observe_sqlite(&db, settings, false, 1, 1).is_err());
    assert_eq!(db.total_changes(), before);
}

#[test]
fn capacity_json_keeps_full_integer_precision_without_a_new_limit() {
    assert_eq!(
        serde_json::to_value(ByteMeasurement::measured(u64::MAX)).unwrap()["bytes"],
        u64::MAX.to_string()
    );
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    db.execute(
        "UPDATE memory_principal_quotas SET max_bytes=?1",
        [i64::MAX],
    )
    .unwrap();
    let json = serde_json::to_value(observe(&fixture, &db, false)).unwrap();
    assert_eq!(json["quota"]["counters"]["max_bytes"], i64::MAX.to_string());
}
