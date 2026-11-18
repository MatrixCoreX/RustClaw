use rusqlite::{Connection, ToSql};

/// Retire the exact owned projection and its derived work before deleting its IDs.
/// Targets and parameters come only from the canonical/internal index selectors.
pub(super) fn remove(
    db: &Connection,
    targets: &str,
    parameters: &[&dyn ToSql],
) -> anyhow::Result<usize> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_retrieval_mutation_requires_transaction"
    );
    super::projection_transaction::atomic(db, |db| {
        // Text-only low-level callers may have no vector pipeline. Never create
        // missing derived tables during a mutation or swallow errors in present ones.
        let present = |table: &str| -> rusqlite::Result<bool> {
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
        };
        let now = crate::now_ts_u64() as i64;
        let mut values = parameters.to_vec();
        values.push(&now);
        let timestamp = format!("?{}", values.len());
        if present("memory_embedding_jobs")? {
            db.execute(
                &format!(
                    "UPDATE memory_embedding_jobs SET cancel_requested=1,updated_at_ts={timestamp}
                     WHERE cancel_requested=0 AND status IN ('queued','retry_wait','running')
                       AND {}",
                    owned_targets("memory_embedding_jobs", targets)
                ),
                values.as_slice(),
            )?;
        }
        if present("memory_vector_rows")? {
            db.execute(
                &format!(
                    "UPDATE memory_vector_rows SET status='tombstone',updated_at_ts={timestamp}
                     WHERE status='active' AND {}",
                    owned_targets("memory_vector_rows", targets)
                ),
                values.as_slice(),
            )?;
        }
        if present("memory_retrieval_index_fts")? {
            db.execute(
                &format!("DELETE FROM memory_retrieval_index_fts WHERE rowid IN ({targets})"),
                parameters,
            )?;
        }
        Ok(db.execute(
            &format!("DELETE FROM memory_retrieval_index WHERE id IN ({targets})"),
            parameters,
        )?)
    })
}

/// Keep ownership checks while exposing exact IDs to the derived lookup index.
pub(super) fn owned_targets(table: &str, targets: &str) -> String {
    format!(
        "{table}.retrieval_id IN ({targets}) AND EXISTS(
             SELECT 1 FROM memory_retrieval_index i WHERE i.id={table}.retrieval_id
               AND i.principal_id={table}.principal_id
               AND i.scope_kind={table}.scope_kind AND i.scope_ref={table}.scope_ref)"
    )
}
