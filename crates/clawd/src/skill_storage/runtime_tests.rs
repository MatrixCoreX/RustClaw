use super::*;

#[test]
fn kb_runtime_schema_matches_the_normalized_skill_contract() {
    let runtime = SkillStorageRuntime::test_default();
    let db = runtime
        .pool_for("kb")
        .expect("KB owner")
        .get()
        .expect("KB db");
    let columns = {
        let mut stmt = db
            .prepare("PRAGMA table_info(kb_namespaces)")
            .expect("namespace columns");
        stmt.query_map([], |row| row.get::<_, String>(1))
            .expect("query columns")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect columns")
    };
    assert!(!columns.iter().any(|column| column == "payload_json"));
    assert!(columns.iter().any(|column| column == "next_chunk_seq"));
    assert!(columns.iter().any(|column| column == "embedding_version"));
    for table in ["kb_documents", "kb_chunks", "kb_ingest_jobs"] {
        let exists: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .expect("normalized table");
        assert_eq!(exists, 1, "table={table}");
    }
    let version: i64 = db
        .query_row(
            "SELECT schema_version FROM skill_storage_metadata WHERE skill_name='kb'",
            [],
            |row| row.get(0),
        )
        .expect("KB schema version");
    assert_eq!(version, schema::KB_SCHEMA_VERSION);
}

#[test]
fn kb_runtime_does_not_overwrite_a_legacy_payload_before_skill_migration() {
    let db = Connection::open_in_memory().expect("legacy KB db");
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

    schema::ensure_kb_schema(&db).expect("legacy-compatible schema");

    let has_payload: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('kb_namespaces') WHERE name='payload_json'",
            [],
            |row| row.get(0),
        )
        .expect("legacy payload column");
    let version: i64 = db
        .query_row(
            "SELECT schema_version FROM skill_storage_metadata WHERE skill_name='kb'",
            [],
            |row| row.get(0),
        )
        .expect("legacy schema version");
    assert_eq!(has_payload, 1);
    assert_eq!(version, 1);
    for table in ["kb_documents", "kb_chunks", "kb_ingest_jobs"] {
        let exists: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .expect("legacy table check");
        assert_eq!(exists, 0, "table={table}");
    }
}

#[test]
fn runtime_keeps_crypto_and_kb_in_separate_pools() {
    let runtime = SkillStorageRuntime::test_default();
    let crypto = runtime
        .pool_for("crypto")
        .expect("crypto owner")
        .get()
        .expect("crypto db");
    let kb = runtime
        .pool_for("kb")
        .expect("KB owner")
        .get()
        .expect("kb db");
    let crypto_has_credentials: i64 = crypto
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='exchange_api_credentials'",
            [],
            |row| row.get(0),
        )
        .expect("crypto schema");
    let kb_has_credentials: i64 = kb
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='exchange_api_credentials'",
            [],
            |row| row.get(0),
        )
        .expect("kb schema");
    assert_eq!(crypto_has_credentials, 1);
    assert_eq!(kb_has_credentials, 0);
}

#[test]
fn clearing_one_skill_never_removes_another_skills_rows() {
    let runtime = SkillStorageRuntime::test_default();
    runtime
        .pool_for("crypto")
        .expect("crypto owner")
        .get()
        .expect("crypto db")
        .execute(
            "INSERT INTO exchange_api_credentials
                (user_key, exchange, api_key, api_secret, enabled, updated_at)
             VALUES ('rk-user', 'okx', 'key', 'secret', 1, '1')",
            [],
        )
        .expect("seed crypto");
    runtime
        .pool_for("kb")
        .expect("KB owner")
        .get()
        .expect("KB db")
        .execute(
            "INSERT INTO kb_namespaces
                (owner_user_key, namespace, updated_at_epoch, next_chunk_seq,
                 revision, parser_version, chunker_version, embedding_version)
             VALUES ('rk-user', 'docs', 1, 1, 1, 'plain-v1', 'chars-v1', 'none')",
            [],
        )
        .expect("seed KB");

    let removed = runtime
        .clear_skill_data("crypto", "sqlite")
        .expect("clear crypto");

    assert!(removed.data_present_before);
    assert_eq!(removed.rows_deleted, 1);
    assert_eq!(
        runtime
            .data_state("crypto", "sqlite")
            .expect("crypto state"),
        "empty"
    );
    assert_eq!(
        runtime.data_state("kb", "sqlite").expect("KB state"),
        "present"
    );
}

#[test]
fn clearing_directory_storage_removes_only_the_selected_skill_directory() {
    let runtime = SkillStorageRuntime::test_default();
    let selected = runtime
        .directory_path("media_download")
        .expect("selected skill directory");
    let neighbor = runtime
        .directory_path("another_skill")
        .expect("neighbor skill directory");
    std::fs::create_dir_all(selected.join("modelscope/snapshot"))
        .expect("selected nested directory");
    std::fs::write(selected.join("modelscope/snapshot/model.bin"), b"model")
        .expect("selected private file");
    std::fs::write(neighbor.join("keep.bin"), b"keep").expect("neighbor private file");

    assert_eq!(
        runtime
            .data_state("media_download", "directory")
            .expect("selected state"),
        "present"
    );
    let removed = runtime
        .clear_skill_data("media_download", "directory")
        .expect("clear selected directory");
    assert!(removed.data_present_before);
    assert_eq!(removed.files_deleted, 1);
    assert!(!selected.exists());
    assert_eq!(
        std::fs::read(neighbor.join("keep.bin")).expect("neighbor remains"),
        b"keep"
    );
}

#[test]
fn clearing_unowned_sqlite_storage_preserves_non_database_files() {
    let runtime = SkillStorageRuntime::test_default();
    let directory = runtime
        .directory_path("sqlite_fixture")
        .expect("sqlite fixture directory");
    std::fs::write(directory.join("state.db"), b"database").expect("sqlite database");
    std::fs::write(directory.join("keep.bin"), b"keep").expect("neighbor file");

    assert_eq!(
        runtime
            .data_state("sqlite_fixture", "sqlite")
            .expect("sqlite state"),
        "present"
    );
    let removed = runtime
        .clear_skill_data("sqlite_fixture", "sqlite")
        .expect("clear sqlite data");
    assert_eq!(removed.files_deleted, 1);
    assert_eq!(
        runtime
            .data_state("sqlite_fixture", "sqlite")
            .expect("cleared sqlite state"),
        "empty"
    );
    assert_eq!(
        std::fs::read(directory.join("keep.bin")).expect("non-database file survives"),
        b"keep"
    );
}
