use anyhow::{Context, Result};
use rusqlite::{params, Connection, TransactionBehavior};

use super::database::{ensure_schema, open};
use super::namespace_index::persist_namespace;
use super::vector_index::replace_ann_projection;
use super::{
    load_ingest_job_from, upsert_ingest_job, validate_job_owner, validate_owner,
    CheckpointConflict, IngestJob, KbRuntime, NamespaceIndex, SaveOutcome,
};

pub(crate) fn save_namespace_and_job(
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

pub(super) fn rebuild_ann_projection(
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
