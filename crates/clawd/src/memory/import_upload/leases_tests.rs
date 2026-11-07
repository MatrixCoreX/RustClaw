use super::*;

#[test]
fn incomplete_upload_is_not_claimed_or_modified_by_the_parser() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"abcd", 2);
    setup.put_all(&upload.upload_id, b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    let before = status(&db, &setup.actor, &upload.upload_id).unwrap();
    error(
        claim_parser(&db, &setup.actor, &upload.upload_id, 102, policy(2)),
        UploadError::Incomplete,
    );
    assert_eq!(
        status(&db, &setup.actor, &upload.upload_id).unwrap(),
        before
    );
}

#[test]
fn parser_expiry_fences_old_workers_without_losing_raw_chunks() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"abcd", 2);
    setup.put_all(&upload.upload_id, b"abcd", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    let first = claim_parser(&db, &setup.actor, &upload.upload_id, 102, policy(2)).unwrap();
    db.execute(
        "INSERT INTO memory_import_upload_items VALUES(?1,0,'{\"private\":true}')",
        [&upload.upload_id],
    )
    .unwrap();
    error(
        claim_parser(&db, &setup.actor, &upload.upload_id, 103, policy(2)),
        UploadError::ParserBusy,
    );
    assert_eq!(count(&db, "memory_import_upload_items"), 1);
    let second = claim_parser(&db, &setup.actor, &upload.upload_id, 113, policy(2)).unwrap();
    assert_ne!(first.token, second.token);
    assert_eq!(count(&db, "memory_import_upload_items"), 0);
    assert_eq!(count(&db, "memory_import_upload_chunks"), 2);
    error(
        renew_parser(&db, &setup.actor, &first, 114, policy(2)),
        UploadError::LeaseChanged,
    );
    error(
        ChunkReader::new(&db, &setup.actor, &first),
        UploadError::LeaseChanged,
    );
    assert!(ChunkReader::new(&db, &setup.actor, &second).is_ok());
}

#[test]
fn an_unstolen_parser_renews_after_expiry_instead_of_wall_clock_killing_work() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"ab", 2);
    setup.put_all(&upload.upload_id, b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    let lease = claim_parser(&db, &setup.actor, &upload.upload_id, 102, policy(2)).unwrap();
    renew_parser(&db, &setup.actor, &lease, 1000, policy(2)).unwrap();
    error(
        claim_parser(&db, &setup.actor, &upload.upload_id, 1001, policy(2)),
        UploadError::ParserBusy,
    );
    let mut output = Vec::new();
    let mut reader = ChunkReader::new(&db, &setup.actor, &lease).unwrap();
    reader.read_to_end(&mut output).unwrap();
    reader.verify_complete().unwrap();
    assert_eq!(output, b"ab");
}

#[test]
fn cancellation_releases_only_owned_staging_even_when_capacity_is_missing() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"ab", 2);
    setup.put_all(&upload.upload_id, b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    let other = create(
        &db,
        &setup.other,
        &CreateUpload {
            file_digest: digest(b"cd"),
            total_bytes: 2,
        },
        policy(2),
        100,
    )
    .unwrap();
    append(
        &db,
        &setup.other,
        &other.upload_id,
        0,
        &digest(b"cd"),
        b"cd",
        101,
    )
    .unwrap();
    let lease = claim_parser(&db, &setup.actor, &upload.upload_id, 102, policy(2)).unwrap();
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    let cancelled = cancel(&db, &setup.actor, &upload.upload_id, 103).unwrap();
    assert_eq!(cancelled.state, UploadState::Cancelled);
    assert_eq!(
        cancel(&db, &setup.actor, &upload.upload_id, 104).unwrap(),
        cancelled
    );
    assert_eq!(count(&db, "memory_import_upload_chunks"), 1);
    assert_eq!(
        status(&db, &setup.other, &other.upload_id)
            .unwrap()
            .received_bytes,
        "2"
    );
    error(
        renew_parser(&db, &setup.actor, &lease, 105, policy(2)),
        UploadError::LeaseChanged,
    );
    assert_eq!(count(&db, "memory_facts"), 0);
}

#[test]
fn last_chunk_retry_remains_idempotent_after_parser_claim() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"ab", 2);
    setup.put_all(&upload.upload_id, b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    claim_parser(&db, &setup.actor, &upload.upload_id, 102, policy(2)).unwrap();
    let before = status(&db, &setup.actor, &upload.upload_id).unwrap();
    assert_eq!(
        append(
            &db,
            &setup.actor,
            &upload.upload_id,
            0,
            &digest(b"ab"),
            b"ab",
            103
        )
        .unwrap(),
        before
    );
    assert_eq!(count(&db, "memory_import_upload_chunks"), 1);
}
