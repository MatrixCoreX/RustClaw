use super::{
    default_chunker_version, default_embedding_version, default_parser_version, sha256_hex, Chunk,
    DocMeta, KbRuntime, NamespaceIndex,
};
use crate::ingest::{CheckpointConflict, IngestJob};
use anyhow::{anyhow, Context, Result};
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension, Transaction,
    TransactionBehavior,
};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(super) const STORAGE_SCHEMA_VERSION: u32 = 7;
const LEGACY_JSON_MIGRATION_ID: &str = "legacy-kb-json-v1";
const SQLITE_V1_MIGRATION_ID: &str = "kb-payload-json-to-normalized-v2";
const SCHEMA_V6_CUTOVER_MIGRATION_ID: &str = "kb-schema-v6-cutover";
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
const EXACT_VECTOR_LIMIT_STANDARD: usize = 4_096;
const EXACT_VECTOR_LIMIT_CONSTRAINED: usize = 1_024;
const ANN_MAX_CANDIDATES_STANDARD: usize = 4_096;
const ANN_MAX_CANDIDATES_CONSTRAINED: usize = 1_024;

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

struct SemanticCandidateSet {
    scores: Vec<(String, f64)>,
    profile_id: String,
    generation: Option<u64>,
    degradation: Option<String>,
    backend: &'static str,
    rows_scored: usize,
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

#[cfg(test)]
pub(super) fn save_namespace_and_job(
    runtime: &KbRuntime,
    index: &NamespaceIndex,
    job: &IngestJob,
    expected_checkpoint: &IngestJob,
) -> Result<SaveOutcome> {
    validate_owner(runtime, index)?;
    validate_job_owner(runtime, job)?;
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    // Compare and commit under one write transaction so a stale batch cannot
    // overwrite a newer checkpoint or revive a cancelled job.
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = load_ingest_job_from(&tx, runtime, &job.job_id)?;
    if current.next_file_index != expected_checkpoint.next_file_index
        || current.status != expected_checkpoint.status
    {
        return Err(CheckpointConflict { current }.into());
    }
    let outcome = persist_namespace(&tx, index)?;
    upsert_ingest_job(&tx, job)?;
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

pub(super) fn start_vector_reindex(
    runtime: &KbRuntime,
    namespace: &str,
) -> Result<VectorReindexStatus> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_owner_quota_row(&tx, &runtime.scope_user_key)?;
    if let Some(active_job_id) =
        active_ingest_job_for_namespace(&tx, &runtime.scope_user_key, namespace)?
    {
        return Err(StoragePolicyError::concurrency(
            "namespace_active_ingest_jobs",
            0,
            1,
            active_job_id,
        )
        .into());
    }
    let revision = tx
        .query_row(
            "SELECT revision FROM kb_namespaces
             WHERE owner_user_key = ?1 AND namespace = ?2",
            params![runtime.scope_user_key, namespace],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow!("namespace not found"))?
        .max(0) as u64;
    let profile = claw_core::retrieval_embedding::local_hash_profile();
    let expected_rows = count_rows(
        &tx,
        "kb_chunks",
        "owner_user_key = ?1 AND namespace = ?2",
        &runtime.scope_user_key,
        namespace,
    )?;
    let source_digest = vector_source_digest(&tx, &runtime.scope_user_key, namespace)?;
    let existing = tx
        .query_row(
            "SELECT generation FROM kb_vector_generations
             WHERE owner_user_key = ?1 AND namespace = ?2
               AND state = 'building' AND profile_id = ?3
               AND namespace_revision = ?4 AND source_digest = ?5
             ORDER BY generation DESC LIMIT 1",
            params![
                runtime.scope_user_key,
                namespace,
                profile.profile_id,
                revision as i64,
                source_digest,
            ],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let generation = if let Some(generation) = existing {
        generation
    } else {
        let max_active_jobs = owner_quotas(&tx, &runtime.scope_user_key)?
            .max_active_jobs
            .min(runtime.active_job_limit());
        let active_jobs = owner_active_job_count(&tx, &runtime.scope_user_key)?;
        let replaced_jobs = tx
            .query_row(
                "SELECT COUNT(*) FROM kb_vector_generations
                 WHERE owner_user_key = ?1 AND namespace = ?2 AND state = 'building'",
                params![runtime.scope_user_key, namespace],
                |row| row.get::<_, i64>(0),
            )?
            .max(0) as u64;
        let projected_jobs = active_jobs.saturating_sub(replaced_jobs).saturating_add(1);
        if projected_jobs > max_active_jobs {
            return Err(StoragePolicyError::concurrency(
                "owner_active_kb_jobs",
                max_active_jobs,
                projected_jobs,
                latest_owner_active_job_id(&tx, &runtime.scope_user_key)?,
            )
            .into());
        }
        let now = super::now_epoch();
        tx.execute(
            "UPDATE kb_vector_generations SET state = 'stale', updated_at_epoch = ?3
             WHERE owner_user_key = ?1 AND namespace = ?2 AND state = 'building'",
            params![runtime.scope_user_key, namespace, now],
        )?;
        let generation = tx.query_row(
            "SELECT COALESCE(MAX(generation), 0) + 1
             FROM kb_vector_generations
             WHERE owner_user_key = ?1 AND namespace = ?2",
            params![runtime.scope_user_key, namespace],
            |row| row.get::<_, i64>(0),
        )?;
        tx.execute(
            "INSERT INTO kb_vector_generations (
                owner_user_key, namespace, generation, profile_id,
                namespace_revision, state, cursor_chunk_id, expected_rows,
                indexed_rows, source_digest, projection_digest,
                created_at_epoch, updated_at_epoch, activated_at_epoch
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'building', NULL, ?6, 0, ?7, '', ?8, ?8, NULL)",
            params![
                runtime.scope_user_key,
                namespace,
                generation,
                profile.profile_id,
                revision as i64,
                expected_rows as i64,
                source_digest,
                now,
            ],
        )?;
        generation
    };
    let status =
        load_vector_reindex_status_from(&tx, &runtime.scope_user_key, namespace, Some(generation))?;
    tx.commit()?;
    Ok(status)
}

pub(super) fn resume_vector_reindex(
    runtime: &KbRuntime,
    namespace: &str,
    generation: u64,
    batch_size: usize,
) -> Result<VectorReindexStatus> {
    if !(1..=256).contains(&batch_size) {
        return Err(anyhow!(
            "vector reindex batch_size must be between 1 and 256"
        ));
    }
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let status = load_vector_reindex_status_from(
        &tx,
        &runtime.scope_user_key,
        namespace,
        Some(generation as i64),
    )?;
    if !status.is_resumable() {
        tx.commit()?;
        return Ok(status);
    }
    let current_revision = tx
        .query_row(
            "SELECT revision FROM kb_namespaces
             WHERE owner_user_key = ?1 AND namespace = ?2",
            params![runtime.scope_user_key, namespace],
            |row| row.get::<_, i64>(0),
        )?
        .max(0) as u64;
    let current_source_digest = vector_source_digest(&tx, &runtime.scope_user_key, namespace)?;
    if current_revision != status.namespace_revision
        || current_source_digest != status.source_digest
    {
        tx.execute(
            "UPDATE kb_vector_generations SET state = 'stale', updated_at_epoch = ?4
             WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
            params![
                runtime.scope_user_key,
                namespace,
                generation as i64,
                super::now_epoch(),
            ],
        )?;
        let stale = load_vector_reindex_status_from(
            &tx,
            &runtime.scope_user_key,
            namespace,
            Some(generation as i64),
        )?;
        tx.commit()?;
        return Ok(stale);
    }
    let chunks = {
        let mut stmt = tx.prepare(
            "SELECT chunk_id, text, text_sha256
             FROM kb_chunks
             WHERE owner_user_key = ?1 AND namespace = ?2
               AND (?3 IS NULL OR chunk_id > ?3)
             ORDER BY chunk_id LIMIT ?4",
        )?;
        let rows = stmt
            .query_map(
                params![
                    runtime.scope_user_key,
                    namespace,
                    status.cursor_chunk_id,
                    batch_size as i64,
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let profile = claw_core::retrieval_embedding::local_hash_profile();
    let now = super::now_epoch();
    for (chunk_id, text, content_digest) in &chunks {
        let output = claw_core::retrieval_embedding::embed_local_hash_with_context(
            text,
            claw_core::retrieval_embedding::EmbeddingPurpose::Document,
            &runtime.scope_user_key,
        )?;
        if output.profile != profile || output.content_digest != format!("sha256:{content_digest}")
        {
            return Err(anyhow!("embedding_projection_verification_failed"));
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
                runtime.scope_user_key,
                namespace,
                generation as i64,
                chunk_id,
                profile.profile_id,
                profile.dimensions as i64,
                content_digest,
                vector_to_json(&output.vector),
                now,
            ],
        )?;
        replace_ann_projection(
            &tx,
            &runtime.scope_user_key,
            namespace,
            generation as i64,
            chunk_id,
            &profile.profile_id,
            &output.vector,
        )?;
    }
    let cursor = chunks
        .last()
        .map(|(chunk_id, _, _)| chunk_id.clone())
        .or(status.cursor_chunk_id);
    let indexed_rows =
        count_projection_rows(&tx, &runtime.scope_user_key, namespace, generation as i64)?;
    let projection_digest =
        vector_projection_digest(&tx, &runtime.scope_user_key, namespace, generation as i64)?;
    tx.execute(
        "UPDATE kb_vector_generations SET
            cursor_chunk_id = ?4, indexed_rows = ?5, projection_digest = ?6,
            updated_at_epoch = ?7
         WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
        params![
            runtime.scope_user_key,
            namespace,
            generation as i64,
            cursor,
            indexed_rows as i64,
            projection_digest,
            now,
        ],
    )?;
    let has_more = tx.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM kb_chunks
            WHERE owner_user_key = ?1 AND namespace = ?2
              AND (?3 IS NULL OR chunk_id > ?3)
         )",
        params![runtime.scope_user_key, namespace, cursor],
        |row| row.get::<_, bool>(0),
    )?;
    if !has_more {
        let verified = indexed_rows == status.expected_rows
            && current_source_digest
                == vector_source_digest(&tx, &runtime.scope_user_key, namespace)?
            && verify_projection_vectors(
                &tx,
                &runtime.scope_user_key,
                namespace,
                generation as i64,
                &profile,
            )?
            && verify_ann_projection(
                &tx,
                &runtime.scope_user_key,
                namespace,
                generation as i64,
                indexed_rows,
            )?;
        if verified {
            activate_vector_generation(
                &tx,
                &runtime.scope_user_key,
                namespace,
                generation as i64,
                current_revision,
                now,
            )?;
        } else {
            tx.execute(
                "UPDATE kb_vector_generations SET state = 'stale', updated_at_epoch = ?4
                 WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
                params![runtime.scope_user_key, namespace, generation as i64, now,],
            )?;
        }
    }
    enforce_storage_quotas(&tx, &runtime.scope_user_key)?;
    let result = load_vector_reindex_status_from(
        &tx,
        &runtime.scope_user_key,
        namespace,
        Some(generation as i64),
    )?;
    tx.commit()?;
    Ok(result)
}

pub(super) fn load_vector_reindex_status(
    runtime: &KbRuntime,
    namespace: &str,
    generation: Option<u64>,
) -> Result<VectorReindexStatus> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    load_vector_reindex_status_from(
        &db,
        &runtime.scope_user_key,
        namespace,
        generation.map(|value| value as i64),
    )
}

