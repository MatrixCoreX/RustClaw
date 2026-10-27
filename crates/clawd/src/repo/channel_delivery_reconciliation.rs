//! Explicit owner decisions over the existing delivery receipts and dispatch leases.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    ConfirmReceived,
    RetryUnknown,
    RetryFailed,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Decision {
    pub operation: Operation,
    pub snapshot_digest: String,
    pub decision_id: uuid::Uuid,
    #[serde(default)]
    pub confirmed_duplicate_risk: bool,
}

#[derive(Clone, Serialize)]
struct Claim {
    delivery_id: String,
    channel: String,
    adapter: String,
    state: String,
    expires_at: i64,
}
#[derive(Clone, Serialize)]
struct Part {
    key: String,
    receipt: Option<ChannelDeliveryReceipt>,
    claim: Option<Claim>,
}
#[derive(Serialize)]
struct Snapshot {
    parent: Part,
    parts: Vec<Part>,
}

fn exists(db: &Connection, table: &str) -> anyhow::Result<bool> {
    Ok(db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn part(db: &Connection, key: String) -> anyhow::Result<Part> {
    let receipt = load_channel_delivery_receipt_from_db(db, &key)?;
    let claim = db.query_row("SELECT delivery_id,channel,adapter,state,lease_expires_at_ts FROM channel_delivery_dispatch_claims WHERE idempotency_key=?1", [&key], |r| Ok(Claim {
        delivery_id:r.get(0)?,channel:r.get(1)?,adapter:r.get(2)?,state:r.get(3)?,expires_at:r.get(4)?,
    })).optional()?;
    Ok(Part {
        key,
        receipt,
        claim,
    })
}

fn snapshot(db: &Connection, task_id: &str) -> anyhow::Result<Option<Snapshot>> {
    if !exists(db, "channel_delivery_dispatch_claims")? {
        return Ok(None);
    }
    let delivery_id = format!("delivery:{task_id}:terminal");
    let key: Option<String> = db.query_row("SELECT idempotency_key FROM channel_delivery_dispatch_claims WHERE delivery_id=?1 LIMIT 1", [&delivery_id], |r| r.get(0)).optional()?;
    let Some(key) = key else {
        return Ok(None);
    };
    let parent = part(db, key.clone())?;
    let keys: Option<String> = if exists(db, "channel_delivery_part_plans")? {
        db.query_row("SELECT part_keys_json FROM channel_delivery_part_plans WHERE idempotency_key=?1 AND task_id=?2", params![key,task_id], |r| r.get(0)).optional()?
    } else {
        None
    };
    let keys: Vec<String> = keys
        .map(|v| serde_json::from_str(&v))
        .transpose()?
        .unwrap_or_else(|| vec![key]);
    if keys.is_empty() || keys.len() > 128 {
        anyhow::bail!("delivery_part_plan_invalid");
    }
    let parts = keys
        .into_iter()
        .map(|key| part(db, key))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Some(Snapshot { parent, parts }))
}

fn accepted(part: &Part) -> bool {
    part.receipt.as_ref().is_some_and(|r| {
        matches!(
            r.status,
            ChannelDeliveryStatus::Accepted
                | ChannelDeliveryStatus::Delivered
                | ChannelDeliveryStatus::Read
        )
    })
}
fn active(part: &Part, now: i64) -> bool {
    part.claim
        .as_ref()
        .is_some_and(|c| c.state == "dispatching" && c.expires_at > now)
}
fn retryable(part: &Part) -> bool {
    !accepted(part)
        && part
            .receipt
            .as_ref()
            .is_some_and(|r| r.retryable && !ambiguous_receipt(r))
}
fn ambiguous_receipt(r: &ChannelDeliveryReceipt) -> bool {
    matches!(
        r.error_code.as_deref(),
        Some(
            "channel.provider.transport"
                | "channel.provider.invalid_response"
                | "channel.provider.unknown"
                | "channel_delivery_part_query_required"
        )
    )
}
fn unknown(part: &Part, now: i64) -> bool {
    if accepted(part) || active(part, now) {
        return false;
    }
    if let Some(r) = &part.receipt {
        return ambiguous_receipt(r);
    }
    part.claim
        .as_ref()
        .is_some_and(|c| c.state == "query_required" || c.state == "dispatching")
}
fn digest(value: &impl Serialize) -> anyhow::Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn status_without_dispatch(
    db: &Connection,
    task_id: &str,
    now: i64,
) -> anyhow::Result<&'static str> {
    if exists(db, "channel_terminal_delivery_outbox")? {
        let queued = db.query_row(
            "SELECT state, lease_expires_at_ts FROM channel_terminal_delivery_outbox WHERE task_id=?1",
            [task_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        ).optional()?;
        if let Some((state, expires)) = queued {
            return Ok(match state.as_str() {
                "pending" => "pending",
                "dispatching" if expires > now => "in_progress",
                "dispatching" => "pending",
                _ => "unavailable",
            });
        }
    }
    if exists(db, "tasks")? {
        let status = db
            .query_row(
                "SELECT status FROM tasks WHERE task_id=?1",
                [task_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if matches!(status.as_deref(), Some("queued" | "running")) {
            return Ok("pending");
        }
    }
    Ok("unavailable")
}

fn provider_proof_status(s: &Snapshot) -> Option<&'static str> {
    // Multipart webhook IDs belong to the parent. Its callback aggregate is
    // authoritative even when immutable child receipts still say accepted.
    let parent = s.parent.receipt.as_ref().map(|receipt| receipt.status);
    if parent == Some(ChannelDeliveryStatus::Read)
        || s.parts.iter().all(|part| {
            part.receipt
                .as_ref()
                .is_some_and(|r| r.status == ChannelDeliveryStatus::Read)
        })
    {
        Some("read")
    } else if matches!(parent, Some(ChannelDeliveryStatus::Delivered))
        || s.parts.iter().all(|part| {
            part.receipt.as_ref().is_some_and(|r| {
                matches!(
                    r.status,
                    ChannelDeliveryStatus::Delivered | ChannelDeliveryStatus::Read
                )
            })
        })
    {
        Some("delivered")
    } else {
        None
    }
}

pub(crate) fn inspect(db: &Connection, task_id: &str, now: i64) -> anyhow::Result<Value> {
    let Some(s) = snapshot(db, task_id)? else {
        return Ok(
            json!({"schema_version":1,"status":status_without_dispatch(db, task_id, now)?,"can_reconcile":false}),
        );
    };
    let count = s.parts.len();
    let accepted_count = s.parts.iter().filter(|p| accepted(p)).count();
    let unknown_count = s.parts.iter().filter(|p| unknown(p, now)).count();
    let retryable_count = s.parts.iter().filter(|p| retryable(p)).count();
    let in_progress = active(&s.parent, now) || s.parts.iter().any(|p| active(p, now));
    let user_confirmed=exists(db,"channel_delivery_reconciliations")? && db.query_row("SELECT 1 FROM channel_delivery_reconciliations WHERE task_id=?1 AND operation='confirm_received' LIMIT 1",[task_id], |_|Ok(())).optional()?.is_some();
    let status = if in_progress {
        "in_progress"
    } else if unknown_count > 0 || (unknown(&s.parent, now) && accepted_count < count) {
        "query_required"
    } else if accepted_count == count {
        if let Some(status) = provider_proof_status(&s) {
            status
        } else if user_confirmed {
            "user_confirmed"
        } else {
            "accepted"
        }
    } else if accepted_count > 0 {
        "partial"
    } else {
        "failed"
    };
    Ok(
        json!({"schema_version":1,"status":status,"part_count":count,"accepted_count":accepted_count,
        "unknown_count":unknown_count,"retryable_count":retryable_count,"can_reconcile":!in_progress && (unknown_count>0 || retryable_count>0),
        "snapshot_digest":digest(&s)?,"provider_query":"callback_only_or_unsupported","user_confirmation_recorded":user_confirmed,
        "parts":s.parts.iter().enumerate().map(|(i,p)| json!({"part_index":i,"status":if accepted(p){"accepted"}else if unknown(p,now){"unknown"}else if active(p,now){"in_progress"}else if p.receipt.is_none(){"pending"}else{"failed"},"retryable":retryable(p)})).collect::<Vec<_>>() }),
    )
}

fn ensure_decision_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS channel_delivery_reconciliations (
        task_id TEXT NOT NULL, decision_id TEXT NOT NULL, actor_principal TEXT NOT NULL,
        request_digest TEXT NOT NULL, operation TEXT NOT NULL, snapshot_digest TEXT NOT NULL,
        recorded_at_ts INTEGER NOT NULL, part_indexes_json TEXT NOT NULL, PRIMARY KEY(task_id,decision_id));
        CREATE TRIGGER IF NOT EXISTS delete_delivery_reconciliations_on_task AFTER DELETE ON tasks BEGIN
        DELETE FROM channel_delivery_reconciliations WHERE task_id=OLD.task_id; END;")?;
    Ok(())
}

fn save_receipt(db: &Connection, receipt: &ChannelDeliveryReceipt) -> anyhow::Result<()> {
    receipt.validate().map_err(|e| anyhow!(e.to_string()))?;
    let raw = serde_json::to_string(receipt)?;
    let hash = receipt_digest(&raw);
    db.execute("INSERT INTO channel_delivery_receipts (idempotency_key,delivery_id,channel,adapter,status,receipt_json,receipt_digest,created_at_ts,updated_at_ts)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8) ON CONFLICT(idempotency_key) DO UPDATE SET status=excluded.status,receipt_json=excluded.receipt_json,receipt_digest=excluded.receipt_digest,updated_at_ts=excluded.updated_at_ts",
        params![receipt.idempotency_key,receipt.delivery_id,channel_token(receipt)?,receipt.adapter,status_token(receipt.status),raw,hash,receipt.updated_at_ts])?;
    db.execute("INSERT OR IGNORE INTO channel_delivery_receipt_events (idempotency_key,status,receipt_json,receipt_digest,recorded_at_ts) VALUES (?1,?2,?3,?4,?5)",params![receipt.idempotency_key,status_token(receipt.status),raw,hash,receipt.updated_at_ts])?;
    Ok(())
}

fn receipt_for(p: &Part, now: i64) -> anyhow::Result<ChannelDeliveryReceipt> {
    if let Some(r) = &p.receipt {
        let mut r = r.clone();
        r.updated_at_ts = r.updated_at_ts.max(now as u64);
        return Ok(r);
    }
    let c = p
        .claim
        .as_ref()
        .ok_or_else(|| anyhow!("delivery_dispatch_missing"))?;
    Ok(ChannelDeliveryReceipt {
        schema_version: 1,
        delivery_id: c.delivery_id.clone(),
        idempotency_key: p.key.clone(),
        channel: serde_json::from_value(json!(c.channel))?,
        adapter: c.adapter.clone(),
        status: ChannelDeliveryStatus::Failed,
        provider_message_ids: vec![],
        parts: vec![],
        error_code: Some("channel_delivery_part_query_required".into()),
        message_key: Some("channel.error.delivery_failed".into()),
        diagnostic_id: Some(format!("delivery:{}", uuid::Uuid::new_v4())),
        provider_error_code: None,
        retryable: false,
        updated_at_ts: now.max(1) as u64,
    })
}

pub(crate) fn reconcile(
    db: &mut Connection,
    task_id: &str,
    identity: &claw_core::types::AuthIdentity,
    decision: &Decision,
    now: i64,
) -> anyhow::Result<Value> {
    if decision.snapshot_digest.len() != 64
        || !decision
            .snapshot_digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        anyhow::bail!("delivery_snapshot_invalid");
    }
    if decision.operation == Operation::RetryUnknown && !decision.confirmed_duplicate_risk {
        anyhow::bail!("delivery_duplicate_risk_confirmation_required");
    }
    ensure_decision_schema(db)?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let authorized=tx.query_row("SELECT 1 FROM tasks t JOIN auth_keys a ON a.principal_id=t.principal_id WHERE t.task_id=?1 AND t.principal_id=?2 AND a.user_key=?3 AND a.enabled=1 AND t.status IN ('succeeded','failed','canceled','timeout')",params![task_id,identity.principal_id,identity.user_key], |_|Ok(())).optional()?.is_some();
    if !authorized {
        anyhow::bail!("task_delivery_unauthorized");
    }
    // Recheck the channel binding inside the write transaction, not just in HTTP auth.
    let bound_key:Option<String>=tx.query_row("SELECT b.user_key FROM tasks t JOIN channel_bindings b ON b.channel=t.channel JOIN auth_keys a ON a.user_key=b.user_key AND a.enabled=1 WHERE t.task_id=?1 AND (NULLIF(TRIM(t.external_user_id),'') IS NULL OR b.external_user_id=TRIM(t.external_user_id)) AND b.external_chat_id=COALESCE(NULLIF(TRIM(t.external_chat_id),''),NULLIF(TRIM(t.external_user_id),'')) ORDER BY b.id DESC LIMIT 1",[task_id],|r|r.get(0)).optional()?;
    if bound_key.as_deref() != Some(identity.user_key.as_str()) {
        anyhow::bail!("task_delivery_unauthorized");
    }
    let request_digest = digest(decision)?;
    let old:Option<(String,String)>=tx.query_row("SELECT request_digest,actor_principal FROM channel_delivery_reconciliations WHERE task_id=?1 AND decision_id=?2",params![task_id,decision.decision_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((hash, actor)) = old {
        if hash != request_digest || actor != identity.principal_id {
            anyhow::bail!("delivery_decision_conflict");
        }
        return Ok(
            json!({"schema_version":1,"status":"already_recorded","confirmation_source":"user"}),
        );
    }
    let s = snapshot(&tx, task_id)?.ok_or_else(|| anyhow!("delivery_dispatch_missing"))?;
    if digest(&s)? != decision.snapshot_digest {
        anyhow::bail!("delivery_snapshot_changed");
    }
    if active(&s.parent, now) || s.parts.iter().any(|p| active(p, now)) {
        anyhow::bail!("delivery_dispatch_active");
    }
    let mut changed = 0;
    let mut changed_indexes = Vec::new();
    for (index, p) in s.parts.iter().enumerate() {
        let eligible = match decision.operation {
            Operation::RetryFailed => retryable(p),
            _ => unknown(p, now),
        };
        if !eligible {
            continue;
        }
        let mut r = receipt_for(p, now)?;
        if decision.operation == Operation::ConfirmReceived {
            r.status = ChannelDeliveryStatus::Accepted;
            r.retryable = false;
            r.error_code = None;
            r.message_key = None;
            r.diagnostic_id = None;
            r.provider_error_code = None;
            for part in &mut r.parts {
                part.status = ChannelDeliveryStatus::Accepted;
                part.error_code = None;
            }
        } else {
            r.status = ChannelDeliveryStatus::Failed;
            r.retryable = true;
            r.error_code = Some("channel_delivery_user_authorized_retry".into());
            r.message_key = Some("channel.error.delivery_failed".into());
            r.diagnostic_id = Some(format!("delivery:{}", decision.decision_id));
        }
        save_receipt(&tx, &r)?;
        tx.execute("UPDATE channel_delivery_dispatch_claims SET state='receipt_recorded',lease_token=NULL,lease_expires_at_ts=0 WHERE idempotency_key=?1",[&p.key])?;
        changed += 1;
        changed_indexes.push(index);
    }
    if changed == 0 {
        anyhow::bail!("delivery_no_eligible_parts");
    }
    // Re-enter the existing aggregate lease; accepted children are skipped there.
    if !s.parts.iter().any(|p| p.key == s.parent.key) {
        let mut parent = receipt_for(&s.parent, now)?;
        parent.retryable = true;
        parent.status = ChannelDeliveryStatus::Failed;
        parent.error_code = Some("channel_delivery_reconciliation_pending".into());
        parent.message_key = Some("channel.error.delivery_failed".into());
        parent.diagnostic_id = Some(format!("delivery:{}", decision.decision_id));
        save_receipt(&tx, &parent)?;
    }
    tx.execute("UPDATE channel_delivery_dispatch_claims SET state='receipt_recorded',lease_token=NULL,lease_expires_at_ts=0 WHERE idempotency_key=?1",[&s.parent.key])?;
    tx.execute("INSERT INTO channel_terminal_delivery_outbox (task_id,state,next_attempt_at_ts,created_at_ts,updated_at_ts) VALUES (?1,'pending',0,?2,?2)
        ON CONFLICT(task_id) DO UPDATE SET state='pending',lease_token=NULL,lease_expires_at_ts=0,next_attempt_at_ts=0,attempt_count=0,updated_at_ts=excluded.updated_at_ts",params![task_id,now])?;
    tx.execute(
        "INSERT INTO channel_delivery_reconciliations VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            task_id,
            decision.decision_id.to_string(),
            identity.principal_id,
            request_digest,
            serde_json::to_value(decision.operation)?
                .as_str()
                .unwrap_or_default(),
            decision.snapshot_digest,
            now,
            serde_json::to_string(&changed_indexes)?
        ],
    )?;
    tx.commit()?;
    Ok(
        json!({"schema_version":1,"status":"queued","changed_part_count":changed,"confirmation_source":"user","provider_delivery_proof":false}),
    )
}

#[cfg(test)]
#[path = "channel_delivery_reconciliation_tests.rs"]
mod tests;
