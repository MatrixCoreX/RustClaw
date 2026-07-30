use super::{
    default_chunker_version, default_embedding_version, default_parser_version, sha256_hex, Chunk,
    DocMeta, KbRuntime, NamespaceIndex,
};
use crate::ingest::IngestJob;
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const STORAGE_SCHEMA_VERSION: u32 = 3;
const LEGACY_JSON_MIGRATION_ID: &str = "legacy-kb-json-v1";
const SQLITE_V1_MIGRATION_ID: &str = "kb-payload-json-to-normalized-v2";

#[derive(Debug, Clone, Copy)]
pub(super) struct SaveOutcome {
    pub(super) revision: u64,
    pub(super) total_docs: usize,
    pub(super) total_chunks: usize,
    pub(super) retrieval_rows: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DeleteOutcome {
    pub(super) removed_docs: usize,
    pub(super) removed_chunks: usize,
}

pub(super) struct SearchCandidateSet {
    pub(super) index: NamespaceIndex,
    pub(super) total_chunks: usize,
    pub(super) retrieval_mode: &'static str,
}

pub(super) fn initialize(runtime: &KbRuntime) -> Result<()> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    migrate_legacy_json(runtime, &mut db)?;
    integrity_check(&db)
}

pub(super) fn namespace_exists(runtime: &KbRuntime, namespace: &str) -> Result<bool> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    namespace_exists_in(&db, &runtime.scope_user_key, namespace)
}

pub(super) fn load_namespace(runtime: &KbRuntime, namespace: &str) -> Result<NamespaceIndex> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    load_namespace_from(&db, &runtime.scope_user_key, namespace)
}

pub(super) fn save_namespace(runtime: &KbRuntime, index: &NamespaceIndex) -> Result<SaveOutcome> {
    validate_owner(runtime, index)?;
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let tx = db.transaction()?;
    let outcome = persist_namespace(&tx, index)?;
    tx.commit()?;
    Ok(outcome)
}

pub(super) fn save_namespace_and_job(
    runtime: &KbRuntime,
    index: &NamespaceIndex,
    job: &IngestJob,
) -> Result<SaveOutcome> {
    validate_owner(runtime, index)?;
    validate_job_owner(runtime, job)?;
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let tx = db.transaction()?;
    let outcome = persist_namespace(&tx, index)?;
    upsert_ingest_job(&tx, job)?;
    tx.commit()?;
    Ok(outcome)
}

