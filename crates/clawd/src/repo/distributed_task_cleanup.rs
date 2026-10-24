//! Result retirement for one host-enrolled, acknowledged distributed task.
use anyhow::ensure;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{task_result_retention::DeliveryCleanupEnrollment, AppState};

fn table_exists(db: &rusqlite::Connection, name: &str) -> anyhow::Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |row| row.get(0),
    )?)
}

pub(crate) fn validate_task(
    db: &rusqlite::Connection,
    task_id: &str,
    enrollment: &DeliveryCleanupEnrollment,
) -> anyhow::Result<Option<Value>> {
    let row = db.query_row(
        "SELECT kind, status, idempotency_key, payload_json, result_json FROM tasks WHERE task_id=?1",
        [task_id], |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?,
            row.get::<_,Option<String>>(2)?, row.get::<_,String>(3)?, row.get::<_,Option<String>>(4)?)))
        .optional()?.ok_or_else(|| anyhow::anyhow!("work_cleanup_task_missing"))?;
    let payload: Value = serde_json::from_str(&row.3)?;
    ensure!(
        row.0 == "run_skill"
            && matches!(
                row.1.as_str(),
                "succeeded" | "failed" | "canceled" | "timeout"
            )
            && row.2.as_deref() == Some(enrollment.idempotency_key().as_str())
            && enrollment.matches_payload(&payload),
        "work_cleanup_task_binding_invalid"
    );
    Ok(row.4.as_deref().map(serde_json::from_str).transpose()?)
}

pub(crate) fn retire_result(
    state: &AppState,
    task_id: &str,
    enrollment: &DeliveryCleanupEnrollment,
) -> anyhow::Result<()> {
    ensure!(
        crate::task_result_retention::read_enrollment(&state.skill_rt.workspace_root, task_id)?
            .as_ref()
            == Some(enrollment),
        "work_cleanup_enrollment_missing"
    );
    let mut db = state.core.db.get()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let result = validate_task(&tx, task_id, enrollment)?;
    // Keep an explicit tombstone and digests, never pretend the retired archive
    // is still replayable or rewrite hashes of an unrelated audit stream.
    let summary = if result
        .as_ref()
        .and_then(|r| r.get("result_retention"))
        .and_then(Value::as_str)
        == Some("removed_after_delivery_ack")
    {
        result.unwrap_or(Value::Null)
    } else {
        let (event_count, final_event_hash) = if table_exists(&tx, "task_event_archive")? {
            (tx.query_row("SELECT COUNT(*) FROM task_event_archive WHERE task_id=?1", [task_id], |r| r.get::<_,u64>(0))?,
             tx.query_row("SELECT event_hash FROM task_event_archive WHERE task_id=?1 ORDER BY seq DESC LIMIT 1", [task_id], |r| r.get::<_,String>(0)).optional()?)
        } else {
            (0, None)
        };
        json!({"schema_version":1, "result_retention":"removed_after_delivery_ack",
            "work_id":enrollment.work_id, "attempt":enrollment.attempt,
            "result_sha256":hex::encode(Sha256::digest(result.unwrap_or(Value::Null).to_string().as_bytes())),
            "retired_event_count":event_count,"final_event_hash":final_event_hash})
    };
    let payload = json!({"_task_source":"distributed_work", "_requested_capability":enrollment.capability,
        "_delivery_cleanup":enrollment});
    tx.execute(
        "UPDATE tasks SET payload_json=?2, result_json=?3, error_text=NULL WHERE task_id=?1",
        params![task_id, payload.to_string(), summary.to_string()],
    )?;
    // All are task-scoped host projections. Shared memories, audit_logs,
    // auth/credentials, conversations and skill-private data are untouched.
    for table in [
        "task_event_stream",
        "task_event_archive",
        "task_event_snapshots",
        "task_event_artifacts",
        "task_checkpoint_actions",
        "task_plans",
        "task_control_mailbox",
    ] {
        if table_exists(&tx, table)? {
            tx.execute(&format!("DELETE FROM {table} WHERE task_id=?1"), [task_id])?;
        }
    }
    if table_exists(&tx, "task_mutation_ledger")? {
        tx.execute(
            "UPDATE task_mutation_ledger SET receipt_json=NULL, verification_json=NULL,
            reconciliation_json=NULL WHERE task_id=?1",
            [task_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[path = "distributed_task_cleanup_tests.rs"]
mod tests;
