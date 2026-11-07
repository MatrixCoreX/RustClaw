use rusqlite::Connection;

/// Standalone writes own an Immediate transaction; composed writes own only a
/// private rollback scope and never commit or invalidate their caller's work.
pub(crate) fn atomic_mutation<T>(
    db: &Connection,
    operation: impl FnOnce(&Connection) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    crate::memory::projection_transaction::atomic(db, operation)
}
