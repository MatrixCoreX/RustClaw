use super::*;

#[path = "quota_test_fixtures.rs"]
mod fixtures;
#[path = "quota_identity_tests.rs"]
mod identity;

use fixtures::*;

#[test]
fn legacy_startup_seeds_owned_accounts_with_exact_utf8_nul_source_usage() {
    let state = legacy();
    let (key, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    source(&db, &key, &owner, "中\0文🐼");
    assert_eq!(count(&db, "memory_principal_quotas"), 0);
    initialize_startup_defaults(&db, &state.policy.memory, 100).unwrap();
    let row = quota(&db, &owner);
    assert_eq!((row.3, row.4), (1, "中\0文🐼".len() as i64));
    assert_eq!(row.0, state.policy.memory.max_rows as i64);
    assert_eq!(count(&db, "memory_quota_initializations"), 1);
}

#[test]
fn legacy_existing_policy_and_spent_cost_are_not_reset_by_the_migration() {
    let state = legacy();
    let (_, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    crate::memory::retention::ensure_principal_quota(&db, &state.policy.memory, &owner, 1).unwrap();
    db.execute("UPDATE memory_principal_quotas SET max_rows=17,max_bytes=123,max_background_cost_microunits=44,used_background_cost_microunits=9,revision=23,updated_at_ts=7",[]).unwrap();
    let before = quota(&db, &owner);
    initialize_startup_defaults(&db, &state.policy.memory, 100).unwrap();
    assert_eq!(quota(&db, &owner), before);
}

#[test]
fn legacy_invalid_policy_is_preserved_as_invalid_not_repaired_to_an_allow() {
    let state = legacy();
    let (_, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    crate::memory::retention::ensure_principal_quota(&db, &state.policy.memory, &owner, 1).unwrap();
    db.execute(
        "UPDATE memory_principal_quotas SET max_bytes='invalid',revision=41",
        [],
    )
    .unwrap();
    initialize_startup_defaults(&db, &state.policy.memory, 100).unwrap();
    let value: (String, i64) = db
        .query_row(
            "SELECT max_bytes,revision FROM memory_principal_quotas WHERE principal_id=?1",
            [&owner],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(value, ("invalid".into(), 41));
    assert_eq!(
        crate::memory::capacity::automatic_generation_availability(&db, &owner).unwrap(),
        crate::memory::capacity::GenerationAvailability::PrincipalQuotaInvalid
    );
}

#[test]
fn repeated_startup_does_not_recreate_an_initialized_accounts_lost_policy() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let (_, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    db.execute(
        "DELETE FROM memory_principal_quotas WHERE principal_id=?1",
        [&owner],
    )
    .unwrap();
    let before = db.total_changes();
    initialize_startup_defaults(&db, &state.policy.memory, 200).unwrap();
    assert_eq!(count(&db, "memory_principal_quotas"), 0);
    assert_eq!(db.total_changes(), before);
    assert_eq!(
        crate::memory::capacity::automatic_generation_availability(&db, &owner).unwrap(),
        crate::memory::capacity::GenerationAvailability::PrincipalQuotaMissing
    );
}

#[test]
fn legacy_receipts_prevent_reinitialization_even_if_global_checkpoint_is_lost() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let (_, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    db.execute(
        "DELETE FROM memory_principal_quotas WHERE principal_id=?1",
        [&owner],
    )
    .unwrap();
    db.execute(
        "DELETE FROM runtime_schema_migrations WHERE migration_id=?1",
        [MIGRATION_ID],
    )
    .unwrap();
    let before = db.total_changes();
    let error = initialize_startup_defaults(&db, &state.policy.memory, 200).unwrap_err();
    assert_eq!(
        error.to_string(),
        "memory_quota_initialization_marker_missing"
    );
    assert_eq!(db.total_changes(), before);
    assert_eq!(count(&db, "memory_principal_quotas"), 0);
}

#[test]
fn startup_fails_closed_when_its_receipt_table_is_missing_or_its_digest_changes() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().unwrap();
    db.execute_batch("DROP TABLE memory_quota_initializations")
        .unwrap();
    let before = db.total_changes();
    assert!(initialize_startup_defaults(&db, &state.policy.memory, 200).is_err());
    assert_eq!(db.total_changes(), before);
    db.execute(
        "UPDATE runtime_schema_migrations SET schema_digest='future' WHERE migration_id=?1",
        [MIGRATION_ID],
    )
    .unwrap();
    assert_eq!(
        initialize_startup_defaults(&db, &state.policy.memory, 200)
            .unwrap_err()
            .to_string(),
        "memory_quota_initialization_digest_mismatch"
    );
}

#[test]
fn failed_legacy_default_publication_rolls_back_all_new_policy_and_receipts() {
    let state = legacy();
    let _ = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    db.execute_batch("CREATE TRIGGER reject_quota BEFORE INSERT ON memory_principal_quotas BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    assert!(initialize_startup_defaults(&db, &state.policy.memory, 100).is_err());
    assert!(db.is_autocommit());
    assert!(applied(&db).unwrap().is_none());
    assert_eq!(count(&db, "memory_principal_quotas"), 0);
    let receipts: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name='memory_quota_initializations'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 0, "DDL and data share the failed transaction");
}

#[test]
fn legacy_unassigned_or_unknown_sources_do_not_create_unowned_policy() {
    let state = legacy();
    let (_, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    source(&db, "unknown", "unknown-principal", "not owned");
    initialize_startup_defaults(&db, &state.policy.memory, 100).unwrap();
    assert_eq!(count(&db, "memory_principal_quotas"), 1);
    assert_eq!(quota(&db, &owner).3, 0);
    assert_eq!(count(&db, "memories"), 1);
}

#[test]
fn configuration_changes_do_not_rewrite_existing_policy_on_restart() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let (_, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    let before = quota(&db, &owner);
    let mut changed = state.policy.memory.clone();
    changed.max_rows = 1;
    changed.principal_max_bytes = 1;
    changed.principal_background_cost_microunits = 1;
    initialize_startup_defaults(&db, &changed, 200).unwrap();
    assert_eq!(quota(&db, &owner), before);
}

#[test]
fn update_only_usage_keeps_policy_revision_spent_cost_and_pressure_unchanged() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let (key, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    db.execute("UPDATE memory_principal_quotas SET max_rows=17,max_bytes=123,max_background_cost_microunits=44,used_background_cost_microunits=9,revision=23",[]).unwrap();
    crate::memory::capacity_test_fixtures::pressure(&db, "explicit_write_blocked");
    source(&db, &key, &owner, "中\0文");
    crate::memory::retention::refresh_existing_principal_usage(&db, &owner, 111).unwrap();
    assert_eq!(
        quota(&db, &owner),
        (17, 123, 44, 1, "中\0文".len() as i64, 9, 23, 111)
    );
    assert_eq!(
        crate::memory::capacity::StoragePressure::load(&db).unwrap(),
        crate::memory::capacity::StoragePressure::ExplicitWriteBlocked
    );
}

#[test]
fn update_only_usage_is_write_free_for_a_missing_policy() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let (key, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    source(&db, &key, &owner, "source");
    db.execute(
        "DELETE FROM memory_principal_quotas WHERE principal_id=?1",
        [&owner],
    )
    .unwrap();
    let before = db.total_changes();
    crate::memory::retention::refresh_existing_principal_usage(&db, &owner, 111).unwrap();
    assert_eq!(db.total_changes(), before);
    assert_eq!(count(&db, "memory_principal_quotas"), 0);
}
