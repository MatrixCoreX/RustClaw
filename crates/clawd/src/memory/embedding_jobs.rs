use std::time::Duration;

use anyhow::anyhow;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::embedding::{EmbeddingProviderError, EmbeddingRequestItem};
#[path = "embedding_commit.rs"]
mod commit;
#[path = "embedding_dispatch.rs"]
mod dispatch;
#[path = "embedding_source.rs"]
mod source;
#[path = "embedding_snapshot_maintenance.rs"]
mod snapshot_maintenance;

const CONTROL_MIGRATION_ID: &str = "016_memory_embedding_controls_v1";
const CONTROL_MIGRATION_SQL: &str =
    include_str!("../../../../migrations/016_memory_embedding_controls.sql");
const CIRCUIT_MIGRATION_ID: &str = "017_memory_embedding_circuit_v1";
const CIRCUIT_MIGRATION_SQL: &str =
    include_str!("../../../../migrations/017_memory_embedding_circuit.sql");

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct EmbeddingJob {
    job_id: String,
    retrieval_id: i64,
    principal_id: String,
    scope_kind: String,
    scope_ref: String,
    profile_id: String,
    profile_generation: u64,
    request_item_id: String,
    projection_version: String,
    projection_digest: String,
    consent_policy_digest: String,
    attempt: u64,
}

