use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::database::{
    active_ingest_job_for_namespace, count_rows, enforce_storage_quotas, ensure_owner_quota_row,
    ensure_schema, latest_owner_active_job_id, open, owner_active_job_count, owner_quotas,
};
use super::{
    nonnegative_u64, nonnegative_usize, sha256_hex, vector_to_json, KbRuntime, NamespaceIndex,
    StoragePolicyError, VectorReindexStatus,
};

pub(crate) fn start_vector_reindex(
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
        let now = crate::now_epoch();
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

pub(crate) fn resume_vector_reindex(
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
                crate::now_epoch(),
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
    let now = crate::now_epoch();
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

pub(crate) fn load_vector_reindex_status(
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

pub(crate) fn cancel_vector_reindex(
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
                crate::now_epoch(),
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

pub(super) fn replace_ann_projection(
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

pub(super) fn verify_ann_projection(
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

pub(super) fn ann_projection_available(
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

pub(super) fn ensure_writable_vector_generation(
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
                    crate::now_epoch(),
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
                crate::now_epoch(),
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
    let now = crate::now_epoch();
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

pub(super) fn refresh_active_vector_generation(
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
            crate::now_epoch(),
        ],
    )?;
    Ok(())
}
