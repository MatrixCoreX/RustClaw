use rusqlite::{Connection, Transaction, TransactionBehavior};

/// An index refresh must roll back locally even if its caller keeps its outer transaction.
pub(super) fn atomic<T>(
    db: &Connection,
    operation: impl FnOnce(&Connection) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    if db.is_autocommit() {
        let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
        let result = operation(&transaction)?;
        transaction.commit()?;
        return Ok(result);
    }
    // rusqlite 0.32 exposes savepoints only through a mutable Connection.
    // This private scope supports existing borrowed-connection callers, without
    // committing their parent transaction or accepting a caller-supplied name.
    let mut scope = NestedProjection {
        db,
        name: format!("memory_projection_{}", uuid::Uuid::new_v4().simple()),
        released: false,
    };
    db.execute_batch(&format!("SAVEPOINT {}", scope.name))?;
    let result = operation(db)?;
    db.execute_batch(&format!("RELEASE SAVEPOINT {}", scope.name))?;
    scope.released = true;
    Ok(result)
}

struct NestedProjection<'a> {
    db: &'a Connection,
    name: String,
    released: bool,
}

impl Drop for NestedProjection<'_> {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.db.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT {0}; RELEASE SAVEPOINT {0}",
                self.name
            ));
        }
    }
}

#[cfg(test)]
#[path = "projection_transaction_tests.rs"]
mod tests;
