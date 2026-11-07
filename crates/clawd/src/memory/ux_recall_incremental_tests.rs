use super::*;
use crate::memory::indexing;

#[test]
fn incremental_preference_indexing_does_not_reenable_explicitly_disabled_content() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "preference",
        "incremental-pref",
    );
    record_feedback(
        &db,
        &fixture.principal,
        "incremental-pref",
        &feedback(MemoryFeedbackKind::DoNotUse),
        200,
    )
    .unwrap();
    assert!(rows(&db, &fixture.principal).is_empty());
    indexing::index_preference_entries(
        &db,
        1,
        7,
        &fixture.key,
        &[(
            "response_format".into(),
            "markdown".into(),
            1.0,
            "user_message".into(),
        )],
        201,
    )
    .unwrap();
    assert!(
        rows(&db, &fixture.principal).is_empty(),
        "incremental indexing must retain explicit preference recall suppression"
    );
    assert_no_orphan_fts(&db);
}

#[test]
fn incremental_transcript_indexing_does_not_reenable_explicitly_disabled_content() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "recent",
        "incremental-recent",
    );
    let id: i64 = db
        .query_row(
            "SELECT id FROM memories WHERE memory_id='incremental-recent'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    record_feedback(
        &db,
        &fixture.principal,
        "incremental-recent",
        &feedback(MemoryFeedbackKind::DoNotUse),
        200,
    )
    .unwrap();
    assert!(rows(&db, &fixture.principal).is_empty());
    indexing::index_memory_row(
        &db,
        1,
        7,
        &fixture.key,
        id,
        "user",
        "Original private transcript",
        "episodic_event",
        0.5,
        false,
        100,
    )
    .unwrap();
    assert!(
        rows(&db, &fixture.principal).is_empty(),
        "incremental indexing must retain explicit transcript recall suppression"
    );
    assert_no_orphan_fts(&db);
}

#[test]
fn incremental_fact_indexing_does_not_reenable_explicitly_disabled_content() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "fact",
        "incremental-fact",
    );
    let id: i64 = db
        .query_row(
            "SELECT id FROM memory_facts WHERE memory_id='incremental-fact'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    record_feedback(
        &db,
        &fixture.principal,
        "incremental-fact",
        &feedback(MemoryFeedbackKind::DoNotUse),
        200,
    )
    .unwrap();
    assert!(rows(&db, &fixture.principal).is_empty());
    indexing::upsert_memory_fact_retrieval_row(
        &db,
        1,
        &fixture.key,
        "fixture_profile",
        id,
        "Original private fact",
        1.0,
        201,
    )
    .unwrap();
    assert!(
        rows(&db, &fixture.principal).is_empty(),
        "incremental indexing must retain explicit fact recall suppression"
    );
    assert_no_orphan_fts(&db);
}

#[test]
fn stale_incremental_preference_payload_cannot_overwrite_a_confirmed_correction() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "preference",
        "stale-pref-source",
    );
    correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "stale-pref-source",
        &correction(),
        200,
    )
    .unwrap();
    let before = state(&db);
    let result = indexing::index_preference_entries(
        &db,
        1,
        7,
        &fixture.key,
        &[(
            "response_format".into(),
            "markdown".into(),
            1.0,
            "user_message".into(),
        )],
        201,
    );
    assert!(
        result.is_err(),
        "a stale canonical snapshot must not be silently indexed as current content"
    );
    assert_eq!(
        state(&db),
        before,
        "stale incremental work must preserve the corrected canonical,FTS,index and outbox"
    );
}