pub(super) fn save_ingest_job(runtime: &KbRuntime, job: &IngestJob) -> Result<()> {
    validate_job_owner(runtime, job)?;
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let tx = db.transaction()?;
    upsert_ingest_job(&tx, job)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn load_ingest_job(runtime: &KbRuntime, job_id: &str) -> Result<IngestJob> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let payload = db
        .query_row(
            "SELECT payload_json FROM kb_ingest_jobs
             WHERE owner_user_key = ?1 AND job_id = ?2",
            params![runtime.scope_user_key, job_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow!("ingest job not found"))?;
    let job: IngestJob =
        serde_json::from_str(&payload).context("stored ingest job is malformed")?;
    validate_job_owner(runtime, &job)?;
    Ok(job)
}

pub(super) fn list_namespaces(runtime: &KbRuntime) -> Result<Vec<NamespaceIndex>> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let mut stmt = db.prepare(
        "SELECT namespace FROM kb_namespaces
         WHERE owner_user_key = ?1
         ORDER BY updated_at_epoch DESC, namespace ASC",
    )?;
    let names = stmt
        .query_map(params![runtime.scope_user_key], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    names
        .iter()
        .map(|namespace| load_namespace_from(&db, &runtime.scope_user_key, namespace))
        .collect()
}

pub(super) fn delete_namespace(runtime: &KbRuntime, namespace: &str) -> Result<DeleteOutcome> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let tx = db.transaction()?;
    let paths = {
        let mut stmt = tx.prepare(
            "SELECT path FROM kb_documents
             WHERE owner_user_key = ?1 AND namespace = ?2
             ORDER BY path",
        )?;
        let rows = stmt
            .query_map(params![runtime.scope_user_key, namespace], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let removed_chunks = count_rows(
        &tx,
        "kb_chunks",
        "owner_user_key = ?1 AND namespace = ?2",
        &runtime.scope_user_key,
        namespace,
    )?;
    for path in &paths {
        delete_document(&tx, &runtime.scope_user_key, namespace, path)?;
    }
    let removed = tx.execute(
        "DELETE FROM kb_namespaces WHERE owner_user_key = ?1 AND namespace = ?2",
        params![runtime.scope_user_key, namespace],
    )?;
    if removed == 0 {
        return Err(anyhow!("namespace not found"));
    }
    tx.commit()?;
    Ok(DeleteOutcome {
        removed_docs: paths.len(),
        removed_chunks,
    })
}

pub(super) fn load_search_candidates(
    runtime: &KbRuntime,
    namespace: &str,
    terms: &[String],
    limit: usize,
) -> Result<SearchCandidateSet> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let mut index = load_namespace_header(&db, &runtime.scope_user_key, namespace)?;
    let total_chunks = count_rows(
        &db,
        "kb_chunks",
        "owner_user_key = ?1 AND namespace = ?2",
        &runtime.scope_user_key,
        namespace,
    )?;
    let source_prefix = format!("kb:{}:{}:", runtime.scope_user_key.trim(), namespace.trim());
    let mut chunk_ids = if has_fts(&db)? && !terms.is_empty() {
        fts_candidate_chunk_ids(&db, &runtime.scope_user_key, &source_prefix, terms, limit)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let retrieval_mode = if chunk_ids.is_empty() {
        chunk_ids = fallback_candidate_chunk_ids(&db, &runtime.scope_user_key, namespace, limit)?;
        "bounded_scan"
    } else {
        "fts5_candidates"
    };
    index.chunks = load_chunks_by_ids(&db, &runtime.scope_user_key, namespace, &chunk_ids)?;
    Ok(SearchCandidateSet {
        index,
        total_chunks,
        retrieval_mode,
    })
}

pub(super) fn storage_summary(runtime: &KbRuntime) -> serde_json::Value {
    json!({
        "kind": "sqlite",
        "schema_version": STORAGE_SCHEMA_VERSION,
        "skill_name": "kb",
        "database_identity": database_identity(&runtime.storage_database_path),
        "layout": "normalized_documents_chunks_with_resumable_jobs_v3",
    })
}

fn open(runtime: &KbRuntime) -> Result<Connection> {
    if !runtime.storage_database_path.is_absolute() {
        return Err(anyhow!("skill storage database path must be absolute"));
    }
    if runtime
        .storage_database_path
        .file_name()
        .and_then(|value| value.to_str())
        != Some("state.db")
    {
        return Err(anyhow!("skill storage database identity is invalid"));
    }
    if let Some(parent) = runtime.storage_database_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let db = Connection::open(&runtime.storage_database_path).with_context(|| {
        format!(
            "open KB skill storage failed: {}",
            runtime.storage_database_path.display()
        )
    })?;
    db.busy_timeout(Duration::from_millis(
        runtime.storage_busy_timeout_ms.max(1),
    ))?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "synchronous", "NORMAL")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    Ok(db)
}

fn ensure_schema(db: &mut Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS skill_storage_metadata (
            skill_name TEXT PRIMARY KEY,
            schema_version INTEGER NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS skill_storage_migrations (
            migration_id TEXT PRIMARY KEY,
            source_identity TEXT NOT NULL,
            source_rows INTEGER NOT NULL,
            verified_digest TEXT NOT NULL,
            completed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS memory_retrieval_index (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_kind TEXT NOT NULL,
            source_memory_id INTEGER,
            source_pref_key TEXT,
            source_ref TEXT,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT,
            memory_kind TEXT NOT NULL,
            role TEXT,
            search_text TEXT NOT NULL,
            trigger_text TEXT,
            topic_tags TEXT NOT NULL DEFAULT '',
            vector_json TEXT NOT NULL DEFAULT '[]',
            embedding_model TEXT NOT NULL DEFAULT 'local-hash-v1',
            embedding_dims INTEGER NOT NULL DEFAULT 24,
            embedding_version TEXT NOT NULL DEFAULT 'local-hash-v1',
            metadata_json TEXT NOT NULL DEFAULT '{}',
            salience REAL NOT NULL DEFAULT 0.5,
            success_state TEXT NOT NULL DEFAULT 'neutral',
            tool_or_skill_name TEXT,
            created_at_ts INTEGER NOT NULL DEFAULT 0,
            updated_at_ts INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_kb_retrieval_source_identity
        ON memory_retrieval_index(user_key, source_ref);
        CREATE INDEX IF NOT EXISTS idx_kb_retrieval_scope_updated
        ON memory_retrieval_index(user_key, updated_at_ts DESC);",
    )?;
    let _ = db.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memory_retrieval_index_fts
         USING fts5(search_text, topic_tags);",
    );

    if table_has_column(db, "kb_namespaces", "payload_json")? {
        if table_exists(db, "kb_namespaces_v1")? {
            return Err(anyhow!(
                "both active and staged legacy KB namespace tables exist"
            ));
        }
        db.execute("ALTER TABLE kb_namespaces RENAME TO kb_namespaces_v1", [])?;
    }
    create_normalized_schema(db)?;
    migrate_sqlite_v1(db)?;
    db.execute(
        "INSERT INTO skill_storage_metadata (skill_name, schema_version)
         VALUES ('kb', ?1)
         ON CONFLICT(skill_name) DO UPDATE SET
            schema_version = excluded.schema_version,
            updated_at = CURRENT_TIMESTAMP",
        params![STORAGE_SCHEMA_VERSION],
    )?;
    Ok(())
}

fn create_normalized_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS kb_namespaces (
            owner_user_key TEXT NOT NULL,
            namespace TEXT NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            next_chunk_seq INTEGER NOT NULL,
            revision INTEGER NOT NULL,
            parser_version TEXT NOT NULL,
            chunker_version TEXT NOT NULL,
            embedding_version TEXT NOT NULL,
            PRIMARY KEY(owner_user_key, namespace)
        );
        CREATE INDEX IF NOT EXISTS idx_kb_namespaces_owner_updated
        ON kb_namespaces(owner_user_key, updated_at_epoch DESC);
        CREATE TABLE IF NOT EXISTS kb_documents (
            owner_user_key TEXT NOT NULL,
            namespace TEXT NOT NULL,
            path TEXT NOT NULL,
            file_type TEXT NOT NULL,
            mtime_epoch INTEGER NOT NULL,
            size_bytes INTEGER NOT NULL,
            chunk_count INTEGER NOT NULL,
            content_sha256 TEXT NOT NULL,
            parser_version TEXT NOT NULL,
            chunker_version TEXT NOT NULL,
            PRIMARY KEY(owner_user_key, namespace, path),
            FOREIGN KEY(owner_user_key, namespace)
                REFERENCES kb_namespaces(owner_user_key, namespace)
                ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_kb_documents_owner_namespace_type
        ON kb_documents(owner_user_key, namespace, file_type, path);
        CREATE TABLE IF NOT EXISTS kb_chunks (
            owner_user_key TEXT NOT NULL,
            namespace TEXT NOT NULL,
            chunk_id TEXT NOT NULL,
            document_path TEXT NOT NULL,
            file_type TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            text TEXT NOT NULL,
            text_sha256 TEXT NOT NULL,
            len_tokens INTEGER NOT NULL,
            mtime_epoch INTEGER NOT NULL,
            PRIMARY KEY(owner_user_key, namespace, chunk_id),
            FOREIGN KEY(owner_user_key, namespace, document_path)
                REFERENCES kb_documents(owner_user_key, namespace, path)
                ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_kb_chunks_owner_namespace_document
        ON kb_chunks(owner_user_key, namespace, document_path, ordinal);
        CREATE TABLE IF NOT EXISTS kb_ingest_jobs (
            owner_user_key TEXT NOT NULL,
            job_id TEXT NOT NULL,
            namespace TEXT NOT NULL,
            operation TEXT NOT NULL,
            status TEXT NOT NULL,
            next_file_index INTEGER NOT NULL,
            total_files INTEGER NOT NULL,
            payload_json TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            PRIMARY KEY(owner_user_key, job_id)
        );
        CREATE INDEX IF NOT EXISTS idx_kb_ingest_jobs_owner_status_updated
        ON kb_ingest_jobs(owner_user_key, status, updated_at_epoch DESC);",
    )?;
    Ok(())
}

fn upsert_ingest_job(tx: &Transaction<'_>, job: &IngestJob) -> Result<()> {
    let payload = serde_json::to_string(job)?;
    tx.execute(
        "INSERT INTO kb_ingest_jobs (
            owner_user_key, job_id, namespace, operation, status,
            next_file_index, total_files, payload_json,
            created_at_epoch, updated_at_epoch
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(owner_user_key, job_id) DO UPDATE SET
            namespace = excluded.namespace,
            operation = excluded.operation,
            status = excluded.status,
            next_file_index = excluded.next_file_index,
            total_files = excluded.total_files,
            payload_json = excluded.payload_json,
            updated_at_epoch = excluded.updated_at_epoch",
        params![
            job.owner_user_key,
            job.job_id,
            job.namespace,
            job.operation,
            job.status,
            job.next_file_index as i64,
            job.manifest.len() as i64,
            payload,
            job.created_at_epoch,
            job.updated_at_epoch,
        ],
    )?;
    Ok(())
}

fn validate_job_owner(runtime: &KbRuntime, job: &IngestJob) -> Result<()> {
    if job.owner_user_key != runtime.scope_user_key {
        return Err(anyhow!("KB ingest job owner mismatch"));
    }
    if job.job_id.trim().is_empty() || job.namespace.trim().is_empty() {
        return Err(anyhow!("KB ingest job identity is incomplete"));
    }
    Ok(())
}

fn migrate_sqlite_v1(db: &mut Connection) -> Result<()> {
    if !table_exists(db, "kb_namespaces_v1")? {
        return Ok(());
    }
    let complete = migration_complete(db, SQLITE_V1_MIGRATION_ID)?;
    if complete {
        db.execute("DROP TABLE kb_namespaces_v1", [])?;
        return Ok(());
    }
    let snapshots = {
        let mut stmt = db.prepare(
            "SELECT payload_json FROM kb_namespaces_v1
             ORDER BY owner_user_key, namespace",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut snapshots = Vec::new();
        for row in rows {
            snapshots.push(
                serde_json::from_str::<NamespaceIndex>(&row?)
                    .context("legacy KB SQLite payload is malformed")?,
            );
        }
        snapshots
    };
    validate_snapshots(&snapshots)?;
    let digest = snapshot_digest(&snapshots);
    let tx = db.transaction()?;
    for snapshot in &snapshots {
        persist_namespace(&tx, snapshot)?;
    }
    verify_snapshots(&tx, &snapshots)?;
    tx.execute(
        "INSERT INTO skill_storage_migrations (
            migration_id, source_identity, source_rows, verified_digest
         ) VALUES (?1, ?2, ?3, ?4)",
        params![
            SQLITE_V1_MIGRATION_ID,
            "kb_namespaces.payload_json",
            snapshots.len() as i64,
            digest
        ],
    )?;
    tx.execute("DROP TABLE kb_namespaces_v1", [])?;
    tx.commit()?;
    Ok(())
}

fn migrate_legacy_json(runtime: &KbRuntime, db: &mut Connection) -> Result<()> {
    if migration_complete(db, LEGACY_JSON_MIGRATION_ID)? {
        return Ok(());
    }
    let root = legacy_root(runtime);
    let files = collect_json_files(&root)?;
    let mut snapshots = Vec::new();
    for path in &files {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("read legacy KB snapshot failed: {}", path.display()))?;
        snapshots.push(
            serde_json::from_str::<NamespaceIndex>(&raw)
                .with_context(|| format!("parse legacy KB snapshot failed: {}", path.display()))?,
        );
    }
    validate_snapshots(&snapshots)?;
    let digest = snapshot_digest(&snapshots);
    let tx = db.transaction()?;
    for snapshot in &snapshots {
        persist_namespace(&tx, snapshot)?;
    }
    verify_snapshots(&tx, &snapshots)?;
    tx.execute(
        "INSERT INTO skill_storage_migrations (
            migration_id, source_identity, source_rows, verified_digest
         ) VALUES (?1, ?2, ?3, ?4)",
        params![
            LEGACY_JSON_MIGRATION_ID,
            "legacy-json-snapshots",
            snapshots.len() as i64,
            digest
        ],
    )?;
    tx.commit()?;
    for path in &files {
        fs::remove_file(path)
            .with_context(|| format!("remove migrated KB snapshot failed: {}", path.display()))?;
    }
    prune_empty_directories(&root);
    Ok(())
}

