use super::boundary_test_fixtures as setup;
use super::*;

#[path = "ux_budget_test_fixtures.rs"]
mod support;

fn reject_correction(db: &Connection, fixture: &support::Fixture, expected: &str) {
    let before = support::state(db);
    let error = correct_memory(
        db,
        &fixture.principal,
        &fixture.principal,
        "witness",
        &support::large_correction(),
        103,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), expected);
    assert_eq!(support::state(db), before);
}

#[test]
fn explicit_configuration_roundtrips_u64_without_another_default() {
    let mut fixture = support::fixture(false);
    support::limit(&mut fixture, u64::MAX);
    let db = fixture.state().core.db.get().unwrap();
    let budget = crate::memory::storage_budget::Snapshot::load(&db).unwrap();
    assert_eq!(budget.limit, u64::MAX);
    assert_eq!(
        budget.pressure_revision,
        db.query_row(
            "SELECT revision FROM memory_storage_pressure WHERE singleton_id=1",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap()
    );
    let before = support::state(&db);
    let changed = db.total_changes();
    db.pragma_update(None, "query_only", true).unwrap();
    assert_eq!(
        crate::memory::storage_budget::Snapshot::load(&db).unwrap(),
        budget
    );
    assert_eq!(support::state(&db), before);
    assert_eq!(db.total_changes(), changed);
}

#[test]
fn missing_budget_row_rejects_growth_without_recreating_it() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute("DELETE FROM memory_storage_budget", []).unwrap();
    reject_correction(&db, &fixture, "memory_storage_policy_missing");
}

#[test]
fn invalid_budget_rejects_growth_without_normalizing_it() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute(
        "UPDATE memory_storage_budget SET configured_limit_bytes='not-a-budget'",
        [],
    )
    .unwrap();
    reject_correction(&db, &fixture, "memory_storage_policy_invalid");
}

#[test]
fn missing_marker_cannot_adopt_an_owned_budget_on_restart() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute(
        "DELETE FROM runtime_schema_migrations WHERE migration_id='031_memory_storage_budget_v1'",
        [],
    )
    .unwrap();
    let before = support::state(&db);
    let error = crate::memory::retention::initialize_storage_pressure(
        &db,
        &fixture.state().policy.memory,
        103,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_policy_marker_missing");
    assert_eq!(support::state(&db), before);
    assert_eq!(db.query_row(
        "SELECT COUNT(*) FROM runtime_schema_migrations WHERE migration_id='031_memory_storage_budget_v1'",
        [], |row|row.get::<_,i64>(0)).unwrap(), 0);
}

#[test]
fn restart_updates_only_configuration_projection_and_pressure() {
    let mut fixture = support::fixture(false);
    let before = {
        let db = fixture.state().core.db.get().unwrap();
        support::state(&db)
    };
    support::limit(&mut fixture, 9_000_000);
    let db = fixture.state().core.db.get().unwrap();
    let mut after = support::state(&db);
    let mut preserved = before;
    for table in ["memory_storage_budget", "memory_storage_pressure"] {
        after.remove(table);
        preserved.remove(table);
    }
    assert_eq!(after, preserved);
    assert_eq!(
        crate::memory::storage_budget::Snapshot::load(&db)
            .unwrap()
            .limit,
        9_000_000
    );
}

#[test]
fn crossing_actual_domain_budget_rolls_back_every_private_effect() {
    let mut fixture = support::fixture(false);
    let budget = {
        let db = fixture.state().core.db.get().unwrap();
        let page: u64 = db
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .unwrap();
        support::domain(&db, &fixture) + page
    };
    support::limit(&mut fixture, budget);
    let db = fixture.state().core.db.get().unwrap();
    assert!(support::domain(&db, &fixture) < budget);
    reject_correction(&db, &fixture, "memory_storage_write_blocked");
    assert!(db.is_autocommit());
}

