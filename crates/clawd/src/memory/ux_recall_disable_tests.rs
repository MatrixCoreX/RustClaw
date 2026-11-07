use super::*;

#[test]
fn fact_do_not_use_removes_index_and_fts_and_survives_rebuild() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "fact",
        "disabled-fact",
    );
    record_feedback(
        &db,
        &fixture.principal,
        "disabled-fact",
        &feedback(MemoryFeedbackKind::DoNotUse),
        200,
    )
    .unwrap();
    assert!(
        rows(&db, &fixture.principal).is_empty(),
        "disabled fact must stop being recallable immediately"
    );
    assert_no_orphan_fts(&db);
    rebuild(&db);
    assert!(
        rows(&db, &fixture.principal).is_empty(),
        "rebuild must not resurrect disabled facts"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_facts WHERE memory_id='disabled-fact'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "do not use is not canonical content deletion"
    );
}

#[test]
fn preference_do_not_use_survives_rebuild_and_preserves_other_owners() {
    let fixture = setup::fixture();
    let (other_key, other) = setup::another_owner(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "preference",
        "disabled-pref",
    );
    seed(&db, &other_key, &other, "preference", "other-pref");
    let foreign = rows(&db, &other);
    record_feedback(
        &db,
        &fixture.principal,
        "disabled-pref",
        &feedback(MemoryFeedbackKind::DoNotUse),
        200,
    )
    .unwrap();
    assert_eq!(
        rows(&db, &other),
        foreign,
        "another owner's same key remains recallable"
    );
    assert!(
        rows(&db, &fixture.principal).is_empty(),
        "disabled preference must stop being recallable immediately"
    );
    assert_no_orphan_fts(&db);
    rebuild(&db);
    assert!(
        rows(&db, &fixture.principal).is_empty(),
        "rebuild must not resurrect disabled preferences"
    );
    assert_eq!(rows(&db, &other), foreign);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM user_preferences WHERE memory_id='disabled-pref'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "preference content remains available for explicit management"
    );
}

#[test]
fn recent_do_not_use_survives_rebuild_without_deleting_transcript() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "recent",
        "disabled-recent",
    );
    record_feedback(
        &db,
        &fixture.principal,
        "disabled-recent",
        &feedback(MemoryFeedbackKind::DoNotUse),
        200,
    )
    .unwrap();
    assert!(
        rows(&db, &fixture.principal).is_empty(),
        "disabled transcript must stop being recallable immediately"
    );
    assert_no_orphan_fts(&db);
    rebuild(&db);
    assert!(
        rows(&db, &fixture.principal).is_empty(),
        "rebuild must not resurrect disabled transcripts"
    );
    assert_eq!(
        db.query_row(
            "SELECT content FROM memories WHERE memory_id='disabled-recent'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "Original private transcript"
    );
}

#[test]
fn rebuilding_cannot_reenable_a_preference_disabled_by_explicit_feedback() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "preference",
        "rebuild-pref",
    );
    record_feedback(
        &db,
        &fixture.principal,
        "rebuild-pref",
        &feedback(MemoryFeedbackKind::DoNotUse),
        200,
    )
    .unwrap();
    rebuild(&db);
    assert!(
        rows(&db, &fixture.principal).is_empty(),
        "explicit recall suppression must survive index rebuild"
    );
    assert_no_orphan_fts(&db);
}

#[test]
fn rebuilding_cannot_reenable_a_transcript_disabled_by_explicit_feedback() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "recent",
        "rebuild-recent",
    );
    record_feedback(
        &db,
        &fixture.principal,
        "rebuild-recent",
        &feedback(MemoryFeedbackKind::DoNotUse),
        200,
    )
    .unwrap();
    rebuild(&db);
    assert!(
        rows(&db, &fixture.principal).is_empty(),
        "explicit recall suppression must survive index rebuild"
    );
    assert_no_orphan_fts(&db);
}