fn persist_namespace(tx: &Transaction<'_>, index: &NamespaceIndex) -> Result<SaveOutcome> {
    validate_snapshot(index)?;
    let owner = index.owner_user_key.as_str();
    let namespace = index.namespace.as_str();
    let previous_revision = tx
        .query_row(
            "SELECT revision FROM kb_namespaces
             WHERE owner_user_key = ?1 AND namespace = ?2",
            params![owner, namespace],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0)
        .max(0) as u64;
    tx.execute(
        "INSERT INTO kb_namespaces (
            owner_user_key, namespace, updated_at_epoch, next_chunk_seq,
            revision, parser_version, chunker_version, embedding_version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(owner_user_key, namespace) DO NOTHING",
        params![
            owner,
            namespace,
            index.updated_at_epoch,
            index.next_chunk_seq as i64,
            previous_revision as i64,
            effective_version(&index.parser_version, default_parser_version()),
            effective_version(&index.chunker_version, default_chunker_version()),
            effective_version(&index.embedding_version, default_embedding_version()),
        ],
    )?;

    let existing = document_fingerprints(tx, owner, namespace)?;
    let incoming_paths = index.docs.keys().cloned().collect::<HashSet<_>>();
    let removed = existing
        .keys()
        .filter(|path| !incoming_paths.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    let changed = index
        .docs
        .iter()
        .filter(|(path, doc)| existing.get(*path) != Some(&document_fingerprint(doc)))
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    let has_changes = previous_revision == 0 || !removed.is_empty() || !changed.is_empty();
    let revision = if has_changes {
        previous_revision.saturating_add(1).max(index.revision)
    } else {
        previous_revision.max(index.revision)
    };

    for path in &removed {
        delete_document(tx, owner, namespace, path)?;
    }
    for path in &changed {
        delete_document(tx, owner, namespace, path)?;
        let doc = index
            .docs
            .get(path)
            .ok_or_else(|| anyhow!("changed KB document disappeared"))?;
        insert_document(tx, index, doc, revision)?;
    }

    tx.execute(
        "UPDATE kb_namespaces SET
            updated_at_epoch = ?3,
            next_chunk_seq = ?4,
            revision = ?5,
            parser_version = ?6,
            chunker_version = ?7,
            embedding_version = ?8
         WHERE owner_user_key = ?1 AND namespace = ?2",
        params![
            owner,
            namespace,
            index.updated_at_epoch,
            index.next_chunk_seq as i64,
            revision as i64,
            effective_version(&index.parser_version, default_parser_version()),
            effective_version(&index.chunker_version, default_chunker_version()),
            effective_version(&index.embedding_version, default_embedding_version()),
        ],
    )?;
    let total_docs = count_rows(
        tx,
        "kb_documents",
        "owner_user_key = ?1 AND namespace = ?2",
        owner,
        namespace,
    )?;
    let total_chunks = count_rows(
        tx,
        "kb_chunks",
        "owner_user_key = ?1 AND namespace = ?2",
        owner,
        namespace,
    )?;
    Ok(SaveOutcome {
        revision,
        total_docs,
        total_chunks,
        retrieval_rows: total_chunks,
    })
}

fn insert_document(
    tx: &Transaction<'_>,
    index: &NamespaceIndex,
    doc: &DocMeta,
    revision: u64,
) -> Result<()> {
    let owner = index.owner_user_key.as_str();
    let namespace = index.namespace.as_str();
    let content_sha256 = if doc.content_sha256.is_empty() {
        legacy_document_digest(index, &doc.path)
    } else {
        doc.content_sha256.clone()
    };
    tx.execute(
        "INSERT INTO kb_documents (
            owner_user_key, namespace, path, file_type, mtime_epoch, size_bytes,
            chunk_count, content_sha256, parser_version, chunker_version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            owner,
            namespace,
            doc.path,
            doc.file_type,
            doc.mtime_epoch,
            doc.size as i64,
            doc.chunks as i64,
            content_sha256,
            effective_version(&doc.parser_version, default_parser_version()),
            effective_version(&doc.chunker_version, default_chunker_version()),
        ],
    )?;
    let mut chunks = index
        .chunks
        .iter()
        .filter(|chunk| chunk.path == doc.path)
        .collect::<Vec<_>>();
    chunks.sort_by_key(|chunk| chunk.offset);
    if chunks.len() != doc.chunks {
        return Err(anyhow!("KB document chunk count does not match payload"));
    }
    for chunk in chunks {
        insert_chunk(tx, index, chunk, revision)?;
    }
    Ok(())
}