pub(crate) fn spawn_embedding_workers(state: crate::AppState, concurrency: usize) {
    for worker_index in 0..concurrency.max(1) {
        let state = state.clone();
        tokio::spawn(async move {
            let worker_id = format!(
                "memory_embedding_worker:{worker_index}:{}",
                state.worker.worker_id
            );
            loop {
                match run_one_embedding_batch(&state, &worker_id).await {
                    Ok(true) => {}
                    Ok(false) => tokio::time::sleep(Duration::from_millis(500)).await,
                    Err(error) => {
                        tracing::warn!(worker_id, error = %error, "memory_embedding_worker_tick_failed");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        });
    }
}

pub(crate) fn initialize_embedding_runtime(
    db: &Connection,
    config: &claw_core::config::MemoryConfig,
) -> anyhow::Result<super::vector_store::MemoryEmbeddingProfile> {
    super::jobs::ensure_memory_job_schema(db)?;
    let profile = super::vector_store::register_configured_profile(db, config)?;
    ensure_embedding_control_schema(db)?;
    Ok(profile)
}

pub(crate) async fn run_one_embedding_batch(
    state: &crate::AppState,
    worker_id: &str,
) -> anyhow::Result<bool> {
    if commit::recover_one_response(state, worker_id)? {
        return Ok(true);
    }
    let jobs = claim_embedding_batch(state, worker_id)?;
    if jobs.is_empty() {
        return Ok(false);
    }
    let execution = execute_embedding_batch(state, worker_id, &jobs);
    tokio::pin!(execution);
    let heartbeat_seconds = (state.policy.memory.background_lease_seconds.max(15) / 3).clamp(5, 30);
    let result = loop {
        tokio::select! {
            result = &mut execution => break result,
            _ = tokio::time::sleep(Duration::from_secs(heartbeat_seconds)) => {
                if renew_embedding_leases(state, worker_id, &jobs).is_err() {
                    break Err(dispatch::Failure::AfterDispatch(anyhow!(
                        "memory_embedding_heartbeat_lease_lost"
                    )));
                }
            }
        }
    };
    match result {
        Ok(completed) => {
            let db = state.core.db.get()?;
            reconcile_snapshot_generation(
                &db,
                &jobs[0].principal_id,
                &jobs[0].profile_id,
                jobs[0].profile_generation,
            )?;
            if completed > 0 {
                reset_provider_circuit(&db, &jobs[0].principal_id, &jobs[0].profile_id)?;
            }
        }
        Err(dispatch::Failure::Provider(error, failed)) => {
            fail_jobs(state, worker_id, &failed, &error, true)?
        }
        Err(dispatch::Failure::BeforeDispatch(error)) => {
            fail_jobs(state, worker_id, &jobs, &error, false)?
        }
        // The response or dispatch marker remains durable. Recovery never sends
        // the request again just because local publication/acknowledgement failed.
        Err(dispatch::Failure::AfterDispatch(error)) => return Err(error),
    }
    tokio::time::sleep(Duration::from_millis(
        state
            .policy
            .memory
            .embedding_reindex_batch_delay_ms
            .min(60_000),
    ))
    .await;
    Ok(true)
}

fn renew_embedding_leases(
    state: &crate::AppState,
    worker_id: &str,
    jobs: &[EmbeddingJob],
) -> anyhow::Result<()> {
    let db = state.core.db.get()?;
    let now = crate::now_ts_u64() as i64;
    let lease_expires =
        now.saturating_add(state.policy.memory.background_lease_seconds.max(15) as i64);
    for job in jobs {
        db.execute(
            "UPDATE memory_embedding_jobs
             SET lease_expires_at_ts = ?1, updated_at_ts = ?2
             WHERE job_id = ?3 AND status = 'running' AND lease_owner = ?4
               AND cancel_requested = 0 AND lease_expires_at_ts > ?2",
            params![lease_expires, now, job.job_id, worker_id],
        )?;
        // A cancelled/lost item does not abort valid peers already in flight.
    }
    Ok(())
}

fn claim_embedding_batch(
    state: &crate::AppState,
    worker_id: &str,
) -> anyhow::Result<Vec<EmbeddingJob>> {
    let mut db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow!("memory_embedding_db_pool:{error}"))?;
    let now = crate::now_ts_u64() as i64;
    // Schema/profile setup is a startup invariant. Keep the frequent idle poll
    // read-only so multiple embedding workers do not continuously contend for
    // SQLite's single writer lock when there is no work to claim or maintain.
    if !embedding_batch_needs_write(&db, now)? {
        return Ok(Vec::new());
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE memory_embedding_jobs
         SET status='failed',lease_owner=NULL,lease_expires_at_ts=NULL,
             error_code='memory_embedding_response_outcome_unknown',retryable=0,
             updated_at_ts=?1,finished_at_ts=?1
         WHERE status='running' AND lease_expires_at_ts<=?1 AND cancel_requested=0
           AND json_extract(checkpoint_json,'$.phase')='dispatch_started'",
        [now],
    )?;
    tx.execute(
        "UPDATE memory_embedding_jobs
         SET status = 'retry_wait', lease_owner = NULL, lease_expires_at_ts = NULL,
             not_before_ts = ?1, error_code = 'lease_expired', retryable = 1,
             updated_at_ts = ?1
         WHERE status = 'running' AND lease_expires_at_ts <= ?1 AND cancel_requested = 0",
        [now],
    )?;
    tx.execute(
        "UPDATE memory_embedding_jobs
         SET status = 'cancelled', lease_owner = NULL, lease_expires_at_ts = NULL,
             error_code = 'cancel_requested', retryable = 0,
             updated_at_ts = ?1, finished_at_ts = ?1
         WHERE cancel_requested = 1
           AND (status IN ('queued', 'retry_wait')
                OR (status = 'running' AND lease_expires_at_ts <= ?1))",
        [now],
    )?;
    snapshot_maintenance::reconcile(&tx, now)?;
    let job_ids = admitted_embedding_job_ids(
        &tx,
        now,
        state.policy.memory.embedding_batch_size.clamp(1, 256),
    )?;
    let lease_expires = now + state.policy.memory.background_lease_seconds.max(15) as i64;
    for job_id in &job_ids {
        tx.execute(
            "UPDATE memory_embedding_jobs
             SET status = 'running', lease_owner = ?1, lease_expires_at_ts = ?2,
                 attempt = attempt + 1, error_code = NULL, retryable = 0,
                 updated_at_ts = ?3
             WHERE job_id = ?4 AND status IN ('queued', 'retry_wait')
               AND cancel_requested = 0",
            params![worker_id, lease_expires, now, job_id],
        )?;
    }
    let jobs = job_ids
        .iter()
        .filter_map(|job_id| load_job(&tx, job_id).transpose())
        .collect::<anyhow::Result<Vec<_>>>()?;
    tx.commit()?;
    Ok(jobs)
}

fn embedding_batch_needs_write(db: &Connection, now: i64) -> anyhow::Result<bool> {
    let maintenance: bool = db.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM memory_embedding_jobs job
             WHERE (job.status = 'running'
                    AND job.lease_expires_at_ts <= ?1)
                OR (job.cancel_requested = 1
                    AND job.status IN ('queued', 'retry_wait'))
         )",
        [now],
        |row| row.get(0),
    )?;
    Ok(maintenance
        || snapshot_maintenance::needed(db)?
        || !admitted_embedding_job_ids(db, now, 1)?.is_empty())
}

