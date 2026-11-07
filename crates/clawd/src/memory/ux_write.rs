use rusqlite::Connection;

/// Validate only the observation's integrity, inside the mutation's rollback
/// scope. A known whole-database pressure level is not itself a measurement of
/// memory-domain growth. Effect-specific domain-budget admission is separate.
pub(super) fn require_growth_storage_state(db: &Connection) -> anyhow::Result<()> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_write_admission_requires_transaction"
    );
    match crate::memory::capacity::StoragePressure::load(db)? {
        crate::memory::capacity::StoragePressure::Missing => {
            anyhow::bail!("memory_storage_state_missing")
        }
        crate::memory::capacity::StoragePressure::Unknown => {
            anyhow::bail!("memory_storage_state_unknown")
        }
        crate::memory::capacity::StoragePressure::Normal
        | crate::memory::capacity::StoragePressure::DerivedCleanup
        | crate::memory::capacity::StoragePressure::BackfillPaused
        | crate::memory::capacity::StoragePressure::AutomaticGenerationPaused
        | crate::memory::capacity::StoragePressure::ExplicitWriteBlocked => Ok(()),
    }
}

pub(super) struct GrowthAdmission(crate::memory::storage_budget::Snapshot);

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
    pub(super) fn begin(db: &Connection) -> anyhow::Result<Self> {
        require_growth_storage_state(db)?;
        let budget = crate::memory::storage_budget::Snapshot::load(db)?;
        Self::validate_observation(db, budget)?;
        anyhow::ensure!(
            Self::domain_bytes(db)? < budget.limit,
            "memory_storage_write_blocked"
        );
        Ok(Self(budget))
    }

    pub(super) fn finish(self, db: &Connection) -> anyhow::Result<()> {
        require_growth_storage_state(db)?;
        anyhow::ensure!(
            crate::memory::storage_budget::Snapshot::load(db)? == self.0,
            "memory_storage_policy_conflict"
        );
        Self::validate_observation(db, self.0)?;
        anyhow::ensure!(
            Self::domain_bytes(db)? <= self.0.limit,
            "memory_storage_write_blocked"
        );
        Ok(())
    }

    fn domain_bytes(db: &Connection) -> anyhow::Result<u64> {
        match crate::memory::availability::memory_btrees(db) {
            crate::memory::availability::ByteMeasurement::Measured { bytes } => Ok(bytes),
            _ => anyhow::bail!("memory_storage_measurement_unavailable"),
        }
    }

    fn validate_observation(
        db: &Connection,
        budget: crate::memory::storage_budget::Snapshot,
    ) -> anyhow::Result<()> {
        // An explicit stop without the producing observation's configuration
        // binding is conservative. Never infer provenance from reason prose.
        if crate::memory::capacity::StoragePressure::load(db)?
            == crate::memory::capacity::StoragePressure::ExplicitWriteBlocked
        {
            let revision: i64 = db.query_row(
                "SELECT revision FROM memory_storage_pressure WHERE singleton_id=1",
                [],
                |row| row.get(0),
            )?;
            anyhow::ensure!(
                revision == budget.pressure_revision,
                "memory_storage_write_blocked"
            );
        }
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
