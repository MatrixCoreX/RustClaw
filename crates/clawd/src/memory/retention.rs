use claw_core::config::MemoryConfig;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryRetentionReport {
    pub(crate) deleted_source_rows: usize,
    pub(crate) deleted_long_term_rows: usize,
    pub(crate) deleted_candidate_rows: usize,
    pub(crate) deleted_index_rows: usize,
    pub(crate) storage_pressure_state: String,
    pub(crate) observed_bytes: u64,
}

pub(crate) fn cleanup_memory_data(
    db: &Connection,
    config: &MemoryConfig,
    now_ts: i64,
) -> anyhow::Result<MemoryRetentionReport> {
    super::scope::ensure_memory_scope_schema(db)?;
    super::jobs::ensure_memory_job_schema(db)?;
    super::indexing::ensure_retrieval_schema(db)?;
    let transaction = db.unchecked_transaction()?;
    let source_cutoff = now_ts.saturating_sub(config.retention_days as i64 * 86_400);
    let long_term_cutoff = now_ts.saturating_sub(config.long_term_retention_days as i64 * 86_400);
    let candidate_cutoff =
        now_ts.saturating_sub(config.raw_candidate_retention_days as i64 * 86_400);

    let mut deleted_source_rows = transaction.execute(
        "DELETE FROM memories
         WHERE COALESCE(created_at_ts, CAST(created_at AS INTEGER)) < ?1",
        [source_cutoff],
    )?;
    deleted_source_rows += transaction.execute(
        "DELETE FROM memories WHERE id IN (
            SELECT id FROM (
                SELECT id, ROW_NUMBER() OVER (
                    PARTITION BY principal_id ORDER BY id DESC
                ) AS principal_rank
                FROM memories
            ) WHERE principal_rank > ?1
         )",
        [config.max_rows.max(1) as i64],
    )?;

    let mut deleted_long_term_rows = transaction.execute(
        "DELETE FROM long_term_memories
         WHERE COALESCE(updated_at_ts, CAST(updated_at AS INTEGER)) < ?1",
        [long_term_cutoff],
    )?;
    deleted_long_term_rows += transaction.execute(
        "DELETE FROM long_term_memories WHERE id IN (
            SELECT id FROM (
                SELECT id, ROW_NUMBER() OVER (
                    PARTITION BY principal_id ORDER BY id DESC
                ) AS principal_rank
                FROM long_term_memories
            ) WHERE principal_rank > ?1
         )",
        [config.long_term_max_rows.max(1) as i64],
    )?;

    let mut deleted_candidate_rows = transaction.execute(
        "DELETE FROM memory_raw_candidates
         WHERE status IN ('rejected', 'expired') OR created_at_ts < ?1",
        [candidate_cutoff],
    )?;
    deleted_candidate_rows += transaction.execute(
        "DELETE FROM memory_raw_candidates WHERE candidate_id IN (
            SELECT candidate_id FROM (
                SELECT candidate_id, ROW_NUMBER() OVER (
                    PARTITION BY principal_id ORDER BY created_at_ts DESC, candidate_id DESC
                ) AS principal_rank
                FROM memory_raw_candidates WHERE status = 'pending'
            ) WHERE principal_rank > ?1
         )",
        [config.raw_candidate_max_rows_per_principal.max(1) as i64],
    )?;

    transaction.execute(
        "UPDATE memory_evidence SET availability = 'source_unavailable', redacted_excerpt = NULL
         WHERE availability = 'available' AND source_type = 'memory'
           AND NOT EXISTS (
             SELECT 1 FROM memories m
             WHERE CAST(m.id AS TEXT) = memory_evidence.source_ref
           )",
        [],
    )?;
    let deleted_index_rows = transaction.execute(
        "DELETE FROM memory_retrieval_index
         WHERE (source_kind = 'memory' AND source_memory_id IS NOT NULL
                AND NOT EXISTS (SELECT 1 FROM memories m WHERE m.id = source_memory_id))
            OR (source_kind = 'preference' AND source_ref IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1 FROM user_preferences p
                    WHERE p.principal_id = memory_retrieval_index.principal_id
                      AND p.pref_key = memory_retrieval_index.source_ref
                ))
            OR (source_kind = 'memory_fact' AND source_ref IS NOT NULL
                AND NOT EXISTS (
                    SELECT 1 FROM memory_facts f
                    WHERE CAST(f.id AS TEXT) = memory_retrieval_index.source_ref
                       OR f.memory_id = memory_retrieval_index.source_ref
                ))",
        [],
    )?;
    let _ = transaction.execute(
        "DELETE FROM memory_retrieval_index_fts
         WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
        [],
    );

    refresh_principal_usage(&transaction, config, now_ts)?;
    let observed_bytes = sqlite_observed_bytes(&transaction)?;
    let state = pressure_state(observed_bytes, config.storage_soft_limit_bytes.max(1));
    transaction.execute(
        "UPDATE memory_storage_pressure
         SET state = ?1, reason_code = ?2, observed_bytes = ?3,
             revision = revision + 1, updated_at_ts = ?4
         WHERE singleton_id = 1",
        params![
            state,
            if state == "normal" {
                None::<&str>
            } else {
                Some("memory_storage_soft_limit")
            },
            observed_bytes.min(i64::MAX as u64) as i64,
            now_ts,
        ],
    )?;
    record_retention_ledger(
        &transaction,
        "source_transcript",
        deleted_source_rows,
        "retention_or_principal_row_quota",
        now_ts,
    )?;
    record_retention_ledger(
        &transaction,
        "long_term_summary",
        deleted_long_term_rows,
        "retention_or_principal_row_quota",
        now_ts,
    )?;
    record_retention_ledger(
        &transaction,
        "raw_candidate",
        deleted_candidate_rows,
        "candidate_age_or_principal_row_quota",
        now_ts,
    )?;
    transaction.commit()?;

    Ok(MemoryRetentionReport {
        deleted_source_rows,
        deleted_long_term_rows,
        deleted_candidate_rows,
        deleted_index_rows,
        storage_pressure_state: state.to_string(),
        observed_bytes,
    })
}

