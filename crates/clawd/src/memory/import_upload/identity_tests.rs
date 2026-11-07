use super::*;

#[test]
fn original_revoked_key_cannot_renew_or_open_a_parser_with_a_surviving_sibling() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"ab", 2);
    setup.put_all(&upload.upload_id, b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    let lease = claim_parser(&db, &setup.actor, &upload.upload_id, 102, policy(2)).unwrap();
    revoke(&db, &setup.actor, "key", &setup.other);
    let error = renew_parser(&db, &setup.actor, &lease, 103, policy(2)).unwrap_err();
    assert!(error
        .downcast_ref::<crate::repo::auth::RequestIdentityError>()
        .is_some());
    assert!(ChunkReader::new(&db, &setup.actor, &lease).is_err());
    assert_eq!(count(&db, "memory_import_upload_chunks"), 1);
}

#[test]
fn other_owner_cannot_read_append_cancel_or_claim_an_upload() {
    let setup = Setup::new(false);
    let upload = setup.upload(b"ab", 2);
    let db = setup.fixture.state().core.db.get().unwrap();
    error(
        status(&db, &setup.other, &upload.upload_id),
        UploadError::NotFound,
    );
    error(
        append(
            &db,
            &setup.other,
            &upload.upload_id,
            0,
            &digest(b"ab"),
            b"ab",
            101,
        ),
        UploadError::NotFound,
    );
    error(
        cancel(&db, &setup.other, &upload.upload_id, 101),
        UploadError::NotFound,
    );
    error(
        claim_parser(&db, &setup.other, &upload.upload_id, 101, policy(2)),
        UploadError::NotFound,
    );
    assert_eq!(
        status(&db, &setup.actor, &upload.upload_id).unwrap(),
        upload
    );
}

#[test]
fn original_key_binding_role_and_principal_are_rechecked_with_a_surviving_sibling() {
    for which in ["key", "binding", "role", "principal"] {
        let setup = Setup::new(false);
        let upload = setup.upload(b"ab", 2);
        let db = setup.fixture.state().core.db.get().unwrap();
        revoke(&db, &setup.actor, which, &setup.other);
        let error = append(
            &db,
            &setup.actor,
            &upload.upload_id,
            0,
            &digest(b"ab"),
            b"ab",
            101,
        )
        .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::repo::auth::RequestIdentityError>()
                .is_some(),
            "{which}: {error:?}"
        );
        assert!(status(&db, &setup.actor, &upload.upload_id).is_err());
        assert!(cancel(&db, &setup.actor, &upload.upload_id, 101).is_err());
        assert_eq!(count(&db, "memory_import_upload_chunks"), 0);
        assert_eq!(
            db.query_row(
                "SELECT enabled FROM auth_keys WHERE user_key=?1",
                [&setup.sibling],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
}
