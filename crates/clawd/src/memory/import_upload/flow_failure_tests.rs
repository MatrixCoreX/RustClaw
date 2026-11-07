use super::*;

#[test]
fn wrong_export_checksum_releases_provisional_items_but_keeps_resumable_raw_bytes() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let mut bad = export(vec![item("not publishable", "fact")]);
    bad.checksum = digest(b"wrong");
    let raw = serde_json::to_vec(&bad).unwrap();
    let upload = upload_raw(&db, &env.actor, &raw);
    for _ in 0..2 {
        error(
            parse_and_preview(&db, &env.actor, &upload.upload_id, parse_policy(), 102),
            UploadError::ExportChecksumMismatch,
        );
        assert_eq!(
            status(&db, &env.actor, &upload.upload_id).unwrap().state,
            UploadState::Uploading
        );
        assert_eq!(staged_count(&db, &upload.upload_id), 0);
    }
    let saved: i64 = db
        .query_row(
            "SELECT SUM(LENGTH(content)) FROM memory_import_upload_chunks WHERE upload_id=?1",
            [&upload.upload_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, raw.len() as i64);
    assert_eq!(count(&db, "memory_facts"), 0);
    assert_eq!(count(&db, "memory_import_sessions"), 0);
}

#[test]
fn stale_expected_preview_digest_cannot_publish_any_canonical_item() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let (upload, mut p) = preview(&db, &env.actor, vec![item("not yet confirmed", "fact")]);
    p.payload_digest = digest(b"other preview");
    assert_eq!(
        confirm(&db, &env.actor, &p).unwrap_err().to_string(),
        "memory_import_preview_conflict"
    );
    assert_eq!(count(&db, "memory_facts"), 0);
    assert_eq!(staged_count(&db, &upload.upload_id), 1);
    assert_eq!(
        status(&db, &env.actor, &upload.upload_id).unwrap().state,
        UploadState::Preview
    );
}

#[test]
fn failing_second_canonical_insert_rolls_back_the_entire_confirmation() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let (upload, p) = preview(
        &db,
        &env.actor,
        vec![item("first", "fact"), item("second", "fact")],
    );
    db.execute_batch(
        "CREATE TRIGGER fixture_second_item BEFORE INSERT ON memory_facts
        WHEN NEW.fact_text='second' BEGIN SELECT RAISE(ABORT,'fixture_second_item'); END;",
    )
    .unwrap();
    assert!(confirm(&db, &env.actor, &p).is_err());
    assert_eq!(count(&db, "memory_facts"), 0);
    assert_eq!(staged_count(&db, &upload.upload_id), 2);
    assert_eq!(
        status(&db, &env.actor, &upload.upload_id).unwrap().state,
        UploadState::Preview
    );
    db.execute_batch("DROP TRIGGER fixture_second_item")
        .unwrap();
    assert_eq!(confirm(&db, &env.actor, &p).unwrap().imported_items, 2);
}

#[test]
fn revoked_original_binding_cannot_confirm_through_a_surviving_sibling() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let (upload, p) = preview(&db, &env.actor, vec![item("original key boundary", "fact")]);
    super::super::test_fixtures::revoke(&db, &env.actor, "binding", &env.other);
    assert!(
        crate::repo::auth::require_active_principal_credential(&db, &env.actor.principal_id)
            .is_ok()
    );
    assert!(confirm(&db, &env.actor, &p).is_err());
    assert_eq!(count(&db, "memory_facts"), 0);
    assert_eq!(staged_count(&db, &upload.upload_id), 1);
}

#[test]
fn damaged_staged_content_never_matches_the_original_logical_preview_digest() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let (upload, p) = preview(&db, &env.actor, vec![item("original contents", "fact")]);
    db.execute(
        "UPDATE memory_import_upload_items SET item_json=json_set(item_json,'$.content','changed')
        WHERE upload_id=?1",
        [&upload.upload_id],
    )
    .unwrap();
    error(confirm(&db, &env.actor, &p), UploadError::Corrupt);
    assert_eq!(count(&db, "memory_facts"), 0);
    assert_eq!(
        status(&db, &env.actor, &upload.upload_id).unwrap().state,
        UploadState::Preview
    );
    let (other, foreign) = preview(
        &db,
        &env.other,
        vec![item("foreign private staging", "fact")],
    );
    let foreign_metadata: String = db
        .query_row(
            "SELECT preview_json FROM memory_import_sessions WHERE import_id=?1",
            [&foreign.import_id],
            |row| row.get(0),
        )
        .unwrap();
    db.execute(
        "UPDATE memory_import_sessions SET preview_json=?1,payload_digest=?2 WHERE import_id=?3",
        params![foreign_metadata, foreign.payload_digest, p.import_id],
    )
    .unwrap();
    error(
        parse_and_preview(&db, &env.actor, &upload.upload_id, parse_policy(), 104),
        UploadError::Corrupt,
    );
    let mut expected = p;
    expected.payload_digest = foreign.payload_digest;
    error(confirm(&db, &env.actor, &expected), UploadError::Corrupt);
    assert_eq!(staged_count(&db, &other.upload_id), 1);
    assert_eq!(count(&db, "memory_facts"), 0);
}
