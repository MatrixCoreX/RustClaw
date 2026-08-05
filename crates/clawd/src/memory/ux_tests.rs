use rusqlite::{params, Connection};

use super::*;

fn setup_db() -> (Connection, String) {
    let db = Connection::open_in_memory().expect("fixture db");
    db.execute_batch(crate::INIT_SQL).expect("base schema");
    crate::db_init::ensure_schedule_schema(&db).expect("schedule schema");
    crate::db_init::ensure_memory_schema(&db).expect("memory schema");
    crate::db_init::ensure_channel_schema(&db).expect("channel schema");
    crate::repo::auth::ensure_key_auth_schema(&db).expect("auth schema");
    crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
    db.execute(
        "INSERT INTO auth_keys(user_key, role, enabled, created_at)
         VALUES ('ux-key', 'user', 1, '1')",
        [],
    )
    .expect("auth key");
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("principal schema");
    crate::repo::ensure_principal_ownership_schema(&db).expect("ownership schema");
    ensure_memory_ux_schema(&db).expect("ux schema");
    let principal_id = db
        .query_row(
            "SELECT principal_id FROM auth_keys WHERE user_key = 'ux-key'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("principal id");
    (db, principal_id)
}

fn insert_fact(db: &Connection, principal_id: &str, id: &str, content: &str, ts: i64) {
    db.execute(
        "INSERT INTO memory_facts(
            memory_id, user_id, chat_id, user_key, principal_id, scope_kind, scope_ref,
            namespace, fact_key, fact_value, fact_text, source_kind, source_ref,
            created_at_ts, updated_at_ts, status, origin, row_revision, legacy_scope_inferred,
            source_refs_json, evidence_refs_json, trust_tier, sensitivity,
            last_verified_at_ts, modified_at_ts
         ) VALUES (?1, 1, 2, 'ux-key', ?2, 'principal', ?2, 'user_profile',
                   'language', ?3, ?3, 'user_message', 'fixture', ?4, ?4, 'active',
                   'user_confirmed', 1, 0, '[]', '[\"evidence\"]', 'user_confirmed',
                   'normal', ?4, ?4)",
        params![id, principal_id, content, ts],
    )
    .expect("insert fact");
}

#[test]
fn paged_filter_never_returns_another_principal_or_raw_database_id() {
    let (db, principal_id) = setup_db();
    insert_fact(&db, &principal_id, "memory-visible", "Use Chinese", 100);
    insert_fact(&db, "principal-other", "memory-hidden", "Do not leak", 101);
    let result = list_memory_page(
        &db,
        &principal_id,
        &MemoryListFilter {
            search: Some("Chinese".to_string()),
            page_size: Some(10),
            ..MemoryListFilter::default()
        },
        110,
    )
    .expect("list page");
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].id, "memory-visible");
    assert!(serde_json::to_string(&result)
        .expect("serialize")
        .find("raw_id")
        .is_none());
}

#[test]
fn correction_uses_revision_cas_and_supersedes_the_old_fact() {
    let (db, principal_id) = setup_db();
    insert_fact(&db, &principal_id, "memory-old", "Use English", 100);
    let corrected = correct_memory(
        &db,
        &principal_id,
        &principal_id,
        "memory-old",
        &MemoryCorrectionRequest {
            expected_revision: 1,
            content: "Use Chinese".to_string(),
        },
        200,
    )
    .expect("correct");
    assert_eq!(corrected.status, "corrected");
    assert!(correct_memory(
        &db,
        &principal_id,
        &principal_id,
        "memory-old",
        &MemoryCorrectionRequest {
            expected_revision: 1,
            content: "stale write".to_string(),
        },
        201,
    )
    .unwrap_err()
    .to_string()
    .contains("memory_revision_conflict"));
    let old_status: String = db
        .query_row(
            "SELECT status FROM memory_facts WHERE memory_id = 'memory-old'",
            [],
            |row| row.get(0),
        )
        .expect("old status");
    assert_eq!(old_status, "superseded");
}

