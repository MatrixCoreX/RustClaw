use super::{
    default_chunker_version, default_embedding_version, default_parser_version, sha256_hex, Chunk,
    DocMeta, KbRuntime, NamespaceIndex,
};
use crate::ingest::{CheckpointConflict, IngestJob};
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::json;
use std::collections::{HashMap, HashSet};

mod database;
mod namespace_index;
mod retrieval;
mod vector_index;

use database::{
    active_vector_job_for_namespace, count_rows, database_identity, ensure_owner_quota_row,
    ensure_schema, latest_owner_active_job_id, migrate_legacy_json, open, owner_active_job_count,
    owner_quotas, run_bounded_storage_maintenance,
};
use namespace_index::{
    delete_document, load_namespace_from, load_namespace_header, load_namespace_summary,
    namespace_exists_in, persist_namespace, persist_namespace_delta,
};
pub(super) use retrieval::load_search_candidates;
pub(super) use vector_index::{
    cancel_vector_reindex, load_vector_reindex_status, resume_vector_reindex, start_vector_reindex,
};

#[cfg(test)]
#[path = "storage_support_tests.rs"]
mod test_support;
#[cfg(test)]
pub(super) use test_support::save_namespace_and_job;

pub(super) const STORAGE_SCHEMA_VERSION: u32 = 7;

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

#[derive(Debug)]
pub(super) struct StoragePolicyError {
    error_code: &'static str,
    resource: &'static str,
    limit: u64,
    actual: u64,
    retryable: bool,
    active_job_id: Option<String>,
}

impl std::fmt::Display for StoragePolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "KB storage policy rejected {}: actual {} exceeds limit {}",
            self.resource, self.actual, self.limit
        )
    }
}

impl std::error::Error for StoragePolicyError {}

impl StoragePolicyError {
    pub(super) fn extra(&self) -> serde_json::Value {
        json!({
            "schema_version": 1,
            "source_skill": "kb",
            "status": "error",
            "error_code": self.error_code,
            "message_key": format!("skill.kb.{}", self.error_code),
            "retryable": self.retryable,
            "resource": self.resource,
            "limit": self.limit,
            "actual": self.actual,
            "active_job_id": self.active_job_id,
        })
    }

    fn quota(resource: &'static str, limit: u64, actual: u64) -> Self {
        Self {
            error_code: "storage_quota_exceeded",
            resource,
            limit,
            actual,
            retryable: false,
            active_job_id: None,
        }
    }

    fn concurrency(resource: &'static str, limit: u64, actual: u64, job_id: String) -> Self {
        Self {
            error_code: "storage_concurrency_limit",
            resource,
            limit,
            actual,
            retryable: true,
            active_job_id: Some(job_id),
        }
    }
}

pub(super) struct SearchCandidateSet {
    pub(super) index: NamespaceIndex,
    pub(super) total_chunks: usize,
    pub(super) retrieval_mode: &'static str,
    pub(super) lexical_chunk_ids: HashSet<String>,
    pub(super) semantic_scores: HashMap<String, f64>,
    pub(super) embedding_profile_id: String,
    pub(super) vector_generation: Option<u64>,
    pub(super) vector_degradation: Option<String>,
    pub(super) lexical_latency_ms: f64,
    pub(super) semantic_latency_ms: f64,
    pub(super) materialization_latency_ms: f64,
    pub(super) semantic_backend: &'static str,
    pub(super) semantic_rows_scored: usize,
}

#[derive(Debug, Clone, Copy)]
struct StorageQuotas {
    max_namespaces: u64,
    max_documents: u64,
    max_chunks: u64,
    max_vectors: u64,
    max_database_bytes: u64,
    max_active_jobs: u64,
}

#[derive(Debug, Clone)]
pub(super) struct NamespaceSummary {
    pub(super) namespace: String,
    pub(super) updated_at_epoch: i64,
    pub(super) revision: u64,
    pub(super) parser_version: String,
    pub(super) chunker_version: String,
    pub(super) embedding_version: String,
    pub(super) active_vector_generation: Option<u64>,
    pub(super) document_count: usize,
    pub(super) chunk_count: usize,
}

#[derive(Debug, Clone)]
pub(super) struct DocumentPage {
    pub(super) summary: NamespaceSummary,
    pub(super) documents: Vec<DocMeta>,
    pub(super) has_more: bool,
    pub(super) next_after_path: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct VectorReindexStatus {
    pub(super) namespace: String,
    pub(super) generation: u64,
    pub(super) profile_id: String,
    pub(super) namespace_revision: u64,
    pub(super) state: String,
    pub(super) cursor_chunk_id: Option<String>,
    pub(super) expected_rows: usize,
    pub(super) indexed_rows: usize,
    pub(super) source_digest: String,
    pub(super) projection_digest: String,
    pub(super) active_generation: Option<u64>,
    pub(super) created_at_epoch: i64,
    pub(super) updated_at_epoch: i64,
}

impl VectorReindexStatus {
    pub(super) fn is_complete(&self) -> bool {
        self.state == "active"
    }

