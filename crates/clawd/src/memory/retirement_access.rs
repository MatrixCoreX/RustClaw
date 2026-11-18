//! Startup-owned physical access paths; ordinary mutations never initialize them.
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const MIGRATION_ID: &str = "033_memory_owned_retirement_access_v1";
const MIGRATION_SQL: &str =
    include_str!("../../../../migrations/033_memory_owned_retirement_access.sql");
const INDEXES: &[(&str, &str, &[&str])] = &[
    (
        "memory_retrieval_index",
        "idx_memory_retrieval_owned_source",
        &[
            "principal_id",
            "scope_kind",
            "scope_ref",
            "source_kind",
            "source_ref",
        ],
    ),
    (
        "memory_embedding_jobs",
        "idx_memory_embedding_jobs_retirement",
        &["retrieval_id", "cancel_requested", "status"],
    ),
];

fn digest() -> String {
    format!("sha256:{:x}", Sha256::digest(MIGRATION_SQL.as_bytes()))
}

fn validate_indexes(db: &Connection) -> anyhow::Result<()> {
    for &(table, name, columns) in INDEXES {
        let metadata: Option<(String, i64, String, i64)> = db
            .query_row(
                "SELECT s.tbl_name,l.[unique],l.origin,l.partial
                 FROM main.sqlite_schema s JOIN pragma_index_list(?1,'main') l ON l.name=s.name
                 WHERE s.type='index' AND s.name=?2",
                params![table, name],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        anyhow::ensure!(
            metadata == Some((table.to_string(), 0, "c".to_string(), 0)),
            "memory_retirement_access_index_invalid:{name}"
        );
        let mut statement = db.prepare(
            "SELECT name,[desc],coll FROM pragma_index_xinfo(?1,'main') WHERE key=1 ORDER BY seqno",
        )?;
        let actual = statement
            .query_map([name], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        anyhow::ensure!(
            actual.len() == columns.len()
                && actual
                    .iter()
                    .zip(columns)
                    .all(|((column, descending, collation), expected)| {
                        column.as_deref() == Some(*expected)
                            && *descending == 0
                            && collation.as_deref() == Some("BINARY")
                    }),
            "memory_retirement_access_index_invalid:{name}"
        );
    }
    Ok(())
}

fn verified(db: &Connection) -> anyhow::Result<bool> {
    let applied: Option<String> = db
        .query_row(
            "SELECT schema_digest FROM main.runtime_schema_migrations WHERE migration_id=?1",
            [MIGRATION_ID],
            |row| row.get(0),
        )
        .optional()?;
    let Some(applied) = applied else {
        return Ok(false);
    };
    anyhow::ensure!(
        applied == digest(),
        "runtime_schema_migration_digest_mismatch:{MIGRATION_ID}"
    );
    validate_indexes(db)?;
    Ok(true)
}

/// Explicit startup only, after scope and embedding schema initialization.
/// Valid repeated initialization reads metadata only. Concurrent initializers
/// recheck the receipt after acquiring their writer transaction/savepoint.
pub(crate) fn ensure_retirement_access_schema(db: &Connection) -> anyhow::Result<()> {
    if verified(db)? {
        return Ok(());
    }
    super::projection_transaction::atomic(db, |db| {
        if verified(db)? {
            return Ok(());
        }
        db.execute_batch(MIGRATION_SQL)?;
        validate_indexes(db)?;
        db.execute(
            "INSERT INTO main.runtime_schema_migrations(migration_id,schema_digest,applied_at)
             VALUES (?1,?2,?3)",
            params![MIGRATION_ID, digest(), crate::now_ts()],
        )?;
        Ok(())
    })
}

#[cfg(test)]
#[path = "retirement_access_failure_tests.rs"]
mod failure_tests;
#[cfg(test)]
#[path = "retirement_access_test_fixtures.rs"]
mod fixtures;
#[cfg(test)]
#[path = "retirement_access_migration_tests.rs"]
mod migration_tests;
#[cfg(test)]
#[path = "retirement_access_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "retirement_access_work_tests.rs"]
mod work_tests;
