use std::cmp::Ordering;

use claw_core::config::MemoryConfig;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

const MIGRATION_ID: &str = "014_memory_vector_pipeline_v1";
const MIGRATION_SQL: &str = include_str!("../../../../migrations/014_memory_vector_pipeline.sql");
pub(crate) const LOCAL_PROFILE_ID: &str = "memory_embedding:local_default";
pub(crate) const PROJECTION_VERSION: &str = "memory_searchable_projection_v2";
const VECTOR_FORMAT: &str = "f32le_v1";
const VECTOR_MAGIC: &[u8; 5] = b"MVEC\x01";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct MemoryEmbeddingProfile {
    pub(crate) profile_id: String,
    pub(crate) provider_kind: String,
    pub(crate) endpoint_ref: Option<String>,
    pub(crate) credential_ref: Option<String>,
    pub(crate) model_name: String,
    pub(crate) dimensions: usize,
    pub(crate) normalization: String,
    pub(crate) projection_version: String,
    pub(crate) profile_version: String,
    pub(crate) remote_consent_required: bool,
    pub(crate) generation: u64,
    pub(crate) config_digest: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorNeighbor {
    pub(crate) retrieval_id: i64,
    pub(crate) score: f32,
}

pub(crate) trait MemoryVectorIndex {
    fn upsert(
        &self,
        db: &Connection,
        profile: &MemoryEmbeddingProfile,
        row: &VectorWrite<'_>,
    ) -> anyhow::Result<()>;

    fn nearest(
        &self,
        db: &Connection,
        access: &super::scope::ResolvedMemoryAccess,
        profile: &MemoryEmbeddingProfile,
        query: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<VectorNeighbor>>;

    fn tombstone_orphans(&self, db: &Connection) -> anyhow::Result<usize>;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ExactSqliteVectorIndex;

pub(crate) struct VectorWrite<'a> {
    pub(crate) retrieval_id: i64,
    pub(crate) principal_id: &'a str,
    pub(crate) scope_kind: &'a str,
    pub(crate) scope_ref: &'a str,
    pub(crate) projection_digest: &'a str,
    pub(crate) vector: &'a [f32],
}

impl MemoryVectorIndex for ExactSqliteVectorIndex {
    fn upsert(
        &self,
        db: &Connection,
        profile: &MemoryEmbeddingProfile,
        row: &VectorWrite<'_>,
    ) -> anyhow::Result<()> {
        validate_vector(row.vector, profile.dimensions, &profile.normalization)?;
        let blob = encode_vector_blob(row.vector)?;
        let checksum = digest_bytes(&blob);
        let now = crate::now_ts_u64() as i64;
        db.execute(
            "INSERT INTO memory_vector_rows(
                retrieval_id, principal_id, scope_kind, scope_ref, profile_id,
                generation, projection_version, projection_digest, vector_format,
                dimensions, normalization, vector_blob, vector_checksum, status,
                created_at_ts, updated_at_ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                       'active', ?14, ?14)
             ON CONFLICT(retrieval_id, profile_id, generation) DO UPDATE SET
                principal_id = excluded.principal_id,
                scope_kind = excluded.scope_kind,
                scope_ref = excluded.scope_ref,
                generation = excluded.generation,
                projection_version = excluded.projection_version,
                projection_digest = excluded.projection_digest,
                vector_format = excluded.vector_format,
                dimensions = excluded.dimensions,
                normalization = excluded.normalization,
                vector_blob = excluded.vector_blob,
                vector_checksum = excluded.vector_checksum,
                status = 'active', updated_at_ts = excluded.updated_at_ts",
            params![
                row.retrieval_id,
                row.principal_id,
                row.scope_kind,
                row.scope_ref,
                profile.profile_id,
                profile.generation as i64,
                profile.projection_version,
                row.projection_digest,
                VECTOR_FORMAT,
                profile.dimensions as i64,
                profile.normalization,
                blob,
                checksum,
                now,
            ],
        )?;
        Ok(())
    }

    fn nearest(
        &self,
        db: &Connection,
        access: &super::scope::ResolvedMemoryAccess,
        profile: &MemoryEmbeddingProfile,
        query: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<VectorNeighbor>> {
        validate_vector(query, profile.dimensions, &profile.normalization)?;
        let generation = active_generation_for_principal(
            db,
            &access.principal_id,
            &profile.profile_id,
            profile.generation,
        )?;
        let mut stmt = db.prepare(
            "SELECT retrieval_id, vector_blob, vector_checksum
             FROM memory_vector_rows
             WHERE principal_id = ?1 AND profile_id = ?2 AND generation = ?3
               AND status = 'active'
               AND ((scope_kind = 'principal' AND scope_ref = ?4)
                 OR (scope_kind = 'conversation' AND ?5 IS NOT NULL AND scope_ref = ?5)
                 OR (scope_kind = 'project' AND ?6 IS NOT NULL AND scope_ref = ?6))",
        )?;
        let rows = stmt.query_map(
            params![
                access.principal_id,
                profile.profile_id,
                generation as i64,
                access.principal_scope_ref,
                access.conversation_scope_ref,
                access.project_scope_ref,
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?;
        let mut neighbors = Vec::new();
        for row in rows {
            let (retrieval_id, blob, checksum) = row?;
            if digest_bytes(&blob) != checksum {
                continue;
            }
            let Ok(vector) = decode_vector_blob(&blob) else {
                continue;
            };
            if vector.len() != query.len() {
                continue;
            }
            neighbors.push(VectorNeighbor {
                retrieval_id,
                score: cosine_similarity(query, &vector),
            });
        }
        neighbors.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.retrieval_id.cmp(&right.retrieval_id))
        });
        neighbors.truncate(limit.max(1));
        Ok(neighbors)
    }

    fn tombstone_orphans(&self, db: &Connection) -> anyhow::Result<usize> {
        let changed = db.execute(
            "UPDATE memory_vector_rows SET status = 'tombstone', updated_at_ts = ?1
             WHERE status = 'active' AND NOT EXISTS (
                SELECT 1 FROM memory_retrieval_index i
                WHERE i.id = memory_vector_rows.retrieval_id
             )",
            [crate::now_ts_u64() as i64],
        )?;
        Ok(changed)
    }
}

pub(crate) fn ensure_vector_pipeline_schema(db: &Connection) -> anyhow::Result<()> {
    if let Some(applied) = migration_digest(db)? {
        anyhow::ensure!(
            applied == migration_manifest_digest(),
            "runtime_schema_migration_digest_mismatch:{MIGRATION_ID}"
        );
    }
    if db.is_autocommit() {
        let tx = db.unchecked_transaction()?;
        apply_migration(&tx)?;
        tx.commit()?;
    } else {
        apply_migration(db)?;
    }
    Ok(())
}

fn apply_migration(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(MIGRATION_SQL)?;
    db.execute(
        "INSERT INTO runtime_schema_migrations(migration_id, schema_digest, applied_at)
         VALUES (?1, ?2, ?3) ON CONFLICT(migration_id) DO NOTHING",
        params![MIGRATION_ID, migration_manifest_digest(), crate::now_ts()],
    )?;
    Ok(())
}

pub(crate) fn configured_profile(cfg: &MemoryConfig) -> anyhow::Result<MemoryEmbeddingProfile> {
    let provider_kind = cfg.embedding_provider_kind.trim().to_ascii_lowercase();
    anyhow::ensure!(
        matches!(provider_kind.as_str(), "local" | "remote_http" | "mock"),
        "memory_embedding_provider_kind_invalid"
    );
    let normalization = cfg.embedding_normalization.trim().to_ascii_lowercase();
    anyhow::ensure!(
        matches!(normalization.as_str(), "unit_length" | "none"),
        "memory_embedding_normalization_invalid"
    );
    let metric = cfg.embedding_metric.trim().to_ascii_lowercase();
    anyhow::ensure!(metric == "cosine", "memory_embedding_metric_invalid");
    anyhow::ensure!(
        cfg.embedding_batch_size > 0 && cfg.embedding_batch_size <= 256,
        "memory_embedding_batch_limit_invalid"
    );
    anyhow::ensure!(
        cfg.embedding_connect_timeout_ms >= 100
            && cfg.embedding_connect_timeout_ms <= 120_000
            && cfg.embedding_idle_timeout_ms >= 100
            && cfg.embedding_idle_timeout_ms <= 120_000
            && cfg.embedding_query_timeout_ms >= 100
            && cfg.embedding_query_timeout_ms <= 120_000,
        "memory_embedding_timeout_invalid"
    );
    anyhow::ensure!(
        cfg.embedding_retry_max_attempts > 0
            && cfg.embedding_circuit_failure_threshold > 0
            && cfg.embedding_circuit_reset_seconds > 0,
        "memory_embedding_retry_policy_invalid"
    );
    anyhow::ensure!(
        cfg.embedding_query_cache_ttl_seconds > 0
            && cfg.embedding_query_cache_max_bytes >= 1024
            && cfg.embedding_max_request_bytes >= 1024,
        "memory_embedding_limit_invalid"
    );
    let dimensions = if provider_kind == "local" {
        super::embedding::LOCAL_HASH_DIMS
    } else {
        cfg.embedding_dims
    };
    anyhow::ensure!(
        dimensions > 0 && dimensions <= 65_536,
        "memory_embedding_dims_invalid"
    );
    if provider_kind == "remote_http" {
        anyhow::ensure!(
            !cfg.embedding_endpoint_ref.trim().is_empty()
                && !cfg.embedding_credential_ref.trim().is_empty(),
            "memory_embedding_remote_refs_required"
        );
    }
    let model_name = if provider_kind == "local" {
        super::embedding::LOCAL_HASH_MODEL_ID.to_string()
    } else {
        cfg.embedding_model.trim().to_string()
    };
    let profile_version = if provider_kind == "local" {
        super::embedding::LOCAL_HASH_VERSION.to_string()
    } else {
        cfg.embedding_version.trim().to_string()
    };
    let config_digest = digest_json(&json!({
        "provider_kind": provider_kind,
        "endpoint_ref": cfg.embedding_endpoint_ref,
        "credential_ref": cfg.embedding_credential_ref,
        "model_name": model_name,
        "dimensions": dimensions,
        "normalization": normalization,
        "projection_version": PROJECTION_VERSION,
        "profile_version": profile_version,
        "remote_consent_required": cfg.embedding_remote_opt_in_required,
        "metric": metric,
        "batch_limit": cfg.embedding_batch_size.clamp(1, 256),
        "connect_timeout_ms": cfg.embedding_connect_timeout_ms,
        "idle_timeout_ms": cfg.embedding_idle_timeout_ms,
        "request_timeout_ms": cfg.embedding_query_timeout_ms.clamp(100, 120_000),
        "retry_max_attempts": cfg.embedding_retry_max_attempts,
        "circuit_failure_threshold": cfg.embedding_circuit_failure_threshold,
        "circuit_reset_seconds": cfg.embedding_circuit_reset_seconds,
        "query_cache_ttl_seconds": cfg.embedding_query_cache_ttl_seconds,
        "query_cache_max_bytes": cfg.embedding_query_cache_max_bytes,
        "reindex_batch_delay_ms": cfg.embedding_reindex_batch_delay_ms.min(60_000),
        "max_request_bytes": cfg.embedding_max_request_bytes,
    }));
    let profile_id = format!(
        "memory_embedding:{}:{}:{}:{}",
        provider_kind,
        short_digest(model_name.as_bytes()),
        short_digest(profile_version.as_bytes()),
        short_digest(config_digest.as_bytes())
    );
    Ok(MemoryEmbeddingProfile {
        profile_id: if provider_kind == "local" {
            LOCAL_PROFILE_ID.to_string()
        } else {
            profile_id
        },
        provider_kind,
        endpoint_ref: non_empty(&cfg.embedding_endpoint_ref),
        credential_ref: non_empty(&cfg.embedding_credential_ref),
        model_name,
        dimensions,
        normalization,
        projection_version: PROJECTION_VERSION.to_string(),
        profile_version,
        remote_consent_required: cfg.embedding_remote_opt_in_required,
        generation: 1,
        config_digest,
    })
}

pub(crate) fn register_configured_profile(
    db: &Connection,
    cfg: &MemoryConfig,
) -> anyhow::Result<MemoryEmbeddingProfile> {
    ensure_vector_pipeline_schema(db)?;
    register_profile(db, &local_profile())?;
    let profile = configured_profile(cfg)?;
    register_profile(db, &profile)?;
    load_profile(db, &profile.profile_id)?
        .ok_or_else(|| anyhow::anyhow!("memory_embedding_profile_missing_after_registration"))
}

pub(crate) fn active_generation_for_principal(
    db: &Connection,
    principal_id: &str,
    profile_id: &str,
    fallback_generation: u64,
) -> anyhow::Result<u64> {
    Ok(db
        .query_row(
            "SELECT generation FROM memory_vector_snapshots
             WHERE principal_id = ?1 AND profile_id = ?2 AND state = 'active'
             ORDER BY generation DESC LIMIT 1",
            params![principal_id, profile_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .map(|value| value.max(0) as u64)
        .unwrap_or(fallback_generation))
}

fn register_profile(db: &Connection, profile: &MemoryEmbeddingProfile) -> anyhow::Result<()> {
    let now = crate::now_ts_u64() as i64;
    db.execute(
        "INSERT INTO memory_embedding_profiles(
            profile_id, provider_kind, endpoint_ref, credential_ref, model_name,
            dimensions, normalization, projection_version, profile_version,
            remote_consent_required, state, generation, config_digest,
            created_at_ts, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active', ?11, ?12, ?13, ?13)
         ON CONFLICT(profile_id) DO UPDATE SET
            endpoint_ref = excluded.endpoint_ref, credential_ref = excluded.credential_ref,
            model_name = excluded.model_name, dimensions = excluded.dimensions,
            normalization = excluded.normalization,
            projection_version = excluded.projection_version,
            profile_version = excluded.profile_version,
            remote_consent_required = excluded.remote_consent_required,
            config_digest = excluded.config_digest, updated_at_ts = excluded.updated_at_ts",
        params![
            profile.profile_id,
            profile.provider_kind,
            profile.endpoint_ref,
            profile.credential_ref,
            profile.model_name,
            profile.dimensions as i64,
            profile.normalization,
            profile.projection_version,
            profile.profile_version,
            if profile.remote_consent_required {
                1
            } else {
                0
            },
            profile.generation as i64,
            profile.config_digest,
            now,
        ],
    )?;
    Ok(())
}

pub(crate) fn local_profile() -> MemoryEmbeddingProfile {
    let config_digest = digest_json(&json!({
        "provider_kind": "local",
        "model_name": super::embedding::LOCAL_HASH_MODEL_ID,
        "dimensions": super::embedding::LOCAL_HASH_DIMS,
        "normalization": "unit_length",
        "projection_version": PROJECTION_VERSION,
        "profile_version": super::embedding::LOCAL_HASH_VERSION,
    }));
    MemoryEmbeddingProfile {
        profile_id: LOCAL_PROFILE_ID.to_string(),
        provider_kind: "local".to_string(),
        endpoint_ref: None,
        credential_ref: None,
        model_name: super::embedding::LOCAL_HASH_MODEL_ID.to_string(),
        dimensions: super::embedding::LOCAL_HASH_DIMS,
        normalization: "unit_length".to_string(),
        projection_version: PROJECTION_VERSION.to_string(),
        profile_version: super::embedding::LOCAL_HASH_VERSION.to_string(),
        remote_consent_required: false,
        generation: 1,
        config_digest,
    }
}

pub(crate) fn enqueue_retrieval_embedding(
    db: &Connection,
    retrieval_id: i64,
    principal_id: Option<&str>,
    scope_kind: &str,
    scope_ref: Option<&str>,
    search_text: &str,
) -> anyhow::Result<Option<String>> {
    let Some(principal_id) = principal_id.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let scope_ref = scope_ref.unwrap_or(principal_id);
    ensure_vector_pipeline_schema(db)?;
    let projection_digest = searchable_projection_digest(search_text);
    let job_id = format!("memory_embedding_job_{}", uuid::Uuid::new_v4().simple());
    let request_item_id = format!("memory_embedding_item:{retrieval_id}:{projection_digest}");
    let now = crate::now_ts_u64() as i64;
    db.execute(
        "INSERT INTO memory_embedding_jobs(
            job_id, retrieval_id, principal_id, scope_kind, scope_ref, profile_id, profile_generation,
            request_item_id, projection_version, projection_digest,
            consent_policy_digest, status, not_before_ts, created_at_ts, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9, ?10, 'queued', ?11, ?11, ?11)
         ON CONFLICT(retrieval_id, profile_id, profile_generation, projection_digest) DO NOTHING",
        params![
            job_id,
            retrieval_id,
            principal_id,
            scope_kind,
            scope_ref,
            LOCAL_PROFILE_ID,
            request_item_id,
            PROJECTION_VERSION,
            projection_digest,
            digest_bytes(b"local_no_outbound"),
            now,
        ],
    )?;
    Ok((db.changes() > 0).then_some(job_id))
}

pub(crate) fn load_profile(
    db: &Connection,
    profile_id: &str,
) -> anyhow::Result<Option<MemoryEmbeddingProfile>> {
    db.query_row(
        "SELECT profile_id, provider_kind, endpoint_ref, credential_ref, model_name,
                dimensions, normalization, projection_version, profile_version,
                remote_consent_required, generation, config_digest
         FROM memory_embedding_profiles WHERE profile_id = ?1 AND state IN ('active', 'building')",
        [profile_id],
        |row| {
            Ok(MemoryEmbeddingProfile {
                profile_id: row.get(0)?,
                provider_kind: row.get(1)?,
                endpoint_ref: row.get(2)?,
                credential_ref: row.get(3)?,
                model_name: row.get(4)?,
                dimensions: row.get::<_, i64>(5)?.max(0) as usize,
                normalization: row.get(6)?,
                projection_version: row.get(7)?,
                profile_version: row.get(8)?,
                remote_consent_required: row.get::<_, i64>(9)? != 0,
                generation: row.get::<_, i64>(10)?.max(0) as u64,
                config_digest: row.get(11)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub(crate) fn searchable_projection_digest(text: &str) -> String {
    digest_json(&json!({
        "projection_version": PROJECTION_VERSION,
        "text": text,
    }))
}

pub(crate) fn encode_vector_blob(vector: &[f32]) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        vector.iter().all(|value| value.is_finite()),
        "memory_vector_non_finite"
    );
    let dims =
        u32::try_from(vector.len()).map_err(|_| anyhow::anyhow!("memory_vector_too_large"))?;
    let mut out = Vec::with_capacity(VECTOR_MAGIC.len() + 4 + vector.len() * 4);
    out.extend_from_slice(VECTOR_MAGIC);
    out.extend_from_slice(&dims.to_le_bytes());
    for value in vector {
        out.extend_from_slice(&value.to_le_bytes());
    }
    Ok(out)
}

pub(crate) fn decode_vector_blob(blob: &[u8]) -> anyhow::Result<Vec<f32>> {
    anyhow::ensure!(
        blob.len() >= 9 && &blob[..5] == VECTOR_MAGIC,
        "memory_vector_format_invalid"
    );
    let dims = u32::from_le_bytes(blob[5..9].try_into()?) as usize;
    anyhow::ensure!(blob.len() == 9 + dims * 4, "memory_vector_length_invalid");
    let mut out = Vec::with_capacity(dims);
    for chunk in blob[9..].chunks_exact(4) {
        let value = f32::from_le_bytes(chunk.try_into()?);
        anyhow::ensure!(value.is_finite(), "memory_vector_non_finite");
        out.push(value);
    }
    Ok(out)
}

pub(crate) fn validate_vector(
    vector: &[f32],
    dimensions: usize,
    normalization: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(vector.len() == dimensions, "memory_vector_dims_mismatch");
    anyhow::ensure!(
        vector.iter().all(|value| value.is_finite()),
        "memory_vector_non_finite"
    );
    if normalization == "unit_length" && vector.iter().any(|value| value.abs() > f32::EPSILON) {
        let norm = vector
            .iter()
            .map(|value| (*value as f64) * (*value as f64))
            .sum::<f64>()
            .sqrt();
        anyhow::ensure!((norm - 1.0).abs() <= 0.01, "memory_vector_not_normalized");
    }
    Ok(())
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for (left, right) in a.iter().zip(b.iter()) {
        dot += left * right;
        norm_a += left * left;
        norm_b += right * right;
    }
    if norm_a <= f32::EPSILON || norm_b <= f32::EPSILON {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

fn migration_digest(db: &Connection) -> anyhow::Result<Option<String>> {
    db.query_row(
        "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id = ?1",
        [MIGRATION_ID],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn migration_manifest_digest() -> String {
    digest_bytes(MIGRATION_SQL.as_bytes())
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn digest_json(value: &serde_json::Value) -> String {
    digest_bytes(&serde_json::to_vec(value).unwrap_or_default())
}

fn digest_bytes(value: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(value))
}

fn short_digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))[..16].to_string()
}

#[cfg(test)]
#[path = "vector_store_tests.rs"]
mod tests;
