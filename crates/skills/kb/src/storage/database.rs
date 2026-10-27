use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::namespace_index::{legacy_document_digest, load_namespace_from, persist_namespace};
use super::{
    build_topic_tags, default_chunker_version, default_parser_version, effective_version,
    nonnegative_u64, nonnegative_usize, sha256_hex, KbRuntime, NamespaceIndex, StoragePolicyError,
    StorageQuotas, STORAGE_SCHEMA_VERSION,
};

const LEGACY_JSON_MIGRATION_ID: &str = "legacy-kb-json-v1";
const SQLITE_V1_MIGRATION_ID: &str = "kb-payload-json-to-normalized-v2";
pub(super) const SCHEMA_V6_CUTOVER_MIGRATION_ID: &str = "kb-schema-v6-cutover";
const SCHEMA_V7_CHUNK_LOCATION_MIGRATION_ID: &str = "kb-schema-v7-chunk-location";
const WAL_AUTOCHECKPOINT_PAGES: i64 = 256;
const WAL_JOURNAL_SIZE_LIMIT_BYTES: i64 = 8 * 1024 * 1024;
const STORAGE_MAINTENANCE_INTERVAL_SECONDS: i64 = 24 * 60 * 60;
const INCREMENTAL_VACUUM_PAGES: i64 = 64;
const DEFAULT_MAX_NAMESPACES_PER_OWNER: u64 = 256;
const DEFAULT_MAX_DOCUMENTS_PER_OWNER: u64 = 100_000;
const DEFAULT_MAX_CHUNKS_PER_OWNER: u64 = 1_000_000;
const DEFAULT_MAX_VECTORS_PER_OWNER: u64 = 1_000_000;
const DEFAULT_MAX_DATABASE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const DEFAULT_MAX_ACTIVE_JOBS_PER_OWNER: u64 = 4;

pub(super) fn open(runtime: &KbRuntime) -> Result<Connection> {
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
    let initialize_incremental_vacuum = fs::metadata(&runtime.storage_database_path)
        .map(|metadata| metadata.len() == 0)
        .unwrap_or(true);
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
    if initialize_incremental_vacuum {
        db.pragma_update(None, "auto_vacuum", 2_i64)?;
    }
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "synchronous", "NORMAL")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    let wal_autocheckpoint_pages = if runtime.constrained_storage() {
        64
    } else {
        WAL_AUTOCHECKPOINT_PAGES
    };
    let journal_size_limit_bytes = if runtime.constrained_storage() {
        4 * 1024 * 1024
    } else {
        WAL_JOURNAL_SIZE_LIMIT_BYTES
    };
    db.pragma_update(None, "wal_autocheckpoint", wal_autocheckpoint_pages)?;
    db.pragma_update(None, "journal_size_limit", journal_size_limit_bytes)?;
    if let Some(cache_size_kib) = runtime.storage_cache_size_kib.filter(|value| *value > 0) {
        db.pragma_update(None, "cache_size", -(i64::from(cache_size_kib)))?;
    }
    if let Some(mmap_size_bytes) = runtime.storage_mmap_size_bytes {
        db.pragma_update(
            None,
            "mmap_size",
            mmap_size_bytes.min(i64::MAX as u64) as i64,
        )?;
    }
    if runtime.storage_temp_store.as_deref() == Some("file") {
        db.pragma_update(None, "temp_store", 1_i64)?;
    }
    Ok(db)
}

