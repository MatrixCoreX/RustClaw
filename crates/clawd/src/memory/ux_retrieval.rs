use rusqlite::ToSql;

use super::*;
use crate::memory::{
    retrieval_source_ref_for_memory, retrieval_source_ref_for_memory_fact,
    retrieval_source_ref_for_preference, MEMORY_FACT_SOURCE_REF_PREFIX, RETRIEVAL_SOURCE_MEMORY,
    RETRIEVAL_SOURCE_MEMORY_FACT, RETRIEVAL_SOURCE_PREFERENCE,
};

const ORPHAN_TARGETS: &str = "SELECT i.id FROM memory_retrieval_index i
     WHERE i.principal_id = ?1
       AND ((i.source_kind = 'memory' AND i.source_memory_id IS NOT NULL AND NOT EXISTS(
             SELECT 1 FROM memories m
             WHERE m.id = i.source_memory_id AND m.principal_id = i.principal_id))
         OR (i.source_kind = 'memory_fact' AND NOT EXISTS(
             SELECT 1 FROM memory_facts f
             WHERE f.id = CAST(SUBSTR(i.source_ref, LENGTH(?2) + 1) AS INTEGER)
               AND f.principal_id = i.principal_id AND ?2 || CAST(f.id AS TEXT) = i.source_ref)))";

pub(super) struct RetrievalIdentity {
    source_kind: &'static str,
    source_ref: String,
    pub(super) source_memory_id: Option<i64>,
    source_pref_key: Option<String>,
}

pub(super) fn identity(
    db: &Connection,
    memory: &MutableMemory,
) -> anyhow::Result<RetrievalIdentity> {
    let row_id = |table: &str| -> anyhow::Result<i64> {
        Ok(db.query_row(
            &format!("SELECT id FROM {table} WHERE memory_id = ?1 AND principal_id = ?2"),
            params![memory.id, memory.principal_id],
            |row| row.get(0),
        )?)
    };
    let (source_kind, source_ref, source_memory_id, source_pref_key) = match memory.kind.as_str() {
        "fact" => (
            RETRIEVAL_SOURCE_MEMORY_FACT,
            retrieval_source_ref_for_memory_fact(row_id("memory_facts")?),
            None,
            None,
        ),
        "preference" => {
            let key = snapshot_str(&memory.snapshot, "item_key")?;
            (
                RETRIEVAL_SOURCE_PREFERENCE,
                retrieval_source_ref_for_preference(key),
                None,
                Some(key.to_owned()),
            )
        }
        "recent" => {
            let id = row_id("memories")?;
            (
                RETRIEVAL_SOURCE_MEMORY,
                retrieval_source_ref_for_memory(id),
                Some(id),
                None,
            )
        }
        _ => anyhow::bail!("memory_kind_unsupported"),
    };
    Ok(RetrievalIdentity {
        source_kind,
        source_ref,
        source_memory_id,
        source_pref_key,
    })
}

pub(super) fn remove_identity(
    db: &Connection,
    principal_id: &str,
    identity: &RetrievalIdentity,
) -> anyhow::Result<()> {
    remove_targets(
        db,
        "SELECT id FROM memory_retrieval_index
         WHERE principal_id = ?1 AND source_kind = ?2
           AND (source_ref = ?3 OR source_memory_id = ?4 OR source_pref_key = ?5)",
        params![
            principal_id,
            identity.source_kind,
            identity.source_ref,
            identity.source_memory_id,
            identity.source_pref_key
        ],
    )?;
    Ok(())
}

/// Canonical mutations have an admitted scope; never affect another conversation.
pub(super) fn remove_canonical(db: &Connection, memory: &MutableMemory) -> anyhow::Result<()> {
    let identity = identity(db, memory)?;
    remove_targets(
        db,
        "SELECT id FROM memory_retrieval_index
         WHERE principal_id=?1 AND source_kind=?2 AND source_ref=?3
           AND scope_kind=?4 AND scope_ref=?5",
        params![
            memory.principal_id,
            identity.source_kind,
            identity.source_ref,
            memory.scope_kind,
            memory.scope_ref
        ],
    )?;
    Ok(())
}

