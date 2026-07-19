use anyhow::{anyhow, Context};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::db_init::DbPool;

const INIT_TASK_CHECKPOINT_ACTION_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS task_checkpoint_actions (
    task_id              TEXT NOT NULL,
    checkpoint_id        TEXT NOT NULL,
    tool_or_skill        TEXT NOT NULL,
    action_ref           TEXT NOT NULL,
    args_json            TEXT NOT NULL,
    output_contract_json TEXT,
    continuation_actions_json TEXT,
    integrity_hash       TEXT NOT NULL,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    PRIMARY KEY (task_id, checkpoint_id),
    FOREIGN KEY (task_id) REFERENCES tasks(task_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_task_checkpoint_actions_updated
    ON task_checkpoint_actions(updated_at);
"#;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TaskCheckpointAction {
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) tool_or_skill: String,
    pub(crate) action_ref: String,
    pub(crate) args: Value,
    pub(crate) output_contract: Option<Value>,
    pub(crate) continuation_actions: Option<Value>,
}

pub(crate) fn upsert_task_checkpoint_action(
    pool: &DbPool,
    task_id: &str,
    checkpoint_id: &str,
    tool_or_skill: &str,
    action_ref: &str,
    args: &Value,
    output_contract: Option<&Value>,
    continuation_actions: Option<&Value>,
) -> anyhow::Result<()> {
    let task_id = required_text(task_id, "task_id")?;
    let checkpoint_id = required_text(checkpoint_id, "checkpoint_id")?;
    let tool_or_skill = required_machine_ref(tool_or_skill, "tool_or_skill")?;
    let action_ref = required_machine_ref(action_ref, "action_ref")?;
    if !args.is_object() {
        return Err(anyhow!("checkpoint_action_args_not_object"));
    }
    if output_contract.is_some_and(|value| !value.is_object()) {
        return Err(anyhow!("checkpoint_output_contract_not_object"));
    }
    if continuation_actions.is_some_and(|value| !value.is_array()) {
        return Err(anyhow!("checkpoint_continuation_actions_not_array"));
    }
    let args_json =
        serde_json::to_string(args).context("checkpoint_action_args_serialize_failed")?;
    let output_contract_json = output_contract
        .map(serde_json::to_string)
        .transpose()
        .context("checkpoint_output_contract_serialize_failed")?;
    let continuation_actions_json = continuation_actions
        .map(serde_json::to_string)
        .transpose()
        .context("checkpoint_continuation_actions_serialize_failed")?;
    let integrity_hash = checkpoint_action_integrity_hash(
        task_id,
        checkpoint_id,
        tool_or_skill,
        action_ref,
        &args_json,
        output_contract_json.as_deref(),
        continuation_actions_json.as_deref(),
    );
    let now = crate::now_ts_u64() as i64;
    let db = pool.get().context("checkpoint_action_db_pool_failed")?;
    ensure_task_checkpoint_action_schema(&db)?;
    db.execute(
        "INSERT INTO task_checkpoint_actions (
             task_id, checkpoint_id, tool_or_skill, action_ref, args_json,
             output_contract_json, continuation_actions_json, integrity_hash, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT(task_id, checkpoint_id) DO UPDATE SET
             tool_or_skill = excluded.tool_or_skill,
             action_ref = excluded.action_ref,
             args_json = excluded.args_json,
             output_contract_json = excluded.output_contract_json,
             continuation_actions_json = excluded.continuation_actions_json,
             integrity_hash = excluded.integrity_hash,
             updated_at = excluded.updated_at",
        params![
            task_id,
            checkpoint_id,
            tool_or_skill,
            action_ref,
            args_json,
            output_contract_json,
            continuation_actions_json,
            integrity_hash,
            now,
        ],
    )?;
    Ok(())
}

