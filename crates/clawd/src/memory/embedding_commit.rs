use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{load_job, source, EmbeddingJob};
use crate::memory::vector_store::{MemoryEmbeddingProfile, MemoryVectorIndex, VectorWrite};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    schema_version: u32,
    phase: String,
    job: EmbeddingJob,
    profile_digest: String,
    vector: Vec<f32>,
}

fn lease(
    db: &Connection,
    job: &EmbeddingJob,
    worker: &str,
    now: i64,
) -> anyhow::Result<Option<bool>> {
    if load_job(db, &job.job_id)?.as_ref() != Some(job) {
        return Ok(None);
    }
    Ok(db
        .query_row(
            "SELECT cancel_requested FROM memory_embedding_jobs WHERE job_id=?1
         AND status='running' AND lease_owner=?2 AND lease_expires_at_ts>?3",
            params![job.job_id, worker, now],
            |row| row.get(0),
        )
        .optional()?)
}

fn discard(db: &Connection, job: &EmbeddingJob, worker: &str, now: i64) -> anyhow::Result<()> {
    let changed = db.execute(
        "UPDATE memory_embedding_jobs SET status='cancelled',lease_owner=NULL,
         lease_expires_at_ts=NULL,retryable=0,error_code='memory_embedding_response_discarded',
         checkpoint_json=?1,updated_at_ts=?2,finished_at_ts=?2
         WHERE job_id=?3 AND status='running' AND lease_owner=?4",
        params![
            json!({"schema_version":1,"phase":"response_discarded"}).to_string(),
            now,
            job.job_id,
            worker
        ],
    )?;
    anyhow::ensure!(changed == 1, "memory_embedding_completion_lease_lost");
    Ok(())
}

pub(super) fn prepare_dispatch(
    state: &crate::AppState,
    worker: &str,
    profile: &MemoryEmbeddingProfile,
    job: &EmbeddingJob,
) -> anyhow::Result<bool> {
    let mut db = state.core.db.get()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = crate::now_ts_u64() as i64;
    let Some(cancelled) = lease(&tx, job, worker, now)? else {
        return Ok(false);
    };
    if cancelled || !source::current(&tx, job, profile, state.policy.memory.long_term_enabled)? {
        discard(&tx, job, worker, now)?;
        tx.commit()?;
        return Ok(false);
    }
    tx.execute(
        "UPDATE memory_embedding_jobs SET checkpoint_json=json_set(checkpoint_json,
         '$.schema_version',1,'$.phase','dispatch_started'),updated_at_ts=?1
         WHERE job_id=?2 AND status='running' AND lease_owner=?3",
        params![now, job.job_id, worker],
    )?;
    tx.commit()?;
    Ok(true)
}