#[test]
fn deletion_removes_recall_rows_and_scrubs_undo_snapshot_after_grace() {
    let (db, principal_id) = setup_db();
    insert_fact(&db, &principal_id, "memory-delete", "private fact", 100);
    let result = delete_memory_with_revision(
        &db,
        &principal_id,
        "memory-delete",
        &MemoryMutationRequest {
            expected_revision: 1,
        },
        200,
    )
    .expect("delete");
    assert_eq!(result.status, "deleted");
    let remaining: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_facts WHERE memory_id = 'memory-delete'",
            [],
            |row| row.get(0),
        )
        .expect("remaining");
    assert_eq!(remaining, 0);
    assert_eq!(scrub_expired_deletion_grace(&db, 501).expect("scrub"), 1);
    let snapshot: String = db
        .query_row(
            "SELECT previous_snapshot_json FROM memory_revisions WHERE memory_id = 'memory-delete'",
            [],
            |row| row.get(0),
        )
        .expect("revision snapshot");
    assert!(!snapshot.contains("private fact"));
}

#[test]
fn deletion_can_be_undone_during_grace_but_not_after_scrub() {
    let (db, principal_id) = setup_db();
    insert_fact(
        &db,
        &principal_id,
        "memory-undo-delete",
        "recoverable fact",
        100,
    );
    let deleted = delete_memory_with_revision(
        &db,
        &principal_id,
        "memory-undo-delete",
        &MemoryMutationRequest {
            expected_revision: 1,
        },
        200,
    )
    .expect("delete");
    let restored = undo_memory_mutation(
        &db,
        &principal_id,
        &MemoryUndoRequest {
            revision_id: deleted.revision_id.expect("delete revision"),
        },
        300,
    )
    .expect("undo delete");
    assert_eq!(restored.status, "restored");
    let content: String = db
        .query_row(
            "SELECT fact_text FROM memory_facts WHERE memory_id = 'memory-undo-delete'",
            [],
            |row| row.get(0),
        )
        .expect("restored fact");
    assert_eq!(content, "recoverable fact");

    let deleted_again = delete_memory_with_revision(
        &db,
        &principal_id,
        "memory-undo-delete",
        &MemoryMutationRequest {
            expected_revision: restored.revision,
        },
        400,
    )
    .expect("delete again");
    scrub_expired_deletion_grace(&db, 701).expect("scrub grace");
    let error = undo_memory_mutation(
        &db,
        &principal_id,
        &MemoryUndoRequest {
            revision_id: deleted_again.revision_id.expect("second delete revision"),
        },
        702,
    )
    .expect_err("scrubbed snapshot cannot be restored");
    assert!(error.to_string().contains("memory_undo_expired"));
}

#[test]
fn correction_can_be_undone_without_last_write_wins() {
    let (db, principal_id) = setup_db();
    insert_fact(
        &db,
        &principal_id,
        "memory-correction-undo",
        "Use English",
        100,
    );
    let corrected = correct_memory(
        &db,
        &principal_id,
        &principal_id,
        "memory-correction-undo",
        &MemoryCorrectionRequest {
            expected_revision: 1,
            content: "Use Chinese".to_string(),
        },
        200,
    )
    .expect("correct");
    undo_memory_mutation(
        &db,
        &principal_id,
        &MemoryUndoRequest {
            revision_id: corrected.revision_id.expect("correction revision"),
        },
        250,
    )
    .expect("undo correction");
    let (content, status): (String, String) = db
        .query_row(
            "SELECT fact_text, status FROM memory_facts
             WHERE memory_id = 'memory-correction-undo'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("old fact active");
    assert_eq!(
        (content.as_str(), status.as_str()),
        ("Use English", "active")
    );
    let replacement_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_facts WHERE supersedes_memory_id = 'memory-correction-undo'",
            [],
            |row| row.get(0),
        )
        .expect("replacement count");
    assert_eq!(replacement_count, 0);
}

