use super::super::*;
use super::fixtures::*;

#[test]
fn fresh_identity_after_initialized_empty_startup_receives_defaults_atomically() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let (_, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    assert_eq!(quota(&db, &owner).0, state.policy.memory.max_rows as i64);
    assert_eq!(count(&db, "memory_quota_initializations"), 1);
    let kind: String = db
        .query_row(
            "SELECT initialization_kind FROM memory_quota_initializations WHERE principal_id=?1",
            [owner],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kind, "new_identity");
    let settings = crate::memory::settings::resolve_principal_memory_settings(
        &db,
        &owner,
        state.policy.memory.long_term_enabled,
    )
    .unwrap();
    assert!(
        !settings.generate_memory,
        "default quota is not consent to generate memory"
    );
}

#[test]
fn new_identity_defaults_failure_rolls_back_key_binding_and_principal() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().unwrap();
    let tables = [
        "auth_keys",
        "principals",
        "credential_bindings",
        "memory_principal_quotas",
        "memory_quota_initializations",
    ];
    let before = tables.map(|table| count(&db, table));
    db.execute_batch("CREATE TRIGGER reject_new_quota BEFORE INSERT ON memory_principal_quotas BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    drop(db);
    assert!(crate::repo::auth::create_auth_key(&state, "user").is_err());
    let db = state.core.db.get().unwrap();
    assert_eq!(tables.map(|table| count(&db, table)), before);
}

#[test]
fn identity_creation_before_memory_contract_waits_for_explicit_startup() {
    let state = legacy();
    let (_, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    assert_eq!(count(&db, "memory_principal_quotas"), 0);
    initialize_startup_defaults(&db, &state.policy.memory, 100).unwrap();
    assert_eq!(quota(&db, &owner).3, 0);
    assert_eq!(count(&db, "memory_quota_initializations"), 1);
}

#[test]
fn repeated_identity_initialization_does_not_repair_its_lost_policy() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let (_, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    db.execute(
        "DELETE FROM memory_principal_quotas WHERE principal_id=?1",
        [&owner],
    )
    .unwrap();
    let before = db.total_changes();
    let tx = db.unchecked_transaction().unwrap();
    initialize_new_identity(&tx, &state.policy.memory, &owner, 200).unwrap();
    tx.commit().unwrap();
    assert_eq!(count(&db, "memory_principal_quotas"), 0);
    assert_eq!(db.total_changes(), before);
}

#[test]
fn administrator_rotation_preserves_owner_policy_and_does_not_replace_lost_policy() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let (_, owner) = identity(&state, "admin");
    let db = state.core.db.get().unwrap();
    db.execute("UPDATE memory_principal_quotas SET max_bytes=123,used_background_cost_microunits=7,revision=9",[]).unwrap();
    let before = quota(&db, &owner);
    drop(db);
    let (_, rotated) = identity(&state, "admin");
    assert_eq!(rotated, owner);
    let db = state.core.db.get().unwrap();
    assert_eq!(quota(&db, &owner), before);
    db.execute(
        "DELETE FROM memory_principal_quotas WHERE principal_id=?1",
        [&owner],
    )
    .unwrap();
    drop(db);
    let (_, rotated) = identity(&state, "admin");
    assert_eq!(rotated, owner);
    assert_eq!(
        count(&state.core.db.get().unwrap(), "memory_principal_quotas"),
        0
    );
}

#[test]
fn new_accounts_use_current_defaults_without_rewriting_other_accounts() {
    let mut state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let (_, first) = identity(&state, "user");
    let before = quota(&state.core.db.get().unwrap(), &first);
    state.policy.memory.max_rows = 17;
    state.policy.memory.principal_max_bytes = 123;
    let (_, second) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    assert_eq!(quota(&db, &first), before);
    assert_eq!((quota(&db, &second).0, quota(&db, &second).1), (17, 123));
}

#[test]
fn new_identity_initialization_rejects_a_call_outside_its_creation_transaction() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().unwrap();
    let before = db.total_changes();
    assert_eq!(
        initialize_new_identity(&db, &state.policy.memory, "not-created", 200)
            .unwrap_err()
            .to_string(),
        "memory_quota_initialization_requires_identity_transaction"
    );
    assert_eq!(db.total_changes(), before);
}

#[test]
fn corrupt_or_missing_initialization_digest_does_not_create_an_authentication_identity() {
    for sql in [
        "UPDATE runtime_schema_migrations SET schema_digest='changed' WHERE migration_id=?1",
        "DELETE FROM runtime_schema_migrations WHERE migration_id=?1",
    ] {
        let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let db = state.core.db.get().unwrap();
        let before = count(&db, "auth_keys");
        db.execute(sql, [MIGRATION_ID]).unwrap();
        drop(db);
        assert!(crate::repo::auth::create_auth_key(&state, "user").is_err());
        assert_eq!(count(&state.core.db.get().unwrap(), "auth_keys"), before);
    }
}

#[test]
fn identity_initializer_refuses_nonexistent_and_inactive_owners_without_writes() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let (_, owner) = identity(&state, "user");
    let db = state.core.db.get().unwrap();
    db.execute(
        "UPDATE principals SET status='frozen' WHERE principal_id=?1",
        [&owner],
    )
    .unwrap();
    for id in [owner.as_str(), "nonexistent"] {
        let before = db.total_changes();
        let tx = db.unchecked_transaction().unwrap();
        assert_eq!(
            initialize_new_identity(&tx, &state.policy.memory, id, 200)
                .unwrap_err()
                .to_string(),
            "memory_quota_owner_inactive"
        );
        tx.rollback().unwrap();
        assert_eq!(db.total_changes(), before);
    }
}

#[test]
#[cfg(target_pointer_width = "64")]
fn unrepresentable_row_default_does_not_create_negative_policy_or_identity() {
    let mut state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.policy.memory.max_rows = usize::MAX;
    assert!(crate::repo::auth::create_auth_key(&state, "user").is_err());
    let db = state.core.db.get().unwrap();
    assert_eq!(count(&db, "auth_keys"), 0);
    assert_eq!(count(&db, "memory_principal_quotas"), 0);
    assert_eq!(count(&db, "memory_quota_initializations"), 0);
}
