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
        _ => Ok(()),
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
