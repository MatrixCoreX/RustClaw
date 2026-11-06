use claw_core::config::MemoryConfig;
use rusqlite::{params, Connection};

pub(super) const NOW: i64 = 2_000_000_000;
pub(super) const PRINCIPAL: &str = "principal-accounting";

pub(super) fn fixture() -> (Connection, MemoryConfig) {
    let db = super::super::setup_db();
    crate::memory::scope::ensure_memory_scope_schema(&db).expect("scope schema");
    crate::memory::jobs::ensure_memory_job_schema(&db).expect("jobs schema");
    let mut config = MemoryConfig::default();
    config.max_rows = 100;
    config.principal_max_bytes = 1_000;
    config.principal_background_cost_microunits = 1_000;
    config.retention_days = 3650;
    config.storage_soft_limit_bytes = u64::MAX / 2;
    (db, config)
}

pub(super) fn source(db: &Connection, principal: Option<&str>, content: &str, at: i64) {
    db.execute(
        "INSERT INTO memories(
            memory_id, user_id, chat_id, user_key, principal_id, scope_kind,
            scope_ref, channel, role, content, created_at, created_at_ts
         ) VALUES (?1, 1, 1, ?2, ?3, 'principal', ?2, 'ui', 'user', ?4, ?5, ?6)",
        params![
            format!("memory-accounting-{}", uuid::Uuid::new_v4().simple()),
            principal.unwrap_or("unassigned-fixture"),
            principal,
            content,
            at.to_string(),
            at,
        ],
    )
    .expect("fixture source");
}

pub(super) fn usage(db: &Connection, principal: &str) -> (i64, i64) {
    db.query_row(
        "SELECT used_rows, used_bytes FROM memory_principal_quotas WHERE principal_id = ?1",
        [principal],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("principal usage")
}

pub(super) fn delete_sources(db: &Connection, principal: &str) {
    db.execute("DELETE FROM memories WHERE principal_id = ?1", [principal])
        .expect("fixture deletion");
}

pub(super) fn set_admin_policy(db: &Connection) {
    db.execute(
        "UPDATE memory_principal_quotas SET max_rows = 17, max_bytes = 12345,
            max_background_cost_microunits = 700, used_background_cost_microunits = 9,
            revision = 23, updated_at_ts = ?2 WHERE principal_id = ?1",
        params![PRINCIPAL, NOW - 10],
    )
    .expect("administrator policy");
}

pub(super) fn policy(db: &Connection) -> (i64, i64, i64, i64, i64) {
    db.query_row(
        "SELECT max_rows, max_bytes, max_background_cost_microunits,
                used_background_cost_microunits, revision
         FROM memory_principal_quotas WHERE principal_id = ?1",
        [PRINCIPAL],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )
    .expect("policy and spent cost")
}

pub(super) fn updated_at(db: &Connection) -> i64 {
    db.query_row(
        "SELECT updated_at_ts FROM memory_principal_quotas WHERE principal_id = ?1",
        [PRINCIPAL],
        |row| row.get(0),
    )
    .expect("quota timestamp")
}

pub(super) fn total_changes(db: &Connection) -> i64 {
    db.query_row("SELECT total_changes()", [], |row| row.get(0))
        .expect("connection changes")
}

pub(super) fn pressure(db: &Connection) -> (String, Option<String>, i64, i64, i64) {
    db.query_row(
        "SELECT state, reason_code, observed_bytes, revision, updated_at_ts
         FROM memory_storage_pressure WHERE singleton_id = 1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )
    .expect("global pressure state")
}
