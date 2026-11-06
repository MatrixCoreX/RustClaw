use super::boundary_test_fixtures as setup;
use super::*;

#[test]
fn json_export_contains_all_owned_kinds_beyond_one_list_page() {
    let fixture = setup::fixture();
    let (other_key, other) = setup::another_owner(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    for index in 0..201 {
        setup::fact(
            &db,
            &fixture.key,
            &fixture.principal,
            &format!("owned-fact-{index:03}"),
            &format!("Project record number {index} retained for later reference."),
            100 + index,
        );
    }
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "owned-recent",
        "A useful recent source message.",
    );
    setup::preference(&db, &fixture.key, &fixture.principal, "owned-preference");
    setup::fact(
        &db,
        &other_key,
        &other,
        "foreign-fact",
        "Other account data must stay private.",
        1000,
    );
    let export = export_memory(&db, &fixture.principal, 2000).unwrap();
    assert_eq!(
        export.items.len(),
        203,
        "an export is not the first list page"
    );
    for id in [
        "owned-recent",
        "owned-preference",
        "owned-fact-000",
        "owned-fact-200",
    ] {
        assert!(
            export.items.iter().any(|item| item.id == id),
            "missing {id}"
        );
    }
    assert!(!export.items.iter().any(|item| item.id == "foreign-fact"));
    assert_eq!(
        export.checksum,
        format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&export.items).unwrap())
        )
    );
}

#[test]
fn markdown_export_contains_every_owned_item_beyond_one_list_page() {
    let fixture = setup::fixture();
    let (other_key, other) = setup::another_owner(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    for index in 0..101 {
        setup::fact(
            &db,
            &fixture.key,
            &fixture.principal,
            &format!("owned-item-{index:03}"),
            &format!("A distinct useful record number {index}."),
            100 + index,
        );
    }
    setup::fact(
        &db,
        &other_key,
        &other,
        "private-foreign",
        "A separate private record.",
        1000,
    );
    let export = export_memory_markdown(&db, &fixture.principal, 2000).unwrap();
    for index in 0..101 {
        let heading = format!("## fact · owned-item-{index:03}\n");
        assert!(
            export.content.contains(&heading),
            "markdown omitted item {index}"
        );
    }
    assert!(!export.content.contains("private-foreign"));
    assert_eq!(
        export.checksum,
        format!("sha256:{:x}", Sha256::digest(export.content.as_bytes()))
    );
}

#[test]
fn full_clear_invalidates_undo_of_a_previously_deleted_transcript() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "deleted-recent",
        "Recently deleted private source content.",
    );
    let undo = setup::delete(&db, &fixture.principal, "deleted-recent");
    setup::clear(&db, &fixture.principal, "transcript_and_derived");
    let restored = undo_memory_mutation(&db, &fixture.principal, &undo, 102);
    assert!(
        restored.is_err(),
        "a pre-clear undo cannot resurrect cleared transcript data"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memories WHERE principal_id=?1",
            [&fixture.principal],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn full_clear_invalidates_undo_of_a_previously_deleted_derived_memory() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "deleted-derived",
        "Previously deleted derived data.",
        90,
    );
    let undo = setup::delete(&db, &fixture.principal, "deleted-derived");
    setup::clear(&db, &fixture.principal, "transcript_and_derived");
    assert!(
        undo_memory_mutation(&db, &fixture.principal, &undo, 102).is_err(),
        "a pre-clear undo cannot resurrect cleared derived data"
    );
}

#[test]
fn full_clear_invalidates_a_previously_prepared_import() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "visible-before-clear",
        "Visible memory before the clear.",
        90,
    );
    let preview = setup::preview(
        &db,
        &fixture.principal,
        "A distinct imported project record for future reference.",
    );
    assert_eq!(preview.accepted_items, 1);
    setup::clear(&db, &fixture.principal, "transcript_and_derived");
    assert!(
        confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&preview), 102)
            .is_err(),
        "full clear must invalidate old import staging, not only current canonical rows"
    );
}

