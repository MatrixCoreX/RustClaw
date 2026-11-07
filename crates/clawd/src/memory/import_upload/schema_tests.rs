use super::*;

#[test]
fn explicit_startup_is_idempotent_and_status_is_query_only() {
    let setup = Setup::new(true);
    let upload = setup.upload(b"content", 4);
    let db = setup.fixture.state().core.db.get().unwrap();
    initialize_schema(&db).unwrap();
    let before: i64 = db
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    db.pragma_update(None, "query_only", "ON").unwrap();
    assert_eq!(
        status(&db, &setup.actor, &upload.upload_id).unwrap(),
        upload
    );
    assert_eq!(
        db.query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn missing_marker_is_not_repaired_or_adopted_by_read_or_startup() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"a", 1);
    let db = setup.fixture.state().core.db.get().unwrap();
    db.execute(
        "DELETE FROM runtime_schema_migrations WHERE migration_id='032_memory_import_uploads_v1'",
        [],
    )
    .unwrap();
    error(
        status(&db, &setup.actor, &upload.upload_id),
        UploadError::SchemaUnavailable,
    );
    error(initialize_schema(&db), UploadError::SchemaUnavailable);
    assert_eq!(count(&db, "memory_import_uploads"), 1);
    assert_eq!(db.query_row("SELECT COUNT(*) FROM runtime_schema_migrations WHERE migration_id='032_memory_import_uploads_v1'",[],|row|row.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn corrupt_marker_fails_closed_without_overwriting_receipt() {
    let setup = Setup::new(false);
    let db = setup.fixture.state().core.db.get().unwrap();
    db.execute("UPDATE runtime_schema_migrations SET schema_digest='corrupt' WHERE migration_id='032_memory_import_uploads_v1'",[]).unwrap();
    error(initialize_schema(&db), UploadError::SchemaUnavailable);
    error(
        status(&db, &setup.actor, "absent"),
        UploadError::SchemaUnavailable,
    );
    assert_eq!(db.query_row("SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id='032_memory_import_uploads_v1'",[],|row|row.get::<_,String>(0)).unwrap(),"corrupt");
}

#[test]
fn missing_schema_read_never_creates_tables() {
    let setup = Setup::new(false);
    let db = setup.fixture.state().core.db.get().unwrap();
    db.execute_batch("DROP TABLE memory_import_upload_items;DROP TABLE memory_import_upload_chunks;DROP TABLE memory_import_uploads;
        DELETE FROM runtime_schema_migrations WHERE migration_id='032_memory_import_uploads_v1';").unwrap();
    db.pragma_update(None, "query_only", "ON").unwrap();
    error(
        status(&db, &setup.actor, "absent"),
        UploadError::SchemaUnavailable,
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name='memory_import_uploads'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn invalid_metadata_and_oversized_file_leave_no_staging() {
    let setup = Setup::new(false);
    let db = setup.fixture.state().core.db.get().unwrap();
    for (file_digest, total_bytes, chunk, expected) in [
        ("sha256:bad".into(), 1, 1, UploadError::InvalidMetadata),
        (digest(b"a"), 0, 1, UploadError::InvalidMetadata),
        (digest(b"a"), 1, 0, UploadError::InvalidMetadata),
        (digest(b"a"), u64::MAX, 1, UploadError::FileTooLarge),
    ] {
        error(
            create(
                &db,
                &setup.actor,
                &CreateUpload {
                    file_digest,
                    total_bytes,
                },
                policy(chunk),
                100,
            ),
            expected,
        );
    }
    assert_eq!(count(&db, "memory_import_uploads"), 0);
}
