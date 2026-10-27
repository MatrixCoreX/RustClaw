use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, Result};
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension, Transaction,
};
use serde_json::json;

use super::database::{
    count_rows, enforce_storage_quotas, ensure_owner_quota_row, has_fts, validate_snapshot,
};
use super::vector_index::{
    ensure_writable_vector_generation, refresh_active_vector_generation, replace_ann_projection,
};
use super::{
    default_chunker_version, default_embedding_version, default_parser_version, effective_version,
    nonnegative_u64, nonnegative_usize, sha256_hex, source_ref, source_ref_parts, vector_to_json,
    Chunk, DocMeta, NamespaceIndex, NamespaceSummary, SaveOutcome,
};

pub(super) fn persist_namespace(
    tx: &Transaction<'_>,
    index: &NamespaceIndex,
) -> Result<SaveOutcome> {
    validate_snapshot(index)?;
    let owner = index.owner_user_key.as_str();
    let namespace = index.namespace.as_str();
    ensure_owner_quota_row(tx, owner)?;
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
    let existing_changed_documents = load_documents_by_paths(tx, owner, namespace, &changed)?;
    let extraction_contract_changed = changed.iter().any(|path| {
        existing_changed_documents.get(path).is_some_and(|stored| {
            index.docs.get(path).is_some_and(|incoming| {
                stored.parser_version != incoming.parser_version
                    || stored.chunker_version != incoming.chunker_version
            })
        })
    });
    let has_changes = previous_revision == 0 || !removed.is_empty() || !changed.is_empty();
    let revision = if has_changes {
        previous_revision.saturating_add(1).max(index.revision)
    } else {
        previous_revision.max(index.revision)
    };
    let vector_generation =
        ensure_writable_vector_generation(tx, index, revision, extraction_contract_changed)?;

    for path in &removed {
        delete_document(tx, owner, namespace, path)?;
    }
    for path in &changed {
        delete_document(tx, owner, namespace, path)?;
        let doc = index
            .docs
            .get(path)
            .ok_or_else(|| anyhow!("changed KB document disappeared"))?;
        insert_document(tx, index, doc, revision, vector_generation)?;
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
    if let Some(generation) = vector_generation {
        refresh_active_vector_generation(tx, owner, namespace, generation, revision)?;
    }
    enforce_storage_quotas(tx, owner)?;
    Ok(SaveOutcome {
        revision,
        total_docs,
        total_chunks,
        retrieval_rows: total_chunks,
    })
}

pub(super) fn persist_namespace_delta(
    tx: &Transaction<'_>,
    delta: &NamespaceIndex,
    requested_removals: &HashSet<String>,
    reset_namespace: bool,
) -> Result<SaveOutcome> {
    validate_snapshot(delta)?;
    let owner = delta.owner_user_key.as_str();
    let namespace = delta.namespace.as_str();
    ensure_owner_quota_row(tx, owner)?;
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
            delta.updated_at_epoch,
            delta.next_chunk_seq as i64,
            previous_revision as i64,
            effective_version(&delta.parser_version, default_parser_version()),
            effective_version(&delta.chunker_version, default_chunker_version()),
            effective_version(&delta.embedding_version, default_embedding_version()),
        ],
    )?;

    let existing_paths = if reset_namespace {
        let mut stmt = tx.prepare(
            "SELECT path FROM kb_documents
             WHERE owner_user_key = ?1 AND namespace = ?2",
        )?;
        let paths = stmt
            .query_map(params![owner, namespace], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        paths
    } else {
        HashSet::new()
    };
    let mut removals = requested_removals.clone();
    removals.extend(existing_paths);
    let mut changed = Vec::new();
    let mut extraction_contract_changed = false;
    for (path, document) in &delta.docs {
        let stored = tx
            .query_row(
                "SELECT path, file_type, mtime_epoch, size_bytes, chunk_count,
                        content_sha256, parser_version, chunker_version
                 FROM kb_documents
                 WHERE owner_user_key = ?1 AND namespace = ?2 AND path = ?3",
                params![owner, namespace, path],
                |row| {
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
                },
            )
            .optional()?;
        if reset_namespace
            || stored
                .as_ref()
                .is_none_or(|stored| document_fingerprint(stored) != document_fingerprint(document))
        {
            extraction_contract_changed |= stored.as_ref().is_some_and(|stored| {
                stored.parser_version != document.parser_version
                    || stored.chunker_version != document.chunker_version
            });
            removals.insert(path.clone());
            changed.push(path.clone());
        }
    }
    let mut effective_removals = Vec::new();
    for path in removals {
        let exists = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM kb_documents
                WHERE owner_user_key = ?1 AND namespace = ?2 AND path = ?3
             )",
            params![owner, namespace, path],
            |row| row.get::<_, bool>(0),
        )?;
        if exists {
            effective_removals.push(path);
        }
    }
    effective_removals.sort();
    changed.sort();
    changed.dedup();
    let has_changes =
        previous_revision == 0 || !effective_removals.is_empty() || !changed.is_empty();
    let revision = if has_changes {
        previous_revision.saturating_add(1).max(delta.revision)
    } else {
        previous_revision.max(delta.revision)
    };
    let vector_generation =
        ensure_writable_vector_generation(tx, delta, revision, extraction_contract_changed)?;

    for path in &effective_removals {
        delete_document(tx, owner, namespace, path)?;
    }
    for path in &changed {
        let document = delta
            .docs
            .get(path)
            .ok_or_else(|| anyhow!("changed KB document disappeared"))?;
        insert_document(tx, delta, document, revision, vector_generation)?;
    }

    tx.execute(
        "UPDATE kb_namespaces SET
            updated_at_epoch = ?3,
            next_chunk_seq = MAX(next_chunk_seq, ?4),
            revision = ?5,
            parser_version = ?6,
            chunker_version = ?7,
            embedding_version = ?8
         WHERE owner_user_key = ?1 AND namespace = ?2",
        params![
            owner,
            namespace,
            delta.updated_at_epoch,
            delta.next_chunk_seq as i64,
            revision as i64,
            effective_version(&delta.parser_version, default_parser_version()),
            effective_version(&delta.chunker_version, default_chunker_version()),
            effective_version(&delta.embedding_version, default_embedding_version()),
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
    if let Some(generation) = vector_generation {
        refresh_active_vector_generation(tx, owner, namespace, generation, revision)?;
    }
    enforce_storage_quotas(tx, owner)?;
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
    vector_generation: Option<i64>,
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
        insert_chunk(tx, index, chunk, revision, vector_generation)?;
    }
    Ok(())
}

