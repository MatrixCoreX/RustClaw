use super::*;

#[test]
fn streaming_reader_preserves_split_unicode_and_validates_full_file_digest() {
    let setup = Setup::new(true);
    let bytes = "中文🐼\\n转写".repeat(20).into_bytes();
    let upload = setup.upload(&bytes, 7);
    setup.put_all(&upload.upload_id, &bytes, 7);
    let db = setup.fixture.state().core.db.get().unwrap();
    let lease = claim_parser(&db, &setup.actor, &upload.upload_id, 102, policy(7)).unwrap();
    db.pragma_update(None, "query_only", "ON").unwrap();
    let mut reader = ChunkReader::new(&db, &setup.actor, &lease).unwrap();
    error(reader.verify_complete(), UploadError::Incomplete);
    let mut actual = Vec::new();
    reader.read_to_end(&mut actual).unwrap();
    reader.verify_complete().unwrap();
    assert_eq!(actual, bytes);
}

#[test]
fn complete_bytes_are_not_verified_until_the_reader_checks_eof() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"ab", 2);
    setup.put_all(&upload.upload_id, b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    let lease = claim_parser(&db, &setup.actor, &upload.upload_id, 102, policy(2)).unwrap();
    let mut reader = ChunkReader::new(&db, &setup.actor, &lease).unwrap();
    reader.read_exact(&mut [0; 2]).unwrap();
    error(reader.verify_complete(), UploadError::Incomplete);
    assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0);
    reader.verify_complete().unwrap();
}

#[test]
fn intact_chunks_with_a_wrong_declared_file_digest_cannot_pass_eof() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"xx", 2);
    setup.put_all(&upload.upload_id, b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    let lease = claim_parser(&db, &setup.actor, &upload.upload_id, 102, policy(2)).unwrap();
    let mut reader = ChunkReader::new(&db, &setup.actor, &lease).unwrap();
    assert!(reader.read_to_end(&mut Vec::new()).is_err());
    error(reader.verify_complete(), UploadError::Incomplete);
    assert_eq!(count(&db, "memory_import_sessions"), 0);
}

#[test]
fn changed_chunk_body_is_rejected_even_when_its_size_is_unchanged() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"ab", 2);
    setup.put_all(&upload.upload_id, b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    let lease = claim_parser(&db, &setup.actor, &upload.upload_id, 102, policy(2)).unwrap();
    db.execute(
        "UPDATE memory_import_upload_chunks SET content=?1 WHERE upload_id=?2",
        params![b"xx".as_slice(), upload.upload_id],
    )
    .unwrap();
    let mut reader = ChunkReader::new(&db, &setup.actor, &lease).unwrap();
    assert!(reader.read_to_end(&mut Vec::new()).is_err());
}

#[test]
fn reopened_database_can_resume_upload_and_steal_only_an_expired_parse() {
    let setup = Setup::new(true);
    let upload = setup.upload(b"abcd", 2);
    setup.put_all(&upload.upload_id, b"ab", 2);
    let path = {
        let db = setup.fixture.state().core.db.get().unwrap();
        db.query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
    };
    let db = Connection::open(path).unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    assert_eq!(
        status(&db, &setup.actor, &upload.upload_id)
            .unwrap()
            .received_bytes,
        "2"
    );
    append(
        &db,
        &setup.actor,
        &upload.upload_id,
        2,
        &digest(b"cd"),
        b"cd",
        102,
    )
    .unwrap();
    let lease = claim_parser(&db, &setup.actor, &upload.upload_id, 103, policy(2)).unwrap();
    let raw =
        serde_json::to_string(&status(&db, &setup.actor, &upload.upload_id).unwrap()).unwrap();
    assert!(
        !raw.contains(&lease.token)
            && !raw.contains(&setup.actor.user_key)
            && !raw.contains("parser_token")
    );
    let replacement = claim_parser(&db, &setup.actor, &upload.upload_id, 114, policy(2)).unwrap();
    let mut reader = ChunkReader::new(&db, &setup.actor, &replacement).unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"abcd");
}