pub(crate) fn load_task_checkpoint_action(
    pool: &DbPool,
    task_id: &str,
    checkpoint_id: &str,
) -> anyhow::Result<Option<TaskCheckpointAction>> {
    let task_id = required_text(task_id, "task_id")?;
    let checkpoint_id = required_text(checkpoint_id, "checkpoint_id")?;
    let db = pool.get().context("checkpoint_action_db_pool_failed")?;
    ensure_task_checkpoint_action_schema(&db)?;
    let row = db
        .query_row(
            "SELECT tool_or_skill, action_ref, args_json, output_contract_json,
                    continuation_actions_json, integrity_hash
             FROM task_checkpoint_actions
             WHERE task_id = ?1 AND checkpoint_id = ?2
             LIMIT 1",
            params![task_id, checkpoint_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((
        tool_or_skill,
        action_ref,
        args_json,
        output_contract_json,
        continuation_actions_json,
        integrity_hash,
    )) = row
    else {
        return Ok(None);
    };
    required_machine_ref(&tool_or_skill, "stored_tool_or_skill")?;
    required_machine_ref(&action_ref, "stored_action_ref")?;
    let expected_hash = checkpoint_action_integrity_hash(
        task_id,
        checkpoint_id,
        &tool_or_skill,
        &action_ref,
        &args_json,
        output_contract_json.as_deref(),
        continuation_actions_json.as_deref(),
    );
    if integrity_hash != expected_hash {
        return Err(anyhow!("checkpoint_action_integrity_mismatch"));
    }
    let args =
        serde_json::from_str::<Value>(&args_json).context("checkpoint_action_args_parse_failed")?;
    if !args.is_object() {
        return Err(anyhow!("stored_checkpoint_action_args_not_object"));
    }
    let output_contract = output_contract_json
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .context("checkpoint_output_contract_parse_failed")?;
    if output_contract
        .as_ref()
        .is_some_and(|value| !value.is_object())
    {
        return Err(anyhow!("stored_checkpoint_output_contract_not_object"));
    }
    let continuation_actions = continuation_actions_json
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .context("checkpoint_continuation_actions_parse_failed")?;
    if continuation_actions
        .as_ref()
        .is_some_and(|value| !value.is_array())
    {
        return Err(anyhow!("stored_checkpoint_continuation_actions_not_array"));
    }
    Ok(Some(TaskCheckpointAction {
        task_id: task_id.to_string(),
        checkpoint_id: checkpoint_id.to_string(),
        tool_or_skill,
        action_ref,
        args,
        output_contract,
        continuation_actions,
    }))
}

fn ensure_task_checkpoint_action_schema(db: &rusqlite::Connection) -> anyhow::Result<()> {
    db.execute_batch(INIT_TASK_CHECKPOINT_ACTION_SQL)?;
    crate::app_helpers::ensure_column_exists(
        db,
        "task_checkpoint_actions",
        "continuation_actions_json",
        concat!(
            "ALTER TABLE ",
            "task_checkpoint_actions ",
            "ADD COLUMN ",
            "continuation_actions_json ",
            "TEXT"
        ),
    )
}

fn required_text<'a>(value: &'a str, field: &str) -> anyhow::Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("{field}_required"));
    }
    Ok(value)
}

fn required_machine_ref<'a>(value: &'a str, field: &str) -> anyhow::Result<&'a str> {
    let value = required_text(value, field)?;
    if value.len() > 256
        || !value.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | '.' | ':')
        })
    {
        return Err(anyhow!("{field}_invalid_machine_reference"));
    }
    Ok(value)
}

fn checkpoint_action_integrity_hash(
    task_id: &str,
    checkpoint_id: &str,
    tool_or_skill: &str,
    action_ref: &str,
    args_json: &str,
    output_contract_json: Option<&str>,
    continuation_actions_json: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    for value in [
        task_id,
        checkpoint_id,
        tool_or_skill,
        action_ref,
        args_json,
        output_contract_json.unwrap_or_default(),
        continuation_actions_json.unwrap_or_default(),
    ] {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
#[path = "task_checkpoint_action_tests.rs"]
mod tests;