fn admitted_embedding_job_ids(
    db: &Connection,
    now: i64,
    limit: usize,
) -> anyhow::Result<Vec<String>> {
    use super::capacity::{GenerationAvailability, StoragePressure};
    let pressure = StoragePressure::load(db)?;
    if pressure.generation_availability() != GenerationAvailability::Available {
        return Ok(Vec::new());
    }
    let mut statement = db.prepare(
        "SELECT job.job_id, job.principal_id, job.profile_id,
                job.consent_policy_digest, job.profile_generation,
                COALESCE((
                    SELECT MAX(snapshot.generation) FROM memory_vector_snapshots snapshot
                    WHERE snapshot.principal_id = job.principal_id
                      AND snapshot.profile_id = job.profile_id AND snapshot.state = 'active'
                ), profile.generation)
         FROM memory_embedding_jobs job
         LEFT JOIN memory_embedding_profiles profile ON profile.profile_id = job.profile_id
         WHERE job.status IN ('queued', 'retry_wait') AND job.cancel_requested = 0
           AND job.not_before_ts <= ?1
           AND COALESCE(json_extract(job.checkpoint_json,'$.phase'),'') != 'response_received'
           AND NOT EXISTS (
                SELECT 1 FROM memory_embedding_circuits circuit
                WHERE circuit.principal_id = job.principal_id
                  AND circuit.profile_id = job.profile_id AND circuit.open_until_ts > ?1
           )
           AND NOT EXISTS (
                SELECT 1 FROM memory_embedding_controls control
                WHERE control.principal_id = job.principal_id
                  AND control.profile_id = job.profile_id AND control.state = 'paused'
           )
         ORDER BY job.not_before_ts, job.created_at_ts, job.job_id",
    )?;
    let candidates = statement.query_map([now], |row| {
        Ok((
            row.get::<_, String>(0)?,
            (
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ),
            row.get::<_, Option<i64>>(5)?,
        ))
    })?;
    let mut partition = None;
    let mut selected = Vec::new();
    for candidate in candidates {
        let (job_id, key, active_generation) = candidate?;
        if pressure == StoragePressure::BackfillPaused && active_generation != Some(key.3) {
            continue;
        }
        // A batch has one exact generation as well as one owner/profile/
        // consent. Blocked reindex work cannot hide active incremental work.
        if let Some(ref expected) = partition {
            if expected != &key {
                continue;
            }
        } else {
            partition = Some(key);
        }
        selected.push(job_id);
        if selected.len() >= limit.max(1) {
            break;
        }
    }
    Ok(selected)
}

