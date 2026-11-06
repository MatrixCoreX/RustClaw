use super::boundary_test_fixtures as setup;
use super::*;

#[test]
fn transcript_only_clear_invalidates_old_transcript_undo() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "recent",
        "A private transcript to be cleared.",
    );
    let undo = setup::delete(&db, &fixture.principal, "recent");
    setup::clear(&db, &fixture.principal, "transcript");
    assert!(undo_memory_mutation(&db, &fixture.principal, &undo, 102).is_err());
}

#[test]
fn clear_refreshes_source_usage_without_resetting_quota_policy_or_spent_cost() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "recent",
        "Source bytes that will be cleared.",
    );
    db.execute_batch(
        "UPDATE memory_principal_quotas SET used_rows=42,used_bytes=999,
        max_rows=321,max_bytes=654,max_background_cost_microunits=9000,
        used_background_cost_microunits=123,revision=7;",
    )
    .unwrap();
    setup::clear(&db, &fixture.principal, "transcript");
    let actual = db
        .query_row(
            "SELECT used_rows,used_bytes,max_rows,max_bytes,
        max_background_cost_microunits,used_background_cost_microunits,revision
        FROM memory_principal_quotas WHERE principal_id=?1",
            [&fixture.principal],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(actual, (0, 0, 321, 654, 9000, 123, 7));
}

#[test]
fn clear_never_creates_a_missing_quota_as_an_unrequested_policy_repair() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    db.execute(
        "DELETE FROM memory_principal_quotas WHERE principal_id=?1",
        [&fixture.principal],
    )
    .unwrap();
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "recent",
        "Clear is permitted without creating policy.",
    );
    setup::clear(&db, &fixture.principal, "transcript_and_derived");
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_principal_quotas WHERE principal_id=?1",
            [&fixture.principal],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn transcript_only_clear_invalidates_a_preview_containing_recent_source_content() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "recent",
        "A recent source in an unconfirmed exported import.",
    );
    let export = export_memory(&db, &fixture.principal, 100).unwrap();
    assert_eq!(export.items[0].kind, "recent");
    let preview = preview_memory_import(
        &db,
        &fixture.principal,
        &MemoryImportPreviewRequest { export },
        100,
    )
    .unwrap();
    assert_eq!(preview.accepted_items, 1);
    setup::clear(&db, &fixture.principal, "transcript");
    assert!(
        confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&preview), 102)
            .is_err()
    );
}

#[test]
fn transcript_only_clear_preserves_derived_retrieval_entries() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::preference(&db, &fixture.key, &fixture.principal, "preference");
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "recent",
        "A recent source with a retrieval entry.",
    );
    crate::memory::indexing::index_preference_entries(
        &db,
        1,
        7,
        &fixture.key,
        &[(
            "response_format".into(),
            "markdown".into(),
            1.0,
            "user".into(),
        )],
        100,
    )
    .unwrap();
    let id: i64 = db
        .query_row(
            "SELECT id FROM memories WHERE memory_id='recent'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    crate::memory::indexing::index_memory_row(
        &db,
        1,
        7,
        &fixture.key,
        id,
        "user",
        "A recent source with a retrieval entry.",
        "episodic_event",
        0.5,
        false,
        100,
    )
    .unwrap();
    let snapshot = || {
        let mut statement = db
            .prepare(
                "SELECT id,source_kind,search_text FROM memory_retrieval_index
            WHERE principal_id=?1 ORDER BY id",
            )
            .unwrap();
        let rows = statement
            .query_map([&fixture.principal], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    };
    let before = snapshot();
    assert!(before.iter().any(|(_, kind, _)| kind == "preference"));
    assert!(before.iter().any(|(_, kind, _)| kind == "memory"));
    let expected = before
        .into_iter()
        .filter(|(_, kind, _)| kind != "memory")
        .collect::<Vec<_>>();
    setup::clear(&db, &fixture.principal, "transcript");
    assert_eq!(
        snapshot(),
        expected,
        "every derived index keeps its identity and content"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE principal_id=?1
        AND source_kind='memory'",
            [&fixture.principal],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