fn insert_chunk(
    tx: &Transaction<'_>,
    index: &NamespaceIndex,
    chunk: &Chunk,
    revision: u64,
    vector_generation: Option<i64>,
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
            ordinal, text, text_sha256, len_tokens, mtime_epoch,
            content_kind, location_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
            chunk.content_kind,
            serde_json::to_string(&chunk.location)?,
        ],
    )?;
    insert_retrieval_row(tx, index, chunk, text, revision, vector_generation)
}

pub(super) fn insert_retrieval_row(
    tx: &Transaction<'_>,
    index: &NamespaceIndex,
    chunk: &Chunk,
    text: &str,
    revision: u64,
    vector_generation: Option<i64>,
) -> Result<()> {
    let token_projection = claw_core::retrieval_embedding::project_retrieval_tokens(
        text,
        claw_core::retrieval_embedding::DEFAULT_RETRIEVAL_TOKEN_BUDGET,
    );
    let document_digest = index
        .docs
        .get(&chunk.path)
        .map(|document| {
            if document.content_sha256.is_empty() {
                legacy_document_digest(index, &chunk.path)
            } else {
                document.content_sha256.clone()
            }
        })
        .unwrap_or_default();
    let metadata = json!({
        "scope_kind": "user",
        "owner_user_key": index.owner_user_key,
        "namespace": index.namespace,
        "namespace_revision": revision,
        "path": chunk.path,
        "file_type": chunk.file_type,
        "mtime_epoch": chunk.mtime_epoch,
        "chunk_id": chunk.chunk_id,
        "document_digest": document_digest,
        "offset": chunk.offset,
        "content_kind": chunk.content_kind,
        "location": chunk.location,
        "token_projection": token_projection.metadata,
        "embedding_version": effective_version(
            &index.embedding_version,
            default_embedding_version()
        ),
    });
    let source_ref = source_ref(index, &chunk.chunk_id);
    let topic_tags = token_projection.tokens.join(" ");
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
            ?3, NULL, ?4, ?5, ?6, ?7, ?8, ?9,
            0.78, 'succeeded', 'kb', ?10, ?10
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
            "[]",
            "none",
            0_i64,
            "none",
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
    if let Some(generation) = vector_generation {
        let profile = claw_core::retrieval_embedding::local_hash_profile();
        let text_sha256 = if chunk.text_sha256.is_empty() {
            sha256_hex(text.as_bytes())
        } else {
            chunk.text_sha256.clone()
        };
        let output = claw_core::retrieval_embedding::embed_local_hash_with_context(
            text,
            claw_core::retrieval_embedding::EmbeddingPurpose::Document,
            &index.owner_user_key,
        )?;
        if output.profile != profile {
            return Err(anyhow!("embedding_profile_mismatch"));
        }
        tx.execute(
            "INSERT INTO kb_vector_projection (
                owner_user_key, namespace, generation, chunk_id, profile_id,
                dimensions, content_digest, vector_json, updated_at_epoch
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(owner_user_key, namespace, generation, chunk_id) DO UPDATE SET
                profile_id = excluded.profile_id,
                dimensions = excluded.dimensions,
                content_digest = excluded.content_digest,
                vector_json = excluded.vector_json,
                updated_at_epoch = excluded.updated_at_epoch",
            params![
                index.owner_user_key,
                index.namespace,
                generation,
                chunk.chunk_id,
                profile.profile_id,
                profile.dimensions as i64,
                text_sha256,
                vector_to_json(&output.vector),
                row_ts,
            ],
        )?;
        replace_ann_projection(
            tx,
            &index.owner_user_key,
            &index.namespace,
            generation,
            &chunk.chunk_id,
            &profile.profile_id,
            &output.vector,
        )?;
    }
    Ok(())
}

