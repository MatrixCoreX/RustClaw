use super::*;

#[test]
fn actual_domain_budget_rejects_private_correction_growth() {
    let mut fixture = support::fixture(false);
    support::hard(&mut fixture);
    let db = fixture.state().core.db.get().unwrap();
    let before = support::state(&db);
    let error = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "witness",
        &support::large_correction(),
        103,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_write_blocked");
    assert_eq!(support::state(&db), before);
    assert!(db.is_autocommit());
}

#[test]
fn actual_domain_budget_rejects_new_private_preview_growth() {
    let mut fixture = support::fixture(false);
    let request = {
        let db = fixture.state().core.db.get().unwrap();
        support::request(&db, &fixture.principal)
    };
    support::hard(&mut fixture);
    let db = fixture.state().core.db.get().unwrap();
    let before = support::state(&db);
    let error = preview_memory_import(&db, &fixture.principal, &request, 103).unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_write_blocked");
    assert_eq!(support::state(&db), before);
}

#[test]
fn actual_domain_budget_rejects_confirmation_preserving_retryable_preview() {
    let mut fixture = support::fixture(false);
    let preview = {
        let db = fixture.state().core.db.get().unwrap();
        let request = support::request(&db, &fixture.principal);
        preview_memory_import(&db, &fixture.principal, &request, 101).unwrap()
    };
    let confirmation = setup::confirmation(&preview);
    let normal = fixture.state().policy.memory.storage_soft_limit_bytes;
    support::hard(&mut fixture);
    {
        let db = fixture.state().core.db.get().unwrap();
        let before = support::state(&db);
        let error = confirm_memory_import(&db, &fixture.principal, &confirmation, 103).unwrap_err();
        assert_eq!(error.to_string(), "memory_storage_write_blocked");
        assert_eq!(support::state(&db), before);
    }
    support::limit(&mut fixture, normal);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        confirm_memory_import(&db, &fixture.principal, &confirmation, 104)
            .unwrap()
            .imported_items,
        1
    );
}

#[test]
fn preference_undo_to_larger_original_is_growth_without_receipt_loss() {
    let mut fixture = support::fixture(false);
    support::preference(&fixture, &"中文\0memory ".repeat(4096));
    let undo = support::correct_preference(&fixture, "short preference");
    support::hard(&mut fixture);
    let db = fixture.state().core.db.get().unwrap();
    let before = support::state(&db);
    let error = undo_memory_mutation(&db, &fixture.principal, &undo, 103).unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_write_blocked");
    assert_eq!(support::state(&db), before);
    assert!(db.is_autocommit());
}

#[test]
fn unknown_storage_state_rejects_private_growth_without_read_repair() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE memory_storage_pressure SET state='future_state';").unwrap();
    let before = support::state(&db);
    let error = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "witness",
        &support::large_correction(),
        103,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_state_unknown");
    assert_eq!(support::state(&db), before);
}
