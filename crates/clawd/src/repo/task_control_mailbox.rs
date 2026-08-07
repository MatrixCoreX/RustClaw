use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::AppState;

const INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS task_control_mailbox (
    task_id TEXT NOT NULL,
    control_seq INTEGER NOT NULL,
    control_id TEXT NOT NULL UNIQUE,
    action TEXT NOT NULL CHECK (action IN ('steer', 'pause', 'resume', 'cancel')),
    issued_by TEXT NOT NULL,
    issued_at INTEGER NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    payload_digest TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'applied', 'rejected')),
    applied_at INTEGER,
    result_code TEXT,
    PRIMARY KEY (task_id, control_seq)
);
CREATE INDEX IF NOT EXISTS idx_task_control_mailbox_pending
ON task_control_mailbox(task_id, status, control_seq);
"#;

#[derive(Debug, Clone)]
pub(crate) struct EnqueueTaskControl {
    pub(crate) task_id: String,
    pub(crate) action: String,
    pub(crate) issued_by: String,
    pub(crate) payload: Value,
    pub(crate) idempotency_key: Option<String>,
    pub(crate) expected_control_seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TaskControlDirective {
    pub(crate) task_id: String,
    pub(crate) control_seq: i64,
    pub(crate) control_id: String,
    pub(crate) action: String,
    pub(crate) issued_by: String,
    pub(crate) issued_at: i64,
    pub(crate) payload: Value,
    pub(crate) payload_digest: String,
    pub(crate) status: String,
}

pub(crate) fn enqueue_task_control(
    state: &AppState,
    input: EnqueueTaskControl,
) -> anyhow::Result<Option<TaskControlDirective>> {
    let task_id =
        machine_token(&input.task_id, 160).ok_or_else(|| anyhow::anyhow!("task_id_invalid"))?;
    let action = match input.action.trim() {
        "steer" | "pause" | "resume" | "cancel" => input.action.trim(),
        _ => anyhow::bail!("task_control_action_invalid"),
    };
    let issued_by = machine_token(&input.issued_by, 160)
        .ok_or_else(|| anyhow::anyhow!("task_control_issued_by_invalid"))?;
    let payload_json = serde_json::to_string(&input.payload)?;
    if payload_json.len() > 64 * 1024 {
        anyhow::bail!("task_control_payload_too_large");
    }
    let payload_digest = format!("sha256:{:x}", Sha256::digest(payload_json.as_bytes()));
    let idempotency_key = input
        .idempotency_key
        .as_deref()
        .and_then(|value| machine_token(value, 200));
    let control_id = format!(
        "ctl:{:x}",
        Sha256::digest(
            format!(
                "{task_id}\n{action}\n{}\n{payload_digest}",
                idempotency_key.as_deref().unwrap_or("")
            )
            .as_bytes()
        )
    );
    let issued_at = crate::now_ts_u64() as i64;
    crate::sqlite_busy_retry::with_sqlite_busy_retry(
        crate::sqlite_busy_retry::SqliteBusyRetryPolicy::default(),
        || {
            let mut db = state
                .core
                .db
                .get()
                .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
            db.execute_batch(INIT_SQL)?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if let Some(existing) = load_by_control_id(&tx, &control_id)? {
                tx.commit()?;
                return Ok(Some(existing));
            }
            let active = tx
                .query_row(
                    "SELECT 1 FROM tasks WHERE task_id = ?1 AND status IN ('queued', 'running') LIMIT 1",
                    params![task_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !active {
                tx.commit()?;
                return Ok(None);
            }
            let current_seq = tx.query_row(
                "SELECT COALESCE(MAX(control_seq), 0) FROM task_control_mailbox WHERE task_id = ?1",
                params![task_id],
                |row| row.get::<_, i64>(0),
            )?;
            if input
                .expected_control_seq
                .is_some_and(|expected| expected != current_seq)
            {
                anyhow::bail!("task_control_version_conflict");
            }
            let control_seq = current_seq.saturating_add(1);
            tx.execute(
                "INSERT INTO task_control_mailbox(
                    task_id, control_seq, control_id, action, issued_by, issued_at,
                    payload_json, payload_digest, status
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending')",
                params![
                    task_id,
                    control_seq,
                    control_id,
                    action,
                    issued_by,
                    issued_at,
                    payload_json,
                    payload_digest
                ],
            )?;
            let directive = load_by_control_id(&tx, &control_id)?.expect("inserted task control");
            tx.commit()?;
            Ok(Some(directive))
        },
    )
}

pub(crate) fn pending_task_control_directives(
    state: &AppState,
    task_id: &str,
    limit: usize,
) -> anyhow::Result<Vec<TaskControlDirective>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    db.execute_batch(INIT_SQL)?;
    let mut stmt = db.prepare(
        "SELECT task_id, control_seq, control_id, action, issued_by, issued_at,
                payload_json, payload_digest, status
         FROM task_control_mailbox
         WHERE task_id = ?1 AND status = 'pending'
         ORDER BY control_seq ASC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![task_id, limit.clamp(1, 64)], row_to_directive)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(crate) fn applied_task_steering_directives(
    state: &AppState,
    task_id: &str,
    limit: usize,
) -> anyhow::Result<Vec<TaskControlDirective>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
    db.execute_batch(INIT_SQL)?;
    let mut stmt = db.prepare(
        "SELECT task_id, control_seq, control_id, action, issued_by, issued_at,
                payload_json, payload_digest, status
         FROM task_control_mailbox
         WHERE task_id = ?1 AND status = 'applied' AND action = 'steer'
         ORDER BY control_seq ASC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![task_id, limit.clamp(1, 64)], row_to_directive)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(crate) fn apply_task_control_directive(
    state: &AppState,
    task_id: &str,
    control_seq: i64,
    result_code: &str,
) -> anyhow::Result<bool> {
    let result_code = machine_token(result_code, 160)
        .ok_or_else(|| anyhow::anyhow!("task_control_result_code_invalid"))?;
    crate::sqlite_busy_retry::with_sqlite_busy_retry(
        crate::sqlite_busy_retry::SqliteBusyRetryPolicy::default(),
        || {
            let db = state
                .core
                .db
                .get()
                .map_err(|error| anyhow::anyhow!("db pool: {error}"))?;
            db.execute_batch(INIT_SQL)?;
            Ok(db.execute(
                "UPDATE task_control_mailbox
                 SET status = 'applied', applied_at = ?3, result_code = ?4
                 WHERE task_id = ?1 AND control_seq = ?2 AND status = 'pending'",
                params![
                    task_id,
                    control_seq,
                    crate::now_ts_u64() as i64,
                    result_code
                ],
            )? == 1)
        },
    )
}

fn load_by_control_id(
    db: &rusqlite::Connection,
    control_id: &str,
) -> rusqlite::Result<Option<TaskControlDirective>> {
    db.query_row(
        "SELECT task_id, control_seq, control_id, action, issued_by, issued_at,
                payload_json, payload_digest, status
         FROM task_control_mailbox WHERE control_id = ?1 LIMIT 1",
        params![control_id],
        row_to_directive,
    )
    .optional()
}

fn row_to_directive(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskControlDirective> {
    let payload_json = row.get::<_, String>(6)?;
    Ok(TaskControlDirective {
        task_id: row.get(0)?,
        control_seq: row.get(1)?,
        control_id: row.get(2)?,
        action: row.get(3)?,
        issued_by: row.get(4)?,
        issued_at: row.get(5)?,
        payload: serde_json::from_str(&payload_json).unwrap_or(Value::Null),
        payload_digest: row.get(7)?,
        status: row.get(8)?,
    })
}

fn machine_token(value: &str, max_chars: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > max_chars
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.'))
    {
        return None;
    }
    Some(value.to_string())
}

#[cfg(test)]
#[path = "task_control_mailbox_tests.rs"]
mod tests;
