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
}

pub(crate) fn upsert_task_checkpoint_action(
    pool: &DbPool,
    task_id: &str,
    checkpoint_id: &str,
    tool_or_skill: &str,
    action_ref: &str,
    args: &Value,
    output_contract: Option<&Value>,
) -> anyhow::Result<()> {
    let task_id = required_text(task_id, "task_id")?;
    let checkpoint_id = required_text(checkpoint_id, "checkpoint_id")?;
    let tool_or_skill = required_machine_ref(tool_or_skill, "tool_or_skill")?;
    let action_ref = required_machine_ref(action_ref, "action_ref")?;
    if !args.is_object() {
        return Err(anyhow!("checkpoint action args must be an object"));
    }
    if output_contract.is_some_and(|value| !value.is_object()) {
        return Err(anyhow!("checkpoint output contract must be an object"));
    }
    let args_json = serde_json::to_string(args).context("serialize checkpoint action args")?;
    let output_contract_json = output_contract
        .map(serde_json::to_string)
        .transpose()
        .context("serialize checkpoint output contract")?;
    let integrity_hash = checkpoint_action_integrity_hash(
        task_id,
        checkpoint_id,
        tool_or_skill,
        action_ref,
        &args_json,
        output_contract_json.as_deref(),
    );
    let now = crate::now_ts_u64() as i64;
    let db = pool.get().context("checkpoint action db pool")?;
    db.execute_batch(INIT_TASK_CHECKPOINT_ACTION_SQL)?;
    db.execute(
        "INSERT INTO task_checkpoint_actions (
             task_id, checkpoint_id, tool_or_skill, action_ref, args_json,
             output_contract_json, integrity_hash, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
         ON CONFLICT(task_id, checkpoint_id) DO UPDATE SET
             tool_or_skill = excluded.tool_or_skill,
             action_ref = excluded.action_ref,
             args_json = excluded.args_json,
             output_contract_json = excluded.output_contract_json,
             integrity_hash = excluded.integrity_hash,
             updated_at = excluded.updated_at",
        params![
            task_id,
            checkpoint_id,
            tool_or_skill,
            action_ref,
            args_json,
            output_contract_json,
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
    let db = pool.get().context("checkpoint action db pool")?;
    db.execute_batch(INIT_TASK_CHECKPOINT_ACTION_SQL)?;
    let row = db
        .query_row(
            "SELECT tool_or_skill, action_ref, args_json, output_contract_json, integrity_hash
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
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((tool_or_skill, action_ref, args_json, output_contract_json, integrity_hash)) = row
    else {
        return Ok(None);
    };
    required_machine_ref(&tool_or_skill, "stored tool_or_skill")?;
    required_machine_ref(&action_ref, "stored action_ref")?;
    let expected_hash = checkpoint_action_integrity_hash(
        task_id,
        checkpoint_id,
        &tool_or_skill,
        &action_ref,
        &args_json,
        output_contract_json.as_deref(),
    );
    if integrity_hash != expected_hash {
        return Err(anyhow!("checkpoint action integrity mismatch"));
    }
    let args = serde_json::from_str::<Value>(&args_json).context("parse checkpoint action args")?;
    if !args.is_object() {
        return Err(anyhow!("stored checkpoint action args must be an object"));
    }
    let output_contract = output_contract_json
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .context("parse checkpoint output contract")?;
    if output_contract
        .as_ref()
        .is_some_and(|value| !value.is_object())
    {
        return Err(anyhow!(
            "stored checkpoint output contract must be an object"
        ));
    }
    Ok(Some(TaskCheckpointAction {
        task_id: task_id.to_string(),
        checkpoint_id: checkpoint_id.to_string(),
        tool_or_skill,
        action_ref,
        args,
        output_contract,
    }))
}

fn required_text<'a>(value: &'a str, field: &str) -> anyhow::Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("{field} is required"));
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
        return Err(anyhow!("{field} is not a valid machine reference"));
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
) -> String {
    let mut hasher = Sha256::new();
    for value in [
        task_id,
        checkpoint_id,
        tool_or_skill,
        action_ref,
        args_json,
        output_contract_json.unwrap_or_default(),
    ] {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
#[path = "task_checkpoint_action_tests.rs"]
mod tests;
