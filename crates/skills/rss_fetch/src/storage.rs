use std::fs;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use super::{RssMachineState, RssRuntime};

const SKILL_NAME: &str = "rss_fetch";
const SCHEMA_VERSION: i64 = 1;
const STATE_ROW_ID: i64 = 1;
const LEGACY_CONFIG_MIGRATION_ID: &str = "legacy-rss-config-machine-state-v1";

#[derive(Debug)]
pub(super) struct StorageLoad {
    pub(super) state: RssMachineState,
    pub(super) cleanup_legacy_config: bool,
}

pub(super) fn initialize_and_load(
    runtime: &RssRuntime,
    legacy: &RssMachineState,
) -> Result<StorageLoad, String> {
    let mut db = open(runtime)?;
    ensure_schema(&db)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| "storage_transaction_failed".to_string())?;
    let migration_complete = migration_exists(&tx, LEGACY_CONFIG_MIGRATION_ID)?;
    let mut state = load_state_from_connection(&tx)?.unwrap_or_default();
    let legacy_present = !legacy.is_empty();

    if legacy_present && !migration_complete {
        if !state.is_empty() && state != *legacy {
            return Err("legacy_storage_state_conflict".to_string());
        }
        state = legacy.clone();
        write_state_to_connection(&tx, &state)?;
        let digest = state_digest(&state)?;
        tx.execute(
            "INSERT INTO skill_storage_migrations (
                migration_id, source_identity, source_rows, verified_digest
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                LEGACY_CONFIG_MIGRATION_ID,
                "configs/rss.toml",
                state.entry_count() as i64,
                digest
            ],
        )
        .map_err(|_| "storage_migration_record_failed".to_string())?;
    } else if load_state_from_connection(&tx)?.is_none() {
        write_state_to_connection(&tx, &state)?;
    }

    verify_state_in_connection(&tx, &state)?;
    tx.commit()
        .map_err(|_| "storage_transaction_commit_failed".to_string())?;
    integrity_check(&db)?;
    Ok(StorageLoad {
        state,
        cleanup_legacy_config: legacy_present,
    })
}

pub(super) fn save_if_unchanged(
    runtime: &RssRuntime,
    expected: &RssMachineState,
    updated: &RssMachineState,
) -> Result<(), String> {
    let mut db = open(runtime)?;
    ensure_schema(&db)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| "storage_transaction_failed".to_string())?;
    let current = load_state_from_connection(&tx)?.unwrap_or_default();
    if state_digest(&current)? != state_digest(expected)? {
        return Err("storage_write_conflict".to_string());
    }
    write_state_to_connection(&tx, updated)?;
    verify_state_in_connection(&tx, updated)?;
    tx.commit()
        .map_err(|_| "storage_transaction_commit_failed".to_string())?;
    integrity_check(&db)
}

fn open(runtime: &RssRuntime) -> Result<Connection, String> {
    if !runtime.storage_database_path.is_absolute() {
        return Err("storage_path_not_absolute".to_string());
    }
    if runtime
        .storage_database_path
        .file_name()
        .and_then(|value| value.to_str())
        != Some("state.db")
    {
        return Err("storage_database_identity_invalid".to_string());
    }
    let parent = runtime
        .storage_database_path
        .parent()
        .ok_or_else(|| "storage_parent_missing".to_string())?;
    fs::create_dir_all(parent).map_err(|_| "storage_directory_create_failed".to_string())?;
    let db = Connection::open(&runtime.storage_database_path)
        .map_err(|_| "storage_database_open_failed".to_string())?;
    db.busy_timeout(Duration::from_millis(
        runtime.storage_busy_timeout_ms.max(1),
    ))
    .map_err(|_| "storage_busy_timeout_failed".to_string())?;
    db.pragma_update(None, "journal_mode", "WAL")
        .map_err(|_| "storage_journal_mode_failed".to_string())?;
    db.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|_| "storage_synchronous_mode_failed".to_string())?;
    Ok(db)
}

