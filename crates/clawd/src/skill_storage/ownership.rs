use crate::db_init::DbPool;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub(crate) struct KbUserDataSnapshot {
    legacy_namespaces: Vec<LegacyNamespaceRow>,
    namespaces: Vec<NamespaceRow>,
    documents: Vec<DocumentRow>,
    chunks: Vec<ChunkRow>,
    ingest_jobs: Vec<IngestJobRow>,
    retrieval_rows: Vec<RetrievalRow>,
}

#[derive(Clone, Debug)]
struct LegacyNamespaceRow {
    owner_user_key: String,
    namespace: String,
    payload_json: String,
    updated_at_epoch: i64,
}

#[derive(Clone, Debug)]
struct NamespaceRow {
    owner_user_key: String,
    namespace: String,
    updated_at_epoch: i64,
    next_chunk_seq: i64,
    revision: i64,
    parser_version: String,
    chunker_version: String,
    embedding_version: String,
}

#[derive(Clone, Debug)]
struct DocumentRow {
    owner_user_key: String,
    namespace: String,
    path: String,
    file_type: String,
    mtime_epoch: i64,
    size_bytes: i64,
    chunk_count: i64,
    content_sha256: String,
    parser_version: String,
    chunker_version: String,
}

#[derive(Clone, Debug)]
struct ChunkRow {
    owner_user_key: String,
    namespace: String,
    chunk_id: String,
    document_path: String,
    file_type: String,
    ordinal: i64,
    text: String,
    text_sha256: String,
    len_tokens: i64,
    mtime_epoch: i64,
}

#[derive(Clone, Debug)]
struct IngestJobRow {
    owner_user_key: String,
    job_id: String,
    namespace: String,
    operation: String,
    status: String,
    next_file_index: i64,
    total_files: i64,
    payload_json: String,
    created_at_epoch: i64,
    updated_at_epoch: i64,
}

#[derive(Clone, Debug)]
struct RetrievalRow {
    source_kind: String,
    source_memory_id: Option<i64>,
    source_pref_key: Option<String>,
    source_ref: Option<String>,
    user_id: i64,
    chat_id: i64,
    user_key: Option<String>,
    memory_kind: String,
    role: Option<String>,
    search_text: String,
    trigger_text: Option<String>,
    topic_tags: String,
    vector_json: String,
    embedding_model: String,
    embedding_dims: i64,
    embedding_version: String,
    metadata_json: String,
    salience: f64,
    success_state: String,
    tool_or_skill_name: Option<String>,
    created_at_ts: i64,
    updated_at_ts: i64,
}

impl KbUserDataSnapshot {
    pub(crate) fn row_count(&self) -> usize {
        self.legacy_namespaces.len()
            + self.namespaces.len()
            + self.documents.len()
            + self.chunks.len()
            + self.ingest_jobs.len()
            + self.retrieval_rows.len()
    }

    fn rebind(mut self, old_user_key: &str, new_user_key: &str) -> anyhow::Result<Self> {
        for namespace in &mut self.legacy_namespaces {
            namespace.owner_user_key = new_user_key.to_string();
            rewrite_owner_field(&mut namespace.payload_json, new_user_key, true)?;
        }
        for namespace in &mut self.namespaces {
            namespace.owner_user_key = new_user_key.to_string();
        }
        for document in &mut self.documents {
            document.owner_user_key = new_user_key.to_string();
        }
        for chunk in &mut self.chunks {
            chunk.owner_user_key = new_user_key.to_string();
        }
        for job in &mut self.ingest_jobs {
            job.owner_user_key = new_user_key.to_string();
            rewrite_owner_field(&mut job.payload_json, new_user_key, true)?;
        }
        let old_prefix = format!("kb:{old_user_key}:");
        let new_prefix = format!("kb:{new_user_key}:");
        for row in &mut self.retrieval_rows {
            row.user_key = Some(new_user_key.to_string());
            if let Some(source_ref) = row.source_ref.as_mut() {
                if source_ref.starts_with(&old_prefix) {
                    *source_ref = format!("{new_prefix}{}", &source_ref[old_prefix.len()..]);
                }
            }
            rewrite_owner_field(&mut row.metadata_json, new_user_key, false)?;
        }
        Ok(self)
    }
}

