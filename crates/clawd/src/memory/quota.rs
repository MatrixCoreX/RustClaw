//! Policy defaults belong to explicit startup/new-identity transactions.
//! Usage refresh, reads and queue admission cannot create or repair policy.
use claw_core::config::MemoryConfig;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

const MIGRATION_ID: &str = "030_memory_quota_initialization_v1";
const MIGRATION_SQL: &str =
    include_str!("../../../../migrations/030_memory_quota_initialization.sql");

fn digest() -> String {
    format!("sha256:{:x}", Sha256::digest(MIGRATION_SQL.as_bytes()))
}

fn applied(db: &Connection) -> anyhow::Result<Option<String>> {
    Ok(db
        .query_row(
            "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id=?1",
            [MIGRATION_ID],
            |row| row.get(0),
        )
        .optional()?)
}

/// Run once, after identity backfill/bootstrap and memory-schema migration.
/// Existing policy (even invalid policy) is preserved, not normalized to defaults.
/// Subsequent starts validate the marker but do not recreate a missing policy.
pub(crate) fn initialize_startup_defaults(
    db: &Connection,
    config: &MemoryConfig,
    now: i64,
) -> anyhow::Result<()> {
    super::jobs::require_memory_job_schema(db)?;
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    if let Some(value) = applied(&tx)? {
        anyhow::ensure!(
            value == digest(),
            "memory_quota_initialization_digest_mismatch"
        );
        // A marker alone is not proof that its owned receipt table survived.
        tx.prepare("SELECT principal_id,initialization_kind,initialized_at_ts FROM memory_quota_initializations LIMIT 0")?;
        tx.commit()?;
        return Ok(());
    }
    let already_present: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='memory_quota_initializations')", [], |row| row.get(0),
    )?;
    anyhow::ensure!(
        !already_present,
        "memory_quota_initialization_marker_missing"
    );
    tx.execute_batch(MIGRATION_SQL)?;
    // First retain ownership of every existing policy, including legacy rows
    // whose identity has since been removed. Never reset their limits or costs.
    tx.execute(
        "INSERT INTO memory_quota_initializations(principal_id,initialization_kind,initialized_at_ts)
         SELECT principal_id,'legacy_startup',?1 FROM memory_principal_quotas
         WHERE length(principal_id)>0 ON CONFLICT(principal_id) DO NOTHING", [now],
    )?;
    let rows = i64::try_from(config.max_rows.max(1))
        .map_err(|_| anyhow::anyhow!("memory_quota_config_invalid"))?;
    tx.execute(
        "INSERT INTO memory_principal_quotas(
            principal_id,max_rows,max_bytes,max_background_cost_microunits,
            used_rows,used_bytes,used_background_cost_microunits,revision,updated_at_ts)
         SELECT p.principal_id,?1,?2,?3,COALESCE(u.source_rows,0),COALESCE(u.source_bytes,0),0,1,?4
         FROM principals p LEFT JOIN (
            SELECT principal_id,COUNT(*) AS source_rows,SUM(LENGTH(CAST(content AS BLOB))) AS source_bytes
            FROM memories WHERE principal_id IS NOT NULL GROUP BY principal_id
         ) u ON u.principal_id=p.principal_id
         WHERE length(p.principal_id)>0 AND NOT EXISTS (
            SELECT 1 FROM memory_quota_initializations i WHERE i.principal_id=p.principal_id
         ) ON CONFLICT(principal_id) DO NOTHING",
        params![rows,config.principal_max_bytes.min(i64::MAX as u64) as i64,
            config.principal_background_cost_microunits.min(i64::MAX as u64) as i64,now],
    )?;
    tx.execute(
        "INSERT INTO memory_quota_initializations(principal_id,initialization_kind,initialized_at_ts)
         SELECT principal_id,'legacy_startup',?1 FROM principals WHERE length(principal_id)>0
         ON CONFLICT(principal_id) DO NOTHING", [now],
    )?;
    tx.execute(
        "INSERT INTO runtime_schema_migrations(migration_id,schema_digest,applied_at) VALUES (?1,?2,?3)",
        params![MIGRATION_ID,digest(),crate::now_ts()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Called only in the transaction that creates a new credential/principal.
/// Identity bootstrap before this memory contract is installed remains valid;
/// the explicit startup migration will initialize those legacy identities.
pub(crate) fn initialize_new_identity(
    db: &Connection,
    config: &MemoryConfig,
    principal: &str,
    now: i64,
) -> anyhow::Result<()> {
    let Some(value) = applied(db)? else {
        let receipts_exist: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='memory_quota_initializations')", [], |row|row.get(0),
        )?;
        anyhow::ensure!(
            !receipts_exist,
            "memory_quota_initialization_marker_missing"
        );
        return Ok(());
    };
    anyhow::ensure!(
        value == digest(),
        "memory_quota_initialization_digest_mismatch"
    );
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_quota_initialization_requires_identity_transaction"
    );
    super::jobs::require_memory_job_schema(db)?;
    let active: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM principals WHERE principal_id=?1 AND status='active')",
        [principal],
        |row| row.get(0),
    )?;
    anyhow::ensure!(active, "memory_quota_owner_inactive");
    let created = db.execute(
        "INSERT INTO memory_quota_initializations(principal_id,initialization_kind,initialized_at_ts)
         VALUES (?1,'new_identity',?2) ON CONFLICT(principal_id) DO NOTHING",
        params![principal,now],
    )?;
    if created == 1 {
        i64::try_from(config.max_rows.max(1))
            .map_err(|_| anyhow::anyhow!("memory_quota_config_invalid"))?;
        super::retention::ensure_principal_quota(db, config, principal, now)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "quota_tests.rs"]
mod tests;