    pub(super) fn is_resumable(&self) -> bool {
        self.state == "building"
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct SearchFilters<'a> {
    pub(super) path_prefix: Option<&'a str>,
    pub(super) file_type: Option<&'a str>,
    pub(super) time_from: Option<i64>,
    pub(super) time_to: Option<i64>,
}

pub(super) fn initialize(runtime: &KbRuntime) -> Result<()> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    migrate_legacy_json(runtime, &mut db)?;
    run_bounded_storage_maintenance(&mut db)
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

pub(super) fn load_namespace_ingest_state(
    runtime: &KbRuntime,
    namespace: &str,
) -> Result<NamespaceIndex> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    load_namespace_header(&db, &runtime.scope_user_key, namespace)
}

pub(super) fn load_document_metadata(
    runtime: &KbRuntime,
    namespace: &str,
    path: &str,
) -> Result<Option<DocMeta>> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    db.query_row(
        "SELECT path, file_type, mtime_epoch, size_bytes, chunk_count,
                content_sha256, parser_version, chunker_version
         FROM kb_documents
         WHERE owner_user_key = ?1 AND namespace = ?2 AND path = ?3",
        params![runtime.scope_user_key, namespace, path],
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
    .optional()
    .map_err(Into::into)
}

pub(super) fn list_document_paths(runtime: &KbRuntime, namespace: &str) -> Result<Vec<String>> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let mut stmt = db.prepare(
        "SELECT path FROM kb_documents
         WHERE owner_user_key = ?1 AND namespace = ?2
         ORDER BY path",
    )?;
    let paths = stmt
        .query_map(params![runtime.scope_user_key, namespace], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(paths)
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

pub(super) fn save_ingest_delta_and_job(
    runtime: &KbRuntime,
    delta: &NamespaceIndex,
    removed_paths: &HashSet<String>,
    reset_namespace: bool,
    job: &IngestJob,
    expected_checkpoint: &IngestJob,
) -> Result<SaveOutcome> {
    validate_owner(runtime, delta)?;
    validate_job_owner(runtime, job)?;
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_owner_quota_row(&tx, &runtime.scope_user_key)?;
    let current = load_ingest_job_from(&tx, runtime, &job.job_id)?;
    if current.next_file_index != expected_checkpoint.next_file_index
        || current.status != expected_checkpoint.status
    {
        return Err(CheckpointConflict { current }.into());
    }
    let outcome = persist_namespace_delta(&tx, delta, removed_paths, reset_namespace)?;
    upsert_ingest_job(&tx, job)?;
    tx.commit()?;
    Ok(outcome)
}

pub(super) fn create_ingest_job(runtime: &KbRuntime, job: &IngestJob) -> Result<()> {
    validate_job_owner(runtime, job)?;
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_owner_quota_row(&tx, &runtime.scope_user_key)?;
    if let Some((active_job_id, _status)) = tx
        .query_row(
            "SELECT job_id, status FROM kb_ingest_jobs
             WHERE owner_user_key = ?1 AND namespace = ?2
               AND status IN ('running', 'waiting')
             ORDER BY updated_at_epoch DESC LIMIT 1",
            params![runtime.scope_user_key, job.namespace],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    {
        return Err(StoragePolicyError::concurrency(
            "namespace_active_ingest_jobs",
            1,
            2,
            active_job_id,
        )
        .into());
    }
    if let Some(active_job_id) =
        active_vector_job_for_namespace(&tx, &runtime.scope_user_key, &job.namespace)?
    {
        return Err(StoragePolicyError::concurrency(
            "namespace_active_vector_reindex_jobs",
            0,
            1,
            active_job_id,
        )
        .into());
    }
    let max_active_jobs = owner_quotas(&tx, &runtime.scope_user_key)?
        .max_active_jobs
        .min(runtime.active_job_limit());
    let active_jobs = owner_active_job_count(&tx, &runtime.scope_user_key)?;
    if active_jobs >= max_active_jobs {
        return Err(StoragePolicyError::concurrency(
            "owner_active_kb_jobs",
            max_active_jobs,
            active_jobs.saturating_add(1),
            latest_owner_active_job_id(&tx, &runtime.scope_user_key)?,
        )
        .into());
    }
    upsert_ingest_job(&tx, job)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn mark_ingest_job_failed(runtime: &KbRuntime, job_id: &str) -> Result<()> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    db.execute(
        "UPDATE kb_ingest_jobs SET status = 'failed', updated_at_epoch = ?3
         WHERE owner_user_key = ?1 AND job_id = ?2
           AND status IN ('running', 'waiting')",
        params![runtime.scope_user_key, job_id, super::now_epoch()],
    )?;
    Ok(())
}

pub(super) fn load_ingest_job(runtime: &KbRuntime, job_id: &str) -> Result<IngestJob> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    load_ingest_job_from(&db, runtime, job_id)
}

pub(super) fn cancel_ingest_job(runtime: &KbRuntime, job_id: &str) -> Result<IngestJob> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut job = load_ingest_job_from(&tx, runtime, job_id)?;
    if job.status != "completed" && job.status != "cancelled" {
        job.status = "cancelled".to_string();
        job.updated_at_epoch = super::now_epoch();
        upsert_ingest_job(&tx, &job)?;
    }
    tx.commit()?;
    Ok(job)
}

fn load_ingest_job_from(db: &Connection, runtime: &KbRuntime, job_id: &str) -> Result<IngestJob> {
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

pub(super) fn list_namespaces(runtime: &KbRuntime) -> Result<Vec<NamespaceSummary>> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let mut stmt = db.prepare(
        "SELECT n.namespace, n.updated_at_epoch, n.revision, n.parser_version,
                n.chunker_version, n.embedding_version, n.active_vector_generation,
                (SELECT COUNT(*) FROM kb_documents d
                 WHERE d.owner_user_key = n.owner_user_key AND d.namespace = n.namespace),
                (SELECT COUNT(*) FROM kb_chunks c
                 WHERE c.owner_user_key = n.owner_user_key AND c.namespace = n.namespace)
         FROM kb_namespaces n
         WHERE n.owner_user_key = ?1
         ORDER BY n.updated_at_epoch DESC, n.namespace ASC",
    )?;
    let rows = stmt.query_map(params![runtime.scope_user_key], |row| {
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
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub(super) fn list_documents(
    runtime: &KbRuntime,
    namespace: &str,
    after_path: Option<&str>,
    limit: usize,
) -> Result<DocumentPage> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let summary = load_namespace_summary(&db, &runtime.scope_user_key, namespace)?;
    let limit = limit.clamp(1, 500);
    let mut stmt = db.prepare(
        "SELECT path, file_type, mtime_epoch, size_bytes, chunk_count,
                content_sha256, parser_version, chunker_version
         FROM kb_documents
         WHERE owner_user_key = ?1 AND namespace = ?2
           AND (?3 IS NULL OR path > ?3)
         ORDER BY path
         LIMIT ?4",
    )?;
    let mut documents = stmt
        .query_map(
            params![
                runtime.scope_user_key,
                namespace,
                after_path,
                limit.saturating_add(1) as i64
            ],
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
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = documents.len() > limit;
    if has_more {
        documents.truncate(limit);
    }
    let next_after_path = has_more
        .then(|| documents.last().map(|document| document.path.clone()))
        .flatten();
    Ok(DocumentPage {
        summary,
        documents,
        has_more,
        next_after_path,
    })
}

pub(super) fn namespace_file_type_counts(
    runtime: &KbRuntime,
    namespace: &str,
) -> Result<HashMap<String, usize>> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    load_namespace_summary(&db, &runtime.scope_user_key, namespace)?;
    let mut stmt = db.prepare(
        "SELECT file_type, COUNT(*) FROM kb_documents
         WHERE owner_user_key = ?1 AND namespace = ?2
         GROUP BY file_type ORDER BY file_type",
    )?;
    let rows = stmt.query_map(params![runtime.scope_user_key, namespace], |row| {
        Ok((
            row.get::<_, String>(0)?,
            nonnegative_usize(row.get::<_, i64>(1)?),
        ))
    })?;
    rows.collect::<rusqlite::Result<HashMap<_, _>>>()
        .map_err(Into::into)
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

pub(super) fn storage_summary(runtime: &KbRuntime) -> serde_json::Value {
    json!({
        "kind": "sqlite",
        "schema_version": STORAGE_SCHEMA_VERSION,
        "skill_name": "kb",
        "database_identity": database_identity(&runtime.storage_database_path),
        "layout": "normalized_documents_chunks_with_locations_and_vector_ann_generations_v7",
    })
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

fn validate_owner(runtime: &KbRuntime, index: &NamespaceIndex) -> Result<()> {
    if index.owner_user_key != runtime.scope_user_key {
        return Err(anyhow!("namespace is owned by another user scope"));
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

fn build_topic_tags(text: &str) -> String {
    super::tokenize_terms(text)
        .into_iter()
        .collect::<Vec<_>>()
        .join(" ")
}

fn vector_to_json(vector: &[f32]) -> String {
    serde_json::to_string(vector).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
