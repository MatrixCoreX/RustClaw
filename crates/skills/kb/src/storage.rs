use super::{KbRuntime, NamespaceIndex};
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const LEGACY_JSON_MIGRATION_ID: &str = "legacy-kb-json-v1";

pub(super) fn initialize(runtime: &KbRuntime) -> Result<()> {
    let db = open(runtime)?;
    ensure_schema(&db)?;
    migrate_legacy_json(runtime, &db)?;
    integrity_check(&db)
}

pub(super) fn namespace_exists(runtime: &KbRuntime, namespace: &str) -> Result<bool> {
    let db = open(runtime)?;
    ensure_schema(&db)?;
    let exists: i64 = db.query_row(
        "SELECT COUNT(*) FROM kb_namespaces
         WHERE owner_user_key = ?1 AND namespace = ?2",
        params![runtime.scope_user_key, namespace],
        |row| row.get(0),
    )?;
    Ok(exists > 0)
}

pub(super) fn load_namespace(runtime: &KbRuntime, namespace: &str) -> Result<NamespaceIndex> {
    let db = open(runtime)?;
    ensure_schema(&db)?;
    let payload = db
        .query_row(
            "SELECT payload_json FROM kb_namespaces
             WHERE owner_user_key = ?1 AND namespace = ?2",
            params![runtime.scope_user_key, namespace],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow!("namespace not found"))?;
    let index = serde_json::from_str::<NamespaceIndex>(&payload)
        .context("namespace payload is malformed")?;
    validate_owner(runtime, &index)?;
    Ok(index)
}

pub(super) fn save_namespace(runtime: &KbRuntime, index: &NamespaceIndex) -> Result<()> {
    validate_owner(runtime, index)?;
    let db = open(runtime)?;
    ensure_schema(&db)?;
    save_namespace_for_owner(&db, index)
}

pub(super) fn list_namespaces(runtime: &KbRuntime) -> Result<Vec<NamespaceIndex>> {
    let db = open(runtime)?;
    ensure_schema(&db)?;
    let mut stmt = db.prepare(
        "SELECT payload_json FROM kb_namespaces
         WHERE owner_user_key = ?1
         ORDER BY updated_at_epoch DESC, namespace ASC",
    )?;
    let rows = stmt.query_map(params![runtime.scope_user_key], |row| {
        row.get::<_, String>(0)
    })?;
    let mut values = Vec::new();
    for row in rows {
        let index = serde_json::from_str::<NamespaceIndex>(&row?)
            .context("namespace payload is malformed")?;
        validate_owner(runtime, &index)?;
        values.push(index);
    }
    Ok(values)
}

pub(super) fn sync_namespace_to_index(
    runtime: &KbRuntime,
    index: &NamespaceIndex,
) -> Result<usize> {
    validate_owner(runtime, index)?;
    let mut db = open(runtime)?;
    ensure_schema(&db)?;
    let source_ref_prefix = format!(
        "kb:{}:{}:",
        runtime.scope_user_key.trim(),
        index.namespace.trim()
    );
    let tx = db.transaction()?;
    tx.execute(
        "DELETE FROM memory_retrieval_index
         WHERE source_kind = 'kb_doc' AND COALESCE(user_key, '') = ?1
           AND source_ref LIKE ?2",
        params![
            runtime.scope_user_key.as_str(),
            format!("{source_ref_prefix}%")
        ],
    )?;
    let mut row_count = 0usize;
    for chunk in &index.chunks {
        let text = chunk.text.trim();
        if text.is_empty() {
            continue;
        }
        let metadata = json!({
            "scope_kind": "user",
            "owner_user_key": runtime.scope_user_key.as_str(),
            "namespace": index.namespace,
            "path": chunk.path,
            "file_type": chunk.file_type,
            "mtime_epoch": chunk.mtime_epoch,
            "chunk_id": chunk.chunk_id,
            "offset": chunk.offset,
        });
        let source_ref = format!(
            "kb:{}:{}:{}",
            runtime.scope_user_key.trim(),
            index.namespace.trim(),
            chunk.chunk_id.trim()
        );
        let topic_tags = build_topic_tags(text);
        let vector_json = vector_to_json(&embed_text_locally(text));
        let row_ts = if chunk.mtime_epoch > 0 {
            chunk.mtime_epoch
        } else {
            index.updated_at_epoch
        };
        tx.execute(
            "INSERT INTO memory_retrieval_index (
                source_kind, source_memory_id, source_pref_key, source_ref,
                user_id, chat_id, user_key, memory_kind, role, search_text,
                trigger_text, topic_tags, vector_json, metadata_json, salience,
                success_state, tool_or_skill_name, created_at_ts, updated_at_ts
             ) VALUES (
                'kb_doc', NULL, NULL, ?1, 0, 0, ?2, 'knowledge_doc', NULL,
                ?3, NULL, ?4, ?5, ?6, 0.78, 'succeeded', 'kb', ?7, ?7
             )",
            params![
                source_ref,
                runtime.scope_user_key,
                text,
                topic_tags,
                vector_json,
                metadata.to_string(),
                row_ts,
            ],
        )?;
        row_count += 1;
    }
    rebuild_fts(&tx)?;
    tx.commit()?;
    Ok(row_count)
}

