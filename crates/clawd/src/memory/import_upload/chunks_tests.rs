use super::*;

#[test]
fn create_resumes_same_file_only_within_the_same_owner() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"abcd", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    let request = CreateUpload {
        file_digest: digest(b"abcd"),
        total_bytes: 4,
    };
    assert_eq!(
        create(&db, &setup.actor, &request, policy(2), 101).unwrap(),
        upload
    );
    let other = create(&db, &setup.other, &request, policy(2), 101).unwrap();
    assert_ne!(other.upload_id, upload.upload_id);
    error(
        create(
            &db,
            &setup.actor,
            &CreateUpload {
                total_bytes: 5,
                ..request
            },
            policy(2),
            101,
        ),
        UploadError::StateConflict,
    );
    assert_eq!(count(&db, "memory_import_uploads"), 2);
}

#[test]
fn duplicate_chunk_does_not_write_or_need_growth_admission() {
    let setup = Setup::new(true);
    let upload = setup.upload(b"abcd", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    let first = append(
        &db,
        &setup.actor,
        &upload.upload_id,
        0,
        &digest(b"ab"),
        b"ab",
        101,
    )
    .unwrap();
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    assert_eq!(
        append(
            &db,
            &setup.actor,
            &upload.upload_id,
            0,
            &digest(b"ab"),
            b"ab",
            102
        )
        .unwrap(),
        first
    );
    assert_eq!(count(&db, "memory_import_upload_chunks"), 1);
    assert!(append(
        &db,
        &setup.actor,
        &upload.upload_id,
        2,
        &digest(b"cd"),
        b"cd",
        102
    )
    .is_err());
    assert_eq!(status(&db, &setup.actor, &upload.upload_id).unwrap(), first);
}

#[test]
fn bad_digest_order_overlap_and_size_never_advance_the_cursor() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"abcd", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    error(
        append(
            &db,
            &setup.actor,
            &upload.upload_id,
            0,
            &digest(b"xx"),
            b"ab",
            101,
        ),
        UploadError::ChunkDigestMismatch,
    );
    error(
        append(
            &db,
            &setup.actor,
            &upload.upload_id,
            2,
            &digest(b"cd"),
            b"cd",
            101,
        ),
        UploadError::OffsetConflict,
    );
    error(
        append(
            &db,
            &setup.actor,
            &upload.upload_id,
            0,
            &digest(b"abc"),
            b"abc",
            101,
        ),
        UploadError::ChunkTooLarge,
    );
    append(
        &db,
        &setup.actor,
        &upload.upload_id,
        0,
        &digest(b"ab"),
        b"ab",
        101,
    )
    .unwrap();
    error(
        append(
            &db,
            &setup.actor,
            &upload.upload_id,
            1,
            &digest(b"bc"),
            b"bc",
            101,
        ),
        UploadError::OffsetConflict,
    );
    error(
        append(
            &db,
            &setup.actor,
            &upload.upload_id,
            0,
            &digest(b"xx"),
            b"xx",
            101,
        ),
        UploadError::OffsetConflict,
    );
    error(
        append(
            &db,
            &setup.actor,
            &upload.upload_id,
            u64::MAX,
            &digest(b"cd"),
            b"cd",
            101,
        ),
        UploadError::OffsetConflict,
    );
    assert_eq!(
        status(&db, &setup.actor, &upload.upload_id)
            .unwrap()
            .received_bytes,
        "2"
    );
    assert_eq!(count(&db, "memory_import_upload_chunks"), 1);
}

#[test]
fn failed_pointer_write_rolls_back_the_inserted_chunk() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    db.execute_batch("CREATE TRIGGER reject_upload_pointer BEFORE UPDATE OF received_bytes ON memory_import_uploads
        BEGIN SELECT RAISE(ABORT,'fixture_pointer_failure');END;").unwrap();
    assert!(append(
        &db,
        &setup.actor,
        &upload.upload_id,
        0,
        &digest(b"ab"),
        b"ab",
        101
    )
    .is_err());
    assert_eq!(count(&db, "memory_import_upload_chunks"), 0);
    assert_eq!(
        status(&db, &setup.actor, &upload.upload_id).unwrap(),
        upload
    );
}

#[test]
fn storage_policy_change_inside_chunk_write_rolls_back_all_effects() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    let before: String = db
        .query_row(
            "SELECT configured_limit_bytes FROM memory_storage_budget",
            [],
            |row| row.get(0),
        )
        .unwrap();
    db.execute_batch(
        "CREATE TRIGGER change_upload_budget AFTER INSERT ON memory_import_upload_chunks
        BEGIN UPDATE memory_storage_budget SET configured_limit_bytes='1';END;",
    )
    .unwrap();
    assert!(append(
        &db,
        &setup.actor,
        &upload.upload_id,
        0,
        &digest(b"ab"),
        b"ab",
        101
    )
    .is_err());
    assert_eq!(count(&db, "memory_import_upload_chunks"), 0);
    assert_eq!(
        status(&db, &setup.actor, &upload.upload_id).unwrap(),
        upload
    );
    assert_eq!(
        db.query_row(
            "SELECT configured_limit_bytes FROM memory_storage_budget",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
}

#[test]
fn nested_chunk_write_cannot_commit_its_callers_transaction() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    append(
        &db,
        &setup.actor,
        &upload.upload_id,
        0,
        &digest(b"ab"),
        b"ab",
        101,
    )
    .unwrap();
    assert!(!db.is_autocommit());
    db.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        status(&db, &setup.actor, &upload.upload_id).unwrap(),
        upload
    );
    assert_eq!(count(&db, "memory_import_upload_chunks"), 0);
}

#[test]
fn read_status_does_not_contend_with_an_independent_sqlite_writer() {
    let setup = Setup::new(true);
    let upload = setup.upload(b"ab", 2);
    let competing = setup.fixture.competing_writer();
    let db = setup.fixture.state().core.db.get().unwrap();
    db.pragma_update(None, "query_only", "ON").unwrap();
    assert_eq!(
        status(&db, &setup.actor, &upload.upload_id).unwrap(),
        upload
    );
    competing.execute_batch("ROLLBACK").unwrap();
}
