use super::boundary_test_fixtures as setup;
use super::*;

#[test]
fn fresh_upload_after_full_clear_gets_a_new_confirmation_identity() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    let content = "A useful archived document explicitly uploaded again after clearing.";
    let before = setup::preview(&db, &fixture.principal, content);
    setup::clear(&db, &fixture.principal, "transcript_and_derived");
    let after = setup::preview(&db, &fixture.principal, content);
    assert_ne!(
        before.import_id, after.import_id,
        "an old UI confirmation must not authorize a new upload"
    );
    assert!(
        confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&before), 102).is_err()
    );
    assert_eq!(
        confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&after), 102)
            .unwrap()
            .imported_items,
        1
    );
}

#[test]
fn confirmed_import_can_be_explicitly_uploaded_again_after_its_content_is_cleared() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    let content = "A previously imported fact explicitly uploaded again after clearing.";
    let before = setup::preview(&db, &fixture.principal, content);
    assert_eq!(
        confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&before), 100)
            .unwrap()
            .imported_items,
        1
    );
    setup::clear(&db, &fixture.principal, "transcript_and_derived");
    let old_body: String = db
        .query_row(
            "SELECT preview_json FROM memory_import_sessions WHERE import_id=?1",
            [&before.import_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !old_body.contains(content),
        "confirmed import payloads also belong to cleared private data"
    );
    let after = setup::preview(&db, &fixture.principal, content);
    assert_ne!(before.import_id, after.import_id);
    assert_eq!(after.accepted_items, 1);
    assert_eq!(
        confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&after), 102)
            .unwrap()
            .imported_items,
        1
    );
}

#[test]
fn failed_private_staging_purge_rolls_back_the_whole_clear() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "visible",
        "Visible canonical data must survive a rejected clear.",
        90,
    );
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "deleted",
        "A separate undo snapshot must survive a rejected clear.",
        90,
    );
    let undo = setup::delete(&db, &fixture.principal, "deleted");
    let preview = setup::preview(
        &db,
        &fixture.principal,
        "A pending import must remain confirmable after a rejected clear.",
    );
    let before: String = db
        .query_row(
            "SELECT previous_snapshot_json FROM memory_revisions WHERE revision_id=?1",
            [&undo.revision_id],
            |row| row.get(0),
        )
        .unwrap();
    let clear = clear_preview(&db, &fixture.principal, "transcript_and_derived").unwrap();
    db.execute_batch(
        "CREATE TRIGGER fixture_clear_failure BEFORE UPDATE ON memory_import_sessions
        BEGIN SELECT RAISE(ABORT,'fixture_clear_failure'); END;",
    )
    .unwrap();
    assert!(clear_with_mode(
        &db,
        &fixture.principal,
        &MemoryBulkClearRequest {
            mode: clear.mode,
            expected_transcript_rows: clear.transcript_rows,
            expected_derived_rows: clear.derived_rows,
        },
        101
    )
    .is_err());
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_facts WHERE memory_id='visible'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT previous_snapshot_json FROM memory_revisions WHERE revision_id=?1",
            [&undo.revision_id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
    db.execute_batch("DROP TRIGGER fixture_clear_failure")
        .unwrap();
    assert_eq!(
        confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&preview), 102)
            .unwrap()
            .imported_items,
        1
    );
    assert_eq!(
        undo_memory_mutation(&db, &fixture.principal, &undo, 102)
            .unwrap()
            .status,
        "restored"
    );
}

#[test]
fn repeated_clear_keeps_redacted_import_audit_binding_stable() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    let preview = setup::preview(
        &db,
        &fixture.principal,
        "Private import data to redact exactly once.",
    );
    setup::clear(&db, &fixture.principal, "transcript_and_derived");
    let read = || {
        db.query_row(
            "SELECT preview_json,payload_digest,status FROM memory_import_sessions
        WHERE import_id=?1",
            [&preview.import_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap()
    };
    let first = read();
    let tombstone: Value = serde_json::from_str(&first.0).unwrap();
    assert_eq!(tombstone["original_payload_digest"], preview.payload_digest);
    assert_eq!(
        first.1,
        format!("sha256:{:x}", Sha256::digest(first.0.as_bytes()))
    );
    assert_eq!(first.2, "cancelled");
    setup::clear(&db, &fixture.principal, "transcript_and_derived");
    assert_eq!(read(), first);
}

#[test]
fn clear_redacts_every_owned_import_without_touching_a_foreign_preview() {
    let fixture = setup::fixture();
    let (_, foreign) = setup::another_owner(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    for index in 0..20 {
        setup::preview(
            &db,
            &fixture.principal,
            &format!("Private owned import document number {index} for later use."),
        );
    }
    let other = setup::preview(
        &db,
        &foreign,
        "Foreign private import must not change during another owner's clear.",
    );
    let read_foreign = || {
        db.query_row(
            "SELECT preview_json,payload_digest,status FROM memory_import_sessions
        WHERE import_id=?1",
            [&other.import_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap()
    };
    let before = read_foreign();
    setup::clear(&db, &fixture.principal, "transcript_and_derived");
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_import_sessions WHERE principal_id=?1
        AND status='cancelled' AND json_extract(preview_json,'$.purged')=1",
            [&fixture.principal],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        20
    );
    assert_eq!(read_foreign(), before);
    assert_eq!(
        confirm_memory_import(&db, &foreign, &setup::confirmation(&other), 102)
            .unwrap()
            .imported_items,
        1
    );
}
