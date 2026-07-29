use super::*;
use crate::{Chunk, DocMeta, KbRuntime, NamespaceIndex};
use std::collections::HashMap;

fn runtime(root: &Path, user_key: &str) -> KbRuntime {
    fs::create_dir_all(root).expect("create KB test workspace");
    KbRuntime {
        scope_user_key: user_key.to_string(),
        workspace_root: root.to_path_buf(),
        storage_database_path: root.join("data/skills/kb/state.db"),
        storage_busy_timeout_ms: 5_000,
        path_policy: rustclaw_skill_sdk::SkillPathPolicy::new(root, None)
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
fn namespace_storage_is_user_scoped_inside_one_skill_database() {
    let root =
        std::env::temp_dir().join(format!("rustclaw-kb-storage-users-{}", std::process::id()));
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
        "rustclaw-kb-storage-migrate-{}",
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
        "rustclaw-kb-storage-sqlite-v1-{}",
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
fn incremental_update_preserves_unaffected_retrieval_row() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-kb-storage-incremental-{}",
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
        "rustclaw-kb-storage-rollback-{}",
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
        "rustclaw-kb-storage-candidates-{}",
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

    let alpha_hits = load_search_candidates(&alpha, "docs", &["needle".to_string()], 10)
        .expect("load FTS candidates");
    assert_eq!(alpha_hits.total_chunks, 3);
    assert_eq!(alpha_hits.retrieval_mode, "fts5_candidates");
    assert_eq!(alpha_hits.index.chunks.len(), 1);
    assert_eq!(alpha_hits.index.chunks[0].path, "a.md");

    let fallback = load_search_candidates(&alpha, "docs", &["absent".to_string()], 2)
        .expect("load bounded fallback candidates");
    assert_eq!(fallback.total_chunks, 3);
    assert_eq!(fallback.retrieval_mode, "bounded_scan");
    assert_eq!(fallback.index.chunks.len(), 2);
    assert!(fallback
        .index
        .chunks
        .iter()
        .all(|chunk| chunk.path != "secret.md"));
    let _ = fs::remove_dir_all(root);
}