pub(super) fn cancel_vector_reindex(
    runtime: &KbRuntime,
    namespace: &str,
    generation: u64,
) -> Result<VectorReindexStatus> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let status = load_vector_reindex_status_from(
        &tx,
        &runtime.scope_user_key,
        namespace,
        Some(generation as i64),
    )?;
    if status.state == "building" {
        tx.execute(
            "DELETE FROM kb_vector_projection
             WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
            params![runtime.scope_user_key, namespace, generation as i64],
        )?;
        tx.execute(
            "UPDATE kb_vector_generations SET
                state = 'cancelled', cursor_chunk_id = NULL,
                indexed_rows = 0, projection_digest = '', updated_at_epoch = ?4
             WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
            params![
                runtime.scope_user_key,
                namespace,
                generation as i64,
                super::now_epoch(),
            ],
        )?;
    }
    let result = load_vector_reindex_status_from(
        &tx,
        &runtime.scope_user_key,
        namespace,
        Some(generation as i64),
    )?;
    tx.commit()?;
    Ok(result)
}

fn load_vector_reindex_status_from(
    db: &Connection,
    owner: &str,
    namespace: &str,
    generation: Option<i64>,
) -> Result<VectorReindexStatus> {
    db.query_row(
        "SELECT g.namespace, g.generation, g.profile_id, g.namespace_revision,
                g.state, g.cursor_chunk_id, g.expected_rows, g.indexed_rows,
                g.source_digest, g.projection_digest, n.active_vector_generation,
                g.created_at_epoch, g.updated_at_epoch
         FROM kb_vector_generations g
         JOIN kb_namespaces n
           ON n.owner_user_key = g.owner_user_key AND n.namespace = g.namespace
         WHERE g.owner_user_key = ?1 AND g.namespace = ?2
           AND (?3 IS NULL OR g.generation = ?3)
         ORDER BY g.generation DESC LIMIT 1",
        params![owner, namespace, generation],
        |row| {
            Ok(VectorReindexStatus {
                namespace: row.get(0)?,
                generation: nonnegative_u64(row.get::<_, i64>(1)?),
                profile_id: row.get(2)?,
                namespace_revision: nonnegative_u64(row.get::<_, i64>(3)?),
                state: row.get(4)?,
                cursor_chunk_id: row.get(5)?,
                expected_rows: nonnegative_usize(row.get::<_, i64>(6)?),
                indexed_rows: nonnegative_usize(row.get::<_, i64>(7)?),
                source_digest: row.get(8)?,
                projection_digest: row.get(9)?,
                active_generation: row.get::<_, Option<i64>>(10)?.map(nonnegative_u64),
                created_at_epoch: row.get(11)?,
                updated_at_epoch: row.get(12)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| anyhow!("vector reindex generation not found"))
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

pub(super) fn load_search_candidates(
    runtime: &KbRuntime,
    namespace: &str,
    terms: &[String],
    query_vector: &[f32],
    filters: SearchFilters<'_>,
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
    let lexical_started = Instant::now();
    let lexical_chunk_ids = if has_fts(&db)? && !terms.is_empty() {
        fts_candidate_chunk_ids(
            &db,
            &runtime.scope_user_key,
            namespace,
            &source_prefix,
            terms,
            filters,
            limit,
        )
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let lexical_latency_ms = elapsed_ms(lexical_started);
    let semantic_started = Instant::now();
    let semantic_candidates = semantic_candidate_scores(
        &db,
        &runtime.scope_user_key,
        namespace,
        query_vector,
        filters,
        limit,
        if runtime.constrained_storage() {
            EXACT_VECTOR_LIMIT_CONSTRAINED
        } else {
            EXACT_VECTOR_LIMIT_STANDARD
        },
        if runtime.constrained_storage() {
            ANN_MAX_CANDIDATES_CONSTRAINED
        } else {
            ANN_MAX_CANDIDATES_STANDARD
        },
    )?;
    let semantic_latency_ms = elapsed_ms(semantic_started);
    let lexical_set = lexical_chunk_ids.iter().cloned().collect::<HashSet<_>>();
    let semantic_scores = semantic_candidates
        .scores
        .iter()
        .cloned()
        .collect::<HashMap<_, _>>();
    let mut chunk_ids = lexical_chunk_ids;
    let mut seen = lexical_set.clone();
    for (chunk_id, _) in semantic_candidates.scores {
        if seen.insert(chunk_id.clone()) {
            chunk_ids.push(chunk_id);
        }
    }
    let retrieval_mode = match (lexical_set.is_empty(), semantic_scores.is_empty()) {
        (false, false) => "hybrid_candidates",
        (false, true) => "fts5_candidates",
        (true, false) => "semantic_candidates",
        (true, true) => "no_match",
    };
    let materialization_started = Instant::now();
    index.chunks = load_chunks_by_ids(&db, &runtime.scope_user_key, namespace, &chunk_ids)?;
    let document_paths = index
        .chunks
        .iter()
        .map(|chunk| chunk.path.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    index.docs = load_documents_by_paths(&db, &runtime.scope_user_key, namespace, &document_paths)?;
    let materialization_latency_ms = elapsed_ms(materialization_started);
    Ok(SearchCandidateSet {
        index,
        total_chunks,
        retrieval_mode,
        lexical_chunk_ids: lexical_set,
        semantic_scores,
        embedding_profile_id: semantic_candidates.profile_id,
        vector_generation: semantic_candidates.generation,
        vector_degradation: semantic_candidates.degradation,
        lexical_latency_ms,
        semantic_latency_ms,
        materialization_latency_ms,
        semantic_backend: semantic_candidates.backend,
        semantic_rows_scored: semantic_candidates.rows_scored,
    })
}

fn elapsed_ms(started: Instant) -> f64 {
    (started.elapsed().as_secs_f64() * 1_000.0 * 1_000.0).round() / 1_000.0
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

fn storage_schema_is_current(db: &Connection) -> Result<bool> {
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
    let now = super::now_epoch();
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

fn activate_vector_generation(
    tx: &Transaction<'_>,
    owner: &str,
    namespace: &str,
    generation: i64,
    revision: u64,
    now: i64,
) -> Result<()> {
    let expected_rows = count_projection_rows(tx, owner, namespace, generation)?;
    if !verify_ann_projection(tx, owner, namespace, generation, expected_rows)? {
        return Err(anyhow!("ann_projection_verification_failed"));
    }
    tx.execute(
        "UPDATE kb_vector_generations SET state = 'retired', updated_at_epoch = ?4
         WHERE owner_user_key = ?1 AND namespace = ?2
           AND state = 'active' AND generation != ?3",
        params![owner, namespace, generation, now],
    )?;
    tx.execute(
        "UPDATE kb_vector_generations SET
            state = 'active', namespace_revision = ?4,
            updated_at_epoch = ?5, activated_at_epoch = ?5
         WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
        params![owner, namespace, generation, revision as i64, now],
    )?;
    tx.execute(
        "UPDATE kb_namespaces SET
            active_vector_generation = ?3, embedding_version = ?4
         WHERE owner_user_key = ?1 AND namespace = ?2",
        params![
            owner,
            namespace,
            generation,
            claw_core::retrieval_embedding::LOCAL_HASH_VERSION,
        ],
    )?;
    Ok(())
}

fn vector_source_digest(db: &Connection, owner: &str, namespace: &str) -> Result<String> {
    let mut stmt = db.prepare(
        "SELECT chunk_id, text_sha256 FROM kb_chunks
         WHERE owner_user_key = ?1 AND namespace = ?2 ORDER BY chunk_id",
    )?;
    let rows = stmt
        .query_map(params![owner, namespace], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut digest_input = Vec::new();
    for (chunk_id, content_digest) in rows {
        digest_input.extend_from_slice(chunk_id.as_bytes());
        digest_input.push(0);
        digest_input.extend_from_slice(content_digest.as_bytes());
        digest_input.push(0xff);
    }
    Ok(sha256_hex(&digest_input))
}

fn vector_projection_digest(
    db: &Connection,
    owner: &str,
    namespace: &str,
    generation: i64,
) -> Result<String> {
    let mut stmt = db.prepare(
        "SELECT chunk_id, content_digest, vector_json
         FROM kb_vector_projection
         WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
         ORDER BY chunk_id",
    )?;
    let rows = stmt
        .query_map(params![owner, namespace, generation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut digest_input = Vec::new();
    for (chunk_id, content_digest, vector_json) in rows {
        digest_input.extend_from_slice(chunk_id.as_bytes());
        digest_input.push(0);
        digest_input.extend_from_slice(content_digest.as_bytes());
        digest_input.push(0);
        digest_input.extend_from_slice(sha256_hex(vector_json.as_bytes()).as_bytes());
        digest_input.push(0xff);
    }
    Ok(sha256_hex(&digest_input))
}

fn count_projection_rows(
    db: &Connection,
    owner: &str,
    namespace: &str,
    generation: i64,
) -> Result<usize> {
    Ok(nonnegative_usize(db.query_row(
        "SELECT COUNT(*) FROM kb_vector_projection
         WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
        params![owner, namespace, generation],
        |row| row.get::<_, i64>(0),
    )?))
}

fn replace_ann_projection(
    db: &Connection,
    owner: &str,
    namespace: &str,
    generation: i64,
    chunk_id: &str,
    profile_id: &str,
    vector: &[f32],
) -> Result<()> {
    db.execute(
        "DELETE FROM kb_vector_ann_projection
         WHERE owner_user_key = ?1 AND namespace = ?2
           AND generation = ?3 AND chunk_id = ?4",
        params![owner, namespace, generation, chunk_id],
    )?;
    for (band, bucket) in claw_core::retrieval_embedding::vector_ann_buckets(vector)
        .into_iter()
        .enumerate()
    {
        db.execute(
            "INSERT INTO kb_vector_ann_projection (
                owner_user_key, namespace, generation, chunk_id, profile_id,
                projection_version, band, bucket
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                owner,
                namespace,
                generation,
                chunk_id,
                profile_id,
                claw_core::retrieval_embedding::VECTOR_ANN_PROJECTION_VERSION,
                band as i64,
                bucket as i64,
            ],
        )?;
    }
    Ok(())
}

#[cfg(test)]
fn rebuild_ann_projection(
    db: &Connection,
    owner: &str,
    namespace: &str,
    generation: i64,
) -> Result<()> {
    db.execute(
        "DELETE FROM kb_vector_ann_projection
         WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
        params![owner, namespace, generation],
    )?;
    let rows = {
        let mut stmt = db.prepare(
            "SELECT chunk_id, profile_id, vector_json FROM kb_vector_projection
             WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
             ORDER BY chunk_id",
        )?;
        let rows = stmt
            .query_map(params![owner, namespace, generation], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (chunk_id, profile_id, vector_json) in rows {
        let vector = serde_json::from_str::<Vec<f32>>(&vector_json)
            .context("decode vector while rebuilding ANN projection")?;
        replace_ann_projection(
            db,
            owner,
            namespace,
            generation,
            &chunk_id,
            &profile_id,
            &vector,
        )?;
    }
    Ok(())
}

fn verify_ann_projection(
    db: &Connection,
    owner: &str,
    namespace: &str,
    generation: i64,
    expected_vectors: usize,
) -> Result<bool> {
    let expected_rows = expected_vectors
        .checked_mul(claw_core::retrieval_embedding::VECTOR_ANN_BANDS)
        .ok_or_else(|| anyhow!("ANN projection row count overflow"))?;
    let rows = db.query_row(
        "SELECT COUNT(*) FROM kb_vector_ann_projection
         WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
           AND projection_version = ?4",
        params![
            owner,
            namespace,
            generation,
            claw_core::retrieval_embedding::VECTOR_ANN_PROJECTION_VERSION,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(nonnegative_usize(rows) == expected_rows)
}

fn ann_projection_available(
    db: &Connection,
    owner: &str,
    namespace: &str,
    generation: i64,
    profile_id: &str,
    expected_vectors: usize,
) -> Result<bool> {
    if expected_vectors == 0 {
        return Ok(true);
    }
    let Some(chunk_id) = db.query_row(
        "SELECT MIN(chunk_id) FROM kb_vector_projection
             WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
        params![owner, namespace, generation],
        |row| row.get::<_, Option<String>>(0),
    )?
    else {
        return Ok(false);
    };
    let mut stmt = db.prepare(
        "SELECT profile_id, projection_version
         FROM kb_vector_ann_projection
              INDEXED BY sqlite_autoindex_kb_vector_ann_projection_1
         WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
           AND chunk_id = ?4 AND band = ?5",
    )?;
    for band in 0..claw_core::retrieval_embedding::VECTOR_ANN_BANDS {
        let projection = stmt
            .query_row(
                params![owner, namespace, generation, chunk_id, band as i64],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if projection.as_ref().is_none_or(|(actual_profile, version)| {
            actual_profile != profile_id
                || version != claw_core::retrieval_embedding::VECTOR_ANN_PROJECTION_VERSION
        }) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn verify_projection_vectors(
    db: &Connection,
    owner: &str,
    namespace: &str,
    generation: i64,
    profile: &claw_core::retrieval_embedding::EmbeddingProfile,
) -> Result<bool> {
    let mut stmt = db.prepare(
        "SELECT profile_id, dimensions, vector_json FROM kb_vector_projection
         WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
    )?;
    let rows = stmt.query_map(params![owner, namespace, generation], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (profile_id, dimensions, vector_json) = row?;
        let vector = match serde_json::from_str::<Vec<f32>>(&vector_json) {
            Ok(vector) => vector,
            Err(_) => return Ok(false),
        };
        if profile_id != profile.profile_id
            || dimensions != profile.dimensions as i64
            || vector.len() != profile.dimensions
            || vector.iter().any(|value| !value.is_finite())
        {
            return Ok(false);
        }
    }
    Ok(true)
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

fn persist_namespace_delta(
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

fn ensure_writable_vector_generation(
    tx: &Transaction<'_>,
    index: &NamespaceIndex,
    revision: u64,
    extraction_contract_changed: bool,
) -> Result<Option<i64>> {
    let profile = claw_core::retrieval_embedding::local_hash_profile();
    let active = tx
        .query_row(
            "SELECT n.active_vector_generation, g.profile_id, g.namespace_revision, g.state
             FROM kb_namespaces n
             LEFT JOIN kb_vector_generations g
               ON g.owner_user_key = n.owner_user_key
              AND g.namespace = n.namespace
              AND g.generation = n.active_vector_generation
             WHERE n.owner_user_key = ?1 AND n.namespace = ?2",
            params![index.owner_user_key, index.namespace],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((generation, profile_id, _generation_revision, state)) = active else {
        return Err(anyhow!(
            "namespace missing while preparing vector generation"
        ));
    };
    if let Some(generation) = generation {
        if extraction_contract_changed {
            tx.execute(
                "UPDATE kb_vector_generations SET state = 'stale', updated_at_epoch = ?4
                 WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
                params![
                    index.owner_user_key,
                    index.namespace,
                    generation,
                    super::now_epoch(),
                ],
            )?;
            tx.execute(
                "UPDATE kb_namespaces SET active_vector_generation = NULL
                 WHERE owner_user_key = ?1 AND namespace = ?2",
                params![index.owner_user_key, index.namespace],
            )?;
            return Ok(None);
        }
        if profile_id.as_deref() == Some(profile.profile_id.as_str())
            && state.as_deref() == Some("active")
            && verify_ann_projection(
                tx,
                &index.owner_user_key,
                &index.namespace,
                generation,
                count_projection_rows(tx, &index.owner_user_key, &index.namespace, generation)?,
            )?
        {
            return Ok(Some(generation));
        }
        tx.execute(
            "UPDATE kb_vector_generations SET state = 'stale', updated_at_epoch = ?4
             WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
            params![
                index.owner_user_key,
                index.namespace,
                generation,
                super::now_epoch(),
            ],
        )?;
        return Ok(None);
    }

    let existing_chunks = count_rows(
        tx,
        "kb_chunks",
        "owner_user_key = ?1 AND namespace = ?2",
        &index.owner_user_key,
        &index.namespace,
    )?;
    if existing_chunks != 0 {
        return Ok(None);
    }
    let generation = tx.query_row(
        "SELECT COALESCE(MAX(generation), 0) + 1
         FROM kb_vector_generations WHERE owner_user_key = ?1 AND namespace = ?2",
        params![index.owner_user_key, index.namespace],
        |row| row.get::<_, i64>(0),
    )?;
    let now = super::now_epoch();
    tx.execute(
        "INSERT INTO kb_vector_generations (
            owner_user_key, namespace, generation, profile_id,
            namespace_revision, state, cursor_chunk_id, expected_rows,
            indexed_rows, source_digest, projection_digest,
            created_at_epoch, updated_at_epoch, activated_at_epoch
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', NULL, 0, 0, ?6, ?6, ?7, ?7, ?7)",
        params![
            index.owner_user_key,
            index.namespace,
            generation,
            profile.profile_id,
            revision as i64,
            sha256_hex(&[]),
            now,
        ],
    )?;
    tx.execute(
        "UPDATE kb_namespaces SET active_vector_generation = ?3
         WHERE owner_user_key = ?1 AND namespace = ?2",
        params![index.owner_user_key, index.namespace, generation],
    )?;
    Ok(Some(generation))
}

fn refresh_active_vector_generation(
    tx: &Transaction<'_>,
    owner: &str,
    namespace: &str,
    generation: i64,
    revision: u64,
) -> Result<()> {
    let profile = claw_core::retrieval_embedding::local_hash_profile();
    let expected_rows = count_rows(
        tx,
        "kb_chunks",
        "owner_user_key = ?1 AND namespace = ?2",
        owner,
        namespace,
    )?;
    let indexed_rows = count_projection_rows(tx, owner, namespace, generation)?;
    if indexed_rows != expected_rows
        || !verify_projection_vectors(tx, owner, namespace, generation, &profile)?
        || !verify_ann_projection(tx, owner, namespace, generation, expected_rows)?
    {
        return Err(anyhow!("embedding_projection_verification_failed"));
    }
    let source_digest = vector_source_digest(tx, owner, namespace)?;
    let projection_digest = vector_projection_digest(tx, owner, namespace, generation)?;
    let cursor = tx.query_row(
        "SELECT MAX(chunk_id) FROM kb_vector_projection
         WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
        params![owner, namespace, generation],
        |row| row.get::<_, Option<String>>(0),
    )?;
    tx.execute(
        "UPDATE kb_vector_generations SET
            profile_id = ?4, namespace_revision = ?5, state = 'active',
            cursor_chunk_id = ?6, expected_rows = ?7, indexed_rows = ?7,
            source_digest = ?8, projection_digest = ?9, updated_at_epoch = ?10
         WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3",
        params![
            owner,
            namespace,
            generation,
            profile.profile_id,
            revision as i64,
            cursor,
            expected_rows as i64,
            source_digest,
            projection_digest,
            super::now_epoch(),
        ],
    )?;
    Ok(())
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

fn insert_retrieval_row(
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

fn load_namespace_summary(
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

fn fts_candidate_chunk_ids(
    db: &Connection,
    owner: &str,
    namespace: &str,
    source_prefix: &str,
    terms: &[String],
    filters: SearchFilters<'_>,
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
    let path_pattern = filters.path_prefix.map(path_descendant_pattern);
    let mut stmt = db.prepare(
        "SELECT c.chunk_id
         FROM memory_retrieval_index_fts
         JOIN memory_retrieval_index i ON i.id = memory_retrieval_index_fts.rowid
         JOIN kb_chunks c
           ON c.owner_user_key = ?2
          AND c.namespace = ?3
          AND c.chunk_id = substr(i.source_ref, length(?4) + 1)
         WHERE memory_retrieval_index_fts MATCH ?1
           AND i.source_kind = 'kb_doc'
           AND i.user_key = ?2
           AND substr(i.source_ref, 1, length(?4)) = ?4
           AND (?5 IS NULL OR c.document_path = ?5 OR c.document_path LIKE ?6 ESCAPE '\\')
           AND (?7 IS NULL OR c.file_type = ?7)
           AND (?8 IS NULL OR c.mtime_epoch >= ?8)
           AND (?9 IS NULL OR c.mtime_epoch <= ?9)
         ORDER BY bm25(memory_retrieval_index_fts), i.id
         LIMIT ?10",
    )?;
    let chunk_ids = stmt
        .query_map(
            params![
                query,
                owner,
                namespace,
                source_prefix,
                filters.path_prefix,
                path_pattern,
                filters.file_type,
                filters.time_from,
                filters.time_to,
                limit.max(1) as i64
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(chunk_ids)
}

fn semantic_candidate_scores(
    db: &Connection,
    owner: &str,
    namespace: &str,
    query_vector: &[f32],
    filters: SearchFilters<'_>,
    limit: usize,
    exact_vector_limit: usize,
    ann_max_candidates: usize,
) -> Result<SemanticCandidateSet> {
    let profile = claw_core::retrieval_embedding::local_hash_profile();
    if query_vector.is_empty() {
        return Ok(SemanticCandidateSet {
            scores: Vec::new(),
            profile_id: profile.profile_id,
            generation: None,
            degradation: Some("embedding_query_empty".to_string()),
            backend: "none",
            rows_scored: 0,
        });
    }
    let active = db
        .query_row(
            "SELECT n.active_vector_generation, n.revision,
                    g.profile_id, g.namespace_revision, g.state, g.indexed_rows,
                    p.model_id, p.dimensions, p.normalization, p.profile_version
             FROM kb_namespaces n
             LEFT JOIN kb_vector_generations g
               ON g.owner_user_key = n.owner_user_key
              AND g.namespace = n.namespace
              AND g.generation = n.active_vector_generation
             LEFT JOIN kb_embedding_profiles p ON p.profile_id = g.profile_id
             WHERE n.owner_user_key = ?1 AND n.namespace = ?2",
            params![owner, namespace],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, i64>(1)?.max(0) as u64,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?
                        .map(|value| value.max(0) as u64),
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        Some(generation),
        namespace_revision,
        profile_id,
        generation_revision,
        state,
        generation_indexed_rows,
        model_id,
        dimensions,
        normalization,
        profile_version,
    )) = active
    else {
        return Ok(SemanticCandidateSet {
            scores: Vec::new(),
            profile_id: profile.profile_id,
            generation: None,
            degradation: Some("embedding_projection_missing".to_string()),
            backend: "none",
            rows_scored: 0,
        });
    };
    let generation_u64 = generation.max(0) as u64;
    let compatible = profile_id.as_deref() == Some(profile.profile_id.as_str())
        && generation_revision == Some(namespace_revision)
        && state.as_deref() == Some("active")
        && model_id.as_deref() == Some(profile.model_id.as_str())
        && dimensions == Some(profile.dimensions as i64)
        && normalization.as_deref() == Some(profile.normalization.as_str())
        && profile_version.as_deref() == Some(profile.version.as_str());
    if !compatible {
        return Ok(SemanticCandidateSet {
            scores: Vec::new(),
            profile_id: profile.profile_id,
            generation: Some(generation_u64),
            degradation: Some("embedding_projection_stale".to_string()),
            backend: "none",
            rows_scored: 0,
        });
    }
    let unfiltered = filters.path_prefix.is_none()
        && filters.file_type.is_none()
        && filters.time_from.is_none()
        && filters.time_to.is_none();
    let generation_rows = nonnegative_usize(generation_indexed_rows.unwrap_or(0));
    let exact_rows = if unfiltered && generation_rows > exact_vector_limit.max(1) {
        None
    } else {
        let rows = load_filtered_vector_rows(
            db,
            owner,
            namespace,
            generation,
            &profile.profile_id,
            profile.dimensions,
            filters,
            (!unfiltered).then_some(exact_vector_limit.max(1) + 1),
        )?;
        (rows.len() <= exact_vector_limit.max(1)).then_some(rows)
    };
    let (backend, rows) = if let Some(rows) = exact_rows {
        ("exact", rows)
    } else {
        if !ann_projection_available(
            db,
            owner,
            namespace,
            generation,
            &profile.profile_id,
            generation_rows.max(1),
        )? {
            return Ok(SemanticCandidateSet {
                scores: Vec::new(),
                profile_id: profile.profile_id,
                generation: Some(generation_u64),
                degradation: Some("ann_projection_stale".to_string()),
                backend: "ann_projection",
                rows_scored: 0,
            });
        }
        let path_pattern = filters.path_prefix.map(path_descendant_pattern);
        let buckets = claw_core::retrieval_embedding::vector_ann_buckets(query_vector);
        let mut stmt = db.prepare(
            "WITH ann_matches(chunk_id) AS (
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 0 AND bucket = ?12
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 1 AND bucket = ?13
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 2 AND bucket = ?14
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 3 AND bucket = ?15
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 4 AND bucket = ?16
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 5 AND bucket = ?17
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 6 AND bucket = ?18
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 7 AND bucket = ?19
             )
             SELECT c.chunk_id, v.vector_json
             FROM ann_matches a
             CROSS JOIN kb_chunks c
               ON c.owner_user_key = ?1
              AND c.namespace = ?2
              AND c.chunk_id = a.chunk_id
             CROSS JOIN kb_vector_projection v
               ON v.owner_user_key = ?1
              AND v.namespace = ?2
              AND v.generation = ?3
              AND v.chunk_id = a.chunk_id
             WHERE v.profile_id = ?4 AND v.dimensions = ?5
               AND (?7 IS NULL OR c.document_path = ?7 OR c.document_path LIKE ?8 ESCAPE '\\')
               AND (?9 IS NULL OR c.file_type = ?9)
               AND (?10 IS NULL OR c.mtime_epoch >= ?10)
               AND (?11 IS NULL OR c.mtime_epoch <= ?11)
             GROUP BY c.chunk_id, v.vector_json
             ORDER BY COUNT(*) DESC, c.chunk_id
             LIMIT ?20",
        )?;
        let candidate_limit = limit.max(256).min(ann_max_candidates.max(1));
        let rows = stmt
            .query_map(
                params![
                    owner,
                    namespace,
                    generation,
                    profile.profile_id,
                    profile.dimensions as i64,
                    claw_core::retrieval_embedding::VECTOR_ANN_PROJECTION_VERSION,
                    filters.path_prefix,
                    path_pattern,
                    filters.file_type,
                    filters.time_from,
                    filters.time_to,
                    buckets[0] as i64,
                    buckets[1] as i64,
                    buckets[2] as i64,
                    buckets[3] as i64,
                    buckets[4] as i64,
                    buckets[5] as i64,
                    buckets[6] as i64,
                    buckets[7] as i64,
                    candidate_limit as i64,
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ("ann_projection", rows)
    };
    let rows_scored = rows.len();
    let mut scored = rows
        .into_iter()
        .filter_map(|(chunk_id, vector_json)| {
            let vector = serde_json::from_str::<Vec<f32>>(&vector_json).ok()?;
            let score =
                claw_core::retrieval_embedding::cosine_similarity(query_vector, &vector) as f64;
            (score >= 0.35).then_some((chunk_id, score))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    scored.truncate(limit.max(1));
    Ok(SemanticCandidateSet {
        scores: scored,
        profile_id: profile.profile_id,
        generation: Some(generation_u64),
        degradation: None,
        backend,
        rows_scored,
    })
}

fn path_descendant_pattern(prefix: &str) -> String {
    let escaped = prefix
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("{escaped}/%")
}

fn vector_filter_sql(filters: SearchFilters<'_>) -> (String, Vec<SqlValue>) {
    let mut clauses = String::new();
    let mut values = Vec::new();
    if let Some(prefix) = filters.path_prefix {
        clauses.push_str(
            " AND (c.document_path = ? OR (c.document_path >= ? AND c.document_path < ?))",
        );
        values.push(SqlValue::Text(prefix.to_string()));
        values.push(SqlValue::Text(format!("{prefix}/")));
        values.push(SqlValue::Text(format!("{prefix}0")));
    }
    if let Some(file_type) = filters.file_type {
        clauses.push_str(" AND c.file_type = ?");
        values.push(SqlValue::Text(file_type.to_string()));
    }
    if let Some(time_from) = filters.time_from {
        clauses.push_str(" AND c.mtime_epoch >= ?");
        values.push(SqlValue::Integer(time_from));
    }
    if let Some(time_to) = filters.time_to {
        clauses.push_str(" AND c.mtime_epoch <= ?");
        values.push(SqlValue::Integer(time_to));
    }
    (clauses, values)
}

fn vector_query_params(
    owner: &str,
    namespace: &str,
    generation: i64,
    profile_id: &str,
    dimensions: usize,
    filters: SearchFilters<'_>,
) -> (String, Vec<SqlValue>) {
    let (clauses, filter_values) = vector_filter_sql(filters);
    let mut values = vec![
        SqlValue::Integer(generation),
        SqlValue::Text(owner.to_string()),
        SqlValue::Text(namespace.to_string()),
        SqlValue::Text(profile_id.to_string()),
        SqlValue::Integer(dimensions as i64),
    ];
    values.extend(filter_values);
    (clauses, values)
}

fn load_filtered_vector_rows(
    db: &Connection,
    owner: &str,
    namespace: &str,
    generation: i64,
    profile_id: &str,
    dimensions: usize,
    filters: SearchFilters<'_>,
    max_rows: Option<usize>,
) -> Result<Vec<(String, String)>> {
    let (clauses, mut values) = vector_query_params(
        owner, namespace, generation, profile_id, dimensions, filters,
    );
    let chunk_index = if filters.path_prefix.is_some() {
        "idx_kb_chunks_owner_namespace_document"
    } else {
        "idx_kb_chunks_owner_namespace_filter"
    };
    let limit_clause = if let Some(max_rows) = max_rows {
        values.push(SqlValue::Integer(max_rows as i64));
        " LIMIT ?"
    } else {
        ""
    };
    let sql = format!(
        "SELECT c.chunk_id, v.vector_json
         FROM kb_chunks c INDEXED BY {chunk_index}
         CROSS JOIN kb_vector_projection v
           ON v.owner_user_key = c.owner_user_key
          AND v.namespace = c.namespace
          AND v.generation = ?
          AND v.chunk_id = c.chunk_id
         WHERE c.owner_user_key = ? AND c.namespace = ?
           AND v.profile_id = ? AND v.dimensions = ?{clauses}
         ORDER BY c.chunk_id{limit_clause}"
    );
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(values), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
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
    if chunk_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", chunk_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT chunk_id, document_path, file_type, ordinal, text,
                text_sha256, len_tokens, mtime_epoch, content_kind, location_json
         FROM kb_chunks
         WHERE owner_user_key = ? AND namespace = ? AND chunk_id IN ({placeholders})"
    );
    let mut values = Vec::with_capacity(chunk_ids.len() + 2);
    values.push(SqlValue::Text(owner.to_string()));
    values.push(SqlValue::Text(namespace.to_string()));
    values.extend(chunk_ids.iter().cloned().map(SqlValue::Text));
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values), |row| {
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
    let mut by_id = rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|chunk| (chunk.chunk_id.clone(), chunk))
        .collect::<HashMap<_, _>>();
    Ok(chunk_ids
        .iter()
        .filter_map(|chunk_id| by_id.remove(chunk_id))
        .collect())
}

fn load_documents_by_paths(
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

fn ensure_owner_quota_row(db: &Connection, owner: &str) -> Result<()> {
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
            super::now_epoch(),
        ],
    )?;
    Ok(())
}

fn owner_quotas(db: &Connection, owner: &str) -> Result<StorageQuotas> {
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

fn active_ingest_job_for_namespace(
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

fn active_vector_job_for_namespace(
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

fn owner_active_job_count(db: &Connection, owner: &str) -> Result<u64> {
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

fn latest_owner_active_job_id(db: &Connection, owner: &str) -> Result<String> {
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

fn enforce_storage_quotas(db: &Connection, owner: &str) -> Result<()> {
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

fn run_bounded_storage_maintenance(db: &mut Connection) -> Result<()> {
    let now = super::now_epoch();
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
        .collect::<Vec<_>>()
        .join(" ")
}

fn vector_to_json(vector: &[f32]) -> String {
    serde_json::to_string(vector).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
