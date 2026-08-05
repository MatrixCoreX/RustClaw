use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{AppState, ClaimedTask};

use super::ContextCompactionPlan;

const MIGRATION_ID: &str = "013_context_compaction_lifecycle_v1";
const MIGRATION_SQL: &str =
    include_str!("../../../../migrations/013_context_compaction_lifecycle.sql");
const LEASE_SECONDS: i64 = 600;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ContextCompactionLease {
    pub(crate) principal_id: String,
    pub(crate) conversation_ref: String,
    pub(crate) lineage_id: String,
    pub(crate) owner: String,
    pub(crate) base_generation: u64,
    pub(crate) generation: u64,
    pub(crate) snapshot_digest: String,
    pub(crate) snapshot_task_row_id: i64,
    pub(crate) snapshot_event_ranges: Value,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ContextCompactionCommit {
    pub(crate) record: Value,
    pub(crate) uncovered_tail_task_count: u64,
}

pub(crate) fn ensure_context_compaction_lifecycle_schema(db: &Connection) -> anyhow::Result<()> {
    crate::repo::ensure_principal_ownership_schema(db)?;
    if let Some(applied) = migration_digest(db)? {
        anyhow::ensure!(
            applied == migration_manifest_digest(),
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
    db.execute_batch(MIGRATION_SQL)?;
    db.execute(
        "INSERT INTO runtime_schema_migrations(migration_id, schema_digest, applied_at)
         VALUES (?1, ?2, ?3) ON CONFLICT(migration_id) DO NOTHING",
        params![MIGRATION_ID, migration_manifest_digest(), crate::now_ts()],
    )?;
    Ok(())
}

pub(crate) fn begin_context_compaction(
    state: &AppState,
    task: &ClaimedTask,
    plan: &mut ContextCompactionPlan,
) -> anyhow::Result<ContextCompactionLease> {
    let mut db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("context_compaction_db_pool:{error}"))?;
    ensure_context_compaction_lifecycle_schema(&db)?;
    let principal_id = task_principal_id(&db, task)?;
    let conversation_ref = task_conversation_ref(&principal_id, task)?;
    let lineage_id = format!(
        "lineage_{}",
        short_digest(format!("{principal_id}\0{conversation_ref}").as_bytes())
    );
    let snapshot_event_ranges = plan.source_snapshot();
    let snapshot_digest = digest_json(&snapshot_event_ranges);
    let now = crate::now_ts_u64() as i64;
    let owner = format!("compaction:{}:{}", task.task_id, uuid::Uuid::new_v4());

    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let snapshot_task_row_id = conversation_task_head(&tx, &principal_id, &conversation_ref, task)?;
    tx.execute(
        "INSERT INTO context_compaction_states(
            principal_id, conversation_ref, lineage_id, generation, updated_at_ts
         ) VALUES (?1, ?2, ?3, 0, ?4)
         ON CONFLICT(principal_id, conversation_ref) DO NOTHING",
        params![principal_id, conversation_ref, lineage_id, now],
    )?;
    let state_row: (i64, Option<String>, Option<i64>) = tx.query_row(
        "SELECT generation, lease_owner, lease_expires_at_ts
         FROM context_compaction_states
         WHERE principal_id = ?1 AND conversation_ref = ?2",
        params![principal_id, conversation_ref],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if state_row
        .1
        .as_deref()
        .is_some_and(|active_owner| active_owner != owner)
        && state_row.2.is_some_and(|expires| expires > now)
    {
        anyhow::bail!("context_compaction_lease_busy");
    }
    let base_generation = u64::try_from(state_row.0.max(0)).unwrap_or(0);
    let generation = base_generation.saturating_add(1);
    let changed = tx.execute(
        "UPDATE context_compaction_states
         SET lease_owner = ?1, lease_expires_at_ts = ?2, updated_at_ts = ?3
         WHERE principal_id = ?4 AND conversation_ref = ?5 AND generation = ?6
           AND (lease_owner IS NULL OR lease_expires_at_ts IS NULL OR lease_expires_at_ts <= ?3)",
        params![
            owner,
            now.saturating_add(LEASE_SECONDS),
            now,
            principal_id,
            conversation_ref,
            base_generation as i64,
        ],
    )?;
    anyhow::ensure!(changed == 1, "context_compaction_lease_busy");
    tx.commit()?;
    plan.set_generation(generation);

    Ok(ContextCompactionLease {
        principal_id,
        conversation_ref,
        lineage_id,
        owner,
        base_generation,
        generation,
        snapshot_digest,
        snapshot_task_row_id,
        snapshot_event_ranges,
    })
}

pub(crate) fn complete_context_compaction(
    state: &AppState,
    task: &ClaimedTask,
    lease: &ContextCompactionLease,
    record: &Value,
) -> anyhow::Result<ContextCompactionCommit> {
    let mut db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("context_compaction_db_pool:{error}"))?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let uncovered_tail_task_count = conversation_task_count_after(
        &tx,
        &lease.principal_id,
        &lease.conversation_ref,
        task,
        lease.snapshot_task_row_id,
    )?;
    let now = crate::now_ts_u64() as i64;
    let record_id = record
        .get("compaction_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("context_compaction:unknown")
        .to_string();
    let mut durable_record = record.clone();
    if let Some(object) = durable_record.as_object_mut() {
        object.insert(
            "lifecycle".to_string(),
            json!({
                "schema_version": 1,
                "lineage_id": lease.lineage_id,
                "base_generation": lease.base_generation,
                "generation": lease.generation,
                "snapshot_digest": lease.snapshot_digest,
                "snapshot_task_row_id": lease.snapshot_task_row_id,
                "uncovered_tail_task_count": uncovered_tail_task_count,
                "tail_policy": "preserved_for_next_turn",
                "lease_policy": "single_writer_generation_cas",
            }),
        );
    }
    let changed = tx.execute(
        "UPDATE context_compaction_states
         SET generation = ?1, lease_owner = NULL, lease_expires_at_ts = NULL,
             last_snapshot_digest = ?2, last_snapshot_task_row_id = ?3,
             last_record_id = ?4, revision = revision + 1, updated_at_ts = ?5
         WHERE principal_id = ?6 AND conversation_ref = ?7
           AND generation = ?8 AND lease_owner = ?9",
        params![
            lease.generation as i64,
            lease.snapshot_digest,
            lease.snapshot_task_row_id,
            record_id,
            now,
            lease.principal_id,
            lease.conversation_ref,
            lease.base_generation as i64,
            lease.owner,
        ],
    )?;
    anyhow::ensure!(changed == 1, "context_compaction_generation_cas_conflict");
    tx.execute(
        "INSERT INTO context_compaction_records(
            record_id, principal_id, conversation_ref, lineage_id, generation,
            source_task_id, snapshot_digest, snapshot_task_row_id,
            snapshot_event_ranges_json, uncovered_tail_task_count, record_json,
            status, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'valid', ?12)",
        params![
            record_id,
            lease.principal_id,
            lease.conversation_ref,
            lease.lineage_id,
            lease.generation as i64,
            task.task_id,
            lease.snapshot_digest,
            lease.snapshot_task_row_id,
            lease.snapshot_event_ranges.to_string(),
            uncovered_tail_task_count as i64,
            durable_record.to_string(),
            now,
        ],
    )?;
    tx.commit()?;
    Ok(ContextCompactionCommit {
        record: durable_record,
        uncovered_tail_task_count,
    })
}

pub(crate) fn abandon_context_compaction(
    state: &AppState,
    lease: &ContextCompactionLease,
) -> anyhow::Result<()> {
    let db = state.core.db.get()?;
    db.execute(
        "UPDATE context_compaction_states
         SET lease_owner = NULL, lease_expires_at_ts = NULL, updated_at_ts = ?1
         WHERE principal_id = ?2 AND conversation_ref = ?3
           AND generation = ?4 AND lease_owner = ?5",
        params![
            crate::now_ts_u64() as i64,
            lease.principal_id,
            lease.conversation_ref,
            lease.base_generation as i64,
            lease.owner,
        ],
    )?;
    Ok(())
}

pub(crate) fn invalidate_compactions_after_rewind(
    state: &AppState,
    task: &ClaimedTask,
    source_task_id: &str,
) -> anyhow::Result<usize> {
    let db = state.core.db.get()?;
    ensure_context_compaction_lifecycle_schema(&db)?;
    let principal_id = task_principal_id(&db, task)?;
    let conversation_ref = task_conversation_ref(&principal_id, task)?;
    let source_head = db
        .query_row(
            "SELECT rowid FROM tasks WHERE task_id = ?1 AND principal_id = ?2 LIMIT 1",
            params![source_task_id, principal_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("context_compaction_rewind_source_missing"))?;
    let now = crate::now_ts_u64() as i64;
    let changed = db.execute(
        "UPDATE context_compaction_records
         SET status = 'invalidated', invalidation_reason_code = 'conversation_rewind',
             invalidated_at_ts = ?1
         WHERE principal_id = ?2 AND conversation_ref = ?3 AND status = 'valid'
           AND snapshot_task_row_id > ?4",
        params![now, principal_id, conversation_ref, source_head],
    )?;
    db.execute(
        "UPDATE context_compaction_states
         SET generation = COALESCE((
                SELECT MAX(generation) FROM context_compaction_records
                WHERE principal_id = ?1 AND conversation_ref = ?2 AND status = 'valid'
             ), 0),
             lease_owner = NULL, lease_expires_at_ts = NULL,
             revision = revision + 1, updated_at_ts = ?3
         WHERE principal_id = ?1 AND conversation_ref = ?2",
        params![principal_id, conversation_ref, now],
    )?;
    Ok(changed)
}

fn task_principal_id(db: &Connection, task: &ClaimedTask) -> anyhow::Result<String> {
    if let Some(principal_id) = db
        .query_row(
            "SELECT principal_id FROM tasks WHERE task_id = ?1 LIMIT 1",
            [&task.task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(principal_id);
    }
    let user_key = task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("context_compaction_principal_missing"))?;
    crate::repo::auth::principal_id_for_user_key(db, user_key)?
        .ok_or_else(|| anyhow::anyhow!("context_compaction_principal_missing"))
}

fn task_conversation_ref(principal_id: &str, task: &ClaimedTask) -> anyhow::Result<String> {
    let conversation_id =
        crate::conversation_state::task_conversation_id(task).unwrap_or_else(|| {
            format!(
                "legacy_{}",
                short_digest(
                    format!(
                        "{}\0{}\0{}",
                        task.channel,
                        task.external_chat_id.as_deref().unwrap_or(""),
                        task.chat_id
                    )
                    .as_bytes()
                )
            )
        });
    crate::memory::scope::conversation_scope_ref(principal_id, &conversation_id)
}

fn conversation_task_head(
    db: &Connection,
    principal_id: &str,
    conversation_ref: &str,
    task: &ClaimedTask,
) -> anyhow::Result<i64> {
    let current_ref = task_conversation_ref(principal_id, task)?;
    if current_ref != conversation_ref {
        anyhow::bail!("context_compaction_conversation_changed");
    }
    let conversation_id = crate::conversation_state::task_conversation_id(task);
    let head = if let Some(conversation_id) = conversation_id {
        db.query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM tasks
             WHERE principal_id = ?1
               AND json_extract(payload_json, '$.conversation_id') = ?2",
            params![principal_id, conversation_id],
            |row| row.get(0),
        )?
    } else {
        db.query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM tasks
             WHERE principal_id = ?1 AND channel = ?2 AND chat_id = ?3",
            params![principal_id, task.channel, task.chat_id],
            |row| row.get(0),
        )?
    };
    Ok(head)
}