fn insert_chunk(
    tx: &Transaction<'_>,
    index: &NamespaceIndex,
    chunk: &Chunk,
    revision: u64,
) -> Result<()> {
    let text = chunk.text.trim();
    if text.is_empty() {
        return Err(anyhow!("KB chunk text must not be empty"));
    }
    let text_sha256 = if chunk.text_sha256.is_empty() {
        sha256_hex(text.as_bytes())
    } else {
        chunk.text_sha256.clone()
    };
    tx.execute(
        "INSERT INTO kb_chunks (
            owner_user_key, namespace, chunk_id, document_path, file_type,
            ordinal, text, text_sha256, len_tokens, mtime_epoch
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            index.owner_user_key,
            index.namespace,
            chunk.chunk_id,
            chunk.path,
            chunk.file_type,
            chunk.offset as i64,
            text,
            text_sha256,
            chunk.len_tokens as i64,
            chunk.mtime_epoch,
        ],
    )?;
    insert_retrieval_row(tx, index, chunk, text, revision)
}

fn insert_retrieval_row(
    tx: &Transaction<'_>,
    index: &NamespaceIndex,
    chunk: &Chunk,
    text: &str,
    revision: u64,
) -> Result<()> {
    let metadata = json!({
        "scope_kind": "user",
        "owner_user_key": index.owner_user_key,
        "namespace": index.namespace,
        "namespace_revision": revision,
        "path": chunk.path,
        "file_type": chunk.file_type,
        "mtime_epoch": chunk.mtime_epoch,
        "chunk_id": chunk.chunk_id,
        "offset": chunk.offset,
        "embedding_version": effective_version(
            &index.embedding_version,
            default_embedding_version()
        ),
    });
    let source_ref = source_ref(index, &chunk.chunk_id);
    let topic_tags = build_topic_tags(text);
    let vector_json = vector_to_json(&embed_text_locally(text));
    let row_ts = if chunk.mtime_epoch > 0 {
        chunk.mtime_epoch
    } else {
        index.updated_at_epoch
    };
    let rowid = tx.query_row(
        "INSERT INTO memory_retrieval_index (
            source_kind, source_memory_id, source_pref_key, source_ref,
            user_id, chat_id, user_key, memory_kind, role, search_text,
            trigger_text, topic_tags, vector_json, embedding_model,
            embedding_dims, embedding_version, metadata_json, salience,
            success_state, tool_or_skill_name, created_at_ts, updated_at_ts
         ) VALUES (
            'kb_doc', NULL, NULL, ?1, 0, 0, ?2, 'knowledge_doc', NULL,
            ?3, NULL, ?4, ?5, 'local-hash-v1', 24, ?6, ?7,
            0.78, 'succeeded', 'kb', ?8, ?8
         )
         ON CONFLICT(user_key, source_ref) DO UPDATE SET
            source_kind = excluded.source_kind,
            source_memory_id = excluded.source_memory_id,
            source_pref_key = excluded.source_pref_key,
            user_id = excluded.user_id,
            chat_id = excluded.chat_id,
            memory_kind = excluded.memory_kind,
            role = excluded.role,
            search_text = excluded.search_text,
            trigger_text = excluded.trigger_text,
            topic_tags = excluded.topic_tags,
            vector_json = excluded.vector_json,
            embedding_model = excluded.embedding_model,
            embedding_dims = excluded.embedding_dims,
            embedding_version = excluded.embedding_version,
            metadata_json = excluded.metadata_json,
            salience = excluded.salience,
            success_state = excluded.success_state,
            tool_or_skill_name = excluded.tool_or_skill_name,
            updated_at_ts = excluded.updated_at_ts
         RETURNING id",
        params![
            source_ref,
            index.owner_user_key,
            text,
            topic_tags,
            vector_json,
            effective_version(&index.embedding_version, default_embedding_version()),
            metadata.to_string(),
            row_ts,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    if has_fts(tx)? {
        // The stable source identity may already exist after importing the
        // legacy main retrieval index. Rebuild its FTS projection whether the
        // row was inserted or updated so staged namespace migration remains
        // idempotent and cannot leave stale searchable text behind.
        tx.execute(
            "DELETE FROM memory_retrieval_index_fts WHERE rowid = ?1",
            params![rowid],
        )?;
        tx.execute(
            "INSERT INTO memory_retrieval_index_fts(rowid, search_text, topic_tags)
             VALUES (?1, ?2, ?3)",
            params![rowid, text, topic_tags],
        )?;
    }
    Ok(())
}

fn delete_document(tx: &Transaction<'_>, owner: &str, namespace: &str, path: &str) -> Result<()> {
    let chunk_ids = {
        let mut stmt = tx.prepare(
            "SELECT chunk_id FROM kb_chunks
             WHERE owner_user_key = ?1 AND namespace = ?2 AND document_path = ?3",
        )?;
        let rows = stmt
            .query_map(params![owner, namespace, path], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for chunk_id in chunk_ids {
        let source_ref = source_ref_parts(owner, namespace, &chunk_id);
        if let Some(rowid) = tx
            .query_row(
                "SELECT id FROM memory_retrieval_index
                 WHERE user_key = ?1 AND source_ref = ?2",
                params![owner, source_ref],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        {
            if has_fts(tx)? {
                tx.execute(
                    "DELETE FROM memory_retrieval_index_fts WHERE rowid = ?1",
                    params![rowid],
                )?;
            }
            tx.execute(
                "DELETE FROM memory_retrieval_index WHERE id = ?1",
                params![rowid],
            )?;
        }
    }
    tx.execute(
        "DELETE FROM kb_documents
         WHERE owner_user_key = ?1 AND namespace = ?2 AND path = ?3",
        params![owner, namespace, path],
    )?;
    Ok(())
}

fn load_namespace_from(db: &Connection, owner: &str, namespace: &str) -> Result<NamespaceIndex> {
    let mut index = load_namespace_header(db, owner, namespace)?;
    {
        let mut stmt = db.prepare(
            "SELECT path, file_type, mtime_epoch, size_bytes, chunk_count,
                    content_sha256, parser_version, chunker_version
             FROM kb_documents
             WHERE owner_user_key = ?1 AND namespace = ?2
             ORDER BY path",
        )?;
        let rows = stmt.query_map(params![owner, namespace], |row| {
            Ok(DocMeta {
                path: row.get(0)?,
                file_type: row.get(1)?,
                mtime_epoch: row.get(2)?,
                size: nonnegative_u64(row.get::<_, i64>(3)?),
                chunks: nonnegative_usize(row.get::<_, i64>(4)?),
                content_sha256: row.get(5)?,
                parser_version: row.get(6)?,
                chunker_version: row.get(7)?,
            })
        })?;
        for row in rows {
            let doc = row?;
            index.docs.insert(doc.path.clone(), doc);
        }
    }
    {
        let mut stmt = db.prepare(
            "SELECT chunk_id, document_path, file_type, ordinal, text,
                    text_sha256, len_tokens, mtime_epoch
             FROM kb_chunks
             WHERE owner_user_key = ?1 AND namespace = ?2
             ORDER BY document_path, ordinal, chunk_id",
        )?;
        let rows = stmt.query_map(params![owner, namespace], |row| {
            Ok(Chunk {
                chunk_id: row.get(0)?,
                path: row.get(1)?,
                file_type: row.get(2)?,
                offset: nonnegative_usize(row.get::<_, i64>(3)?),
                text: row.get(4)?,
                text_sha256: row.get(5)?,
                len_tokens: nonnegative_usize(row.get::<_, i64>(6)?),
                mtime_epoch: row.get(7)?,
            })
        })?;
        index.chunks = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    }
    Ok(index)
}

fn load_namespace_header(db: &Connection, owner: &str, namespace: &str) -> Result<NamespaceIndex> {
    db.query_row(
        "SELECT updated_at_epoch, next_chunk_seq, revision, parser_version,
                chunker_version, embedding_version
         FROM kb_namespaces
         WHERE owner_user_key = ?1 AND namespace = ?2",
        params![owner, namespace],
        |row| {
            Ok(NamespaceIndex {
                namespace: namespace.to_string(),
                owner_user_key: owner.to_string(),
                updated_at_epoch: row.get(0)?,
                next_chunk_seq: nonnegative_u64(row.get::<_, i64>(1)?),
                revision: nonnegative_u64(row.get::<_, i64>(2)?),
                parser_version: row.get(3)?,
                chunker_version: row.get(4)?,
                embedding_version: row.get(5)?,
                docs: HashMap::new(),
                chunks: Vec::new(),
            })
        },
    )
    .optional()?
    .ok_or_else(|| anyhow!("namespace not found"))
}

fn fts_candidate_chunk_ids(
    db: &Connection,
    owner: &str,
    source_prefix: &str,
    terms: &[String],
    limit: usize,
) -> Result<Vec<String>> {
    let query = terms
        .iter()
        .filter(|term| !term.trim().is_empty())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ");
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = db.prepare(
        "SELECT i.source_ref
         FROM memory_retrieval_index_fts
         JOIN memory_retrieval_index i ON i.id = memory_retrieval_index_fts.rowid
         WHERE memory_retrieval_index_fts MATCH ?1
           AND i.source_kind = 'kb_doc'
           AND i.user_key = ?2
           AND substr(i.source_ref, 1, length(?3)) = ?3
         ORDER BY bm25(memory_retrieval_index_fts), i.id
         LIMIT ?4",
    )?;
    let refs = stmt
        .query_map(
            params![query, owner, source_prefix, limit.max(1) as i64],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(refs
        .into_iter()
        .filter_map(|source_ref| source_ref.strip_prefix(source_prefix).map(str::to_string))
        .collect())
}

fn fallback_candidate_chunk_ids(
    db: &Connection,
    owner: &str,
    namespace: &str,
    limit: usize,
) -> Result<Vec<String>> {
    let mut stmt = db.prepare(
        "SELECT chunk_id FROM kb_chunks
         WHERE owner_user_key = ?1 AND namespace = ?2
         ORDER BY document_path, ordinal, chunk_id
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![owner, namespace, limit.max(1) as i64], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn load_chunks_by_ids(
    db: &Connection,
    owner: &str,
    namespace: &str,
    chunk_ids: &[String],
) -> Result<Vec<Chunk>> {
    let mut chunks = Vec::with_capacity(chunk_ids.len());
    for chunk_id in chunk_ids {
        let chunk = db
            .query_row(
                "SELECT chunk_id, document_path, file_type, ordinal, text,
                        text_sha256, len_tokens, mtime_epoch
                 FROM kb_chunks
                 WHERE owner_user_key = ?1 AND namespace = ?2 AND chunk_id = ?3",
                params![owner, namespace, chunk_id],
                |row| {
                    Ok(Chunk {
                        chunk_id: row.get(0)?,
                        path: row.get(1)?,
                        file_type: row.get(2)?,
                        offset: nonnegative_usize(row.get::<_, i64>(3)?),
                        text: row.get(4)?,
                        text_sha256: row.get(5)?,
                        len_tokens: nonnegative_usize(row.get::<_, i64>(6)?),
                        mtime_epoch: row.get(7)?,
                    })
                },
            )
            .optional()?;
        if let Some(chunk) = chunk {
            chunks.push(chunk);
        }
    }
    Ok(chunks)
}

fn document_fingerprints(
    db: &Connection,
    owner: &str,
    namespace: &str,
) -> Result<HashMap<String, String>> {
    let mut stmt = db.prepare(
        "SELECT path, file_type, mtime_epoch, size_bytes, chunk_count,
                content_sha256, parser_version, chunker_version
         FROM kb_documents
         WHERE owner_user_key = ?1 AND namespace = ?2",
    )?;
    let rows = stmt.query_map(params![owner, namespace], |row| {
        let doc = DocMeta {
            path: row.get(0)?,
            file_type: row.get(1)?,
            mtime_epoch: row.get(2)?,
            size: nonnegative_u64(row.get::<_, i64>(3)?),
            chunks: nonnegative_usize(row.get::<_, i64>(4)?),
            content_sha256: row.get(5)?,
            parser_version: row.get(6)?,
            chunker_version: row.get(7)?,
        };
        Ok((doc.path.clone(), document_fingerprint(&doc)))
    })?;
    Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
}

fn document_fingerprint(doc: &DocMeta) -> String {
    sha256_hex(
        format!(
            "{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            doc.path,
            doc.file_type,
            doc.mtime_epoch,
            doc.size,
            doc.chunks,
            doc.content_sha256,
            doc.parser_version,
            doc.chunker_version,
        )
        .as_bytes(),
    )
}

fn legacy_document_digest(index: &NamespaceIndex, path: &str) -> String {
    let mut chunks = index
        .chunks
        .iter()
        .filter(|chunk| chunk.path == path)
        .collect::<Vec<_>>();
    chunks.sort_by_key(|chunk| chunk.offset);
    let mut bytes = Vec::new();
    for chunk in chunks {
        bytes.extend_from_slice(chunk.text.as_bytes());
        bytes.push(0);
    }
    sha256_hex(&bytes)
}

fn validate_owner(runtime: &KbRuntime, index: &NamespaceIndex) -> Result<()> {
    if index.owner_user_key != runtime.scope_user_key {
        return Err(anyhow!("namespace is owned by another user scope"));
    }
    Ok(())
}

fn validate_snapshots(snapshots: &[NamespaceIndex]) -> Result<()> {
    for snapshot in snapshots {
        validate_snapshot(snapshot)?;
    }
    Ok(())
}

fn validate_snapshot(snapshot: &NamespaceIndex) -> Result<()> {
    if snapshot.owner_user_key.trim().is_empty() || snapshot.namespace.trim().is_empty() {
        return Err(anyhow!(
            "legacy KB snapshot has no stable owner or namespace identity"
        ));
    }
    if snapshot
        .chunks
        .iter()
        .any(|chunk| !snapshot.docs.contains_key(&chunk.path))
    {
        return Err(anyhow!("KB snapshot contains an orphan chunk"));
    }
    Ok(())
}

fn verify_snapshots(db: &Connection, snapshots: &[NamespaceIndex]) -> Result<()> {
    for snapshot in snapshots {
        let loaded = load_namespace_from(db, &snapshot.owner_user_key, &snapshot.namespace)?;
        if loaded.docs.len() != snapshot.docs.len() || loaded.chunks.len() != snapshot.chunks.len()
        {
            return Err(anyhow!("legacy KB snapshot row-count verification failed"));
        }
        if normalized_digest(&loaded) != normalized_digest(snapshot) {
            return Err(anyhow!("legacy KB snapshot digest verification failed"));
        }
    }
    Ok(())
}

fn snapshot_digest(snapshots: &[NamespaceIndex]) -> String {
    let mut ordered = snapshots.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        (&left.owner_user_key, &left.namespace).cmp(&(&right.owner_user_key, &right.namespace))
    });
    let mut bytes = Vec::new();
    for snapshot in ordered {
        bytes.extend_from_slice(normalized_digest(snapshot).as_bytes());
        bytes.push(0);
    }
    sha256_hex(&bytes)
}

fn normalized_digest(snapshot: &NamespaceIndex) -> String {
    let mut values = vec![
        snapshot.owner_user_key.clone(),
        snapshot.namespace.clone(),
        snapshot.updated_at_epoch.to_string(),
        snapshot.next_chunk_seq.to_string(),
    ];
    let mut docs = snapshot.docs.values().collect::<Vec<_>>();
    docs.sort_by(|left, right| left.path.cmp(&right.path));
    for doc in docs {
        values.extend([
            doc.path.clone(),
            doc.file_type.clone(),
            doc.mtime_epoch.to_string(),
            doc.size.to_string(),
            doc.chunks.to_string(),
            if doc.content_sha256.is_empty() {
                legacy_document_digest(snapshot, &doc.path)
            } else {
                doc.content_sha256.clone()
            },
            effective_version(&doc.parser_version, default_parser_version()),
            effective_version(&doc.chunker_version, default_chunker_version()),
        ]);
    }
    let mut chunks = snapshot.chunks.iter().collect::<Vec<_>>();
    chunks.sort_by(|left, right| {
        (&left.path, left.offset, &left.chunk_id).cmp(&(&right.path, right.offset, &right.chunk_id))
    });
    for chunk in chunks {
        values.extend([
            chunk.chunk_id.clone(),
            chunk.path.clone(),
            chunk.file_type.clone(),
            chunk.offset.to_string(),
            chunk.text.clone(),
            chunk.len_tokens.to_string(),
            chunk.mtime_epoch.to_string(),
        ]);
    }
    sha256_hex(values.join("\0").as_bytes())
}

fn migration_complete(db: &Connection, migration_id: &str) -> Result<bool> {
    Ok(db
        .query_row(
            "SELECT 1 FROM skill_storage_migrations WHERE migration_id = ?1",
            params![migration_id],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false))
}

fn namespace_exists_in(db: &Connection, owner: &str, namespace: &str) -> Result<bool> {
    let exists: i64 = db.query_row(
        "SELECT COUNT(*) FROM kb_namespaces
         WHERE owner_user_key = ?1 AND namespace = ?2",
        params![owner, namespace],
        |row| row.get(0),
    )?;
    Ok(exists > 0)
}

fn count_rows(
    db: &Connection,
    table: &str,
    predicate: &str,
    owner: &str,
    namespace: &str,
) -> Result<usize> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE {predicate}");
    let count: i64 = db.query_row(&sql, params![owner, namespace], |row| row.get(0))?;
    Ok(nonnegative_usize(count))
}

fn table_exists(db: &Connection, table: &str) -> Result<bool> {
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?1",
        params![table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn table_has_column(db: &Connection, table: &str, column: &str) -> Result<bool> {
    if !table_exists(db, table)? {
        return Ok(false);
    }
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names.iter().any(|name| name == column))
}

fn has_fts(db: &Connection) -> Result<bool> {
    table_exists(db, "memory_retrieval_index_fts")
}

fn integrity_check(db: &Connection) -> Result<()> {
    let result: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if result != "ok" {
        return Err(anyhow!("KB skill storage integrity check failed"));
    }
    Ok(())
}

fn source_ref(index: &NamespaceIndex, chunk_id: &str) -> String {
    source_ref_parts(&index.owner_user_key, &index.namespace, chunk_id)
}

fn source_ref_parts(owner: &str, namespace: &str, chunk_id: &str) -> String {
    format!(
        "kb:{}:{}:{}",
        owner.trim(),
        namespace.trim(),
        chunk_id.trim()
    )
}

fn effective_version(value: &str, fallback: String) -> String {
    if value.trim().is_empty() {
        fallback
    } else {
        value.to_string()
    }
}

fn nonnegative_u64(value: i64) -> u64 {
    value.max(0) as u64
}

fn nonnegative_usize(value: i64) -> usize {
    value.max(0) as usize
}

fn legacy_root(runtime: &KbRuntime) -> PathBuf {
    if let Ok(value) = std::env::var("KB_ROOT") {
        let path = PathBuf::from(value);
        return if path.is_absolute() {
            path
        } else {
            runtime.workspace_root.join(path)
        };
    }
    runtime.workspace_root.join("data").join("kb")
}

fn collect_json_files(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("read legacy KB directory failed: {}", directory.display()))?
        {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn prune_empty_directories(root: &Path) {
    if !root.exists() {
        return;
    }
    let mut directories = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        directories.push(directory.clone());
        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    pending.push(entry.path());
                }
            }
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        let _ = fs::remove_dir(directory);
    }
}

fn database_identity(path: &Path) -> String {
    format!("sha256:{}", sha256_hex(path.as_os_str().as_encoded_bytes()))
}

fn build_topic_tags(text: &str) -> String {
    super::tokenize_terms(text)
        .into_iter()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ")
}

fn embed_text_locally(text: &str) -> Vec<f32> {
    const DIMS: usize = 24;
    let mut vector = vec![0.0_f32; DIMS];
    for token in super::tokenize_terms(text) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        token.hash(&mut hasher);
        let index = (hasher.finish() as usize) % DIMS;
        vector[index] += 1.0;
    }
    normalize_vector(&mut vector);
    vector
}

fn vector_to_json(vector: &[f32]) -> String {
    serde_json::to_string(vector).unwrap_or_else(|_| "[]".to_string())
}

fn normalize_vector(vector: &mut [f32]) {
    let norm = vector
        .iter()
        .map(|value| (*value as f64) * (*value as f64))
        .sum::<f64>()
        .sqrt() as f32;
    if norm <= f32::EPSILON {
        return;
    }
    for value in vector {
        *value /= norm;
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
