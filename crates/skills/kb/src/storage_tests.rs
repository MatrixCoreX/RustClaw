use super::database::{
    integrity_check, storage_schema_is_current, table_exists, table_has_column,
    SCHEMA_V6_CUTOVER_MIGRATION_ID,
};
use super::namespace_index::insert_retrieval_row;
use super::retrieval::semantic_candidate_scores;
use super::test_support::rebuild_ann_projection;
use super::vector_index::verify_ann_projection;
use super::*;
use crate::{Chunk, DocMeta, KbRuntime, NamespaceIndex};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

fn runtime(root: &Path, user_key: &str) -> KbRuntime {
    fs::create_dir_all(root).expect("create KB test workspace");
    KbRuntime {
        scope_user_key: user_key.to_string(),
        workspace_root: root.to_path_buf(),
        storage_database_path: root.join("data/skills/kb/state.db"),
        storage_busy_timeout_ms: 5_000,
        storage_resource_profile: "standard".to_string(),
        storage_cache_size_kib: None,
        storage_mmap_size_bytes: None,
        storage_temp_store: None,
        path_policy: skill_sdk::SkillPathPolicy::new(root, None)
            .expect("create KB test path policy"),
    }
}

fn snapshot(owner: &str, namespace: &str, documents: &[(&str, &str)]) -> NamespaceIndex {
    let mut docs = HashMap::new();
    let mut chunks = Vec::new();
    for (ordinal, (path, text)) in documents.iter().enumerate() {
        let chunk_id = format!("{namespace}-{}", ordinal + 1);
        docs.insert(
            (*path).to_string(),
            DocMeta {
                path: (*path).to_string(),
                file_type: "md".to_string(),
                mtime_epoch: 10 + ordinal as i64,
                size: text.len() as u64,
                chunks: 1,
                content_sha256: crate::sha256_hex(text.as_bytes()),
                parser_version: crate::default_parser_version(),
                chunker_version: crate::default_chunker_version(),
            },
        );
        chunks.push(Chunk {
            chunk_id,
            path: (*path).to_string(),
            file_type: "md".to_string(),
            offset: 0,
            text: (*text).to_string(),
            len_tokens: 1,
            mtime_epoch: 10 + ordinal as i64,
            text_sha256: crate::sha256_hex(text.as_bytes()),
            content_kind: "plain_text".to_string(),
            location: crate::ingest_extract::DocumentLocation::default(),
        });
    }
    NamespaceIndex {
        namespace: namespace.to_string(),
        owner_user_key: owner.to_string(),
        updated_at_epoch: 20,
        next_chunk_seq: documents.len() as u64 + 1,
        revision: 0,
        parser_version: crate::default_parser_version(),
        chunker_version: crate::default_chunker_version(),
        embedding_version: crate::default_embedding_version(),
        docs,
        chunks,
    }
}

#[test]
fn sqlite_resource_contract_is_applied_to_skill_owned_connections() {
    let root = std::env::temp_dir().join(format!(
        "kb-resource-profile-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let mut runtime = runtime(&root, "user:resource-profile");
    runtime.storage_resource_profile = "constrained".to_string();
    runtime.storage_cache_size_kib = Some(384);
    runtime.storage_mmap_size_bytes = Some(0);
    runtime.storage_temp_store = Some("file".to_string());

    let db = open(&runtime).expect("open profiled KB storage");
    let cache_size: i64 = db
        .query_row("PRAGMA cache_size", [], |row| row.get(0))
        .expect("cache size");
    let mmap_size: i64 = db
        .query_row("PRAGMA mmap_size", [], |row| row.get(0))
        .unwrap_or(0);
    let temp_store: i64 = db
        .query_row("PRAGMA temp_store", [], |row| row.get(0))
        .expect("temp store");
    let busy_timeout: i64 = db
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .expect("busy timeout");
    let journal_mode: String = db
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode");
    let synchronous: i64 = db
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .expect("synchronous mode");
    let foreign_keys: i64 = db
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("foreign keys");
    let wal_autocheckpoint: i64 = db
        .query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))
        .expect("WAL auto-checkpoint");
    let journal_size_limit: i64 = db
        .query_row("PRAGMA journal_size_limit", [], |row| row.get(0))
        .expect("journal size limit");
    let auto_vacuum: i64 = db
        .query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
        .expect("auto vacuum mode");

    assert_eq!(cache_size, -384);
    assert_eq!(mmap_size, 0);
    assert_eq!(temp_store, 1);
    assert_eq!(busy_timeout, 5_000);
    assert_eq!(journal_mode, "wal");
    assert_eq!(synchronous, 1);
    assert_eq!(foreign_keys, 1);
    assert_eq!(wal_autocheckpoint, 64);
    assert_eq!(journal_size_limit, 4 * 1024 * 1024);
    assert_eq!(auto_vacuum, 2);
    drop(db);
    std::fs::remove_dir_all(root).expect("remove KB profile fixture");
}

