use super::capacity_test_fixtures::{scalar, Fixture};

#[test]
fn initialized_capacity_allows_automatic_generation_without_repair_writes() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    let before = scalar(&db, "SELECT total_changes()");
    assert!(super::retention::automatic_generation_allowed(&db, &fixture.principal).unwrap());
    assert_eq!(scalar(&db, "SELECT total_changes()"), before);
}

#[test]
fn missing_pressure_is_unavailable_without_recreating_the_singleton_in_a_read() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    let before = scalar(&db, "SELECT total_changes()");
    let allowed = super::retention::automatic_generation_allowed(&db, &fixture.principal)
        .expect("missing runtime evidence has a structured unavailable decision");
    assert!(
        !allowed,
        "missing pressure must not impersonate normal capacity"
    );
    assert_eq!(scalar(&db, "SELECT total_changes()"), before);
}

#[test]
fn unknown_persisted_pressure_is_unavailable_not_an_implicit_normal_state() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    // Fault injection affects only this disposable database.
    db.execute_batch(
        "PRAGMA ignore_check_constraints = ON;
        UPDATE memory_storage_pressure SET state = 'unknown_future_state';
        PRAGMA ignore_check_constraints = OFF;",
    )
    .unwrap();
    assert!(
        !super::retention::automatic_generation_allowed(&db, &fixture.principal)
            .expect("unknown state is unavailable, not an unstructured worker error")
    );
}

#[test]
fn missing_principal_quota_is_unavailable_without_a_read_path_policy_reset() {
    let fixture = Fixture::memory();
    let db = fixture.state().core.db.get().unwrap();
    db.execute(
        "DELETE FROM memory_principal_quotas WHERE principal_id = ?1",
        [&fixture.principal],
    )
    .unwrap();
    let before = scalar(&db, "SELECT total_changes()");
    assert!(
        !super::retention::automatic_generation_allowed(&db, &fixture.principal)
            .expect("missing quota is unavailable")
    );
    assert_eq!(scalar(&db, "SELECT total_changes()"), before);
}
