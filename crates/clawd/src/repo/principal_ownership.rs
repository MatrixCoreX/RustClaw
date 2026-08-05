use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const MIGRATION_ID: &str = "009_principal_ownership_v1";
const MIGRATION_MANIFEST: &str = include_str!("../../../../migrations/009_principal_ownership.sql");

const PRINCIPAL_COLUMNS: &[(&str, &str)] = &[
    ("tasks", "user_key"),
    ("scheduled_jobs", "user_key"),
    ("channel_bindings", "user_key"),
    ("pending_channel_bind_sessions", "user_key"),
    ("webd_login_accounts", "user_key"),
    ("memories", "user_key"),
    ("long_term_memories", "user_key"),
    ("user_preferences", "user_key"),
    ("memory_facts", "user_key"),
    ("memory_retrieval_index", "user_key"),
    ("followup_frames", "user_key"),
    ("clarify_states", "user_key"),
    ("observed_facts_states", "user_key"),
    ("conversation_states", "user_key"),
];

const OWNER_PRINCIPAL_COLUMNS: &[(&str, &str)] = &[
    ("conversation_metadata", "owner_user_key"),
    ("conversation_archives", "owner_user_key"),
];

pub(crate) fn ensure_principal_ownership_schema(db: &Connection) -> anyhow::Result<()> {
    super::auth::ensure_principal_identity_schema(db)?;
    if let Some(applied_digest) = migration_digest(db)? {
        anyhow::ensure!(
            applied_digest == manifest_digest(),
            "runtime_schema_migration_digest_mismatch:{MIGRATION_ID}"
        );
    }

    if db.is_autocommit() {
        let tx = db.unchecked_transaction()?;
        apply_migration(&tx)?;
        tx.commit()?;
    } else {
        apply_migration(db)?;
    }
    Ok(())
}

fn apply_migration(db: &Connection) -> anyhow::Result<()> {
    for &(table, legacy_key_column) in PRINCIPAL_COLUMNS {
        if !table_exists(db, table)? || !column_exists(db, table, legacy_key_column)? {
            continue;
        }
        add_column_if_missing(db, table, "principal_id")?;
        backfill_column(db, table, legacy_key_column, "principal_id")?;
        create_principal_index(db, table, "principal_id")?;
    }
    for &(table, legacy_key_column) in OWNER_PRINCIPAL_COLUMNS {
        if !table_exists(db, table)? || !column_exists(db, table, legacy_key_column)? {
            continue;
        }
        add_column_if_missing(db, table, "owner_principal_id")?;
        backfill_column(db, table, legacy_key_column, "owner_principal_id")?;
        create_principal_index(db, table, "owner_principal_id")?;
    }
    if table_exists(db, "conversation_metadata")? {
        db.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_conversation_metadata_principal_conversation
             ON conversation_metadata(owner_principal_id, conversation_id)
             WHERE owner_principal_id IS NOT NULL;",
        )?;
    }
    if table_exists(db, "conversation_archives")? {
        db.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_conversation_archives_principal_conversation
             ON conversation_archives(owner_principal_id, conversation_id)
             WHERE owner_principal_id IS NOT NULL;",
        )?;
    }
    db.execute(
        "INSERT INTO runtime_schema_migrations (migration_id, schema_digest, applied_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(migration_id) DO NOTHING",
        params![MIGRATION_ID, manifest_digest(), crate::now_ts()],
    )?;
    Ok(())
}

fn add_column_if_missing(db: &Connection, table: &str, column: &str) -> anyhow::Result<()> {
    if column_exists(db, table, column)? {
        return Ok(());
    }
    db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} TEXT"))?;
    Ok(())
}

fn backfill_column(
    db: &Connection,
    table: &str,
    legacy_key_column: &str,
    principal_column: &str,
) -> anyhow::Result<()> {
    db.execute(
        &format!(
            "UPDATE {table}
             SET {principal_column} = (
                SELECT principal_id FROM auth_keys
                WHERE auth_keys.user_key = {table}.{legacy_key_column}
             )
             WHERE {principal_column} IS NULL
               AND {legacy_key_column} IS NOT NULL
               AND EXISTS (
                    SELECT 1 FROM auth_keys
                    WHERE auth_keys.user_key = {table}.{legacy_key_column}
                      AND auth_keys.principal_id IS NOT NULL
               )"
        ),
        [],
    )?;
    Ok(())
}

fn create_principal_index(db: &Connection, table: &str, column: &str) -> anyhow::Result<()> {
    db.execute_batch(&format!(
        "CREATE INDEX IF NOT EXISTS idx_{table}_{column}
         ON {table}({column}) WHERE {column} IS NOT NULL"
    ))?;
    Ok(())
}

fn table_exists(db: &Connection, table: &str) -> anyhow::Result<bool> {
    db.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |_| Ok(true),
    )
    .optional()
    .map(|value| value.unwrap_or(false))
    .map_err(Into::into)
}

fn column_exists(db: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for current in columns {
        if current?.eq_ignore_ascii_case(column) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn migration_digest(db: &Connection) -> anyhow::Result<Option<String>> {
    db.query_row(
        "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id = ?1",
        [MIGRATION_ID],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn manifest_digest() -> String {
    format!("sha256:{:x}", Sha256::digest(MIGRATION_MANIFEST.as_bytes()))
}

#[cfg(test)]
#[path = "principal_ownership_tests.rs"]
mod tests;