#[test]
fn full_clear_scrubs_owned_snapshot_and_import_content_while_retaining_audit_identity() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    let snapshot_content = "A private archived snapshot sentinel for a project record.";
    let import_content = "A private archived import sentinel for a different project record.";
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "snapshot",
        snapshot_content,
        90,
    );
    let undo = setup::delete(&db, &fixture.principal, "snapshot");
    let preview = setup::preview(&db, &fixture.principal, import_content);
    setup::clear(&db, &fixture.principal, "transcript_and_derived");
    let snapshot: String = db
        .query_row(
            "SELECT previous_snapshot_json FROM memory_revisions
        WHERE revision_id=?1 AND principal_id=?2",
            params![undo.revision_id, fixture.principal],
            |row| row.get(0),
        )
        .expect("audit revision identity remains");
    assert!(
        !snapshot.contains(snapshot_content),
        "full clear must not leave the old private snapshot payload"
    );
    let (body, status): (String, String) = db
        .query_row(
            "SELECT preview_json,status FROM memory_import_sessions
        WHERE import_id=?1 AND principal_id=?2",
            params![preview.import_id, fixture.principal],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("audit import identity remains");
    assert!(!body.contains(import_content));
    assert_ne!(status, "preview");
}

#[test]
fn transcript_only_clear_preserves_derived_memory_undo() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "derived",
        "Derived data outside a transcript-only clear.",
        90,
    );
    let undo = setup::delete(&db, &fixture.principal, "derived");
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "recent",
        "A separate recent transcript.",
    );
    setup::clear(&db, &fixture.principal, "transcript");
    assert_eq!(
        undo_memory_mutation(&db, &fixture.principal, &undo, 102)
            .unwrap()
            .status,
        "restored"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memories WHERE principal_id=?1",
            [&fixture.principal],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn clearing_one_owner_preserves_another_owners_undo_snapshot_and_content() {
    let fixture = setup::fixture();
    let (other_key, other) = setup::another_owner(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    setup::fact(
        &db,
        &other_key,
        &other,
        "foreign-derived",
        "Other owner private data.",
        90,
    );
    let undo = setup::delete(&db, &other, "foreign-derived");
    setup::clear(&db, &fixture.principal, "transcript_and_derived");
    assert_eq!(
        undo_memory_mutation(&db, &other, &undo, 102)
            .unwrap()
            .status,
        "restored"
    );
    assert_eq!(
        db.query_row(
            "SELECT fact_text FROM memory_facts WHERE principal_id=?1",
            [&other],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "Other owner private data."
    );
}

#[test]
fn small_json_export_retains_checksum_and_private_owner_boundary() {
    let fixture = setup::fixture();
    let (other_key, other) = setup::another_owner(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "own",
        "One owned small export record.",
        90,
    );
    setup::fact(
        &db,
        &other_key,
        &other,
        "foreign",
        "One foreign small export record.",
        91,
    );
    let export = export_memory(&db, &fixture.principal, 100).unwrap();
    assert_eq!(export.items.len(), 1);
    assert_eq!(export.items[0].id, "own");
    assert_eq!(
        export.checksum,
        format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&export.items).unwrap())
        )
    );
}

#[test]
fn transcript_only_clear_preserves_a_derived_import_preview() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    let preview = setup::preview(
        &db,
        &fixture.principal,
        "A derived import that is outside transcript-only clearing.",
    );
    assert_eq!(preview.accepted_items, 1);
    setup::recent(
        &db,
        &fixture.key,
        &fixture.principal,
        "recent",
        "Only this recent record should be cleared.",
    );
    setup::clear(&db, &fixture.principal, "transcript");
    let imported =
        confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&preview), 102)
            .unwrap();
    assert_eq!(imported.imported_items, 1);
}