fn remove_targets(
    db: &Connection,
    targets: &str,
    parameters: &[&dyn ToSql],
) -> anyhow::Result<usize> {
    crate::memory::projection_retirement::remove(db, targets, parameters)
}

#[cfg(test)]
#[path = "ux_retrieval_tests.rs"]
mod tests;

pub(crate) fn check_memory_consistency(
    db: &Connection,
    principal_id: &str,
    repair: bool,
    now_ts: i64,
) -> anyhow::Result<MemoryConsistencyReport> {
    require_memory_ux_schema(db)?;
    let deleted_canonical_rows = db.query_row(
        "SELECT COUNT(*) FROM memory_revisions r
         WHERE r.principal_id = ?1 AND r.operation = 'delete'
           AND r.undo_expires_at_ts IS NOT NULL
           AND (EXISTS(SELECT 1 FROM memory_facts f WHERE f.memory_id = r.memory_id)
             OR EXISTS(SELECT 1 FROM user_preferences p WHERE p.memory_id = r.memory_id)
             OR EXISTS(SELECT 1 FROM memories m WHERE m.memory_id = r.memory_id))",
        [principal_id],
        |row| row.get::<_, i64>(0),
    )?;
    let deleted_retrieval_rows = db.query_row(
        "SELECT COUNT(*) FROM memory_retrieval_index i
         WHERE i.principal_id = ?1 AND EXISTS(
           SELECT 1 FROM memory_revisions r
            WHERE r.principal_id = i.principal_id AND r.operation = 'delete'
              AND (i.source_ref = r.memory_id)
         )",
        [principal_id],
        |row| row.get::<_, i64>(0),
    )?;
    let orphan_retrieval_rows = db.query_row(
        &format!("SELECT COUNT(*) FROM ({ORPHAN_TARGETS})"),
        params![principal_id, MEMORY_FACT_SOURCE_REF_PREFIX],
        |row| row.get::<_, i64>(0),
    )?;
    let stale_jobs = db.query_row(
        "SELECT COUNT(*) FROM memory_jobs j WHERE j.principal_id = ?1
           AND j.status IN ('queued', 'retry_wait', 'running')
           AND j.source_task_id IS NOT NULL
           AND NOT EXISTS(
             SELECT 1 FROM memory_source_events e
              WHERE e.principal_id = j.principal_id
                AND e.source_task_id = j.source_task_id
                AND e.source_sequence BETWEEN COALESCE(j.source_event_start, e.source_sequence)
                                          AND COALESCE(j.source_event_end, e.source_sequence)
           )",
        [principal_id],
        |row| row.get::<_, i64>(0),
    )?;
    let mut repaired_rows = 0i64;
    if repair {
        let transaction = db.unchecked_transaction()?;
        repaired_rows += remove_targets(
            &transaction,
            ORPHAN_TARGETS,
            params![principal_id, MEMORY_FACT_SOURCE_REF_PREFIX],
        )? as i64;
        repaired_rows += transaction.execute(
            "UPDATE memory_jobs SET cancel_requested = 1, updated_at_ts = ?2
             WHERE principal_id = ?1 AND status IN ('queued', 'retry_wait', 'running')
               AND source_task_id IS NOT NULL
               AND NOT EXISTS(
                 SELECT 1 FROM memory_source_events e
                  WHERE e.principal_id = memory_jobs.principal_id
                    AND e.source_task_id = memory_jobs.source_task_id
                    AND e.source_sequence BETWEEN COALESCE(memory_jobs.source_event_start, e.source_sequence)
                                              AND COALESCE(memory_jobs.source_event_end, e.source_sequence)
               )",
            params![principal_id, now_ts],
        )? as i64;
        transaction.commit()?;
    }
    Ok(MemoryConsistencyReport {
        schema_version: 1,
        deleted_canonical_rows,
        deleted_retrieval_rows,
        orphan_retrieval_rows,
        stale_jobs,
        repaired_rows,
    })
}
