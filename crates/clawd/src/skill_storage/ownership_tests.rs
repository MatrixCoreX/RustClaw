use super::*;

fn seed_retrieval_row(db: &rusqlite::Connection, user_key: &str) {
    db.execute(
        "INSERT INTO memory_retrieval_index (
            source_kind, source_ref, user_id, chat_id, user_key, memory_kind,
            search_text, metadata_json, created_at_ts, updated_at_ts
         ) VALUES (
            'kb_doc', ?1, 0, 0, ?2, 'knowledge_doc', 'manual', ?3, 1, 1
         )",
        params![
            format!("kb:{user_key}:docs:chunk-1"),
            user_key,
            serde_json::json!({
                "owner_user_key": user_key,
                "namespace": "docs"
            })
            .to_string()
        ],
    )
    .expect("retrieval row");
}

#[test]
fn rebind_moves_all_normalized_kb_rows_for_only_the_selected_user() {
    let runtime = crate::skill_storage::SkillStorageRuntime::test_default();
    let db = runtime
        .pool_for("kb")
        .expect("KB owner")
        .get()
        .expect("KB db");
    db.execute(
        "INSERT INTO kb_namespaces (
            owner_user_key, namespace, updated_at_epoch, next_chunk_seq,
            revision, parser_version, chunker_version, embedding_version
         ) VALUES ('rk-old', 'docs', 1, 2, 1, 'plain-v1', 'chars-v1', 'none')",
        [],
    )
    .expect("namespace");
    db.execute(
        "INSERT INTO kb_documents (
            owner_user_key, namespace, path, file_type, mtime_epoch,
            size_bytes, chunk_count, content_sha256, parser_version,
            chunker_version
         ) VALUES (
            'rk-old', 'docs', 'guide.md', 'markdown', 1, 6, 1,
            'document-digest', 'plain-v1', 'chars-v1'
         )",
        [],
    )
    .expect("document");
    db.execute(
        "INSERT INTO kb_chunks (
            owner_user_key, namespace, chunk_id, document_path, file_type,
            ordinal, text, text_sha256, len_tokens, mtime_epoch
         ) VALUES (
            'rk-old', 'docs', 'chunk-1', 'guide.md', 'markdown', 0,
            'manual', 'chunk-digest', 1, 1
         )",
        [],
    )
    .expect("chunk");
    db.execute(
        "INSERT INTO kb_ingest_jobs (
            owner_user_key, job_id, namespace, operation, status,
            next_file_index, total_files, payload_json, created_at_epoch,
            updated_at_epoch
         ) VALUES (
            'rk-old', 'job-1', 'docs', 'ingest', 'running', 0, 1,
            '{\"owner_user_key\":\"rk-old\",\"job_id\":\"job-1\"}', 1, 1
         )",
        [],
    )
    .expect("ingest job");
    seed_retrieval_row(&db, "rk-old");
    seed_retrieval_row(&db, "rk-other");
    drop(db);

    assert_eq!(
        runtime
            .rebind_kb_user_key("rk-old", "rk-new")
            .expect("rebind"),
        5
    );
    let db = runtime
        .pool_for("kb")
        .expect("KB owner")
        .get()
        .expect("KB db");
    for table in [
        "kb_namespaces",
        "kb_documents",
        "kb_chunks",
        "kb_ingest_jobs",
    ] {
        let moved: i64 = db
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE owner_user_key='rk-new'"),
                [],
                |row| row.get(0),
            )
            .expect("moved row count");
        assert_eq!(moved, 1, "table={table}");
    }
    let (job_payload, source_ref, metadata): (String, String, String) = db
        .query_row(
            "SELECT j.payload_json, r.source_ref, r.metadata_json
             FROM kb_ingest_jobs j
             JOIN memory_retrieval_index r ON r.user_key = j.owner_user_key
             WHERE j.owner_user_key = 'rk-new'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("rebound identities");
    assert!(job_payload.contains("\"owner_user_key\":\"rk-new\""));
    assert!(source_ref.starts_with("kb:rk-new:"));
    assert!(metadata.contains("\"owner_user_key\":\"rk-new\""));
    let untouched: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE user_key='rk-other'",
            [],
            |row| row.get(0),
        )
        .expect("untouched user");
    assert_eq!(untouched, 1);
}

#[test]
fn legacy_payload_namespaces_remain_rebindable_before_kb_skill_migration() {
    let manager = r2d2_sqlite::SqliteConnectionManager::memory();
    let pool = r2d2::Pool::builder()
        .max_size(1)
        .build(manager)
        .expect("legacy KB pool");
    let db = pool.get().expect("legacy KB db");
    db.execute_batch(
        "CREATE TABLE kb_namespaces (
            owner_user_key TEXT NOT NULL,
            namespace TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            PRIMARY KEY(owner_user_key, namespace)
         );",
    )
    .expect("legacy namespace schema");
    crate::skill_storage::schema::ensure_kb_schema(&db).expect("complete legacy support schema");
    db.execute(
        "INSERT INTO kb_namespaces
            (owner_user_key, namespace, payload_json, updated_at_epoch)
         VALUES (
            'rk-old', 'docs',
            '{\"owner_user_key\":\"rk-old\",\"namespace\":\"docs\"}', 1
         )",
        [],
    )
    .expect("legacy namespace");
    seed_retrieval_row(&db, "rk-old");
    drop(db);

    assert_eq!(
        rebind_user_key(&pool, "rk-old", "rk-new").expect("legacy rebind"),
        2
    );
    let db = pool.get().expect("legacy KB db");
    let payload: String = db
        .query_row(
            "SELECT payload_json FROM kb_namespaces WHERE owner_user_key='rk-new'",
            [],
            |row| row.get(0),
        )
        .expect("legacy rebound payload");
    assert!(payload.contains("\"owner_user_key\":\"rk-new\""));
}
