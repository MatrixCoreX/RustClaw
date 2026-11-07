use super::boundary_test_fixtures as setup;
use super::*;

#[path = "ux_budget_test_fixtures.rs"]
mod support;

fn unknown(db: &Connection) {
    db.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE memory_storage_pressure SET state='future_state';").unwrap();
}

#[test]
fn missing_state_preview_does_not_store_private_payload() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let request = support::request(&db, &fixture.principal);
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    let before = support::state(&db);
    let error = preview_memory_import(&db, &fixture.principal, &request, 103).unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_state_missing");
    assert_eq!(support::state(&db), before);
}

#[test]
fn unknown_state_confirmation_retains_preview_for_explicit_retry() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let request = support::request(&db, &fixture.principal);
    let preview = preview_memory_import(&db, &fixture.principal, &request, 101).unwrap();
    let confirmation = setup::confirmation(&preview);
    unknown(&db);
    let before = support::state(&db);
    let error = confirm_memory_import(&db, &fixture.principal, &confirmation, 103).unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_state_unknown");
    assert_eq!(support::state(&db), before);
    crate::memory::retention::initialize_storage_pressure(&db, &fixture.state().policy.memory, 104)
        .unwrap();
    assert_eq!(
        confirm_memory_import(&db, &fixture.principal, &confirmation, 105)
            .unwrap()
            .imported_items,
        1
    );
}

#[test]
fn incorrect_feedback_rolls_back_its_record_when_state_is_unknown() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    unknown(&db);
    let before = support::state(&db);
    let error = record_feedback(
        &db,
        &fixture.principal,
        "witness",
        &MemoryFeedbackRequest {
            expected_revision: 1,
            feedback_kind: MemoryFeedbackKind::Incorrect,
            retrieval_event_ref: None,
            corrected_content: Some("An explicit correction".into()),
        },
        103,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_state_unknown");
    assert_eq!(support::state(&db), before);
}

#[test]
fn unknown_state_deleted_content_restore_keeps_undo_receipt() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let undo = setup::delete(&db, &fixture.principal, "witness");
    unknown(&db);
    let before = support::state(&db);
    let error = undo_memory_mutation(&db, &fixture.principal, &undo, 103).unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_state_unknown");
    assert_eq!(support::state(&db), before);
}

#[test]
fn unknown_state_preference_undo_to_smaller_content_still_succeeds() {
    let fixture = support::fixture(false);
    support::preference(&fixture, "small");
    let undo = support::correct_preference(&fixture, &"larger content ".repeat(1024));
    let db = fixture.state().core.db.get().unwrap();
    unknown(&db);
    assert_eq!(
        undo_memory_mutation(&db, &fixture.principal, &undo, 103)
            .unwrap()
            .status,
        "restored"
    );
    assert_eq!(
        db.query_row(
            "SELECT pref_value FROM user_preferences WHERE memory_id='budget-pref'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "small"
    );
}

#[test]
fn unknown_state_fact_undo_releases_replacement_without_losing_history() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let correction = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "witness",
        &support::large_correction(),
        101,
    )
    .unwrap();
    let undo = MemoryUndoRequest {
        revision_id: correction.revision_id.unwrap(),
    };
    unknown(&db);
    assert_eq!(
        undo_memory_mutation(&db, &fixture.principal, &undo, 103)
            .unwrap()
            .status,
        "restored"
    );
    assert_eq!(
        db.query_row(
            "SELECT fact_text FROM memory_facts WHERE memory_id='witness' AND status='active'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        support::ORIGINAL
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_facts WHERE memory_id=?1",
            [correction.replacement_memory_id.unwrap()],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row(
        "SELECT json_extract(previous_snapshot_json,'$.content') FROM memory_revisions WHERE revision_id=?1",
        [undo.revision_id], |row|row.get::<_,String>(0)).unwrap(), support::ORIGINAL);
}

#[test]
fn suppression_feedback_still_works_with_unknown_capacity() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    unknown(&db);
    assert_eq!(
        record_feedback(
            &db,
            &fixture.principal,
            "witness",
            &MemoryFeedbackRequest {
                expected_revision: 1,
                feedback_kind: MemoryFeedbackKind::DoNotUse,
                retrieval_event_ref: None,
                corrected_content: None
            },
            103
        )
        .unwrap()
        .status,
        "do_not_use"
    );
    assert_eq!(db.query_row(
        "SELECT COUNT(*) FROM memory_retrieval_feedback WHERE principal_id=?1 AND memory_id='witness'",
        [&fixture.principal], |row|row.get::<_,i64>(0)).unwrap(), 1);
}

#[test]
fn late_state_corruption_rolls_back_only_the_nested_correction() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch(
        "PRAGMA ignore_check_constraints=ON;
        CREATE TEMP TRIGGER invalidate_capacity AFTER INSERT ON memory_revisions
        BEGIN UPDATE memory_storage_pressure SET state='future_state'; END;
        BEGIN IMMEDIATE;
        UPDATE memory_storage_pressure SET reason_code='parent_marker';",
    )
    .unwrap();
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
    assert!(!db.is_autocommit());
    assert_eq!(support::state(&db), before);
    db.execute_batch("ROLLBACK").unwrap();
}