pub(super) fn ensure_schema(db: &mut Connection) -> Result<()> {
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
        );",
    )?;
    if storage_schema_is_current(db)? {
        return Ok(());
    }
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS kb_storage_maintenance (
            task_name TEXT PRIMARY KEY,
            last_run_epoch INTEGER NOT NULL,
            last_result TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS kb_storage_quotas (
            owner_user_key TEXT PRIMARY KEY,
            max_namespaces INTEGER NOT NULL,
            max_documents INTEGER NOT NULL,
            max_chunks INTEGER NOT NULL,
            max_vectors INTEGER NOT NULL,
            max_database_bytes INTEGER NOT NULL,
            max_active_jobs INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL
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
            embedding_model TEXT NOT NULL DEFAULT 'local-hash-v3',
            embedding_dims INTEGER NOT NULL DEFAULT 24,
            embedding_version TEXT NOT NULL DEFAULT 'local-hash-v3',
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
    clear_inline_vector_payloads(db)?;
    db.execute(
        "INSERT INTO skill_storage_migrations (
            migration_id, source_identity, source_rows, verified_digest
         ) VALUES (?1, 'kb-private-schema', 0, ?2)
         ON CONFLICT(migration_id) DO UPDATE SET
            source_identity = excluded.source_identity,
            source_rows = excluded.source_rows,
            verified_digest = excluded.verified_digest,
            completed_at = CURRENT_TIMESTAMP",
        params![
            SCHEMA_V6_CUTOVER_MIGRATION_ID,
            sha256_hex(SCHEMA_V6_CUTOVER_MIGRATION_ID.as_bytes()),
        ],
    )?;
    db.execute(
        "INSERT INTO skill_storage_migrations (
            migration_id, source_identity, source_rows, verified_digest
         ) VALUES (?1, 'kb-chunks', 0, ?2)
         ON CONFLICT(migration_id) DO UPDATE SET
            source_identity = excluded.source_identity,
            source_rows = excluded.source_rows,
            verified_digest = excluded.verified_digest,
            completed_at = CURRENT_TIMESTAMP",
        params![
            SCHEMA_V7_CHUNK_LOCATION_MIGRATION_ID,
            sha256_hex(SCHEMA_V7_CHUNK_LOCATION_MIGRATION_ID.as_bytes()),
        ],
    )?;
    db.execute(
        "INSERT INTO skill_storage_metadata (skill_name, schema_version)
         VALUES ('kb', ?1)
         ON CONFLICT(skill_name) DO UPDATE SET
            schema_version = MAX(skill_storage_metadata.schema_version, excluded.schema_version),
            updated_at = CURRENT_TIMESTAMP",
        params![STORAGE_SCHEMA_VERSION],
    )?;
    Ok(())
}

pub(super) fn storage_schema_is_current(db: &Connection) -> Result<bool> {
    let version = db
        .query_row(
            "SELECT schema_version FROM skill_storage_metadata WHERE skill_name = 'kb'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0);
    if version < i64::from(STORAGE_SCHEMA_VERSION) {
        return Ok(false);
    }
    if table_exists(db, "kb_namespaces_v1")?
        || table_has_column(db, "kb_namespaces", "payload_json")?
    {
        return Ok(false);
    }
    for table in [
        "kb_storage_maintenance",
        "kb_storage_quotas",
        "memory_retrieval_index",
        "memory_retrieval_index_fts",
        "kb_namespaces",
        "kb_documents",
        "kb_chunks",
        "kb_ingest_jobs",
        "kb_embedding_profiles",
        "kb_vector_generations",
        "kb_vector_projection",
        "kb_vector_ann_projection",
    ] {
        if !table_exists(db, table)? {
            return Ok(false);
        }
    }
    if !table_has_column(db, "kb_namespaces", "active_vector_generation")?
        || !table_has_column(db, "kb_chunks", "content_kind")?
        || !table_has_column(db, "kb_chunks", "location_json")?
    {
        return Ok(false);
    }
    let v6_complete = db.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM skill_storage_migrations WHERE migration_id = ?1
         )",
        [SCHEMA_V6_CUTOVER_MIGRATION_ID],
        |row| row.get::<_, bool>(0),
    )?;
    let v7_complete = db.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM skill_storage_migrations WHERE migration_id = ?1
         )",
        [SCHEMA_V7_CHUNK_LOCATION_MIGRATION_ID],
        |row| row.get::<_, bool>(0),
    )?;
    Ok(v6_complete && v7_complete)
}