#[test]
fn late_budget_corruption_rolls_back_only_nested_work() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "CREATE TEMP TRIGGER invalidate_budget AFTER INSERT ON memory_revisions
        BEGIN UPDATE memory_storage_budget SET configured_limit_bytes='broken'; END;
        BEGIN IMMEDIATE; UPDATE memory_storage_pressure SET reason_code='parent_marker';",
    )
    .unwrap();
    reject_correction(&db, &fixture, "memory_storage_policy_invalid");
    assert!(!db.is_autocommit());
    db.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn missing_budget_allows_export_duplicate_preview_and_deletion() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute("DELETE FROM memory_storage_budget", []).unwrap();
    let before = support::state(&db);
    db.pragma_update(None, "query_only", true).unwrap();
    let export = export_memory(&db, &fixture.principal, 103).unwrap();
    assert_eq!(export.items.len(), 1);
    assert_eq!(support::state(&db), before);
    db.pragma_update(None, "query_only", false).unwrap();
    let preview = preview_memory_import(
        &db,
        &fixture.principal,
        &MemoryImportPreviewRequest { export },
        103,
    )
    .unwrap();
    assert_eq!((preview.accepted_items, preview.duplicate_items), (0, 1));
    assert_eq!(
        support::without_sessions(support::state(&db)),
        support::without_sessions(before)
    );
    let deleted = delete_memory_with_revision(
        &db,
        &fixture.principal,
        "witness",
        &MemoryMutationRequest {
            expected_revision: 1,
        },
        104,
    )
    .unwrap();
    assert_eq!(deleted.status, "deleted");
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM memory_storage_budget", [], |row| row
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}

#[test]
fn source_quota_exhaustion_does_not_charge_canonical_facts() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute(
        "UPDATE memory_principal_quotas SET max_rows=0,max_bytes=0 WHERE principal_id=?1",
        [&fixture.principal],
    )
    .unwrap();
    let before: (i64,i64,i64) = db.query_row(
        "SELECT used_rows,used_bytes,used_background_cost_microunits FROM memory_principal_quotas WHERE principal_id=?1",
        [&fixture.principal], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
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
    assert_eq!(db.query_row(
        "SELECT used_rows,used_bytes,used_background_cost_microunits FROM memory_principal_quotas WHERE principal_id=?1",
        [&fixture.principal], |row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?))).unwrap(), before);
}

#[test]
fn source_undo_uses_utf8_bytes_and_keeps_receipt_until_room_exists() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let content = "中\0a";
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "source-recent",
        content,
    );
    let undo = setup::delete(&db, &fixture.principal, "source-recent");
    db.execute("UPDATE memory_principal_quotas SET max_rows=1,max_bytes=4,max_background_cost_microunits=0,used_background_cost_microunits='billing-unavailable' WHERE principal_id=?1",
        [&fixture.principal]).unwrap();
    let before = support::state(&db);
    let error = undo_memory_mutation(&db, &fixture.principal, &undo, 103).unwrap_err();
    assert_eq!(error.to_string(), "memory_principal_bytes_exhausted");
    assert_eq!(support::state(&db), before);
    db.execute(
        "UPDATE memory_principal_quotas SET max_bytes=5 WHERE principal_id=?1",
        [&fixture.principal],
    )
    .unwrap();
    assert_eq!(
        undo_memory_mutation(&db, &fixture.principal, &undo, 104)
            .unwrap()
            .status,
        "restored"
    );
    assert_eq!(
        db.query_row(
            "SELECT used_rows,used_bytes FROM memory_principal_quotas WHERE principal_id=?1",
            [&fixture.principal],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        )
        .unwrap(),
        (1, 5)
    );
    assert_eq!(db.query_row(
        "SELECT used_background_cost_microunits FROM memory_principal_quotas WHERE principal_id=?1",
        [&fixture.principal], |row|row.get::<_,String>(0)).unwrap(), "billing-unavailable");
}

#[test]
fn source_undo_rechecks_actual_rows_not_stale_usage_counters() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "source-recent",
        "first",
    );
    let undo = setup::delete(&db, &fixture.principal, "source-recent");
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "competing-source",
        "second",
    );
    db.execute("UPDATE memory_principal_quotas SET max_rows=1,used_rows=0,used_bytes=0 WHERE principal_id=?1",
        [&fixture.principal]).unwrap();
    let before = support::state(&db);
    let error = undo_memory_mutation(&db, &fixture.principal, &undo, 103).unwrap_err();
    assert_eq!(error.to_string(), "memory_principal_rows_exhausted");
    assert_eq!(support::state(&db), before);
}

#[test]
fn failed_refresh_rolls_back_pressure_and_budget_together() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "CREATE TEMP TRIGGER reject_budget_refresh BEFORE UPDATE ON memory_storage_budget
        BEGIN SELECT RAISE(ABORT,'injected_budget_refresh_failure'); END;",
    )
    .unwrap();
    let before = support::state(&db);
    let error = crate::memory::retention::initialize_storage_pressure(
        &db,
        &fixture.state().policy.memory,
        103,
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("injected_budget_refresh_failure"));
    assert_eq!(support::state(&db), before);
    assert!(db.is_autocommit());
}
