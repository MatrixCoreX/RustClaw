use rusqlite::Connection;

use crate::memory::growth_availability::{self, ProbePhase};

pub(in crate::memory) struct GrowthAdmission(crate::memory::storage_budget::Snapshot);

/// Only restoring a source transcript consumes the existing source quota.
/// Facts/preferences, derived rows and metadata are not relabelled as sources.
pub(super) fn validate_restored_source_quota(
    db: &Connection,
    principal: &str,
    now_ts: i64,
) -> anyhow::Result<()> {
    use rusqlite::{types::ValueRef, OptionalExtension};
    // Source restoration is independent of background spending, including an
    // unavailable background counter. Never coerce damaged source limits.
    let limits = db
        .query_row(
            "SELECT max_rows,max_bytes FROM memory_principal_quotas WHERE principal_id=?1",
            [principal],
            |row| {
                Ok(match (row.get_ref(0)?, row.get_ref(1)?) {
                    (ValueRef::Integer(rows), ValueRef::Integer(bytes))
                        if rows >= 0 && bytes >= 0 =>
                    {
                        Some((rows, bytes))
                    }
                    _ => None,
                })
            },
        )
        .optional()?;
    let (max_rows, max_bytes) = match limits {
        Some(Some(values)) => values,
        None => anyhow::bail!("memory_principal_quota_missing"),
        Some(None) => anyhow::bail!("memory_principal_quota_invalid"),
    };
    let (rows, bytes): (i64,i64) = db.query_row(
        "SELECT COUNT(*),COALESCE(SUM(LENGTH(CAST(content AS BLOB))),0) FROM memories WHERE principal_id=?1",
        [principal], |row|Ok((row.get(0)?,row.get(1)?)),
    )?;
    anyhow::ensure!(rows <= max_rows, "memory_principal_rows_exhausted");
    anyhow::ensure!(bytes <= max_bytes, "memory_principal_bytes_exhausted");
    crate::memory::retention::refresh_existing_principal_usage(db, principal, now_ts)
}

impl GrowthAdmission {
    pub(in crate::memory) fn begin(db: &Connection) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !db.is_autocommit(),
            "memory_write_admission_requires_transaction"
        );
        let budget = growth_availability::probe(db, ProbePhase::BeforeGrowth, || {
            crate::memory::availability::memory_btrees(db)
        })
        .admit()?;
        Ok(Self(budget))
    }

    pub(in crate::memory) fn finish(self, db: &Connection) -> anyhow::Result<()> {
        anyhow::ensure!(
            !db.is_autocommit(),
            "memory_write_admission_requires_transaction"
        );
        let observation = growth_availability::probe(db, ProbePhase::AfterGrowth, || {
            crate::memory::availability::memory_btrees(db)
        });
        anyhow::ensure!(
            observation.validated_snapshot()? == self.0,
            "memory_storage_policy_conflict"
        );
        observation.admit()?;
        Ok(())
    }
}

/// Standalone writes own an Immediate transaction; composed writes own only a
/// private rollback scope and never commit or invalidate their caller's work.
pub(crate) fn atomic_mutation<T>(
    db: &Connection,
    operation: impl FnOnce(&Connection) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    crate::memory::projection_transaction::atomic(db, operation)
}