fn clear_inline_vector_payloads(db: &mut Connection) -> Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let has_fts_projection = has_fts(&tx)?;
    let missing_topics = {
        let mut stmt = tx.prepare(
            "SELECT id, search_text FROM memory_retrieval_index
             WHERE source_kind = 'kb_doc' AND topic_tags = ''
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (row_id, search_text) in missing_topics {
        let topic_tags = build_topic_tags(&search_text);
        tx.execute(
            "UPDATE memory_retrieval_index SET topic_tags = ?2 WHERE id = ?1",
            params![row_id, topic_tags],
        )?;
        if has_fts_projection {
            tx.execute(
                "DELETE FROM memory_retrieval_index_fts WHERE rowid = ?1",
                params![row_id],
            )?;
            tx.execute(
                "INSERT INTO memory_retrieval_index_fts(rowid, search_text, topic_tags)
                 VALUES (?1, ?2, ?3)",
                params![row_id, search_text, topic_tags],
            )?;
        }
    }
    tx.execute(
        "UPDATE memory_retrieval_index
         SET vector_json = '[]', embedding_model = 'none',
             embedding_dims = 0, embedding_version = 'none'
         WHERE source_kind = 'kb_doc'
           AND (vector_json != '[]' OR embedding_model != 'none'
                OR embedding_dims != 0 OR embedding_version != 'none')",
        [],
    )?;
    tx.execute(
        "UPDATE kb_namespaces SET embedding_version = ?1
         WHERE active_vector_generation IS NOT NULL AND embedding_version != ?1",
        params![claw_core::retrieval_embedding::LOCAL_HASH_VERSION],
    )?;
    tx.commit()?;
    Ok(())
}

fn create_normalized_schema(db: &mut Connection) -> Result<()> {
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
        CREATE INDEX IF NOT EXISTS idx_kb_documents_owner_namespace_digest
        ON kb_documents(owner_user_key, namespace, content_sha256, path);
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
            content_kind TEXT NOT NULL DEFAULT 'plain_text',
            location_json TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY(owner_user_key, namespace, chunk_id),
            FOREIGN KEY(owner_user_key, namespace, document_path)
                REFERENCES kb_documents(owner_user_key, namespace, path)
                ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_kb_chunks_owner_namespace_document
        ON kb_chunks(owner_user_key, namespace, document_path, ordinal);
        CREATE INDEX IF NOT EXISTS idx_kb_chunks_owner_namespace_filter
        ON kb_chunks(
            owner_user_key, namespace, file_type, mtime_epoch, document_path, chunk_id
        );
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
    if !table_has_column(db, "kb_namespaces", "active_vector_generation")? {
        db.execute(
            "ALTER TABLE kb_namespaces ADD COLUMN active_vector_generation INTEGER",
            [],
        )?;
    }
    if !table_has_column(db, "kb_chunks", "content_kind")? {
        db.execute(
            "ALTER TABLE kb_chunks ADD COLUMN content_kind TEXT NOT NULL DEFAULT 'plain_text'",
            [],
        )?;
    }
    if !table_has_column(db, "kb_chunks", "location_json")? {
        db.execute(
            "ALTER TABLE kb_chunks ADD COLUMN location_json TEXT NOT NULL DEFAULT '{}'",
            [],
        )?;
    }
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS kb_embedding_profiles (
            profile_id TEXT PRIMARY KEY,
            provider_kind TEXT NOT NULL,
            model_id TEXT NOT NULL,
            dimensions INTEGER NOT NULL,
            normalization TEXT NOT NULL,
            profile_version TEXT NOT NULL,
            cross_language_semantic INTEGER NOT NULL,
            config_digest TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS kb_vector_generations (
            owner_user_key TEXT NOT NULL,
            namespace TEXT NOT NULL,
            generation INTEGER NOT NULL,
            profile_id TEXT NOT NULL,
            namespace_revision INTEGER NOT NULL,
            state TEXT NOT NULL,
            cursor_chunk_id TEXT,
            expected_rows INTEGER NOT NULL,
            indexed_rows INTEGER NOT NULL,
            source_digest TEXT NOT NULL,
            projection_digest TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            activated_at_epoch INTEGER,
            PRIMARY KEY(owner_user_key, namespace, generation),
            FOREIGN KEY(owner_user_key, namespace)
                REFERENCES kb_namespaces(owner_user_key, namespace)
                ON DELETE CASCADE,
            FOREIGN KEY(profile_id)
                REFERENCES kb_embedding_profiles(profile_id)
        );
        CREATE INDEX IF NOT EXISTS idx_kb_vector_generations_state
        ON kb_vector_generations(owner_user_key, namespace, state, generation DESC);
        CREATE INDEX IF NOT EXISTS idx_kb_vector_generations_resume
        ON kb_vector_generations(
            owner_user_key, namespace, state, profile_id, namespace_revision,
            source_digest, generation DESC
        );
        CREATE TABLE IF NOT EXISTS kb_vector_projection (
            owner_user_key TEXT NOT NULL,
            namespace TEXT NOT NULL,
            generation INTEGER NOT NULL,
            chunk_id TEXT NOT NULL,
            profile_id TEXT NOT NULL,
            dimensions INTEGER NOT NULL,
            content_digest TEXT NOT NULL,
            vector_json TEXT NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            PRIMARY KEY(owner_user_key, namespace, generation, chunk_id),
            FOREIGN KEY(owner_user_key, namespace, generation)
                REFERENCES kb_vector_generations(owner_user_key, namespace, generation)
                ON DELETE CASCADE,
            FOREIGN KEY(owner_user_key, namespace, chunk_id)
                REFERENCES kb_chunks(owner_user_key, namespace, chunk_id)
                ON DELETE CASCADE,
            FOREIGN KEY(profile_id)
                REFERENCES kb_embedding_profiles(profile_id)
        );
        CREATE INDEX IF NOT EXISTS idx_kb_vector_projection_generation
        ON kb_vector_projection(owner_user_key, namespace, generation, chunk_id);
        CREATE TABLE IF NOT EXISTS kb_vector_ann_projection (
            owner_user_key TEXT NOT NULL,
            namespace TEXT NOT NULL,
            generation INTEGER NOT NULL,
            chunk_id TEXT NOT NULL,
            profile_id TEXT NOT NULL,
            projection_version TEXT NOT NULL,
            band INTEGER NOT NULL,
            bucket INTEGER NOT NULL,
            PRIMARY KEY(owner_user_key, namespace, generation, chunk_id, band),
            FOREIGN KEY(owner_user_key, namespace, generation, chunk_id)
                REFERENCES kb_vector_projection(owner_user_key, namespace, generation, chunk_id)
                ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_kb_vector_ann_bucket
        ON kb_vector_ann_projection(
            owner_user_key, namespace, generation, profile_id,
            projection_version, band, bucket, chunk_id
        );",
    )?;
    register_local_embedding_profile(db)?;
    Ok(())
}

fn register_local_embedding_profile(db: &Connection) -> Result<()> {
    let profile = claw_core::retrieval_embedding::local_hash_profile();
    profile.validate()?;
    let profile_json = serde_json::to_vec(&profile)?;
    let now = crate::now_epoch();
    db.execute(
        "INSERT INTO kb_embedding_profiles (
            profile_id, provider_kind, model_id, dimensions, normalization,
            profile_version, cross_language_semantic, config_digest,
            created_at_epoch, updated_at_epoch
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT(profile_id) DO UPDATE SET
            provider_kind = excluded.provider_kind,
            model_id = excluded.model_id,
            dimensions = excluded.dimensions,
            normalization = excluded.normalization,
            profile_version = excluded.profile_version,
            cross_language_semantic = excluded.cross_language_semantic,
            config_digest = excluded.config_digest,
            updated_at_epoch = excluded.updated_at_epoch",
        params![
            profile.profile_id,
            profile.provider_kind,
            profile.model_id,
            profile.dimensions as i64,
            profile.normalization,
            profile.version,
            if profile.cross_language_semantic {
                1
            } else {
                0
            },
            sha256_hex(&profile_json),
            now,
        ],
    )?;
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

pub(super) fn migrate_legacy_json(runtime: &KbRuntime, db: &mut Connection) -> Result<()> {
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

fn validate_snapshots(snapshots: &[NamespaceIndex]) -> Result<()> {
    for snapshot in snapshots {
        validate_snapshot(snapshot)?;
    }
    Ok(())
}

pub(super) fn validate_snapshot(snapshot: &NamespaceIndex) -> Result<()> {
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

pub(super) fn ensure_owner_quota_row(db: &Connection, owner: &str) -> Result<()> {
    let namespaces = owner_row_count(db, "kb_namespaces", owner)?;
    let documents = owner_row_count(db, "kb_documents", owner)?;
    let chunks = owner_row_count(db, "kb_chunks", owner)?;
    let vectors = owner_row_count(db, "kb_vector_projection", owner)?;
    db.execute(
        "INSERT INTO kb_storage_quotas (
            owner_user_key, max_namespaces, max_documents, max_chunks,
            max_vectors, max_database_bytes, max_active_jobs, updated_at_epoch
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(owner_user_key) DO NOTHING",
        params![
            owner,
            namespaces.max(DEFAULT_MAX_NAMESPACES_PER_OWNER) as i64,
            documents.max(DEFAULT_MAX_DOCUMENTS_PER_OWNER) as i64,
            chunks.max(DEFAULT_MAX_CHUNKS_PER_OWNER) as i64,
            vectors.max(DEFAULT_MAX_VECTORS_PER_OWNER) as i64,
            DEFAULT_MAX_DATABASE_BYTES as i64,
            DEFAULT_MAX_ACTIVE_JOBS_PER_OWNER as i64,
            crate::now_epoch(),
        ],
    )?;
    Ok(())
}

pub(super) fn owner_quotas(db: &Connection, owner: &str) -> Result<StorageQuotas> {
    db.query_row(
        "SELECT max_namespaces, max_documents, max_chunks, max_vectors,
                max_database_bytes, max_active_jobs
         FROM kb_storage_quotas WHERE owner_user_key = ?1",
        params![owner],
        |row| {
            Ok(StorageQuotas {
                max_namespaces: nonnegative_u64(row.get::<_, i64>(0)?),
                max_documents: nonnegative_u64(row.get::<_, i64>(1)?),
                max_chunks: nonnegative_u64(row.get::<_, i64>(2)?),
                max_vectors: nonnegative_u64(row.get::<_, i64>(3)?),
                max_database_bytes: nonnegative_u64(row.get::<_, i64>(4)?),
                max_active_jobs: nonnegative_u64(row.get::<_, i64>(5)?),
            })
        },
    )
    .map_err(Into::into)
}

pub(super) fn active_ingest_job_for_namespace(
    db: &Connection,
    owner: &str,
    namespace: &str,
) -> Result<Option<String>> {
    db.query_row(
        "SELECT job_id FROM kb_ingest_jobs
         WHERE owner_user_key = ?1 AND namespace = ?2
           AND status IN ('running', 'waiting')
         ORDER BY updated_at_epoch DESC LIMIT 1",
        params![owner, namespace],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn active_vector_job_for_namespace(
    db: &Connection,
    owner: &str,
    namespace: &str,
) -> Result<Option<String>> {
    db.query_row(
        "SELECT 'vector:' || namespace || ':' || generation
         FROM kb_vector_generations
         WHERE owner_user_key = ?1 AND namespace = ?2 AND state = 'building'
         ORDER BY updated_at_epoch DESC LIMIT 1",
        params![owner, namespace],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn owner_active_job_count(db: &Connection, owner: &str) -> Result<u64> {
    let count = db.query_row(
        "SELECT
            (SELECT COUNT(*) FROM kb_ingest_jobs
             WHERE owner_user_key = ?1 AND status IN ('running', 'waiting'))
          + (SELECT COUNT(*) FROM kb_vector_generations
             WHERE owner_user_key = ?1 AND state = 'building')",
        params![owner],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(nonnegative_u64(count))
}

pub(super) fn latest_owner_active_job_id(db: &Connection, owner: &str) -> Result<String> {
    db.query_row(
        "SELECT job_ref FROM (
            SELECT 'ingest:' || job_id AS job_ref, updated_at_epoch
            FROM kb_ingest_jobs
            WHERE owner_user_key = ?1 AND status IN ('running', 'waiting')
            UNION ALL
            SELECT 'vector:' || namespace || ':' || generation AS job_ref, updated_at_epoch
            FROM kb_vector_generations
            WHERE owner_user_key = ?1 AND state = 'building'
         ) ORDER BY updated_at_epoch DESC LIMIT 1",
        params![owner],
        |row| row.get::<_, String>(0),
    )
    .map_err(Into::into)
}

pub(super) fn enforce_storage_quotas(db: &Connection, owner: &str) -> Result<()> {
    ensure_owner_quota_row(db, owner)?;
    let quotas = owner_quotas(db, owner)?;
    for (resource, actual, limit) in [
        (
            "namespaces",
            owner_row_count(db, "kb_namespaces", owner)?,
            quotas.max_namespaces,
        ),
        (
            "documents",
            owner_row_count(db, "kb_documents", owner)?,
            quotas.max_documents,
        ),
        (
            "chunks",
            owner_row_count(db, "kb_chunks", owner)?,
            quotas.max_chunks,
        ),
        (
            "vectors",
            owner_row_count(db, "kb_vector_projection", owner)?,
            quotas.max_vectors,
        ),
    ] {
        if actual > limit {
            return Err(StoragePolicyError::quota(resource, limit, actual).into());
        }
    }
    let page_count: i64 = db.query_row("PRAGMA page_count", [], |row| row.get(0))?;
    let page_size: i64 = db.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    let database_bytes = (page_count.max(0) as u64).saturating_mul(page_size.max(0) as u64);
    if database_bytes > quotas.max_database_bytes {
        return Err(StoragePolicyError::quota(
            "database_bytes",
            quotas.max_database_bytes,
            database_bytes,
        )
        .into());
    }
    Ok(())
}

fn owner_row_count(db: &Connection, table: &str, owner: &str) -> Result<u64> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE owner_user_key = ?1");
    let count: i64 = db.query_row(&sql, params![owner], |row| row.get(0))?;
    Ok(nonnegative_u64(count))
}

pub(super) fn count_rows(
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

pub(super) fn table_exists(db: &Connection, table: &str) -> Result<bool> {
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?1",
        params![table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub(super) fn table_has_column(db: &Connection, table: &str, column: &str) -> Result<bool> {
    if !table_exists(db, table)? {
        return Ok(false);
    }
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names.iter().any(|name| name == column))
}

pub(super) fn has_fts(db: &Connection) -> Result<bool> {
    table_exists(db, "memory_retrieval_index_fts")
}

pub(super) fn integrity_check(db: &Connection) -> Result<()> {
    let result: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if result != "ok" {
        return Err(anyhow!("KB skill storage integrity check failed"));
    }
    Ok(())
}

pub(super) fn run_bounded_storage_maintenance(db: &mut Connection) -> Result<()> {
    let now = crate::now_epoch();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let last_run = tx
        .query_row(
            "SELECT last_run_epoch FROM kb_storage_maintenance
             WHERE task_name = 'bounded_compaction'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0);
    if now.saturating_sub(last_run) < STORAGE_MAINTENANCE_INTERVAL_SECONDS {
        tx.commit()?;
        return Ok(());
    }
    tx.execute(
        "INSERT INTO kb_storage_maintenance(task_name, last_run_epoch, last_result)
         VALUES ('bounded_compaction', ?1, 'running')
         ON CONFLICT(task_name) DO UPDATE SET
            last_run_epoch = excluded.last_run_epoch,
            last_result = excluded.last_result",
        params![now],
    )?;
    tx.commit()?;

    let result = (|| -> Result<()> {
        integrity_check(db)?;
        db.pragma_update(None, "analysis_limit", 400_i64)?;
        db.execute_batch("PRAGMA optimize;")?;
        let _: (i64, i64, i64) = db.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        let auto_vacuum: i64 = db.query_row("PRAGMA auto_vacuum", [], |row| row.get(0))?;
        if auto_vacuum == 2 {
            db.execute_batch(&format!(
                "PRAGMA incremental_vacuum({INCREMENTAL_VACUUM_PAGES});"
            ))?;
        }
        Ok(())
    })();
    db.execute(
        "UPDATE kb_storage_maintenance SET last_result = ?1
         WHERE task_name = 'bounded_compaction'",
        params![if result.is_ok() { "ok" } else { "failed" }],
    )?;
    result
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

pub(super) fn database_identity(path: &Path) -> String {
    format!("sha256:{}", sha256_hex(path.as_os_str().as_encoded_bytes()))
}