/// Store the actual provider result before attempting its publication. No input,
/// key, endpoint or credential value is added to this private outbox checkpoint.
pub(super) fn receive(
    state: &crate::AppState,
    worker: &str,
    profile: &MemoryEmbeddingProfile,
    job: &EmbeddingJob,
    vector: &[f32],
) -> anyhow::Result<()> {
    crate::memory::vector_store::validate_vector(
        vector,
        profile.dimensions,
        &profile.normalization,
    )?;
    let response = Response {
        schema_version: 1,
        phase: "response_received".into(),
        job: job.clone(),
        profile_digest: profile.config_digest.clone(),
        vector: vector.to_vec(),
    };
    let mut db = state.core.db.get()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = crate::now_ts_u64() as i64;
    let Some(cancelled) = lease(&tx, job, worker, now)? else {
        return Ok(());
    };
    if cancelled {
        discard(&tx, job, worker, now)?;
    } else {
        tx.execute(
            "UPDATE memory_embedding_jobs SET checkpoint_json=?1,updated_at_ts=?2
             WHERE job_id=?3 AND status='running' AND lease_owner=?4",
            params![serde_json::to_string(&response)?, now, job.job_id, worker],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Vector and truthful acknowledgement are one transaction, fenced again after
/// response staging. Each item has its own boundary so one fault cannot roll
/// back successfully completed peers.
pub(super) fn publish(
    state: &crate::AppState,
    worker: &str,
    job: &EmbeddingJob,
) -> anyhow::Result<usize> {
    let mut db = state.core.db.get()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = crate::now_ts_u64() as i64;
    let Some(cancelled) = lease(&tx, job, worker, now)? else {
        return Ok(0);
    };
    if cancelled {
        discard(&tx, job, worker, now)?;
        tx.commit()?;
        return Ok(0);
    }
    let checkpoint: String = tx.query_row(
        "SELECT checkpoint_json FROM memory_embedding_jobs WHERE job_id=?1",
        [&job.job_id],
        |row| row.get(0),
    )?;
    let response: Response = serde_json::from_str(&checkpoint)?;
    anyhow::ensure!(
        response.schema_version == 1
            && response.phase == "response_received"
            && response.job == *job,
        "memory_embedding_response_checkpoint_mismatch"
    );
    let profile = crate::memory::vector_store::load_profile(&tx, &job.profile_id)?;
    let Some(mut profile) = profile else {
        discard(&tx, job, worker, now)?;
        tx.commit()?;
        return Ok(0);
    };
    profile.generation = job.profile_generation;
    if profile.config_digest != response.profile_digest
        || !source::current(&tx, job, &profile, state.policy.memory.long_term_enabled)?
    {
        discard(&tx, job, worker, now)?;
        tx.commit()?;
        return Ok(0);
    }
    crate::memory::vector_store::ExactSqliteVectorIndex.upsert(
        &tx,
        &profile,
        &VectorWrite {
            retrieval_id: job.retrieval_id,
            principal_id: &job.principal_id,
            scope_kind: &job.scope_kind,
            scope_ref: &job.scope_ref,
            projection_digest: &job.projection_digest,
            vector: &response.vector,
        },
    )?;
    let changed = tx.execute(
        "UPDATE memory_embedding_jobs SET status='completed',lease_owner=NULL,
         lease_expires_at_ts=NULL,retryable=0,error_code=NULL,checkpoint_json=?1,
         updated_at_ts=?2,finished_at_ts=?2
         WHERE job_id=?3 AND status='running' AND lease_owner=?4
           AND lease_expires_at_ts>?2 AND cancel_requested=0",
        params![
            json!({"schema_version":1,"phase":"vector_committed",
            "retrieval_id":job.retrieval_id,"profile_generation":job.profile_generation,
            "projection_digest":job.projection_digest})
            .to_string(),
            now,
            job.job_id,
            worker
        ],
    )?;
    anyhow::ensure!(changed == 1, "memory_embedding_completion_lease_lost");
    tx.commit()?;
    Ok(1)
}

/// Local-only recovery is independent of provider, user pause and resource
/// admission for new outbound work. It never spends another provider attempt.
pub(super) fn recover_one_response(state: &crate::AppState, worker: &str) -> anyhow::Result<bool> {
    let mut db = state.core.db.get()?;
    let now = crate::now_ts_u64() as i64;
    let candidate = |db: &Connection| -> anyhow::Result<Option<String>> {
        Ok(db
            .query_row(
                "SELECT job_id FROM memory_embedding_jobs
             WHERE json_extract(checkpoint_json,'$.phase')='response_received'
               AND not_before_ts<=?2
               AND ((status='running' AND (lease_owner=?1 OR lease_expires_at_ts<=?2))
                    OR (status='retry_wait' AND not_before_ts<=?2))
             ORDER BY updated_at_ts,job_id LIMIT 1",
                params![worker, now],
                |row| row.get(0),
            )
            .optional()?)
    };
    if candidate(&db)?.is_none() {
        return Ok(false);
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(id) = candidate(&tx)? else {
        return Ok(false);
    };
    tx.execute(
        "UPDATE memory_embedding_jobs SET status='running',lease_owner=?1,
         lease_expires_at_ts=?2,updated_at_ts=?3 WHERE job_id=?4",
        params![
            worker,
            now.saturating_add(state.policy.memory.background_lease_seconds.max(15) as i64),
            now,
            id
        ],
    )?;
    let job = load_job(&tx, &id)?
        .ok_or_else(|| anyhow::anyhow!("memory_embedding_recovery_job_missing"))?;
    tx.commit()?;
    drop(db);
    let completed = match publish(state, worker, &job) {
        Ok(completed) => completed,
        Err(error) => {
            // Preserve the received response, but yield this local recovery item
            // so an unavailable/corrupt store cannot starve another principal.
            let db = state.core.db.get()?;
            db.execute(
                "UPDATE memory_embedding_jobs SET status='retry_wait',lease_owner=NULL,
                 lease_expires_at_ts=NULL,not_before_ts=?1,retryable=1,
                 error_code='memory_embedding_publication_deferred',updated_at_ts=?2
                 WHERE job_id=?3 AND status='running' AND lease_owner=?4",
                params![now.saturating_add(5), now, job.job_id, worker],
            )?;
            return Err(error);
        }
    };
    let db = state.core.db.get()?;
    super::reconcile_snapshot_generation(
        &db,
        &job.principal_id,
        &job.profile_id,
        job.profile_generation,
    )?;
    if completed > 0 {
        super::reset_provider_circuit(&db, &job.principal_id, &job.profile_id)?;
    }
    Ok(true)
}