async fn execute_embedding_batch(
    state: &crate::AppState,
    worker_id: &str,
    jobs: &[EmbeddingJob],
) -> Result<usize, dispatch::Failure> {
    let first = jobs
        .first()
        .ok_or_else(|| embedding_error("memory_embedding_batch_empty", false, None, None))?;
    let db = state
        .core
        .db
        .get()
        .map_err(|_| embedding_error("memory_embedding_db_pool", true, None, None))?;
    let mut profile = super::vector_store::load_profile(&db, &first.profile_id)
        .map_err(|_| embedding_error("memory_embedding_profile_load_failed", true, None, None))?
        .ok_or_else(|| embedding_error("memory_embedding_profile_missing", false, None, None))?;
    profile.generation = first.profile_generation;
    let mut requests = Vec::with_capacity(jobs.len());
    for job in jobs {
        if job.profile_id != first.profile_id
            || job.profile_generation != first.profile_generation
            || job.principal_id != first.principal_id
            || job.consent_policy_digest != first.consent_policy_digest
        {
            return Err(embedding_error(
                "memory_embedding_batch_partition_mismatch",
                false,
                None,
                None,
            )
            .into());
        }
        let row: Option<(String, Option<String>, String, Option<String>)> = db
            .query_row(
                "SELECT search_text, principal_id, COALESCE(scope_kind, 'principal'), scope_ref
                 FROM memory_retrieval_index WHERE id = ?1",
                [job.retrieval_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|_| {
                embedding_error("memory_embedding_source_load_failed", true, None, None)
            })?;
        let Some((text, principal_id, scope_kind, scope_ref)) = row else {
            return Err(
                embedding_error("memory_embedding_source_missing", false, None, None).into(),
            );
        };
        if principal_id.as_deref() != Some(job.principal_id.as_str())
            || scope_kind != job.scope_kind
            || scope_ref.as_deref() != Some(job.scope_ref.as_str())
            || super::vector_store::searchable_projection_digest(&text) != job.projection_digest
            || job.projection_version != super::vector_store::PROJECTION_VERSION
        {
            return Err(embedding_error(
                "memory_embedding_source_snapshot_mismatch",
                false,
                None,
                None,
            )
            .into());
        }
        if profile.provider_kind == "remote_http" {
            let (safe, redacted) =
                crate::skill_output_artifact::sensitivity_aware_text_model_view(&text);
            if redacted {
                return Err(embedding_error(
                    "memory_embedding_sensitive_input_blocked",
                    false,
                    None,
                    None,
                )
                .into());
            }
            requests.push(EmbeddingRequestItem {
                request_item_id: job.request_item_id.clone(),
                text: safe,
                principal_id: job.principal_id.clone(),
                purpose: claw_core::retrieval_embedding::EmbeddingPurpose::Document,
            });
        } else {
            requests.push(EmbeddingRequestItem {
                request_item_id: job.request_item_id.clone(),
                text,
                principal_id: job.principal_id.clone(),
                purpose: claw_core::retrieval_embedding::EmbeddingPurpose::Document,
            });
        }
    }
    drop(db);
    let provider = super::embedding::provider_for_profile(&profile, &state.policy.memory)?;
    let provider_spec = provider.spec();
    if provider_spec.model_id != profile.model_name
        || provider_spec.dims != profile.dimensions
        || provider_spec.version != profile.profile_version
        || provider_spec.normalization != profile.normalization
    {
        return Err(embedding_error(
            "memory_embedding_provider_profile_mismatch",
            false,
            None,
            None,
        )
        .into());
    }
    dispatch::execute(
        state,
        worker_id,
        &profile,
        provider.as_ref(),
        jobs,
        &requests,
    )
    .await
}

#[cfg(test)]
fn embed_batch_with_payload_split<'a>(
    provider: &'a dyn super::embedding::MemoryEmbeddingProvider,
    requests: &'a [EmbeddingRequestItem],
    max_request_bytes: usize,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<
                    Vec<super::embedding::EmbeddingResponseItem>,
                    EmbeddingProviderError,
                >,
            > + Send
            + 'a,
    >,
> {
    Box::pin(async move {
        let estimated_request_bytes = requests
            .iter()
            .map(|item| item.request_item_id.len().saturating_add(item.text.len()))
            .sum::<usize>();
        if estimated_request_bytes > max_request_bytes && requests.len() > 1 {
            let midpoint = requests.len() / 2;
            let mut left =
                embed_batch_with_payload_split(provider, &requests[..midpoint], max_request_bytes)
                    .await?;
            left.extend(
                embed_batch_with_payload_split(provider, &requests[midpoint..], max_request_bytes)
                    .await?,
            );
            return Ok(left);
        }
        match provider.embed_batch(requests).await {
            Err(error) if error.status_code == Some(413) && requests.len() > 1 => {
                let midpoint = requests.len() / 2;
                let mut left = embed_batch_with_payload_split(
                    provider,
                    &requests[..midpoint],
                    max_request_bytes,
                )
                .await?;
                left.extend(
                    embed_batch_with_payload_split(
                        provider,
                        &requests[midpoint..],
                        max_request_bytes,
                    )
                    .await?,
                );
                Ok(left)
            }
            result => result,
        }
    })
}

fn fail_jobs(
    state: &crate::AppState,
    worker_id: &str,
    jobs: &[EmbeddingJob],
    error: &EmbeddingProviderError,
    provider_failure: bool,
) -> anyhow::Result<()> {
    let db = state.core.db.get()?;
    let now = crate::now_ts_u64() as i64;
    for job in jobs {
        let retryable = error.retryable
            && job.attempt < state.policy.memory.embedding_retry_max_attempts.max(1) as u64;
        let delay = error.retry_after_seconds.unwrap_or_else(|| {
            2_u64
                .saturating_pow(job.attempt.clamp(1, 8) as u32)
                .min(300)
        });
        db.execute(
            "UPDATE memory_embedding_jobs
             SET status = CASE WHEN cancel_requested = 1 THEN 'cancelled'
                               WHEN ?1 != 0 THEN 'retry_wait' ELSE 'failed' END,
                 lease_owner = NULL, lease_expires_at_ts = NULL,
                 not_before_ts = ?2, error_code = ?3, retryable = ?1,
                 checkpoint_json = json_set(checkpoint_json,'$.phase','provider_failed'),
                 updated_at_ts = ?4,
                 finished_at_ts = CASE WHEN cancel_requested = 1 OR ?1 = 0 THEN ?4 ELSE NULL END
             WHERE job_id = ?5 AND status = 'running' AND lease_owner = ?6
               AND COALESCE(json_extract(checkpoint_json,'$.phase'),'') != 'response_received'",
            params![
                if retryable { 1 } else { 0 },
                now.saturating_add(delay as i64),
                error.error_code,
                now,
                job.job_id,
                worker_id,
            ],
        )?;
    }
    reconcile_snapshot_generation(
        &db,
        &jobs[0].principal_id,
        &jobs[0].profile_id,
        jobs[0].profile_generation,
    )?;
    if provider_failure {
        record_provider_failure(
            &db,
            &jobs[0].principal_id,
            &jobs[0].profile_id,
            error,
            &state.policy.memory,
        )?;
    }
    Ok(())
}

pub(crate) fn provider_circuit_open(
    db: &Connection,
    principal_id: &str,
    profile_id: &str,
    now: i64,
) -> anyhow::Result<bool> {
    ensure_embedding_control_schema(db)?;
    Ok(db
        .query_row(
            "SELECT COALESCE(open_until_ts, 0) > ?3 FROM memory_embedding_circuits
             WHERE principal_id = ?1 AND profile_id = ?2",
            params![principal_id, profile_id, now],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false))
}

pub(crate) fn record_provider_failure(
    db: &Connection,
    principal_id: &str,
    profile_id: &str,
    error: &EmbeddingProviderError,
    config: &claw_core::config::MemoryConfig,
) -> anyhow::Result<()> {
    if !error.retryable {
        return Ok(());
    }
    ensure_embedding_control_schema(db)?;
    let now = crate::now_ts_u64() as i64;
    let current_failures = db
        .query_row(
            "SELECT failure_count FROM memory_embedding_circuits
             WHERE principal_id = ?1 AND profile_id = ?2",
            params![principal_id, profile_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or_default();
    let failure_count = current_failures.saturating_add(1);
    let threshold = config.embedding_circuit_failure_threshold.max(1) as i64;
    let open_until = if failure_count >= threshold {
        let reset = config.embedding_circuit_reset_seconds.max(1);
        let retry_after = error.retry_after_seconds.unwrap_or_default();
        Some(now.saturating_add(reset.max(retry_after) as i64))
    } else {
        None
    };
    db.execute(
        "INSERT INTO memory_embedding_circuits(
            principal_id, profile_id, failure_count, open_until_ts,
            last_error_code, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(principal_id, profile_id) DO UPDATE SET
            failure_count = excluded.failure_count,
            open_until_ts = excluded.open_until_ts,
            last_error_code = excluded.last_error_code,
            updated_at_ts = excluded.updated_at_ts",
        params![
            principal_id,
            profile_id,
            failure_count,
            open_until,
            error.error_code,
            now,
        ],
    )?;
    Ok(())
}

pub(crate) fn reset_provider_circuit(
    db: &Connection,
    principal_id: &str,
    profile_id: &str,
) -> anyhow::Result<()> {
    ensure_embedding_control_schema(db)?;
    db.execute(
        "DELETE FROM memory_embedding_circuits
         WHERE principal_id = ?1 AND profile_id = ?2",
        params![principal_id, profile_id],
    )?;
    Ok(())
}

pub(crate) fn enqueue_reindex(
    db: &Connection,
    cfg: &claw_core::config::MemoryConfig,
    principal_id: &str,
    consent_policy_digest: &str,
    remote_allowed: bool,
) -> anyhow::Result<(String, u64, usize)> {
    let profile = super::vector_store::register_configured_profile(db, cfg)?;
    if profile.provider_kind == "remote_http" {
        anyhow::ensure!(remote_allowed, "memory_embedding_remote_consent_required");
    }
    let existing_build: Option<i64> = db
        .query_row(
            "SELECT generation FROM memory_vector_snapshots
             WHERE principal_id = ?1 AND profile_id = ?2 AND state = 'building'
             ORDER BY generation DESC LIMIT 1",
            params![principal_id, profile.profile_id],
            |row| row.get(0),
        )
        .optional()?;
    anyhow::ensure!(
        existing_build.is_none(),
        "memory_embedding_reindex_already_running"
    );
    let latest_generation: i64 = db.query_row(
        "SELECT COALESCE(MAX(generation), ?3) FROM memory_vector_snapshots
         WHERE principal_id = ?1 AND profile_id = ?2",
        params![principal_id, profile.profile_id, profile.generation as i64],
        |row| row.get(0),
    )?;
    let target_generation = (latest_generation.max(0) as u64).saturating_add(1);
    let snapshot_id = format!("memory_vector_snapshot_{}", uuid::Uuid::new_v4().simple());
    let now = crate::now_ts_u64() as i64;
    let tx = db.unchecked_transaction()?;
    let source_rows = {
        let mut stmt = tx.prepare(
            "SELECT id, COALESCE(scope_kind, 'principal'), COALESCE(scope_ref, principal_id), search_text
             FROM memory_retrieval_index
             WHERE principal_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([principal_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let source_digest = digest_rows(&source_rows);
    tx.execute(
        "INSERT INTO memory_vector_snapshots(
            snapshot_id, principal_id, profile_id, generation, row_count,
            source_digest, snapshot_checksum, state, created_at_ts, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', 'building', ?7, ?7)",
        params![
            snapshot_id,
            principal_id,
            profile.profile_id,
            target_generation as i64,
            source_rows.len() as i64,
            source_digest,
            now,
        ],
    )?;
    for (retrieval_id, scope_kind, scope_ref, text) in &source_rows {
        let projection_digest = super::vector_store::searchable_projection_digest(text);
        tx.execute(
            "INSERT INTO memory_embedding_jobs(
                job_id, retrieval_id, principal_id, scope_kind, scope_ref, profile_id,
                profile_generation, request_item_id, projection_version, projection_digest,
                consent_policy_digest, status, not_before_ts, checkpoint_json,
                created_at_ts, updated_at_ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                       'queued', ?12, ?13, ?12, ?12)
             ON CONFLICT(retrieval_id, profile_id, profile_generation, projection_digest) DO NOTHING",
            params![
                format!("memory_embedding_job_{}", uuid::Uuid::new_v4().simple()),
                retrieval_id,
                principal_id,
                scope_kind,
                scope_ref,
                profile.profile_id,
                target_generation as i64,
                format!("memory_embedding_item:{retrieval_id}:{projection_digest}"),
                super::vector_store::PROJECTION_VERSION,
                projection_digest,
                consent_policy_digest,
                now,
                json!({"schema_version":1,"snapshot_id":snapshot_id}).to_string(),
            ],
        )?;
    }
    tx.commit()?;
    if source_rows.is_empty() {
        reconcile_snapshot_generation(db, principal_id, &profile.profile_id, target_generation)?;
    }
    Ok((snapshot_id, target_generation, source_rows.len()))
}

pub(crate) fn cancel_profile_jobs(
    db: &Connection,
    principal_id: &str,
    profile_id: &str,
) -> anyhow::Result<usize> {
    let now = crate::now_ts_u64() as i64;
    let tx = db.unchecked_transaction()?;
    let changed = tx.execute(
        "UPDATE memory_embedding_jobs SET cancel_requested = 1, updated_at_ts = ?1
         WHERE principal_id = ?2 AND profile_id = ?3
           AND status IN ('queued', 'retry_wait', 'running')
           AND profile_generation IN (
             SELECT generation FROM memory_vector_snapshots
             WHERE principal_id = ?2 AND profile_id = ?3 AND state = 'building'
           )",
        params![now, principal_id, profile_id],
    )?;
    tx.execute(
        "UPDATE memory_vector_rows SET status = 'tombstone', updated_at_ts = ?1
         WHERE principal_id = ?2 AND profile_id = ?3 AND status = 'active'
           AND generation IN (
             SELECT generation FROM memory_vector_snapshots
             WHERE principal_id = ?2 AND profile_id = ?3 AND state = 'building'
           )",
        params![now, principal_id, profile_id],
    )?;
    tx.execute(
        "UPDATE memory_vector_snapshots SET state = 'corrupt', updated_at_ts = ?1
         WHERE principal_id = ?2 AND profile_id = ?3 AND state = 'building'",
        params![now, principal_id, profile_id],
    )?;
    tx.commit()?;
    Ok(changed)
}

pub(crate) fn set_profile_paused(
    db: &Connection,
    principal_id: &str,
    profile_id: &str,
    paused: bool,
) -> anyhow::Result<()> {
    ensure_embedding_control_schema(db)?;
    db.execute(
        "INSERT INTO memory_embedding_controls(principal_id, profile_id, state, updated_at_ts)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(principal_id, profile_id) DO UPDATE SET
           state = excluded.state, updated_at_ts = excluded.updated_at_ts",
        params![
            principal_id,
            profile_id,
            if paused { "paused" } else { "active" },
            crate::now_ts_u64() as i64,
        ],
    )?;
    Ok(())
}

pub(crate) fn profile_paused(
    db: &Connection,
    principal_id: &str,
    profile_id: &str,
) -> anyhow::Result<bool> {
    // Control schema setup is a startup invariant. Status reads must not run
    // migrations because UI polling may overlap background writers.
    Ok(db
        .query_row(
            "SELECT state FROM memory_embedding_controls
             WHERE principal_id = ?1 AND profile_id = ?2",
            params![principal_id, profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some_and(|state| state == "paused"))
}

pub(crate) fn revoke_remote_profiles_for_principal(
    db: &Connection,
    principal_id: &str,
) -> anyhow::Result<(usize, usize)> {
    super::vector_store::ensure_vector_pipeline_schema(db)?;
    let now = crate::now_ts_u64() as i64;
    let cancelled = db.execute(
        "UPDATE memory_embedding_jobs SET cancel_requested = 1, updated_at_ts = ?1
         WHERE principal_id = ?2 AND status IN ('queued', 'retry_wait', 'running')
           AND profile_id IN (
             SELECT profile_id FROM memory_embedding_profiles WHERE provider_kind = 'remote_http'
           )",
        params![now, principal_id],
    )?;
    let tombstoned = db.execute(
        "UPDATE memory_vector_rows SET status = 'tombstone', updated_at_ts = ?1
         WHERE principal_id = ?2 AND status = 'active'
           AND profile_id IN (
             SELECT profile_id FROM memory_embedding_profiles WHERE provider_kind = 'remote_http'
           )",
        params![now, principal_id],
    )?;
    Ok((cancelled, tombstoned))
}

fn reconcile_snapshot_generation(
    db: &Connection,
    principal_id: &str,
    profile_id: &str,
    generation: u64,
) -> anyhow::Result<()> {
    let terminal_failures: i64 = db.query_row(
        "SELECT COUNT(*) FROM memory_embedding_jobs
         WHERE principal_id = ?1 AND profile_id = ?2 AND profile_generation = ?3
           AND status IN ('failed', 'cancelled')",
        params![principal_id, profile_id, generation as i64],
        |row| row.get(0),
    )?;
    if terminal_failures != 0 {
        db.execute(
            "UPDATE memory_vector_snapshots SET state = 'corrupt', updated_at_ts = ?1
             WHERE principal_id = ?2 AND profile_id = ?3 AND generation = ?4
               AND state = 'building'",
            params![
                crate::now_ts_u64() as i64,
                principal_id,
                profile_id,
                generation as i64,
            ],
        )?;
        return Ok(());
    }
    let remaining: i64 = db.query_row(
        "SELECT COUNT(*) FROM memory_embedding_jobs
         WHERE principal_id = ?1 AND profile_id = ?2 AND profile_generation = ?3
           AND status NOT IN ('completed', 'cancelled')",
        params![principal_id, profile_id, generation as i64],
        |row| row.get(0),
    )?;
    if remaining != 0 {
        return Ok(());
    }
    let snapshot: Option<(String, i64, String)> = db
        .query_row(
            "SELECT snapshot_id, row_count, source_digest
             FROM memory_vector_snapshots
             WHERE principal_id = ?1 AND profile_id = ?2 AND generation = ?3
               AND state = 'building' LIMIT 1",
            params![principal_id, profile_id, generation as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((snapshot_id, expected_count, source_digest)) = snapshot else {
        return Ok(());
    };
    let actual_count: i64 = db.query_row(
        "SELECT COUNT(*) FROM memory_vector_rows
         WHERE principal_id = ?1 AND profile_id = ?2 AND generation = ?3
           AND status = 'active'",
        params![principal_id, profile_id, generation as i64],
        |row| row.get(0),
    )?;
    if actual_count != expected_count {
        return Ok(());
    }
    let checksum = format!(
        "sha256:{:x}",
        Sha256::digest(format!("{source_digest}\0{actual_count}\0{generation}").as_bytes())
    );
    let now = crate::now_ts_u64() as i64;
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "UPDATE memory_vector_snapshots
         SET state = 'active', snapshot_checksum = ?1, updated_at_ts = ?2
         WHERE snapshot_id = ?3 AND state = 'building'",
        params![checksum, now, snapshot_id],
    )?;
    tx.execute(
        "UPDATE memory_vector_snapshots SET state = 'retired', updated_at_ts = ?1
         WHERE principal_id = ?2 AND profile_id = ?3 AND generation < ?4
           AND state = 'active'",
        params![now, principal_id, profile_id, generation as i64],
    )?;
    tx.commit()?;
    Ok(())
}

fn load_job(db: &Connection, job_id: &str) -> anyhow::Result<Option<EmbeddingJob>> {
    db.query_row(
        "SELECT job_id, retrieval_id, principal_id, scope_kind, scope_ref,
                profile_id, profile_generation, request_item_id, projection_version,
                projection_digest, consent_policy_digest, attempt
         FROM memory_embedding_jobs WHERE job_id = ?1 AND status = 'running'",
        [job_id],
        |row| {
            Ok(EmbeddingJob {
                job_id: row.get(0)?,
                retrieval_id: row.get(1)?,
                principal_id: row.get(2)?,
                scope_kind: row.get(3)?,
                scope_ref: row.get(4)?,
                profile_id: row.get(5)?,
                profile_generation: row.get::<_, i64>(6)?.max(0) as u64,
                request_item_id: row.get(7)?,
                projection_version: row.get(8)?,
                projection_digest: row.get(9)?,
                consent_policy_digest: row.get(10)?,
                attempt: row.get::<_, i64>(11)?.max(0) as u64,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn embedding_error(
    error_code: &'static str,
    retryable: bool,
    retry_after_seconds: Option<u64>,
    status_code: Option<u16>,
) -> EmbeddingProviderError {
    EmbeddingProviderError {
        error_code,
        retryable,
        retry_after_seconds,
        status_code,
    }
}

fn digest_rows(rows: &[(i64, String, String, String)]) -> String {
    let bytes = serde_json::to_vec(rows).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn ensure_embedding_control_schema(db: &Connection) -> anyhow::Result<()> {
    super::vector_store::ensure_vector_pipeline_schema(db)?;
    let digest = format!(
        "sha256:{:x}",
        Sha256::digest(CONTROL_MIGRATION_SQL.as_bytes())
    );
    if let Some(applied) = db
        .query_row(
            "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id = ?1",
            [CONTROL_MIGRATION_ID],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        anyhow::ensure!(
            applied == digest,
            "memory_embedding_control_digest_mismatch"
        );
    }
    db.execute_batch(CONTROL_MIGRATION_SQL)?;
    db.execute(
        "INSERT INTO runtime_schema_migrations(migration_id, schema_digest, applied_at)
         VALUES (?1, ?2, ?3) ON CONFLICT(migration_id) DO NOTHING",
        params![CONTROL_MIGRATION_ID, digest, crate::now_ts()],
    )?;
    let circuit_digest = format!(
        "sha256:{:x}",
        Sha256::digest(CIRCUIT_MIGRATION_SQL.as_bytes())
    );
    if let Some(applied) = db
        .query_row(
            "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id = ?1",
            [CIRCUIT_MIGRATION_ID],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        anyhow::ensure!(
            applied == circuit_digest,
            "memory_embedding_circuit_digest_mismatch"
        );
    }
    db.execute_batch(CIRCUIT_MIGRATION_SQL)?;
    db.execute(
        "INSERT INTO runtime_schema_migrations(migration_id, schema_digest, applied_at)
         VALUES (?1, ?2, ?3) ON CONFLICT(migration_id) DO NOTHING",
        params![CIRCUIT_MIGRATION_ID, circuit_digest, crate::now_ts()],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "embedding_jobs_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "embedding_capacity_tests.rs"]
mod capacity_tests;

#[cfg(test)]
#[path = "embedding_capacity_maintenance_tests.rs"]
mod capacity_maintenance_tests;

#[cfg(test)]
#[path = "embedding_capacity_http_tests.rs"]
mod capacity_http_tests;

#[cfg(test)]
#[path = "embedding_capacity_race_tests.rs"]
mod capacity_race_tests;
