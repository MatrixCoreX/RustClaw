use super::*;

#[test]
fn fact_correction_replaces_index_and_fts_without_retaining_old_content() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "fact",
        "correct-fact",
    );
    let result = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "correct-fact",
        &correction(),
        200,
    )
    .unwrap();
    assert_replacement(
        &db,
        &fixture.principal,
        "fact",
        result.replacement_memory_id.as_deref().unwrap(),
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM memory_facts WHERE memory_id='correct-fact'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "superseded",
        "correction preserves original canonical history"
    );
}

#[test]
fn preference_correction_replaces_index_and_fts_and_preserves_other_owners() {
    let fixture = setup::fixture();
    let (other_key, other) = setup::another_owner(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "preference",
        "correct-pref",
    );
    seed(&db, &other_key, &other, "preference", "foreign-pref");
    let foreign = rows(&db, &other);
    let result = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "correct-pref",
        &correction(),
        200,
    )
    .unwrap();
    assert_eq!(
        rows(&db, &other),
        foreign,
        "same preference key must not cross owners"
    );
    assert_replacement(
        &db,
        &fixture.principal,
        "preference",
        result.replacement_memory_id.as_deref().unwrap(),
    );
}

#[test]
fn fact_correction_undo_restores_only_original_retrieval() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(&db, &fixture.key, &fixture.principal, "fact", "undo-fact");
    let before = rows(&db, &fixture.principal);
    let result = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "undo-fact",
        &correction(),
        200,
    )
    .unwrap();
    undo_memory_mutation(
        &db,
        &fixture.principal,
        &MemoryUndoRequest {
            revision_id: result.revision_id.unwrap(),
        },
        201,
    )
    .unwrap();
    assert_eq!(
        rows(&db, &fixture.principal),
        before,
        "undo must restore the exact original index and FTS content"
    );
    assert_no_orphan_fts(&db);
}

#[test]
fn failed_replacement_index_insert_rolls_back_canonical_revision_and_feedback() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "fact",
        "failed-index",
    );
    db.execute_batch(
        "CREATE TRIGGER fail_replacement_index BEFORE INSERT ON memory_retrieval_index
        BEGIN SELECT RAISE(ABORT,'replacement_index_fixture_failed'); END;",
    )
    .unwrap();
    let before = state(&db);
    let error = record_feedback(
        &db,
        &fixture.principal,
        "failed-index",
        &feedback(MemoryFeedbackKind::Incorrect),
        200,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("replacement_index_fixture_failed"),
        "{error}"
    );
    assert_eq!(
        state(&db),
        before,
        "index failure must roll back every canonical,revision,feedback,FTS and outbox effect"
    );
}

#[test]
fn stale_correction_preserves_all_owners_indexes_and_canonical_rows() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(&db, &fixture.key, &fixture.principal, "fact", "stale-index");
    let before = state(&db);
    let mut request = correction();
    request.expected_revision = 0;
    let error = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "stale-index",
        &request,
        200,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "memory_revision_conflict");
    assert_eq!(state(&db), before);
}