fn rewrite_owner_field(
    raw: &mut String,
    new_user_key: &str,
    require_object: bool,
) -> anyhow::Result<()> {
    let Ok(mut payload) = serde_json::from_str::<Value>(raw) else {
        if require_object {
            anyhow::bail!("KB owned payload is malformed");
        }
        return Ok(());
    };
    let Some(object) = payload.as_object_mut() else {
        if require_object {
            anyhow::bail!("KB owned payload must be an object");
        }
        return Ok(());
    };
    object.insert(
        "owner_user_key".to_string(),
        Value::String(new_user_key.to_string()),
    );
    *raw = serde_json::to_string(&payload)?;
    Ok(())
}

pub(super) fn take_user_data(
    pool: &DbPool,
    user_key: Option<&str>,
) -> anyhow::Result<KbUserDataSnapshot> {
    let mut db = pool
        .get()
        .map_err(|error| anyhow::anyhow!("KB storage pool: {error}"))?;
    let snapshot = select_snapshot(&db, user_key)?;
    let has_documents = table_exists(&db, "kb_documents")?;
    let has_chunks = table_exists(&db, "kb_chunks")?;
    let has_ingest_jobs = table_exists(&db, "kb_ingest_jobs")?;
    let tx = db.transaction()?;
    match user_key {
        Some(user_key) => {
            if has_chunks {
                tx.execute(
                    "DELETE FROM kb_chunks WHERE owner_user_key = ?1",
                    params![user_key],
                )?;
            }
            if has_documents {
                tx.execute(
                    "DELETE FROM kb_documents WHERE owner_user_key = ?1",
                    params![user_key],
                )?;
            }
            if has_ingest_jobs {
                tx.execute(
                    "DELETE FROM kb_ingest_jobs WHERE owner_user_key = ?1",
                    params![user_key],
                )?;
            }
            tx.execute(
                "DELETE FROM kb_namespaces WHERE owner_user_key = ?1",
                params![user_key],
            )?;
            tx.execute(
                "DELETE FROM memory_retrieval_index WHERE user_key = ?1",
                params![user_key],
            )?;
        }
        None => {
            if has_chunks {
                tx.execute("DELETE FROM kb_chunks", [])?;
            }
            if has_documents {
                tx.execute("DELETE FROM kb_documents", [])?;
            }
            if has_ingest_jobs {
                tx.execute("DELETE FROM kb_ingest_jobs", [])?;
            }
            tx.execute("DELETE FROM kb_namespaces", [])?;
            tx.execute("DELETE FROM memory_retrieval_index", [])?;
        }
    }
    rebuild_fts(&tx)?;
    tx.commit()?;
    Ok(snapshot)
}