pub(super) fn storage_summary(runtime: &KbRuntime) -> serde_json::Value {
    json!({
        "kind": "sqlite",
        "schema_version": 1,
        "skill_name": "kb",
        "database_identity": database_identity(&runtime.storage_database_path),
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
    Ok(db)
}

fn ensure_schema(db: &Connection) -> Result<()> {
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
        CREATE TABLE IF NOT EXISTS kb_namespaces (
            owner_user_key TEXT NOT NULL,
            namespace TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            PRIMARY KEY(owner_user_key, namespace)
        );
        CREATE INDEX IF NOT EXISTS idx_kb_namespaces_owner_updated
        ON kb_namespaces(owner_user_key, updated_at_epoch DESC);
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
    db.execute(
        "INSERT INTO skill_storage_metadata (skill_name, schema_version)
         VALUES ('kb', 1)
         ON CONFLICT(skill_name) DO UPDATE SET
            schema_version = excluded.schema_version,
            updated_at = CURRENT_TIMESTAMP",
        [],
    )?;
    Ok(())
}

fn integrity_check(db: &Connection) -> Result<()> {
    let result: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if result != "ok" {
        return Err(anyhow!("KB skill storage integrity check failed"));
    }
    Ok(())
}

fn migrate_legacy_json(runtime: &KbRuntime, db: &Connection) -> Result<()> {
    let complete: bool = db
        .query_row(
            "SELECT 1 FROM skill_storage_migrations WHERE migration_id = ?1",
            params![LEGACY_JSON_MIGRATION_ID],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if complete {
        return Ok(());
    }
    let root = legacy_root(runtime);
    let files = collect_json_files(&root)?;
    let mut snapshots = Vec::new();
    for path in &files {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("read legacy KB snapshot failed: {}", path.display()))?;
        let snapshot = serde_json::from_str::<NamespaceIndex>(&raw)
            .with_context(|| format!("parse legacy KB snapshot failed: {}", path.display()))?;
        if snapshot.owner_user_key.trim().is_empty() || snapshot.namespace.trim().is_empty() {
            return Err(anyhow!(
                "legacy KB snapshot has no stable owner or namespace identity"
            ));
        }
        snapshots.push(snapshot);
    }
    for snapshot in &snapshots {
        save_namespace_for_owner(db, snapshot)?;
    }
    for snapshot in &snapshots {
        let count: i64 = db.query_row(
            "SELECT COUNT(*) FROM kb_namespaces
             WHERE owner_user_key = ?1 AND namespace = ?2",
            params![snapshot.owner_user_key, snapshot.namespace],
            |row| row.get(0),
        )?;
        if count != 1 {
            return Err(anyhow!("legacy KB snapshot verification failed"));
        }
    }
    db.execute(
        "INSERT INTO skill_storage_migrations (
            migration_id, source_identity, source_rows, verified_digest
         ) VALUES (?1, ?2, ?3, ?4)",
        params![
            LEGACY_JSON_MIGRATION_ID,
            "legacy-json-snapshots",
            snapshots.len() as i64,
            snapshot_digest(&snapshots)
        ],
    )?;
    for path in &files {
        fs::remove_file(path)
            .with_context(|| format!("remove migrated KB snapshot failed: {}", path.display()))?;
    }
    prune_empty_directories(&root);
    Ok(())
}

fn save_namespace_for_owner(db: &Connection, index: &NamespaceIndex) -> Result<()> {
    let payload = serde_json::to_string(index)?;
    db.execute(
        "INSERT INTO kb_namespaces (
            owner_user_key, namespace, payload_json, updated_at_epoch
         ) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(owner_user_key, namespace) DO UPDATE SET
            payload_json = excluded.payload_json,
            updated_at_epoch = excluded.updated_at_epoch",
        params![
            index.owner_user_key,
            index.namespace,
            payload,
            index.updated_at_epoch
        ],
    )?;
    Ok(())
}

fn rebuild_fts(db: &Connection) -> Result<()> {
    let has_fts: i64 = db.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='table' AND name='memory_retrieval_index_fts'",
        [],
        |row| row.get(0),
    )?;
    if has_fts == 0 {
        return Ok(());
    }
    db.execute("DELETE FROM memory_retrieval_index_fts", [])?;
    db.execute(
        "INSERT INTO memory_retrieval_index_fts(rowid, search_text, topic_tags)
         SELECT id, search_text, topic_tags FROM memory_retrieval_index",
        [],
    )?;
    Ok(())
}

fn validate_owner(runtime: &KbRuntime, index: &NamespaceIndex) -> Result<()> {
    if index.owner_user_key != runtime.scope_user_key {
        return Err(anyhow!("namespace is owned by another user scope"));
    }
    Ok(())
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

fn snapshot_digest(snapshots: &[NamespaceIndex]) -> String {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    for snapshot in snapshots {
        for value in [
            snapshot.owner_user_key.as_bytes(),
            snapshot.namespace.as_bytes(),
            snapshot.updated_at_epoch.to_string().as_bytes(),
        ] {
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value);
        }
    }
    format!("{:x}", digest.finalize())
}

fn database_identity(path: &Path) -> String {
    use sha2::{Digest, Sha256};

    format!(
        "sha256:{:x}",
        Sha256::digest(path.as_os_str().as_encoded_bytes())
    )
}

fn build_topic_tags(text: &str) -> String {
    tokenize_for_index(text)
        .into_iter()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ")
}

fn embed_text_locally(text: &str) -> Vec<f32> {
    const DIMS: usize = 24;
    let mut vector = vec![0.0_f32; DIMS];
    for token in tokenize_for_index(text) {
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

fn tokenize_for_index(text: &str) -> Vec<String> {
    super::tokenize_terms(text)
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
