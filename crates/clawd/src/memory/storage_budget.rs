//! Startup owns the configured budget projection and its observation binding.
//! Admission only reads it; source quotas and host space are separate contracts.
use claw_core::config::MemoryConfig;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

const MIGRATION_ID: &str = "031_memory_storage_budget_v1";
const MIGRATION_SQL: &str = include_str!("../../../../migrations/031_memory_storage_budget.sql");

fn digest() -> String {
    format!("sha256:{:x}", Sha256::digest(MIGRATION_SQL.as_bytes()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadError {
    Missing,
    Invalid,
    Unavailable,
}

impl ReadError {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::Missing => "memory_storage_policy_missing",
            Self::Invalid => "memory_storage_policy_invalid",
            Self::Unavailable => "memory_storage_policy_unavailable",
        }
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ReadError {}

fn applied(db: &Connection) -> Result<Option<String>, ReadError> {
    db.query_row(
        "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id=?1",
        [MIGRATION_ID],
        |row| row.get(0),
    )
    .optional()
    .map_err(|_| ReadError::Unavailable)
}

/// Explicit startup only, inside its Immediate transaction. A missing receipt
/// must not silently adopt an unowned table; a corrupt schema is not recreated.
pub(super) fn initialize_schema(db: &Connection) -> anyhow::Result<()> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_storage_policy_requires_transaction"
    );
    if let Some(applied) = applied(db)? {
        anyhow::ensure!(applied == digest(), "memory_storage_policy_invalid");
        db.prepare("SELECT singleton_id,configured_limit_bytes,pressure_revision FROM memory_storage_budget LIMIT 0")?;
        return Ok(());
    }
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='memory_storage_budget')",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(!exists, "memory_storage_policy_marker_missing");
    db.execute_batch(MIGRATION_SQL)?;
    db.execute(
        "INSERT INTO runtime_schema_migrations(migration_id,schema_digest,applied_at) VALUES (?1,?2,?3)",
        params![MIGRATION_ID,digest(),crate::now_ts()],
    )?;
    Ok(())
}

fn require_schema(db: &Connection) -> Result<(), ReadError> {
    match applied(db)? {
        Some(applied) if applied == digest() => Ok(()),
        Some(_) => Err(ReadError::Invalid),
        None => Err(ReadError::Missing),
    }
}

/// Pressure observation and configuration projection are published together.
/// This does not reset any per-principal quota, grant consent or enable memory.
pub(super) fn refresh(db: &Connection, config: &MemoryConfig) -> anyhow::Result<()> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_storage_policy_requires_transaction"
    );
    require_schema(db)?;
    let changed = db.execute(
        "INSERT INTO memory_storage_budget(singleton_id,configured_limit_bytes,pressure_revision)
         SELECT 1,?1,revision FROM memory_storage_pressure WHERE singleton_id=1
         ON CONFLICT(singleton_id) DO UPDATE SET
             configured_limit_bytes=excluded.configured_limit_bytes,
             pressure_revision=excluded.pressure_revision",
        [config.storage_soft_limit_bytes.max(1).to_string()],
    )?;
    anyhow::ensure!(changed == 1, "memory_storage_state_missing");
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) limit: u64,
    pub(super) pressure_revision: i64,
}

impl Snapshot {
    pub(super) fn load(db: &Connection) -> Result<Self, ReadError> {
        require_schema(db)?;
        let (limit, revision) = db.query_row(
            "SELECT configured_limit_bytes,pressure_revision FROM memory_storage_budget WHERE singleton_id=1",
            [], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?)),
        ).optional().map_err(|_| ReadError::Invalid)?
         .ok_or(ReadError::Missing)?;
        let bytes = limit.parse::<u64>().map_err(|_| ReadError::Invalid)?;
        if bytes == 0 || bytes.to_string() != limit || revision <= 0 {
            return Err(ReadError::Invalid);
        }
        Ok(Self {
            limit: bytes,
            pressure_revision: revision,
        })
    }
}
