use super::*;

#[test]
fn cancelling_a_preview_removes_only_its_owned_plaintext_and_releases_its_file_identity() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let records = vec![item("cancelled private contents", "fact")];
    let (upload, p) = preview(&db, &env.actor, records.clone());
    let (other, _) = preview(&db, &env.other, vec![item("other owner remains", "fact")]);
    assert_eq!(
        cancel(&db, &env.actor, &upload.upload_id, 104)
            .unwrap()
            .state,
        UploadState::Cancelled
    );
    assert_eq!(
        cancel(&db, &env.actor, &upload.upload_id, 105)
            .unwrap()
            .state,
        UploadState::Cancelled
    );
    assert_eq!(staged_count(&db, &upload.upload_id), 0);
    assert_eq!(staged_count(&db, &other.upload_id), 1);
    assert!(confirm(&db, &env.actor, &p).is_err());
    let fresh = upload_raw(
        &db,
        &env.actor,
        &serde_json::to_vec(&export(records)).unwrap(),
    );
    assert_ne!(fresh.upload_id, upload.upload_id);
}

#[test]
fn full_clear_purges_owned_pending_raw_and_preview_staging_without_touching_another_owner() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let (upload, p) = preview(&db, &env.actor, vec![item("private preview", "fact")]);
    let raw = upload_raw(
        &db,
        &env.actor,
        &serde_json::to_vec(&export(vec![item("private raw", "recent")])).unwrap(),
    );
    let (other, other_preview) =
        preview(&db, &env.other, vec![item("other private preview", "fact")]);
    clear(&db, &env.actor, "transcript_and_derived").unwrap();
    assert_eq!(staged_count(&db, &upload.upload_id), 0);
    assert_eq!(
        status(&db, &env.actor, &raw.upload_id).unwrap().state,
        UploadState::Cancelled
    );
    let bytes: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_import_upload_chunks WHERE upload_id=?1",
            [&raw.upload_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bytes, 0);
    assert!(confirm(&db, &env.actor, &p).is_err());
    assert_eq!(staged_count(&db, &other.upload_id), 1);
    assert_eq!(
        confirm(&db, &env.other, &other_preview)
            .unwrap()
            .imported_items,
        1
    );
}

#[test]
fn recent_clear_withdraws_recent_preview_but_keeps_a_fact_only_preview() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let (recent, recent_preview) =
        preview(&db, &env.actor, vec![item("recent private data", "recent")]);
    let (fact, fact_preview) = preview(&db, &env.actor, vec![item("fact remains pending", "fact")]);
    clear(&db, &env.actor, "transcript_only").unwrap();
    assert_eq!(
        status(&db, &env.actor, &recent.upload_id).unwrap().state,
        UploadState::Cancelled
    );
    assert_eq!(staged_count(&db, &recent.upload_id), 0);
    assert!(confirm(&db, &env.actor, &recent_preview).is_err());
    assert_eq!(staged_count(&db, &fact.upload_id), 1);
    assert_eq!(
        confirm(&db, &env.actor, &fact_preview)
            .unwrap()
            .imported_items,
        1
    );
}

#[test]
fn a_failed_staging_purge_rolls_back_session_tombstones_and_the_whole_clear() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let (upload, p) = preview(
        &db,
        &env.actor,
        vec![item("must remain pending after failure", "fact")],
    );
    db.execute_batch(
        "CREATE TRIGGER fixture_staging_purge BEFORE DELETE ON memory_import_upload_items
        BEGIN SELECT RAISE(ABORT,'fixture_staging_purge'); END;",
    )
    .unwrap();
    assert!(clear(&db, &env.actor, "transcript_and_derived").is_err());
    assert_eq!(staged_count(&db, &upload.upload_id), 1);
    assert_eq!(
        status(&db, &env.actor, &upload.upload_id).unwrap().state,
        UploadState::Preview
    );
    db.execute_batch("DROP TRIGGER fixture_staging_purge")
        .unwrap();
    assert_eq!(confirm(&db, &env.actor, &p).unwrap().imported_items, 1);
}

#[test]
fn confirmed_receipts_keep_audit_metadata_without_retaining_the_imported_plaintext() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let text = "private contents should only remain as canonical data";
    let (upload, p) = preview(&db, &env.actor, vec![item(text, "fact")]);
    confirm(&db, &env.actor, &p).unwrap();
    assert_eq!(staged_count(&db, &upload.upload_id), 0);
    let json: String = db
        .query_row(
            "SELECT preview_json FROM memory_import_sessions WHERE import_id=?1",
            [&p.import_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!json.contains(text));
    assert_eq!(
        status(&db, &env.actor, &upload.upload_id).unwrap().state,
        UploadState::Confirmed
    );
}

#[test]
fn reopened_database_reuses_one_complete_preview_without_reparsing_raw_bytes() {
    let env = Setup::new(true);
    let (upload, p, path) = {
        let db = env.fixture.state().core.db.get().unwrap();
        let (upload, p) = preview(
            &db,
            &env.actor,
            vec![item("durable pending import", "fact")],
        );
        (upload, p, db.path().unwrap().to_owned())
    };
    let db = Connection::open(path).unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    assert_eq!(
        parse_and_preview(&db, &env.actor, &upload.upload_id, parse_policy(), 110).unwrap(),
        p
    );
    assert_eq!(confirm(&db, &env.actor, &p).unwrap().imported_items, 1);
    assert_eq!(
        status(&db, &env.actor, &upload.upload_id).unwrap().state,
        UploadState::Confirmed
    );
}