pub(super) fn restore_user_data(
    pool: &DbPool,
    snapshot: &KbUserDataSnapshot,
) -> anyhow::Result<()> {
    if snapshot.row_count() == 0 {
        return Ok(());
    }
    let mut db = pool
        .get()
        .map_err(|error| anyhow::anyhow!("KB storage pool: {error}"))?;
    let tx = db.transaction()?;
    for row in &snapshot.legacy_namespaces {
        tx.execute(
            "INSERT INTO kb_namespaces
                (owner_user_key, namespace, payload_json, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_user_key, namespace) DO UPDATE SET
                payload_json = excluded.payload_json,
                updated_at_epoch = excluded.updated_at_epoch",
            params![
                row.owner_user_key,
                row.namespace,
                row.payload_json,
                row.updated_at_epoch
            ],
        )?;
    }
    for row in &snapshot.namespaces {
        tx.execute(
            "INSERT INTO kb_namespaces
                (owner_user_key, namespace, updated_at_epoch, next_chunk_seq,
                 revision, parser_version, chunker_version, embedding_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(owner_user_key, namespace) DO UPDATE SET
                updated_at_epoch = excluded.updated_at_epoch,
                next_chunk_seq = excluded.next_chunk_seq,
                revision = excluded.revision,
                parser_version = excluded.parser_version,
                chunker_version = excluded.chunker_version,
                embedding_version = excluded.embedding_version",
            params![
                row.owner_user_key,
                row.namespace,
                row.updated_at_epoch,
                row.next_chunk_seq,
                row.revision,
                row.parser_version,
                row.chunker_version,
                row.embedding_version
            ],
        )?;
    }
    for row in &snapshot.documents {
        tx.execute(
            "INSERT INTO kb_documents (
                owner_user_key, namespace, path, file_type, mtime_epoch,
                size_bytes, chunk_count, content_sha256, parser_version,
                chunker_version
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(owner_user_key, namespace, path) DO UPDATE SET
                file_type = excluded.file_type,
                mtime_epoch = excluded.mtime_epoch,
                size_bytes = excluded.size_bytes,
                chunk_count = excluded.chunk_count,
                content_sha256 = excluded.content_sha256,
                parser_version = excluded.parser_version,
                chunker_version = excluded.chunker_version",
            params![
                row.owner_user_key,
                row.namespace,
                row.path,
                row.file_type,
                row.mtime_epoch,
                row.size_bytes,
                row.chunk_count,
                row.content_sha256,
                row.parser_version,
                row.chunker_version
            ],
        )?;
    }
    for row in &snapshot.chunks {
        tx.execute(
            "INSERT INTO kb_chunks (
                owner_user_key, namespace, chunk_id, document_path, file_type,
                ordinal, text, text_sha256, len_tokens, mtime_epoch
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(owner_user_key, namespace, chunk_id) DO UPDATE SET
                document_path = excluded.document_path,
                file_type = excluded.file_type,
                ordinal = excluded.ordinal,
                text = excluded.text,
                text_sha256 = excluded.text_sha256,
                len_tokens = excluded.len_tokens,
                mtime_epoch = excluded.mtime_epoch",
            params![
                row.owner_user_key,
                row.namespace,
                row.chunk_id,
                row.document_path,
                row.file_type,
                row.ordinal,
                row.text,
                row.text_sha256,
                row.len_tokens,
                row.mtime_epoch
            ],
        )?;
    }
    for row in &snapshot.ingest_jobs {
        tx.execute(
            "INSERT INTO kb_ingest_jobs (
                owner_user_key, job_id, namespace, operation, status,
                next_file_index, total_files, payload_json, created_at_epoch,
                updated_at_epoch
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(owner_user_key, job_id) DO UPDATE SET
                namespace = excluded.namespace,
                operation = excluded.operation,
                status = excluded.status,
                next_file_index = excluded.next_file_index,
                total_files = excluded.total_files,
                payload_json = excluded.payload_json,
                created_at_epoch = excluded.created_at_epoch,
                updated_at_epoch = excluded.updated_at_epoch",
            params![
                row.owner_user_key,
                row.job_id,
                row.namespace,
                row.operation,
                row.status,
                row.next_file_index,
                row.total_files,
                row.payload_json,
                row.created_at_epoch,
                row.updated_at_epoch
            ],
        )?;
    }
    for row in &snapshot.retrieval_rows {
        tx.execute(
            "INSERT INTO memory_retrieval_index (
                source_kind, source_memory_id, source_pref_key, source_ref,
                user_id, chat_id, user_key, memory_kind, role, search_text,
                trigger_text, topic_tags, vector_json, embedding_model,
                embedding_dims, embedding_version, metadata_json, salience,
                success_state, tool_or_skill_name, created_at_ts, updated_at_ts
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22
             )
             ON CONFLICT(user_key, source_ref) DO UPDATE SET
                search_text = excluded.search_text,
                topic_tags = excluded.topic_tags,
                vector_json = excluded.vector_json,
                metadata_json = excluded.metadata_json,
                updated_at_ts = excluded.updated_at_ts",
            params![
                row.source_kind,
                row.source_memory_id,
                row.source_pref_key,
                row.source_ref,
                row.user_id,
                row.chat_id,
                row.user_key,
                row.memory_kind,
                row.role,
                row.search_text,
                row.trigger_text,
                row.topic_tags,
                row.vector_json,
                row.embedding_model,
                row.embedding_dims,
                row.embedding_version,
                row.metadata_json,
                row.salience,
                row.success_state,
                row.tool_or_skill_name,
                row.created_at_ts,
                row.updated_at_ts
            ],
        )?;
    }
    rebuild_fts(&tx)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn rebind_user_key(
    pool: &DbPool,
    old_user_key: &str,
    new_user_key: &str,
) -> anyhow::Result<usize> {
    let original = take_user_data(pool, Some(old_user_key))?;
    let count = original.row_count();
    if count == 0 {
        return Ok(0);
    }
    let rebound = match original.clone().rebind(old_user_key, new_user_key) {
        Ok(rebound) => rebound,
        Err(error) => {
            restore_user_data(pool, &original)?;
            return Err(error);
        }
    };
    if let Err(error) = restore_user_data(pool, &rebound) {
        restore_user_data(pool, &original)?;
        return Err(error);
    }
    Ok(count)
}

fn select_snapshot(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
) -> anyhow::Result<KbUserDataSnapshot> {
    let legacy_schema = table_has_column(db, "kb_namespaces", "payload_json")?;
    let legacy_namespaces = if legacy_schema {
        select_legacy_namespaces(db, user_key)?
    } else {
        Vec::new()
    };
    let namespaces = if legacy_schema {
        Vec::new()
    } else {
        select_namespaces(db, user_key)?
    };
    let documents = if table_exists(db, "kb_documents")? {
        select_documents(db, user_key)?
    } else {
        Vec::new()
    };
    let chunks = if table_exists(db, "kb_chunks")? {
        select_chunks(db, user_key)?
    } else {
        Vec::new()
    };
    let ingest_jobs = if table_exists(db, "kb_ingest_jobs")? {
        select_ingest_jobs(db, user_key)?
    } else {
        Vec::new()
    };
    let retrieval_rows = select_retrieval_rows(db, user_key)?;
    Ok(KbUserDataSnapshot {
        legacy_namespaces,
        namespaces,
        documents,
        chunks,
        ingest_jobs,
        retrieval_rows,
    })
}

fn select_legacy_namespaces(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
) -> anyhow::Result<Vec<LegacyNamespaceRow>> {
    let sql = match user_key {
        Some(_) => {
            "SELECT owner_user_key, namespace, payload_json, updated_at_epoch
             FROM kb_namespaces WHERE owner_user_key = ?1 ORDER BY namespace"
        }
        None => {
            "SELECT owner_user_key, namespace, payload_json, updated_at_epoch
             FROM kb_namespaces ORDER BY owner_user_key, namespace"
        }
    };
    let mut stmt = db.prepare(sql)?;
    let map = |row: &rusqlite::Row<'_>| {
        Ok(LegacyNamespaceRow {
            owner_user_key: row.get(0)?,
            namespace: row.get(1)?,
            payload_json: row.get(2)?,
            updated_at_epoch: row.get(3)?,
        })
    };
    let rows = match user_key {
        Some(user_key) => stmt.query_map(params![user_key], map)?,
        None => stmt.query_map([], map)?,
    };
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn select_namespaces(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
) -> anyhow::Result<Vec<NamespaceRow>> {
    let sql = match user_key {
        Some(_) => {
            "SELECT owner_user_key, namespace, updated_at_epoch, next_chunk_seq,
                    revision, parser_version, chunker_version, embedding_version
             FROM kb_namespaces WHERE owner_user_key = ?1 ORDER BY namespace"
        }
        None => {
            "SELECT owner_user_key, namespace, updated_at_epoch, next_chunk_seq,
                    revision, parser_version, chunker_version, embedding_version
             FROM kb_namespaces ORDER BY owner_user_key, namespace"
        }
    };
    let mut stmt = db.prepare(sql)?;
    let map = |row: &rusqlite::Row<'_>| {
        Ok(NamespaceRow {
            owner_user_key: row.get(0)?,
            namespace: row.get(1)?,
            updated_at_epoch: row.get(2)?,
            next_chunk_seq: row.get(3)?,
            revision: row.get(4)?,
            parser_version: row.get(5)?,
            chunker_version: row.get(6)?,
            embedding_version: row.get(7)?,
        })
    };
    let rows = match user_key {
        Some(user_key) => stmt.query_map(params![user_key], map)?,
        None => stmt.query_map([], map)?,
    };
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn select_documents(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
) -> anyhow::Result<Vec<DocumentRow>> {
    let sql = match user_key {
        Some(_) => {
            "SELECT owner_user_key, namespace, path, file_type, mtime_epoch,
                    size_bytes, chunk_count, content_sha256, parser_version,
                    chunker_version
             FROM kb_documents WHERE owner_user_key = ?1
             ORDER BY namespace, path"
        }
        None => {
            "SELECT owner_user_key, namespace, path, file_type, mtime_epoch,
                    size_bytes, chunk_count, content_sha256, parser_version,
                    chunker_version
             FROM kb_documents ORDER BY owner_user_key, namespace, path"
        }
    };
    let mut stmt = db.prepare(sql)?;
    let map = |row: &rusqlite::Row<'_>| {
        Ok(DocumentRow {
            owner_user_key: row.get(0)?,
            namespace: row.get(1)?,
            path: row.get(2)?,
            file_type: row.get(3)?,
            mtime_epoch: row.get(4)?,
            size_bytes: row.get(5)?,
            chunk_count: row.get(6)?,
            content_sha256: row.get(7)?,
            parser_version: row.get(8)?,
            chunker_version: row.get(9)?,
        })
    };
    let rows = match user_key {
        Some(user_key) => stmt.query_map(params![user_key], map)?,
        None => stmt.query_map([], map)?,
    };
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn select_chunks(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
) -> anyhow::Result<Vec<ChunkRow>> {
    let sql = match user_key {
        Some(_) => {
            "SELECT owner_user_key, namespace, chunk_id, document_path,
                    file_type, ordinal, text, text_sha256, len_tokens, mtime_epoch
             FROM kb_chunks WHERE owner_user_key = ?1
             ORDER BY namespace, document_path, ordinal, chunk_id"
        }
        None => {
            "SELECT owner_user_key, namespace, chunk_id, document_path,
                    file_type, ordinal, text, text_sha256, len_tokens, mtime_epoch
             FROM kb_chunks
             ORDER BY owner_user_key, namespace, document_path, ordinal, chunk_id"
        }
    };
    let mut stmt = db.prepare(sql)?;
    let map = |row: &rusqlite::Row<'_>| {
        Ok(ChunkRow {
            owner_user_key: row.get(0)?,
            namespace: row.get(1)?,
            chunk_id: row.get(2)?,
            document_path: row.get(3)?,
            file_type: row.get(4)?,
            ordinal: row.get(5)?,
            text: row.get(6)?,
            text_sha256: row.get(7)?,
            len_tokens: row.get(8)?,
            mtime_epoch: row.get(9)?,
        })
    };
    let rows = match user_key {
        Some(user_key) => stmt.query_map(params![user_key], map)?,
        None => stmt.query_map([], map)?,
    };
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn select_ingest_jobs(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
) -> anyhow::Result<Vec<IngestJobRow>> {
    let sql = match user_key {
        Some(_) => {
            "SELECT owner_user_key, job_id, namespace, operation, status,
                    next_file_index, total_files, payload_json,
                    created_at_epoch, updated_at_epoch
             FROM kb_ingest_jobs WHERE owner_user_key = ?1 ORDER BY job_id"
        }
        None => {
            "SELECT owner_user_key, job_id, namespace, operation, status,
                    next_file_index, total_files, payload_json,
                    created_at_epoch, updated_at_epoch
             FROM kb_ingest_jobs ORDER BY owner_user_key, job_id"
        }
    };
    let mut stmt = db.prepare(sql)?;
    let map = |row: &rusqlite::Row<'_>| {
        Ok(IngestJobRow {
            owner_user_key: row.get(0)?,
            job_id: row.get(1)?,
            namespace: row.get(2)?,
            operation: row.get(3)?,
            status: row.get(4)?,
            next_file_index: row.get(5)?,
            total_files: row.get(6)?,
            payload_json: row.get(7)?,
            created_at_epoch: row.get(8)?,
            updated_at_epoch: row.get(9)?,
        })
    };
    let rows = match user_key {
        Some(user_key) => stmt.query_map(params![user_key], map)?,
        None => stmt.query_map([], map)?,
    };
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn select_retrieval_rows(
    db: &rusqlite::Connection,
    user_key: Option<&str>,
) -> anyhow::Result<Vec<RetrievalRow>> {
    let sql = match user_key {
        Some(_) => {
            "SELECT source_kind, source_memory_id, source_pref_key, source_ref,
                    user_id, chat_id, user_key, memory_kind, role,
                    search_text, trigger_text, topic_tags, vector_json,
                    embedding_model, embedding_dims, embedding_version,
                    metadata_json, salience, success_state, tool_or_skill_name,
                    created_at_ts, updated_at_ts
             FROM memory_retrieval_index WHERE user_key = ?1 ORDER BY id"
        }
        None => {
            "SELECT source_kind, source_memory_id, source_pref_key, source_ref,
                    user_id, chat_id, user_key, memory_kind, role,
                    search_text, trigger_text, topic_tags, vector_json,
                    embedding_model, embedding_dims, embedding_version,
                    metadata_json, salience, success_state, tool_or_skill_name,
                    created_at_ts, updated_at_ts
             FROM memory_retrieval_index ORDER BY id"
        }
    };
    let mut stmt = db.prepare(sql)?;
    let map = |row: &rusqlite::Row<'_>| {
        Ok(RetrievalRow {
            source_kind: row.get(0)?,
            source_memory_id: row.get(1)?,
            source_pref_key: row.get(2)?,
            source_ref: row.get(3)?,
            user_id: row.get(4)?,
            chat_id: row.get(5)?,
            user_key: row.get(6)?,
            memory_kind: row.get(7)?,
            role: row.get(8)?,
            search_text: row.get(9)?,
            trigger_text: row.get(10)?,
            topic_tags: row.get(11)?,
            vector_json: row.get(12)?,
            embedding_model: row.get(13)?,
            embedding_dims: row.get(14)?,
            embedding_version: row.get(15)?,
            metadata_json: row.get(16)?,
            salience: row.get(17)?,
            success_state: row.get(18)?,
            tool_or_skill_name: row.get(19)?,
            created_at_ts: row.get(20)?,
            updated_at_ts: row.get(21)?,
        })
    };
    let rows = match user_key {
        Some(user_key) => stmt.query_map(params![user_key], map)?,
        None => stmt.query_map([], map)?,
    };
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn table_has_column(db: &rusqlite::Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for current in columns {
        if current? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn table_exists(db: &rusqlite::Connection, table: &str) -> anyhow::Result<bool> {
    Ok(db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false))
}

fn rebuild_fts(db: &rusqlite::Connection) -> anyhow::Result<()> {
    let has_fts = db
        .query_row(
            "SELECT 1 FROM sqlite_master
             WHERE type='table' AND name='memory_retrieval_index_fts'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !has_fts {
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

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;