pub(crate) fn automatic_generation_allowed(
    db: &Connection,
    principal_id: &str,
) -> anyhow::Result<bool> {
    let state = db
        .query_row(
            "SELECT state FROM memory_storage_pressure WHERE singleton_id = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(|| "normal".to_string());
    if matches!(
        state.as_str(),
        "automatic_generation_paused" | "explicit_write_blocked"
    ) {
        return Ok(false);
    }
    let quota_available = db
        .query_row(
            "SELECT used_rows < max_rows AND used_bytes < max_bytes
                    AND used_background_cost_microunits < max_background_cost_microunits
             FROM memory_principal_quotas WHERE principal_id = ?1",
            [principal_id],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(true);
    Ok(quota_available)
}

pub(crate) fn ensure_principal_quota(
    db: &Connection,
    config: &MemoryConfig,
    principal_id: &str,
    now_ts: i64,
) -> anyhow::Result<()> {
    let (used_rows, used_bytes) = db.query_row(
        "SELECT COUNT(*), COALESCE(SUM(LENGTH(content)), 0)
         FROM memories WHERE principal_id = ?1",
        [principal_id],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    db.execute(
        "INSERT INTO memory_principal_quotas(
            principal_id, max_rows, max_bytes, max_background_cost_microunits,
            used_rows, used_bytes, used_background_cost_microunits, revision, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 1, ?7)
         ON CONFLICT(principal_id) DO UPDATE SET
            used_rows = excluded.used_rows, used_bytes = excluded.used_bytes,
            updated_at_ts = excluded.updated_at_ts",
        params![
            principal_id,
            config.max_rows.max(1) as i64,
            config.principal_max_bytes.min(i64::MAX as u64) as i64,
            config
                .principal_background_cost_microunits
                .min(i64::MAX as u64) as i64,
            used_rows,
            used_bytes,
            now_ts,
        ],
    )?;
    Ok(())
}

fn refresh_principal_usage(
    db: &Connection,
    config: &MemoryConfig,
    now_ts: i64,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO memory_principal_quotas(
            principal_id, max_rows, max_bytes, max_background_cost_microunits,
            used_rows, used_bytes, used_background_cost_microunits, revision, updated_at_ts
         )
         SELECT principal_id, ?1, ?2, ?3, COUNT(*), COALESCE(SUM(LENGTH(content)), 0), 0, 1, ?4
         FROM memories WHERE principal_id IS NOT NULL GROUP BY principal_id
         ON CONFLICT(principal_id) DO UPDATE SET
            used_rows = excluded.used_rows, used_bytes = excluded.used_bytes,
            updated_at_ts = excluded.updated_at_ts",
        params![
            config.max_rows.max(1) as i64,
            config.principal_max_bytes.min(i64::MAX as u64) as i64,
            config
                .principal_background_cost_microunits
                .min(i64::MAX as u64) as i64,
            now_ts,
        ],
    )?;
    Ok(())
}

fn sqlite_observed_bytes(db: &Connection) -> anyhow::Result<u64> {
    let page_count = db.query_row("PRAGMA page_count", [], |row| row.get::<_, u64>(0))?;
    let page_size = db.query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0))?;
    Ok(page_count.saturating_mul(page_size))
}

fn pressure_state(observed: u64, soft_limit: u64) -> &'static str {
    let percent = observed.saturating_mul(100) / soft_limit.max(1);
    match percent {
        0..=74 => "normal",
        75..=84 => "derived_cleanup",
        85..=94 => "backfill_paused",
        95..=99 => "automatic_generation_paused",
        _ => "explicit_write_blocked",
    }
}

fn record_retention_ledger(
    db: &Connection,
    object_kind: &str,
    count: usize,
    reason_code: &str,
    now_ts: i64,
) -> anyhow::Result<()> {
    if count == 0 {
        return Ok(());
    }
    let digest = format!(
        "sha256:{:x}",
        Sha256::digest(format!("{object_kind}:{count}:{reason_code}:{now_ts}").as_bytes())
    );
    db.execute(
        "INSERT INTO memory_retention_ledger(
            ledger_id, object_kind, object_count, object_digest, reason_code, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            format!("memory_retention_{}", uuid::Uuid::new_v4().simple()),
            object_kind,
            count as i64,
            digest,
            reason_code,
            now_ts,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod tests;