pub(super) fn delete_document(
    tx: &Transaction<'_>,
    owner: &str,
    namespace: &str,
    path: &str,
) -> Result<()> {
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

pub(super) fn load_namespace_from(
    db: &Connection,
    owner: &str,
    namespace: &str,
) -> Result<NamespaceIndex> {
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
                    text_sha256, len_tokens, mtime_epoch, content_kind, location_json
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
                content_kind: row.get(8)?,
                location: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
            })
        })?;
        index.chunks = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    }
    Ok(index)
}

pub(super) fn load_namespace_header(
    db: &Connection,
    owner: &str,
    namespace: &str,
) -> Result<NamespaceIndex> {
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

pub(super) fn load_namespace_summary(
    db: &Connection,
    owner: &str,
    namespace: &str,
) -> Result<NamespaceSummary> {
    db.query_row(
        "SELECT n.namespace, n.updated_at_epoch, n.revision, n.parser_version,
                n.chunker_version, n.embedding_version, n.active_vector_generation,
                (SELECT COUNT(*) FROM kb_documents d
                 WHERE d.owner_user_key = n.owner_user_key AND d.namespace = n.namespace),
                (SELECT COUNT(*) FROM kb_chunks c
                 WHERE c.owner_user_key = n.owner_user_key AND c.namespace = n.namespace)
         FROM kb_namespaces n
         WHERE n.owner_user_key = ?1 AND n.namespace = ?2",
        params![owner, namespace],
        |row| {
            Ok(NamespaceSummary {
                namespace: row.get(0)?,
                updated_at_epoch: row.get(1)?,
                revision: nonnegative_u64(row.get::<_, i64>(2)?),
                parser_version: row.get(3)?,
                chunker_version: row.get(4)?,
                embedding_version: row.get(5)?,
                active_vector_generation: row.get::<_, Option<i64>>(6)?.map(nonnegative_u64),
                document_count: nonnegative_usize(row.get::<_, i64>(7)?),
                chunk_count: nonnegative_usize(row.get::<_, i64>(8)?),
            })
        },
    )
    .optional()?
    .ok_or_else(|| anyhow!("namespace not found"))
}

pub(super) fn load_documents_by_paths(
    db: &Connection,
    owner: &str,
    namespace: &str,
    paths: &[String],
) -> Result<HashMap<String, DocMeta>> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = std::iter::repeat_n("?", paths.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT path, file_type, mtime_epoch, size_bytes, chunk_count,
                content_sha256, parser_version, chunker_version
         FROM kb_documents
         WHERE owner_user_key = ? AND namespace = ? AND path IN ({placeholders})"
    );
    let mut values = Vec::with_capacity(paths.len() + 2);
    values.push(SqlValue::Text(owner.to_string()));
    values.push(SqlValue::Text(namespace.to_string()));
    values.extend(paths.iter().cloned().map(SqlValue::Text));
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values), |row| {
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
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map(|documents| {
            documents
                .into_iter()
                .map(|document| (document.path.clone(), document))
                .collect()
        })
        .map_err(Into::into)
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

pub(super) fn legacy_document_digest(index: &NamespaceIndex, path: &str) -> String {
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

pub(super) fn namespace_exists_in(db: &Connection, owner: &str, namespace: &str) -> Result<bool> {
    let exists: i64 = db.query_row(
        "SELECT COUNT(*) FROM kb_namespaces
         WHERE owner_user_key = ?1 AND namespace = ?2",
        params![owner, namespace],
        |row| row.get(0),
    )?;
    Ok(exists > 0)
}
