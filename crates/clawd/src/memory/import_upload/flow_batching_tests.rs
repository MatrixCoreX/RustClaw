use super::*;

#[test]
fn more_than_ten_thousand_records_share_one_preview_and_one_confirmation() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let (upload, p) = preview(
        &db,
        &env.actor,
        (0..10_001)
            .map(|_| item("one logical duplicate", "fact"))
            .collect(),
    );
    assert_eq!(p.accepted_items, 10_001);
    assert_eq!(count(&db, "memory_facts"), 0);
    assert_eq!(count(&db, "memory_import_sessions"), 1);
    assert_eq!(staged_count(&db, &upload.upload_id), 10_001);
    let outcome = confirm(&db, &env.actor, &p).unwrap();
    assert_eq!(outcome.imported_items, 1);
    assert_eq!(outcome.existing_items, 10_000);
    assert_eq!(count(&db, "memory_facts"), 1);
    let usage: (i64, i64) = db
        .query_row(
            "SELECT used_rows,used_bytes FROM memory_principal_quotas WHERE principal_id=?1",
            [&env.actor.principal_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(usage, (0, 0));
}

#[test]
fn later_batch_failure_discards_provisional_rows_but_keeps_the_whole_raw_upload() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let raw = serde_json::to_vec(&export(
        (0..1001).map(|_| item("batch recovery", "fact")).collect(),
    ))
    .unwrap();
    let upload = upload_raw(&db, &env.actor, &raw);
    db.execute_batch(
        "CREATE TEMP TABLE fixture_successful_items(item_index INTEGER);
        CREATE TEMP TRIGGER fixture_stage_evidence AFTER INSERT ON memory_import_upload_items
        BEGIN INSERT INTO fixture_successful_items VALUES(NEW.item_index); END;
        CREATE TEMP TRIGGER fixture_later_batch BEFORE INSERT ON memory_import_upload_items
        WHEN NEW.item_index>=1000 BEGIN SELECT RAISE(ABORT,'fixture_later_batch'); END;",
    )
    .unwrap();
    assert!(parse_and_preview(&db, &env.actor, &upload.upload_id, parse_policy(), 102).is_err());
    let committed: i64 = db
        .query_row("SELECT COUNT(*) FROM fixture_successful_items", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(committed > 0 && committed < 1001);
    assert_eq!(staged_count(&db, &upload.upload_id), 0);
    assert_eq!(count(&db, "memory_facts"), 0);
    assert_eq!(count(&db, "memory_import_sessions"), 0);
    assert_eq!(
        status(&db, &env.actor, &upload.upload_id).unwrap().state,
        UploadState::Uploading
    );
    let bytes: i64 = db
        .query_row(
            "SELECT SUM(LENGTH(content)) FROM memory_import_upload_chunks WHERE upload_id=?1",
            [&upload.upload_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bytes, raw.len() as i64);
    db.execute_batch("DROP TRIGGER fixture_later_batch; DROP TRIGGER fixture_stage_evidence; DROP TABLE fixture_successful_items").unwrap();
    let p = parse_and_preview(&db, &env.actor, &upload.upload_id, parse_policy(), 103).unwrap();
    assert_eq!(p.accepted_items, 1001);
}

#[test]
fn damaged_raw_blob_larger_than_its_pinned_chunk_budget_is_rejected_before_delivery() {
    use std::io::Read;
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let raw = serde_json::to_vec(&export(vec![item("bounded raw body", "fact")])).unwrap();
    let upload = upload_raw(&db, &env.actor, &raw);
    let lease = claim_parser(&db, &env.actor, &upload.upload_id, 102, policy(64 * 1024)).unwrap();
    db.execute(
        "UPDATE memory_import_upload_chunks SET byte_count=4194304,content=zeroblob(4194304)
        WHERE upload_id=?1",
        [&upload.upload_id],
    )
    .unwrap();
    let mut reader = ChunkReader::new(&db, &env.actor, &lease).unwrap();
    let error = anyhow::Error::new(reader.read(&mut [0u8; 1]).unwrap_err());
    assert!(error
        .chain()
        .any(|cause| cause.downcast_ref::<UploadError>() == Some(&UploadError::Corrupt)));
    assert_eq!(count(&db, "memory_facts"), 0);
}
