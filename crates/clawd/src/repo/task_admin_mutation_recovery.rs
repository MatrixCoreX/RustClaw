//! Reconcile a stale control request only from evidence bound to that request.
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) fn observed_outcome(
    db: &Connection,
    action: &str,
    target: &str,
    key: &str,
) -> anyhow::Result<Option<Value>> {
    if action == "cancel" {
        return Ok(db.query_row(
            "SELECT user_id,chat_id,channel FROM tasks WHERE task_id=?1 AND status='canceled'",
            [target],
            |row| Ok(json!({ "status":"task_already_cancelled","canceled":0,"already_terminal":true,
                "task_id":target,"user_id":row.get::<_,i64>(0)?,"chat_id":row.get::<_,i64>(1)?,
                "channel":row.get::<_,String>(2)? })),
        ).optional()?);
    }
    if !matches!(action, "steer" | "pause") {
        return Ok(None);
    }
    if !db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='task_control_mailbox')", [], |row| row.get::<_,bool>(0))? {
        return Ok(None);
    }
    let mut stmt = db.prepare(
        "SELECT control_id,control_seq,status,payload_digest FROM task_control_mailbox
        WHERE task_id=?1 AND action=?2 AND issued_by='task_control_api' ORDER BY control_seq DESC",
    )?;
    let mut rows = stmt.query(params![target, action])?;
    while let Some(row) = rows.next()? {
        let digest: String = row.get(3)?;
        let expected = format!(
            "ctl:{:x}",
            Sha256::digest(format!("{target}\n{action}\n{key}\n{digest}").as_bytes())
        );
        let control_id: String = row.get(0)?;
        if control_id != expected {
            continue;
        }
        let state: String = row.get(2)?;
        if state == "rejected" {
            return Ok(None);
        }
        return Ok(Some(json!({
            "schema_version":1,
            "status":if action == "steer" { "task_steering_accepted" } else { "task_pause_requested" },
            "task_id":target, "control_id":control_id, "control_seq":row.get::<_,i64>(1)?,
            "control_status":state, "payload_digest":digest, "recovered":true
        })));
    }
    Ok(None)
}
