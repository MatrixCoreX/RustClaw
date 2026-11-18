use rusqlite::Connection;

const TERMINAL_BUILD: &str = "snapshot.state='building' AND EXISTS(
    SELECT 1 FROM memory_embedding_jobs job
    WHERE job.principal_id=snapshot.principal_id
      AND job.profile_id=snapshot.profile_id
      AND job.profile_generation=snapshot.generation
      AND job.status IN ('failed','cancelled'))";

/// Terminal job recovery must also release its failed build, including builds
/// left behind by an interrupted earlier worker. This frequent probe is read-only.
pub(super) fn needed(db: &Connection) -> anyhow::Result<bool> {
    Ok(db.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM memory_vector_snapshots snapshot WHERE {TERMINAL_BUILD})"
        ),
        [],
        |row| row.get(0),
    )?)
}

/// Share the job-maintenance transaction. Neither active snapshots nor unrelated
/// owners/profiles/generations are retired, and no provider work is re-enqueued.
pub(super) fn reconcile(db: &Connection, now: i64) -> anyhow::Result<()> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_embedding_snapshot_maintenance_requires_transaction"
    );
    db.execute(
        &format!(
            "UPDATE memory_vector_snapshots AS snapshot
            SET state='corrupt',updated_at_ts=?1 WHERE {TERMINAL_BUILD}"
        ),
        [now],
    )?;
    Ok(())
}
