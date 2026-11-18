use super::support::*;
use super::*;

#[test]
fn fact_correction_commits_feedback_replacement_and_reversible_revision_together() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    let result = record_feedback(
        &db,
        &fixture.principal,
        MEMORY_ID,
        &request(MemoryFeedbackKind::Incorrect, Some("  Corrected fact  ")),
        200,
    )
    .unwrap();
    assert_eq!(result.status, "corrected");
    assert_eq!(result.revision, 1);
    assert_eq!(result.undo_until_ts, Some(500));
    assert_feedback(&db, &fixture.principal, MEMORY_ID, "incorrect");
    let replacement = result.replacement_memory_id.as_deref().unwrap();
    let state = db
        .query_row(
            "SELECT fact_text,status,row_revision,supersedes_memory_id
             FROM memory_facts WHERE memory_id=?1 AND principal_id=?2",
            params![replacement, fixture.principal],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        state,
        (
            "Corrected fact".into(),
            "active".into(),
            1,
            MEMORY_ID.into()
        )
    );
    let restored = undo_memory_mutation(
        &db,
        &fixture.principal,
        &MemoryUndoRequest {
            revision_id: result.revision_id.unwrap(),
        },
        201,
    )
    .unwrap();
    assert_eq!(restored.status, "restored");
    assert_eq!(
        db.query_row(
            "SELECT fact_text FROM memory_facts WHERE memory_id=?1",
            [MEMORY_ID],
            |row| { row.get::<_, String>(0) }
        )
        .unwrap(),
        "The original private fact"
    );
    assert_feedback(&db, &fixture.principal, MEMORY_ID, "incorrect");
}

#[test]
fn preference_correction_preserves_feedback_and_existing_revision_contract() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::preference(&db, &fixture.key, &fixture.principal, "feedback-preference");
    let result = record_feedback(
        &db,
        &fixture.principal,
        "feedback-preference",
        &request(MemoryFeedbackKind::Incorrect, Some("  Plain text  ")),
        200,
    )
    .unwrap();
    assert_eq!(result.status, "corrected");
    assert_eq!(result.revision, 1);
    let replacement = result.replacement_memory_id.as_deref().unwrap();
    let row = db
        .query_row(
            "SELECT pref_value,row_revision,supersedes_memory_id FROM user_preferences
             WHERE memory_id=?1 AND principal_id=?2",
            params![replacement, fixture.principal],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row, ("Plain text".into(), 2, "feedback-preference".into()));
    assert!(result.revision_id.is_some());
    assert_feedback(&db, &fixture.principal, "feedback-preference", "incorrect");
}

#[test]
fn irrelevant_feedback_records_each_event_without_mutating_content() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    let mut before = private_state(&db);
    before.retain(|(name, _)| *name != "memory_retrieval_feedback");
    for _ in 0..2 {
        let result = record_feedback(
            &db,
            &fixture.principal,
            MEMORY_ID,
            &request(
                MemoryFeedbackKind::Irrelevant,
                Some("Ignored for this kind"),
            ),
            200,
        )
        .unwrap();
        assert_eq!(result.status, "irrelevant");
        assert_eq!(result.revision, 1);
        assert!(result.replacement_memory_id.is_none());
        assert!(result.revision_id.is_none());
        assert!(result.undo_until_ts.is_none());
    }
    let mut after = private_state(&db);
    after.retain(|(name, _)| *name != "memory_retrieval_feedback");
    assert_eq!(after, before);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_retrieval_feedback",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert!(db.is_autocommit());
}

#[test]
fn do_not_use_commits_feedback_and_existing_recall_disable_result() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    let result = record_feedback(
        &db,
        &fixture.principal,
        MEMORY_ID,
        &request(MemoryFeedbackKind::DoNotUse, Some("Not a correction")),
        200,
    )
    .unwrap();
    assert_eq!(result.status, "do_not_use");
    assert_eq!(result.revision, 1);
    assert!(result.revision_id.is_none());
    assert!(result.replacement_memory_id.is_none());
    assert_feedback(&db, &fixture.principal, MEMORY_ID, "do_not_use");
    let row = db
        .query_row(
            "SELECT status,row_revision FROM memory_facts WHERE memory_id=?1",
            [MEMORY_ID],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("deleted".into(), 2));
}

#[test]
fn stale_revision_is_rejected_without_feedback_or_mutation() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    let mut request = request(MemoryFeedbackKind::Incorrect, Some("Stale replacement"));
    request.expected_revision = 0;
    reject_without_change(
        &db,
        &fixture.principal,
        MEMORY_ID,
        &request,
        "memory_revision_conflict",
    );
}

#[test]
fn another_principals_memory_cannot_receive_feedback() {
    let fixture = fixture();
    let (_, other) = setup::another_owner(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    reject_without_change(
        &db,
        &other,
        MEMORY_ID,
        &request(MemoryFeedbackKind::Incorrect, Some("Foreign replacement")),
        "memory_not_found",
    );
}

#[test]
fn nonexistent_memory_cannot_receive_feedback() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    reject_without_change(
        &db,
        &fixture.principal,
        "missing-feedback-memory",
        &request(MemoryFeedbackKind::Irrelevant, None),
        "memory_not_found",
    );
}

#[test]
fn read_only_connection_cannot_partially_record_feedback() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    db.pragma_update(None, "query_only", "ON").unwrap();
    reject_without_change(
        &db,
        &fixture.principal,
        MEMORY_ID,
        &request(MemoryFeedbackKind::Incorrect, Some("Read only replacement")),
        "attempt to write a readonly database",
    );
}
