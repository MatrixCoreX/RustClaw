//! Principal-scoped, reversible disposition of failed jobs. Archiving never deletes evidence.
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn token(id: &str, checkpoint: &str, updated: i64, error: &Option<String>) -> String {
    let raw = serde_json::to_vec(&(id, checkpoint, updated, error)).unwrap_or_default();
    format!("{:x}", Sha256::digest(raw))
}

pub(crate) fn list(db: &Connection, principal: &str) -> anyhow::Result<Value> {
    let mut statement = db.prepare("SELECT job_id,job_kind,status,attempt,error_code,retryable,checkpoint_json,updated_at_ts
        FROM memory_jobs WHERE principal_id=?1 AND status='failed' ORDER BY updated_at_ts DESC,job_id LIMIT 100")?;
    let rows = statement
        .query_map([principal], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, bool>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let jobs = rows.into_iter().map(|(id, kind, status, attempt, error, retryable, checkpoint, updated)| {
        let value: Value = serde_json::from_str(&checkpoint).unwrap_or(Value::Null);
        json!({"job_id":id,"job_kind":kind,"status":status,"attempt":attempt,"error_code":error,
            "retryable":retryable,"archived":value.pointer("/disposition/archived").and_then(Value::as_bool).unwrap_or(false),
            "updated_at_ts":updated,"review_token":token(&id,&checkpoint,updated,&error)})
    }).collect::<Vec<_>>();
    let total: u64 = db.query_row(
        "SELECT COUNT(*) FROM memory_jobs WHERE principal_id=?1 AND status='failed'",
        [principal],
        |r| r.get(0),
    )?;
    Ok(json!({"schema_version":1,"jobs":jobs,"total_failed":total,"limit":100}))
}

pub(crate) fn disposition(
    db: &mut Connection,
    principal: &str,
    id: &str,
    expected: &str,
    action: &str,
) -> anyhow::Result<Value> {
    anyhow::ensure!(
        matches!(action, "archive" | "unarchive" | "retry"),
        "memory_job_review_invalid_action"
    );
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row = tx.query_row("SELECT checkpoint_json,updated_at_ts,error_code,status FROM memory_jobs WHERE principal_id=?1 AND job_id=?2",
        params![principal,id], |r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,String>(3)?))).optional()?;
    let (checkpoint, updated, error, status) =
        row.ok_or_else(|| anyhow::anyhow!("memory_job_review_not_found"))?;
    anyhow::ensure!(
        status == "failed" && token(id, &checkpoint, updated, &error) == expected,
        "memory_job_review_conflict"
    );
    let mut value: Value = serde_json::from_str(&checkpoint)?;
    anyhow::ensure!(value.is_object(), "memory_job_review_invalid_checkpoint");
    if action == "retry" {
        anyhow::ensure!(
            value
                .pointer("/disposition/archived")
                .and_then(Value::as_bool)
                != Some(true),
            "memory_job_review_archived"
        );
    } else {
        value["disposition"] = json!({"archived":action=="archive","reason_code":"user_reviewed_failure","at_ts":crate::now_ts_u64()});
        tx.execute("UPDATE memory_jobs SET checkpoint_json=?3,updated_at_ts=?4 WHERE principal_id=?1 AND job_id=?2",
            params![principal,id,value.to_string(),crate::now_ts_u64() as i64])?;
    }
    tx.commit()?;
    Ok(json!({"schema_version":1,"status":"ok","action":action}))
}

#[cfg(test)]
#[path = "job_review_tests.rs"]
mod tests;