#[test]
fn current_schema_fast_path_requires_the_complete_v7_projection_set() {
    let root = std::env::temp_dir().join(format!(
        "kb-schema-v7-sentinel-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let runtime = runtime(&root, "user:schema-sentinel");
    initialize(&runtime).expect("initialize complete schema");
    let db = open(&runtime).expect("open complete schema");
    assert!(storage_schema_is_current(&db).expect("inspect complete schema"));
    db.execute("DROP TABLE kb_vector_ann_projection", [])
        .expect("remove disposable projection table");
    assert!(!storage_schema_is_current(&db).expect("inspect partial schema"));
    drop(db);

    initialize(&runtime).expect("repair partial host bootstrap");
    let db = open(&runtime).expect("reopen repaired schema");
    assert!(storage_schema_is_current(&db).expect("inspect repaired schema"));
    drop(db);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn structured_chunk_location_round_trips_through_private_storage() {
    let root = std::env::temp_dir().join(format!(
        "kb-structured-location-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let runtime = runtime(&root, "user:location");
    let mut index = snapshot(
        "user:location",
        "manuals",
        &[("guide.pdf", "Page-specific evidence")],
    );
    index.chunks[0].file_type = "pdf".to_string();
    index.chunks[0].content_kind = "pdf_text_layer".to_string();
    index.chunks[0].location = crate::ingest_extract::DocumentLocation {
        page: Some(7),
        section_path: vec!["Operations".to_string()],
        ..crate::ingest_extract::DocumentLocation::default()
    };
    index.docs.get_mut("guide.pdf").expect("document").file_type = "pdf".to_string();

    save_namespace(&runtime, &index).expect("persist structured location");
    let loaded = load_namespace(&runtime, "manuals").expect("load structured location");

    assert_eq!(loaded.chunks[0].content_kind, "pdf_text_layer");
    assert_eq!(loaded.chunks[0].location.page, Some(7));
    assert_eq!(loaded.chunks[0].location.section_path, ["Operations"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bounded_compaction_is_persistently_throttled_and_reclaims_limited_pages() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-bounded-compaction-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "user:maintenance");
    initialize(&runtime).expect("initialize maintenance database");
    let mut db = open(&runtime).expect("open maintenance fixture");
    db.execute_batch(
        "CREATE TABLE maintenance_fixture(payload BLOB NOT NULL);
         UPDATE kb_storage_maintenance SET last_run_epoch = 0
         WHERE task_name = 'bounded_compaction';",
    )
    .expect("prepare maintenance fixture");
    let tx = db.transaction().expect("start fixture write");
    for _ in 0..128 {
        tx.execute(
            "INSERT INTO maintenance_fixture(payload) VALUES (zeroblob(8192))",
            [],
        )
        .expect("insert fixture page");
    }
    tx.execute("DELETE FROM maintenance_fixture", [])
        .expect("free fixture pages");
    tx.commit().expect("commit free pages");
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint fixture pages");
    let before: i64 = db
        .query_row("PRAGMA freelist_count", [], |row| row.get(0))
        .expect("freelist before maintenance");
    assert!(before > 0);
    drop(db);

    initialize(&runtime).expect("run bounded maintenance");
    let db = open(&runtime).expect("reopen maintained database");
    let after: i64 = db
        .query_row("PRAGMA freelist_count", [], |row| row.get(0))
        .expect("freelist after maintenance");
    let (last_run, last_result): (i64, String) = db
        .query_row(
            "SELECT last_run_epoch, last_result FROM kb_storage_maintenance
             WHERE task_name = 'bounded_compaction'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("maintenance checkpoint");
    assert!(after < before);
    assert!(last_run > 0);
    assert_eq!(last_result, "ok");
    drop(db);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn wal_crash_writer_helper() {
    let Some(root) = std::env::var_os("KB_WAL_CRASH_TEST_ROOT") else {
        return;
    };
    let runtime = runtime(Path::new(&root), "rk-owner");
    initialize(&runtime).expect("initialize crash fixture");
    let committed = snapshot("rk-owner", "committed", &[("guide.md", "durable")]);
    save_namespace(&runtime, &committed).expect("commit durable namespace");

    let mut db = open(&runtime).expect("open uncommitted writer");
    ensure_schema(&mut db).expect("ensure crash schema");
    let tx = db.transaction().expect("start uncommitted transaction");
    tx.execute(
        "INSERT INTO kb_namespaces (
            owner_user_key, namespace, updated_at_epoch, next_chunk_seq, revision,
            parser_version, chunker_version, embedding_version
         ) VALUES (?1, 'uncommitted', 1, 1, 1, 'parser', 'chunker', 'embedding')",
        params![runtime.scope_user_key],
    )
    .expect("stage uncommitted namespace");
    std::process::exit(73);
}

#[test]
fn wal_recovers_committed_state_and_discards_interrupted_transaction() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-wal-recovery-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    let current_exe = std::env::current_exe().expect("current test executable");
    let status = Command::new(current_exe)
        .arg("--exact")
        .arg("storage::tests::wal_crash_writer_helper")
        .arg("--nocapture")
        .env("KB_WAL_CRASH_TEST_ROOT", &root)
        .status()
        .expect("run abrupt WAL writer");
    assert_eq!(status.code(), Some(73));

    let runtime = runtime(&root, "rk-owner");
    initialize(&runtime).expect("recover and validate KB storage");
    assert_eq!(
        load_namespace(&runtime, "committed")
            .expect("load committed namespace after recovery")
            .docs
            .len(),
        1
    );
    assert!(!namespace_exists(&runtime, "uncommitted").expect("check rolled-back namespace"));
    let db = open(&runtime).expect("open recovered database");
    integrity_check(&db).expect("recovered database integrity");
    let checkpoint: (i64, i64, i64) = db
        .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("checkpoint recovered WAL");
    assert_eq!(checkpoint.0, 0, "checkpoint must not be busy");
    assert!(checkpoint.2 <= checkpoint.1);
    drop(db);
    let _ = fs::remove_dir_all(root);
}

fn explain_query_plan(db: &Connection, sql: &str, values: impl rusqlite::Params) -> Vec<String> {
    let mut stmt = db
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare query plan");
    stmt.query_map(values, |row| row.get::<_, String>(3))
        .expect("execute query plan")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect query plan")
}

fn assert_plan_uses(plan: &[String], index: &str) {
    assert!(
        plan.iter().any(|detail| detail.contains(index)),
        "query plan did not use {index}: {plan:?}"
    );
}

#[test]
fn common_kb_queries_use_scope_filter_revision_digest_and_updated_indexes() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-query-plans-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "rk-owner");
    save_namespace(
        &runtime,
        &snapshot(
            "rk-owner",
            "docs",
            &[
                ("guide.md", "indexed document"),
                ("notes.txt", "other text"),
            ],
        ),
    )
    .expect("save query-plan fixture");
    let db = open(&runtime).expect("open query-plan database");

    let updated_plan = explain_query_plan(
        &db,
        "SELECT namespace FROM kb_namespaces
         WHERE owner_user_key = ?1
         ORDER BY updated_at_epoch DESC, namespace ASC",
        params![runtime.scope_user_key],
    );
    assert_plan_uses(&updated_plan, "idx_kb_namespaces_owner_updated");

    let digest = crate::sha256_hex(b"indexed document");
    let digest_plan = explain_query_plan(
        &db,
        "SELECT path FROM kb_documents
         WHERE owner_user_key = ?1 AND namespace = ?2 AND content_sha256 = ?3
         ORDER BY path",
        params![runtime.scope_user_key, "docs", digest],
    );
    assert_plan_uses(&digest_plan, "idx_kb_documents_owner_namespace_digest");

    let filter_plan = explain_query_plan(
        &db,
        "SELECT chunk_id FROM kb_chunks
         WHERE owner_user_key = ?1 AND namespace = ?2 AND file_type = ?3
           AND mtime_epoch >= ?4 AND mtime_epoch <= ?5
         ORDER BY mtime_epoch, document_path, chunk_id",
        params![runtime.scope_user_key, "docs", "md", 0_i64, 100_i64],
    );
    assert_plan_uses(&filter_plan, "idx_kb_chunks_owner_namespace_filter");

    let generation = start_vector_reindex(&runtime, "docs").expect("start vector generation");
    let resume_plan = explain_query_plan(
        &db,
        "SELECT generation FROM kb_vector_generations
         WHERE owner_user_key = ?1 AND namespace = ?2
           AND state = 'building' AND profile_id = ?3
           AND namespace_revision = ?4 AND source_digest = ?5
         ORDER BY generation DESC LIMIT 1",
        params![
            runtime.scope_user_key,
            "docs",
            generation.profile_id,
            generation.namespace_revision as i64,
            generation.source_digest,
        ],
    );
    assert_plan_uses(&resume_plan, "idx_kb_vector_generations_resume");

    drop(db);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn namespace_storage_is_user_scoped_inside_one_skill_database() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-storage-users-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let alpha = runtime(&root, "rk-alpha");
    let beta = runtime(&root, "rk-beta");
    initialize(&alpha).expect("initialize");
    save_namespace(
        &alpha,
        &NamespaceIndex {
            namespace: "docs".to_string(),
            owner_user_key: "rk-alpha".to_string(),
            updated_at_epoch: 1,
            next_chunk_seq: 1,
            docs: HashMap::<String, DocMeta>::new(),
            chunks: Vec::new(),
            ..NamespaceIndex::default()
        },
    )
    .expect("save alpha");
    assert!(namespace_exists(&alpha, "docs").expect("alpha exists"));
    assert!(!namespace_exists(&beta, "docs").expect("beta isolated"));
    assert!(load_namespace(&beta, "docs").is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn legacy_json_migrates_once_and_is_physically_removed() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-storage-migrate-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let legacy = root.join("data/kb/by_user/owner");
    fs::create_dir_all(&legacy).expect("legacy directory");
    let snapshot = NamespaceIndex {
        namespace: "manuals".to_string(),
        owner_user_key: "rk-owner".to_string(),
        updated_at_epoch: 7,
        next_chunk_seq: 1,
        docs: HashMap::new(),
        chunks: Vec::new(),
        ..NamespaceIndex::default()
    };
    let legacy_file = legacy.join("manuals.json");
    fs::write(
        &legacy_file,
        serde_json::to_string_pretty(&snapshot).expect("snapshot"),
    )
    .expect("legacy file");
    let runtime = runtime(&root, "rk-owner");
    initialize(&runtime).expect("first migration");
    initialize(&runtime).expect("second start");
    assert!(!legacy_file.exists());
    assert_eq!(
        load_namespace(&runtime, "manuals")
            .expect("migrated namespace")
            .updated_at_epoch,
        7
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sqlite_v1_payload_migrates_to_normalized_rows_and_drops_legacy_table() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-storage-sqlite-v1-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "rk-owner");
    if let Some(parent) = runtime.storage_database_path.parent() {
        fs::create_dir_all(parent).expect("database parent");
    }
    let db = Connection::open(&runtime.storage_database_path).expect("open v1 database");
    db.execute_batch(
        "CREATE TABLE kb_namespaces (
            owner_user_key TEXT NOT NULL,
            namespace TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            PRIMARY KEY(owner_user_key, namespace)
        );",
    )
    .expect("create v1 schema");
    let original = snapshot("rk-owner", "manuals", &[("guide.md", "deploy safely")]);
    db.execute(
        "INSERT INTO kb_namespaces VALUES (?1, ?2, ?3, ?4)",
        params![
            original.owner_user_key,
            original.namespace,
            serde_json::to_string(&original).expect("serialize v1 payload"),
            original.updated_at_epoch
        ],
    )
    .expect("insert v1 payload");
    drop(db);

    initialize(&runtime).expect("migrate v1 database");
    initialize(&runtime).expect("migration is idempotent");
    let loaded = load_namespace(&runtime, "manuals").expect("load normalized namespace");
    assert_eq!(loaded.docs.len(), 1);
    assert_eq!(loaded.chunks.len(), 1);
    let db = open(&runtime).expect("reopen normalized database");
    assert!(!table_exists(&db, "kb_namespaces_v1").expect("legacy table check"));
    assert!(!table_has_column(&db, "kb_namespaces", "payload_json").expect("payload column check"));
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM kb_documents", [], |row| row
            .get::<_, i64>(0))
            .expect("document count"),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE source_kind = 'kb_doc'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .expect("retrieval count"),
        1
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sqlite_v1_migration_reuses_preimported_retrieval_identity() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-storage-preimported-retrieval-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "rk-owner");
    initialize(&runtime).expect("initialize normalized schema");

    let original = snapshot("rk-owner", "manuals", &[("guide.md", "current text")]);
    let mut db = open(&runtime).expect("open database");
    db.execute_batch(
        "CREATE TABLE kb_namespaces_v1 (
            owner_user_key TEXT NOT NULL,
            namespace TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            PRIMARY KEY(owner_user_key, namespace)
        );",
    )
    .expect("create staged v1 table");
    db.execute(
        "INSERT INTO kb_namespaces_v1 VALUES (?1, ?2, ?3, ?4)",
        params![
            original.owner_user_key,
            original.namespace,
            serde_json::to_string(&original).expect("serialize v1 payload"),
            original.updated_at_epoch
        ],
    )
    .expect("insert staged snapshot");
    let tx = db.transaction().expect("begin preimport transaction");
    insert_retrieval_row(
        &tx,
        &original,
        &original.chunks[0],
        "stale preimported text",
        0,
        None,
    )
    .expect("preimport retrieval identity");
    tx.commit().expect("commit preimport");
    drop(db);

    initialize(&runtime).expect("migrate over preimported retrieval identity");
    initialize(&runtime).expect("repeat migration remains idempotent");
    let db = open(&runtime).expect("reopen migrated database");
    assert!(!table_exists(&db, "kb_namespaces_v1").expect("legacy table check"));
    assert_eq!(
        db.query_row(
            "SELECT search_text FROM memory_retrieval_index
             WHERE user_key = 'rk-owner'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("updated retrieval text"),
        "current text"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index
             WHERE user_key = 'rk-owner'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("one stable retrieval row"),
        1
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn incremental_update_preserves_unaffected_retrieval_row() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-storage-incremental-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "rk-owner");
    let initial = snapshot(
        "rk-owner",
        "docs",
        &[("a.md", "alpha original"), ("b.md", "beta stable")],
    );
    save_namespace(&runtime, &initial).expect("save initial namespace");
    let db = open(&runtime).expect("open database");
    let stable_ref = source_ref_parts("rk-owner", "docs", "docs-2");
    let stable_id: i64 = db
        .query_row(
            "SELECT id FROM memory_retrieval_index WHERE user_key = ?1 AND source_ref = ?2",
            params!["rk-owner", stable_ref],
            |row| row.get(0),
        )
        .expect("stable row id");
    drop(db);

    let mut changed = load_namespace(&runtime, "docs").expect("load initial namespace");
    changed
        .docs
        .get_mut("a.md")
        .expect("a document")
        .content_sha256 = crate::sha256_hex(b"alpha changed");
    changed.docs.get_mut("a.md").expect("a document").size = "alpha changed".len() as u64;
    let chunk = changed
        .chunks
        .iter_mut()
        .find(|chunk| chunk.path == "a.md")
        .expect("a chunk");
    chunk.text = "alpha changed".to_string();
    chunk.text_sha256 = crate::sha256_hex(chunk.text.as_bytes());
    let outcome = save_namespace(&runtime, &changed).expect("incremental update");
    assert_eq!(outcome.total_docs, 2);
    assert_eq!(outcome.retrieval_rows, 2);

    let db = open(&runtime).expect("reopen database");
    let stable_id_after: i64 = db
        .query_row(
            "SELECT id FROM memory_retrieval_index WHERE user_key = ?1 AND source_ref = ?2",
            params!["rk-owner", stable_ref],
            |row| row.get(0),
        )
        .expect("stable row remains");
    assert_eq!(stable_id_after, stable_id);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE user_key = 'rk-owner'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .expect("retrieval row count"),
        2
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn failed_incremental_write_rolls_back_documents_chunks_and_retrieval() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-storage-rollback-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "rk-owner");
    let initial = snapshot("rk-owner", "docs", &[("a.md", "original")]);
    save_namespace(&runtime, &initial).expect("save initial namespace");
    let db = open(&runtime).expect("open database");
    db.execute_batch(
        "CREATE TRIGGER reject_test_chunk BEFORE INSERT ON kb_chunks
         WHEN NEW.text = 'boom'
         BEGIN SELECT RAISE(ABORT, 'forced rollback'); END;",
    )
    .expect("create failure trigger");
    drop(db);

    let mut changed = load_namespace(&runtime, "docs").expect("load initial namespace");
    changed
        .docs
        .get_mut("a.md")
        .expect("document")
        .content_sha256 = crate::sha256_hex(b"boom");
    changed.docs.get_mut("a.md").expect("document").size = 4;
    changed.chunks[0].text = "boom".to_string();
    changed.chunks[0].text_sha256 = crate::sha256_hex(b"boom");
    assert!(save_namespace(&runtime, &changed).is_err());

    let loaded = load_namespace(&runtime, "docs").expect("load rolled-back namespace");
    assert_eq!(loaded.chunks[0].text, "original");
    let db = open(&runtime).expect("reopen database");
    assert_eq!(
        db.query_row(
            "SELECT search_text FROM memory_retrieval_index WHERE user_key = 'rk-owner'",
            [],
            |row| row.get::<_, String>(0)
        )
        .expect("retrieval text"),
        "original"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn search_candidates_are_bounded_and_user_namespace_scoped() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-storage-candidates-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let alpha = runtime(&root, "rk-alpha");
    let beta = runtime(&root, "rk-beta");
    save_namespace(
        &alpha,
        &snapshot(
            "rk-alpha",
            "docs",
            &[
                ("a.md", "needle alpha"),
                ("b.md", "ordinary beta"),
                ("c.md", "ordinary gamma"),
            ],
        ),
    )
    .expect("save alpha namespace");
    save_namespace(
        &beta,
        &snapshot("rk-beta", "docs", &[("secret.md", "needle secret")]),
    )
    .expect("save beta namespace");

    let alpha_hits = load_search_candidates(
        &alpha,
        "docs",
        &["needle".to_string()],
        &claw_core::retrieval_embedding::embed_local_hash("needle"),
        SearchFilters::default(),
        10,
    )
    .expect("load FTS candidates");
    assert_eq!(alpha_hits.total_chunks, 3);
    assert!(matches!(
        alpha_hits.retrieval_mode,
        "fts5_candidates" | "hybrid_candidates"
    ));
    assert!(alpha_hits
        .index
        .chunks
        .iter()
        .any(|chunk| chunk.path == "a.md"));
    assert!(alpha_hits.lexical_chunk_ids.contains("docs-1"));

    let no_match = load_search_candidates(
        &alpha,
        "docs",
        &["absent".to_string()],
        &[],
        SearchFilters::default(),
        2,
    )
    .expect("load empty FTS result");
    assert_eq!(no_match.total_chunks, 3);
    assert_eq!(no_match.retrieval_mode, "no_match");
    assert!(no_match.index.chunks.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn search_filters_are_applied_before_candidate_limit() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-storage-filter-before-limit-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "rk-owner");
    save_namespace(
        &runtime,
        &snapshot(
            "rk-owner",
            "docs",
            &[
                ("other/00.md", "needle common"),
                ("other/01.md", "needle common"),
                ("other/02.md", "needle common"),
                ("wanted/late.md", "needle filtered target"),
            ],
        ),
    )
    .expect("save filtered fixture");

    let hits = load_search_candidates(
        &runtime,
        "docs",
        &["needle".to_string()],
        &claw_core::retrieval_embedding::embed_local_hash("needle"),
        SearchFilters {
            path_prefix: Some("wanted"),
            ..SearchFilters::default()
        },
        1,
    )
    .expect("load filtered candidates");
    assert_eq!(hits.index.chunks.len(), 1);
    assert_eq!(hits.index.chunks[0].path, "wanted/late.md");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn retrieval_metadata_records_bounded_token_projection_truncation() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-token-projection-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "rk-owner");
    let text = (0..4_200)
        .map(|index| format!("token{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    save_namespace(
        &runtime,
        &snapshot("rk-owner", "docs", &[("large.md", text.as_str())]),
    )
    .expect("save bounded token projection fixture");

    let db = open(&runtime).expect("open bounded token projection database");
    let metadata_json: String = db
        .query_row(
            "SELECT metadata_json FROM memory_retrieval_index
             WHERE source_kind = 'kb_doc' AND user_key = 'rk-owner'",
            [],
            |row| row.get(0),
        )
        .expect("load retrieval projection metadata");
    let metadata: serde_json::Value =
        serde_json::from_str(&metadata_json).expect("parse retrieval metadata");
    assert_eq!(
        metadata["token_projection"]["tokenizer_version"],
        claw_core::retrieval_embedding::RETRIEVAL_TOKENIZER_VERSION
    );
    assert_eq!(
        metadata["token_projection"]["token_budget"],
        claw_core::retrieval_embedding::DEFAULT_RETRIEVAL_TOKEN_BUDGET
    );
    assert_eq!(metadata["token_projection"]["truncated"], true);
    assert!(metadata["token_projection"]["dropped_candidates"]
        .as_u64()
        .is_some_and(|count| count > 0));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn initialize_discards_inline_vectors_without_implicit_generation_migration() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-local-hash-v3-migration-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "rk-owner");
    save_namespace(
        &runtime,
        &snapshot(
            "rk-owner",
            "docs",
            &[("guide.md", "中文知识文档的唯一答案位于尾部")],
        ),
    )
    .expect("save current projection");
    {
        let db = open(&runtime).expect("open projection database");
        db.execute(
            "UPDATE kb_namespaces SET active_vector_generation = NULL",
            [],
        )
        .expect("detach current generation");
        db.execute("DELETE FROM kb_vector_generations", [])
            .expect("remove current generation");
        db.execute(
            "UPDATE memory_retrieval_index
             SET vector_json = '[0.25,0.5]', topic_tags = '', embedding_model = 'local-hash-v1',
                 embedding_dims = 24, embedding_version = 'local-hash-v1'",
            [],
        )
        .expect("stage legacy retrieval row");
        db.execute(
            "UPDATE kb_namespaces SET embedding_version = 'local-hash-v1'",
            [],
        )
        .expect("stage legacy namespace");
        db.execute(
            "DELETE FROM skill_storage_migrations WHERE migration_id = ?1",
            [SCHEMA_V6_CUTOVER_MIGRATION_ID],
        )
        .expect("stage pre-cutover migration state");
    }

    initialize(&runtime).expect("apply direct projection cutover");
    let db = open(&runtime).expect("reopen cutover database");
    let (model, dims, version, vector_json, topic_tags) = db
        .query_row(
            "SELECT embedding_model, embedding_dims, embedding_version, vector_json, topic_tags
             FROM memory_retrieval_index",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .expect("load cutover projection row");
    assert_eq!(model, "none");
    assert_eq!(dims, 0);
    assert_eq!(version, "none");
    assert_eq!(vector_json, "[]");
    assert!(topic_tags.contains("尾部"));
    let (active_generation, generation_count) = db
        .query_row(
            "SELECT active_vector_generation,
                    (SELECT COUNT(*) FROM kb_vector_generations)
             FROM kb_namespaces",
            [],
            |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, i64>(1)?)),
        )
        .expect("load vector generation state");
    assert_eq!(active_generation, None);
    assert_eq!(generation_count, 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn vector_reindex_resumes_in_batches_and_switches_only_after_verification() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-vector-reindex-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "rk-owner");
    save_namespace(
        &runtime,
        &snapshot(
            "rk-owner",
            "docs",
            &[
                ("one.md", "first document"),
                ("two.md", "second document"),
                ("three.md", "third document"),
            ],
        ),
    )
    .expect("save projection source");

    let started = start_vector_reindex(&runtime, "docs").expect("start vector reindex");
    assert_eq!(started.state, "building");
    assert_eq!(started.expected_rows, 3);
    let previous_active = started
        .active_generation
        .expect("existing active generation");
    assert_ne!(started.generation, previous_active);

    let first = resume_vector_reindex(&runtime, "docs", started.generation, 1)
        .expect("resume first vector batch");
    assert_eq!(first.state, "building");
    assert_eq!(first.indexed_rows, 1);
    assert_eq!(first.active_generation, Some(previous_active));
    assert!(!first.source_digest.is_empty());
    assert!(!first.projection_digest.is_empty());

    let second = resume_vector_reindex(&runtime, "docs", started.generation, 1)
        .expect("resume second vector batch");
    assert_eq!(second.state, "building");
    assert_eq!(second.indexed_rows, 2);
    assert_eq!(second.active_generation, Some(previous_active));

    let completed = resume_vector_reindex(&runtime, "docs", started.generation, 1)
        .expect("complete vector reindex");
    assert!(completed.is_complete());
    assert_eq!(completed.indexed_rows, completed.expected_rows);
    assert_eq!(completed.active_generation, Some(started.generation));
    let db = open(&runtime).expect("open vector generation database");
    assert_eq!(
        db.query_row(
            "SELECT state FROM kb_vector_generations
             WHERE owner_user_key = 'rk-owner' AND namespace = 'docs' AND generation = ?1",
            params![previous_active as i64],
            |row| row.get::<_, String>(0),
        )
        .expect("old generation state"),
        "retired"
    );
    drop(db);
    let active_candidates = load_search_candidates(
        &runtime,
        "docs",
        &["first".to_string()],
        &claw_core::retrieval_embedding::embed_local_hash("first document"),
        SearchFilters::default(),
        10,
    )
    .expect("load candidates from activated generation");
    assert_eq!(
        active_candidates.vector_generation,
        Some(started.generation)
    );
    assert_ne!(
        active_candidates.vector_generation,
        Some(previous_active),
        "retired generation must not remain searchable"
    );

    let stale_candidate = start_vector_reindex(&runtime, "docs").expect("start stale candidate");
    save_namespace(
        &runtime,
        &snapshot(
            "rk-owner",
            "docs",
            &[
                ("one.md", "first document changed"),
                ("two.md", "second document"),
                ("three.md", "third document"),
            ],
        ),
    )
    .expect("change canonical namespace during shadow build");
    let stale = resume_vector_reindex(&runtime, "docs", stale_candidate.generation, 1)
        .expect("mark mismatched shadow generation stale");
    assert_eq!(stale.state, "stale");
    assert_ne!(stale.active_generation, Some(stale.generation));
    let candidates_after_revision_change = load_search_candidates(
        &runtime,
        "docs",
        &["changed".to_string()],
        &claw_core::retrieval_embedding::embed_local_hash("first document changed"),
        SearchFilters::default(),
        10,
    )
    .expect("load candidates after revision change");
    assert_eq!(
        candidates_after_revision_change.vector_generation,
        stale.active_generation
    );
    assert_ne!(
        candidates_after_revision_change.vector_generation,
        Some(stale_candidate.generation),
        "stale shadow generation must not become searchable"
    );

    let cancellable = start_vector_reindex(&runtime, "docs").expect("start cancellable reindex");
    let cancelled = cancel_vector_reindex(&runtime, "docs", cancellable.generation)
        .expect("cancel vector reindex");
    assert_eq!(cancelled.state, "cancelled");
    assert_eq!(cancelled.indexed_rows, 0);
    assert!(!cancelled.is_resumable());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn extraction_contract_change_invalidates_active_generation_until_reindex() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-extraction-generation-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "rk-owner");
    let initial = snapshot("rk-owner", "docs", &[("guide.md", "stable evidence")]);
    save_namespace(&runtime, &initial).expect("save initial extraction generation");
    let previous_generation = {
        let db = open(&runtime).expect("open initial generation database");
        db.query_row(
            "SELECT active_vector_generation FROM kb_namespaces
             WHERE owner_user_key = 'rk-owner' AND namespace = 'docs'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("initial active generation")
    };

    let mut changed = initial.clone();
    changed
        .docs
        .get_mut("guide.md")
        .expect("changed document")
        .parser_version = "typed-content-next".to_string();
    save_namespace(&runtime, &changed).expect("save parser-version boundary");

    {
        let db = open(&runtime).expect("open invalidated generation database");
        let active = db
            .query_row(
                "SELECT active_vector_generation FROM kb_namespaces
                 WHERE owner_user_key = 'rk-owner' AND namespace = 'docs'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .expect("read active generation");
        assert_eq!(active, None);
        assert_eq!(
            db.query_row(
                "SELECT state FROM kb_vector_generations
                 WHERE owner_user_key = 'rk-owner' AND namespace = 'docs' AND generation = ?1",
                [previous_generation],
                |row| row.get::<_, String>(0),
            )
            .expect("read invalidated generation state"),
            "stale"
        );
    }

    let next = start_vector_reindex(&runtime, "docs").expect("start replacement generation");
    assert_ne!(next.generation as i64, previous_generation);
    let completed = resume_vector_reindex(&runtime, "docs", next.generation, 256)
        .expect("build replacement generation");
    assert!(completed.is_complete());
    assert_eq!(completed.active_generation, Some(next.generation));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ann_projection_bounds_large_generation_candidates_and_is_rebuildable() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-kb-ann-projection-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let runtime = runtime(&root, "rk-owner");
    save_namespace(
        &runtime,
        &snapshot(
            "rk-owner",
            "docs",
            &[
                ("one.md", "alpha deployment guide"),
                ("two.md", "beta weather report"),
                ("three.md", "gamma stock summary"),
                ("four.md", "delta archive notes"),
                ("five.md", "epsilon schedule entry"),
                ("six.md", "zeta process status"),
            ],
        ),
    )
    .expect("save ANN source");
    let db = open(&runtime).expect("open ANN database");
    let generation: i64 = db
        .query_row(
            "SELECT active_vector_generation FROM kb_namespaces
             WHERE owner_user_key = 'rk-owner' AND namespace = 'docs'",
            [],
            |row| row.get(0),
        )
        .expect("active generation");
    let ann_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM kb_vector_ann_projection
             WHERE owner_user_key = 'rk-owner' AND namespace = 'docs'
               AND generation = ?1",
            params![generation],
            |row| row.get(0),
        )
        .expect("ANN row count");
    assert_eq!(
        ann_rows as usize,
        6 * claw_core::retrieval_embedding::VECTOR_ANN_BANDS
    );

    let query = claw_core::retrieval_embedding::embed_local_hash("alpha deployment guide");
    let candidates = semantic_candidate_scores(
        &db,
        "rk-owner",
        "docs",
        &query,
        SearchFilters::default(),
        3,
        2,
        4,
    )
    .expect("ANN candidates");
    assert_eq!(candidates.backend, "ann_projection");
    assert!(candidates.rows_scored <= 4);
    assert!(candidates
        .scores
        .iter()
        .any(|(chunk_id, _)| chunk_id == "docs-1"));

    db.execute(
        "DELETE FROM kb_vector_ann_projection
         WHERE owner_user_key = 'rk-owner' AND namespace = 'docs'
           AND generation = ?1",
        params![generation],
    )
    .expect("remove disposable ANN projection");
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM kb_chunks
             WHERE owner_user_key = 'rk-owner' AND namespace = 'docs'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("canonical chunk count"),
        6
    );
    let stale = semantic_candidate_scores(
        &db,
        "rk-owner",
        "docs",
        &query,
        SearchFilters::default(),
        3,
        2,
        4,
    )
    .expect("stale ANN projection response");
    assert_eq!(stale.degradation.as_deref(), Some("ann_projection_stale"));
    assert!(stale.scores.is_empty());

    rebuild_ann_projection(&db, "rk-owner", "docs", generation)
        .expect("rebuild disposable ANN projection");
    assert!(
        verify_ann_projection(&db, "rk-owner", "docs", generation, 6)
            .expect("verify rebuilt ANN projection")
    );
    let rebuilt = semantic_candidate_scores(
        &db,
        "rk-owner",
        "docs",
        &query,
        SearchFilters::default(),
        3,
        2,
        4,
    )
    .expect("rebuilt ANN candidates");
    assert!(rebuilt
        .scores
        .iter()
        .any(|(chunk_id, _)| chunk_id == "docs-1"));
    drop(db);
    let _ = fs::remove_dir_all(root);
}