fn conversation_task_count_after(
    db: &Connection,
    principal_id: &str,
    conversation_ref: &str,
    task: &ClaimedTask,
    snapshot_task_row_id: i64,
) -> anyhow::Result<u64> {
    let current_ref = task_conversation_ref(principal_id, task)?;
    if current_ref != conversation_ref {
        anyhow::bail!("context_compaction_conversation_changed");
    }
    let count = if let Some(conversation_id) = crate::conversation_state::task_conversation_id(task)
    {
        db.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE principal_id = ?1 AND rowid > ?2
               AND json_extract(payload_json, '$.conversation_id') = ?3",
            params![principal_id, snapshot_task_row_id, conversation_id],
            |row| row.get::<_, i64>(0),
        )?
    } else {
        db.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE principal_id = ?1 AND rowid > ?2 AND channel = ?3 AND chat_id = ?4",
            params![
                principal_id,
                snapshot_task_row_id,
                task.channel,
                task.chat_id
            ],
            |row| row.get::<_, i64>(0),
        )?
    };
    Ok(u64::try_from(count.max(0)).unwrap_or(0))
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

fn migration_manifest_digest() -> String {
    digest_bytes(MIGRATION_SQL.as_bytes())
}

fn digest_json(value: &Value) -> String {
    digest_bytes(value.to_string().as_bytes())
}

fn digest_bytes(value: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(value))
}

fn short_digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))[..24].to_string()
}

#[cfg(test)]
#[path = "context_compaction_lifecycle_tests.rs"]
mod tests;
