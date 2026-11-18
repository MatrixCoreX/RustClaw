use super::*;

#[test]
fn one_whole_preview_and_confirmation_publish_no_early_canonical_rows() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let (upload, p) = preview(
        &db,
        &env.actor,
        vec![item("first", "fact"), item("second", "preference")],
    );
    assert_eq!(p.accepted_items, 2);
    assert_eq!(count(&db, "memory_facts"), 0);
    assert_eq!(count(&db, "memory_import_sessions"), 1);
    assert_eq!(staged_count(&db, &upload.upload_id), 2);
    let saved: String = db
        .query_row(
            "SELECT preview_json FROM memory_import_sessions WHERE import_id=?1",
            [&p.import_id],
            |r| r.get(0),
        )
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(metadata["schema_version"], 2);
    assert!(metadata.get("items").is_none());
    let bytes: i64 = db
        .query_row(
            "SELECT COALESCE(SUM(LENGTH(content)),0) FROM memory_import_upload_chunks
        WHERE upload_id=?1",
            [&upload.upload_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bytes, 0);
    assert_eq!(confirm(&db, &env.actor, &p).unwrap().imported_items, 2);
    assert_eq!(count(&db, "memory_facts"), 1);
    assert_eq!(staged_count(&db, &upload.upload_id), 0);
    assert_eq!(
        status(&db, &env.actor, &upload.upload_id).unwrap().state,
        UploadState::Confirmed
    );
    assert!(confirm(&db, &env.actor, &p).is_err());
}

#[test]
fn staged_preview_matches_legacy_logical_digest_and_reuses_its_receipt() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let records = vec![
        item("legacy first", "fact"),
        item("legacy second", "preference"),
    ];
    let old = ux::preview_memory_import(
        &db,
        &env.actor.principal_id,
        &MemoryImportPreviewRequest {
            export: export(records.clone()),
        },
        100,
    )
    .unwrap();
    let (upload, new) = preview(&db, &env.actor, records);
    assert_eq!(old, new);
    assert_eq!(count(&db, "memory_import_sessions"), 1);
    assert_eq!(confirm(&db, &env.actor, &new).unwrap().imported_items, 2);
    assert_eq!(
        status(&db, &env.actor, &upload.upload_id).unwrap().state,
        UploadState::Confirmed
    );
    assert_eq!(staged_count(&db, &upload.upload_id), 0);
}

#[test]
fn legacy_preview_reuses_a_staged_receipt_and_the_same_atomic_operation() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let records = vec![
        item("stream first", "fact"),
        item("stream second", "recent"),
    ];
    let (upload, new) = preview(&db, &env.actor, records.clone());
    let old = ux::preview_memory_import(
        &db,
        &env.actor.principal_id,
        &MemoryImportPreviewRequest {
            export: export(records),
        },
        100,
    )
    .unwrap();
    assert_eq!(old, new);
    assert_eq!(confirm(&db, &env.actor, &old).unwrap().imported_items, 2);
    assert_eq!(count(&db, "memory_facts"), 2);
    assert_eq!(
        status(&db, &env.actor, &upload.upload_id).unwrap().state,
        UploadState::Confirmed
    );
}

#[test]
fn finalize_and_last_chunk_retries_do_not_duplicate_a_whole_preview() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let raw = serde_json::to_vec(&export(vec![item("retry safely", "fact")])).unwrap();
    let upload = upload_raw(&db, &env.actor, &raw);
    let first = parse_and_preview(&db, &env.actor, &upload.upload_id, parse_policy(), 102).unwrap();
    assert_eq!(
        parse_and_preview(&db, &env.actor, &upload.upload_id, parse_policy(), 103).unwrap(),
        first
    );
    append(
        &db,
        &env.actor,
        &upload.upload_id,
        0,
        &digest(&raw),
        &raw,
        103,
    )
    .unwrap();
    assert_eq!(count(&db, "memory_import_sessions"), 1);
    assert_eq!(staged_count(&db, &upload.upload_id), 1);
    assert_eq!(confirm(&db, &env.actor, &first).unwrap().imported_items, 1);
}

#[test]
fn original_large_unicode_shape_roundtrips_core_without_claiming_http_repair() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let records = (0..48)
        .map(|i| item(&format!("{i:02}{}", "文".repeat(17_998)), "fact"))
        .collect::<Vec<_>>();
    let expected = records
        .iter()
        .map(|r| r.content.clone())
        .collect::<Vec<_>>();
    let raw = serde_json::to_vec(&export(records)).unwrap();
    assert!(raw.len() > 2 * 1024 * 1024);
    let upload = upload_raw(&db, &env.actor, &raw);
    let p = parse_and_preview(&db, &env.actor, &upload.upload_id, parse_policy(), 102).unwrap();
    assert_eq!(p.accepted_items, 48);
    assert_eq!(count(&db, "memory_facts"), 0);
    assert_eq!(confirm(&db, &env.actor, &p).unwrap().imported_items, 48);
    let mut query = db
        .prepare("SELECT fact_text FROM memory_facts WHERE principal_id=?1 ORDER BY fact_text")
        .unwrap();
    let actual = query
        .query_map([&env.actor.principal_id], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn larger_unicode_record_remains_exact_within_the_advertised_resource_budget() {
    let env = Setup::new(false);
    let db = env.fixture.state().core.db.get().unwrap();
    let text = "字".repeat(32_769);
    let (_, p) = preview(&db, &env.actor, vec![item(&text, "fact")]);
    assert_eq!(p.accepted_items, 1);
    assert_eq!(confirm(&db, &env.actor, &p).unwrap().imported_items, 1);
    let actual: String = db
        .query_row("SELECT fact_text FROM memory_facts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(actual, text);
}