fn ensure_schema(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS skill_storage_metadata (
            skill_name TEXT PRIMARY KEY,
            schema_version INTEGER NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS skill_storage_migrations (
            migration_id TEXT PRIMARY KEY,
            source_identity TEXT NOT NULL,
            source_rows INTEGER NOT NULL,
            verified_digest TEXT NOT NULL,
            completed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE IF NOT EXISTS rss_machine_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            payload_json TEXT NOT NULL,
            payload_sha256 TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .map_err(|_| "storage_schema_create_failed".to_string())?;
    db.execute(
        "INSERT INTO skill_storage_metadata (skill_name, schema_version)
         VALUES (?1, ?2)
         ON CONFLICT(skill_name) DO UPDATE SET
           schema_version = excluded.schema_version,
           updated_at = CURRENT_TIMESTAMP",
        params![SKILL_NAME, SCHEMA_VERSION],
    )
    .map_err(|_| "storage_metadata_write_failed".to_string())?;
    let schema_version = db
        .query_row(
            "SELECT schema_version FROM skill_storage_metadata WHERE skill_name = ?1",
            params![SKILL_NAME],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| "storage_metadata_read_failed".to_string())?;
    if schema_version != SCHEMA_VERSION {
        return Err("storage_schema_version_mismatch".to_string());
    }
    Ok(())
}

fn migration_exists(db: &Connection, migration_id: &str) -> Result<bool, String> {
    db.query_row(
        "SELECT 1 FROM skill_storage_migrations WHERE migration_id = ?1",
        params![migration_id],
        |_| Ok(()),
    )
    .optional()
    .map(|value| value.is_some())
    .map_err(|_| "storage_migration_read_failed".to_string())
}

fn load_state_from_connection(db: &Connection) -> Result<Option<RssMachineState>, String> {
    let row = db
        .query_row(
            "SELECT payload_json, payload_sha256 FROM rss_machine_state WHERE id = ?1",
            params![STATE_ROW_ID],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|_| "storage_state_read_failed".to_string())?;
    let Some((payload, persisted_digest)) = row else {
        return Ok(None);
    };
    let state = serde_json::from_str::<RssMachineState>(&payload)
        .map_err(|_| "storage_state_decode_failed".to_string())?;
    if state_digest(&state)? != persisted_digest {
        return Err("storage_state_digest_mismatch".to_string());
    }
    Ok(Some(state))
}

fn write_state_to_connection(db: &Connection, state: &RssMachineState) -> Result<(), String> {
    let payload =
        serde_json::to_string(state).map_err(|_| "storage_state_encode_failed".to_string())?;
    let digest = state_digest(state)?;
    db.execute(
        "INSERT INTO rss_machine_state (id, payload_json, payload_sha256)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
           payload_json = excluded.payload_json,
           payload_sha256 = excluded.payload_sha256,
           updated_at = CURRENT_TIMESTAMP",
        params![STATE_ROW_ID, payload, digest],
    )
    .map_err(|_| "storage_state_write_failed".to_string())?;
    Ok(())
}

fn verify_state_in_connection(db: &Connection, expected: &RssMachineState) -> Result<(), String> {
    let persisted = load_state_from_connection(db)?
        .ok_or_else(|| "storage_state_verification_missing".to_string())?;
    if persisted != *expected
        || persisted.entry_count() != expected.entry_count()
        || state_digest(&persisted)? != state_digest(expected)?
    {
        return Err("storage_state_verification_failed".to_string());
    }
    Ok(())
}

fn state_digest(state: &RssMachineState) -> Result<String, String> {
    let payload =
        serde_json::to_vec(state).map_err(|_| "storage_state_encode_failed".to_string())?;
    Ok(format!("{:x}", Sha256::digest(payload)))
}

fn integrity_check(db: &Connection) -> Result<(), String> {
    let result = db
        .query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
        .map_err(|_| "storage_integrity_check_failed".to_string())?;
    if result != "ok" {
        return Err("storage_integrity_check_failed".to_string());
    }
    Ok(())
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