#[test]
fn import_requires_checksum_preview_and_downgrades_scope_and_trust() {
    let (db, principal_id) = setup_db();
    insert_fact(
        &db,
        &principal_id,
        "memory-exported",
        "safe imported fact",
        100,
    );
    let export = export_memory(&db, &principal_id, 200).expect("export");
    let preview = preview_memory_import(
        &db,
        &principal_id,
        &MemoryImportPreviewRequest {
            export: export.clone(),
        },
        201,
    )
    .expect("preview");
    assert_eq!(preview.scope_kind, "principal");
    assert_eq!(preview.trust_tier, "imported_legacy");
    assert_eq!(preview.duplicate_items, 1);

    let other_principal = "principal-import-target";
    db.execute(
        "INSERT INTO principals(principal_id, role, status, revision, created_at, updated_at)
         VALUES (?1, 'user', 'active', 1, '1', '1')",
        [other_principal],
    )
    .expect("target principal");
    db.execute(
        "INSERT INTO auth_keys(user_key, role, enabled, created_at, principal_id)
         VALUES ('import-target-key', 'user', 1, '1', ?1)",
        [other_principal],
    )
    .expect("target key");
    let preview = preview_memory_import(
        &db,
        other_principal,
        &MemoryImportPreviewRequest {
            export: export.clone(),
        },
        202,
    )
    .expect("target preview");
    let result = confirm_memory_import(
        &db,
        other_principal,
        &MemoryImportConfirmRequest {
            import_id: preview.import_id,
            expected_payload_digest: preview.payload_digest,
        },
        203,
    )
    .expect("confirm import");
    assert_eq!(result.imported_items, 1);
    let (scope, trust, origin): (String, String, String) = db
        .query_row(
            "SELECT scope_kind, trust_tier, origin FROM memory_facts
             WHERE principal_id = ?1 AND source_kind = 'memory_import'",
            [other_principal],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("imported fact");
    assert_eq!(
        (scope.as_str(), trust.as_str(), origin.as_str()),
        ("principal", "imported_legacy", "imported_legacy")
    );

    let mut malformed = export;
    malformed.items[0].content = "tampered".to_string();
    let error = preview_memory_import(
        &db,
        other_principal,
        &MemoryImportPreviewRequest { export: malformed },
        204,
    )
    .expect_err("checksum mismatch");
    assert!(error.to_string().contains("memory_import_checksum_invalid"));
}

#[test]
fn bulk_clear_uses_preview_cas_and_consistency_checker_repairs_orphans() {
    let (db, principal_id) = setup_db();
    insert_fact(&db, &principal_id, "memory-clear", "clear me", 100);
    let preview = clear_preview(&db, &principal_id, "transcript_and_derived").expect("preview");
    let stale = clear_with_mode(
        &db,
        &principal_id,
        &MemoryBulkClearRequest {
            mode: "transcript_and_derived".to_string(),
            expected_transcript_rows: preview.transcript_rows,
            expected_derived_rows: preview.derived_rows + 1,
        },
        200,
    )
    .expect_err("stale preview");
    assert!(stale.to_string().contains("memory_clear_preview_conflict"));

    db.execute(
        "INSERT INTO memory_retrieval_index(
            principal_id, user_key, user_id, chat_id, source_kind, source_ref,
            source_memory_id, memory_kind, role, search_text, topic_tags,
            vector_json, embedding_model, embedding_dims, embedding_version,
            salience, created_at_ts, updated_at_ts, scope_kind, scope_ref
         ) VALUES (?1, 'ux-key', 1, 2, 'memory', 'orphan', 999999,
                   'episodic_event', 'user', 'orphan', 'orphan', '[]',
                   'local-hash-v1', 24, 'local-hash-v1', 0.5, 1, 1,
                   'principal', ?1)",
        [principal_id.as_str()],
    )
    .expect("orphan index");
    let before = check_memory_consistency(&db, &principal_id, false, 200).expect("audit");
    assert_eq!(before.orphan_retrieval_rows, 1);
    let repaired = check_memory_consistency(&db, &principal_id, true, 201).expect("repair");
    assert_eq!(repaired.repaired_rows, 1);
    let after = check_memory_consistency(&db, &principal_id, false, 202).expect("recheck");
    assert_eq!(after.orphan_retrieval_rows, 0);
}
