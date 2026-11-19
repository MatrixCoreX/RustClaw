//! Lossless immutable storage for large, redundant terminal-resume snapshots.
use anyhow::{ensure, Context};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MIN_SNAPSHOT_BYTES: usize = 16 * 1024;
const PROJECTION: &str = "/task_lifecycle/resume_executor_result_projection";

/// The caller owns the same SQLite transaction as the result-row publication.
pub(crate) fn compact_projection(
    db: &Connection,
    task_id: &str,
    result: &mut Value,
) -> anyhow::Result<u64> {
    let Some(value) = result
        .pointer(PROJECTION)
        .and_then(|p| p.get("final_result_json"))
        .filter(|v| v.is_object())
    else {
        return Ok(0);
    };
    let raw = serde_json::to_string(value)?;
    if raw.len() < MIN_SNAPSHOT_BYTES {
        return Ok(0);
    }
    let stored = crate::event_json_storage::encode(&raw, false)?;
    if stored.version != 2 {
        return Ok(0);
    }
    ensure!(
        crate::event_json_storage::decode(&stored.json, stored.version, stored.blob.as_deref())?
            == raw,
        "result_object_roundtrip_failed"
    );
    let digest = hex::encode(Sha256::digest(raw.as_bytes()));
    let id = format!("task_event_payload_{digest}");
    crate::task_event_transport::ensure_event_schema(db)?;
    let inserted = db.execute("INSERT OR IGNORE INTO task_event_artifacts(task_id,artifact_id,payload_json,payload_bytes,created_at_ms,storage_version,storage_blob) VALUES(?1,?2,?3,?4,unixepoch()*1000,?5,?6)", params![task_id,id,stored.json,raw.len() as i64,stored.version,stored.blob])?;
    let reference = json!({"schema_version":1,"kind":"task_event_payload","artifact_id":id,"payload_bytes":raw.len(),"sha256":digest});
    ensure!(
        read_snapshot(db, task_id, &reference)? == *value,
        "result_object_verification_failed"
    );
    let projection = result
        .pointer_mut(PROJECTION)
        .and_then(Value::as_object_mut)
        .context("result_projection_missing")?;
    projection.remove("final_result_json");
    let reference_bytes = reference.to_string().len();
    projection.insert("final_result_ref".to_string(), reference);
    let additional_storage = if inserted == 1 {
        stored.json.len() + stored.blob.as_ref().map_or(0, Vec::len)
    } else {
        0
    };
    Ok(raw
        .len()
        .saturating_add(1)
        .saturating_sub(reference_bytes + additional_storage) as u64)
}

pub(crate) fn valid_reference(value: &Value) -> bool {
    value
        .get("payload_bytes")
        .and_then(Value::as_u64)
        .is_some_and(|bytes| {
            (MIN_SNAPSHOT_BYTES as u64..=crate::event_json_storage::MAX_PACKED_JSON_BYTES as u64)
                .contains(&bytes)
        })
        && value.get("schema_version").and_then(Value::as_u64) == Some(1)
        && value.get("kind").and_then(Value::as_str) == Some("task_event_payload")
        && value
            .get("sha256")
            .and_then(Value::as_str)
            .is_some_and(|sha| sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        && value
            .get("artifact_id")
            .and_then(Value::as_str)
            .zip(value.get("sha256").and_then(Value::as_str))
            .is_some_and(|(id, sha)| id == format!("task_event_payload_{sha}"))
}

pub(crate) fn read_snapshot(
    db: &Connection,
    task_id: &str,
    reference: &Value,
) -> anyhow::Result<Value> {
    ensure!(
        valid_reference(reference),
        "result_object_reference_invalid"
    );
    let id = reference["artifact_id"]
        .as_str()
        .context("result_object_reference_invalid")?;
    let (raw, version, blob): (String,i64,Option<Vec<u8>>) = db.query_row(
        "SELECT payload_json,storage_version,storage_blob FROM task_event_artifacts WHERE task_id=?1 AND artifact_id=?2", params![task_id,id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
        .optional()?.context("result_object_missing")?;
    let decoded = crate::event_json_storage::decode(&raw, version, blob.as_deref())?;
    ensure!(
        Some(decoded.len() as u64) == reference["payload_bytes"].as_u64()
            && hex::encode(Sha256::digest(decoded.as_bytes()))
                == reference["sha256"].as_str().unwrap_or_default(),
        "result_object_digest_mismatch"
    );
    Ok(serde_json::from_str(&decoded)?)
}

#[derive(Default, serde::Serialize)]
pub(crate) struct ResultCompactionReport {
    pub scanned_rows: u64,
    pub compacted_rows: u64,
    pub saved_logical_bytes: u64,
    pub cursor: i64,
}

/// Bounded, resumable historical migration. Running tasks and final replies are preserved.
pub(crate) fn compact_batch(
    db: &mut Connection,
    batch: usize,
) -> anyhow::Result<ResultCompactionReport> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS runtime_result_object_maintenance(singleton INTEGER PRIMARY KEY CHECK(singleton=1),last_rowid INTEGER NOT NULL DEFAULT 0,saved_bytes INTEGER NOT NULL DEFAULT 0,compacted_rows INTEGER NOT NULL DEFAULT 0)")?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let cursor = tx
        .query_row(
            "SELECT last_rowid FROM runtime_result_object_maintenance WHERE singleton=1",
            [],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0);
    let ids = tx
        .prepare("SELECT rowid FROM tasks WHERE rowid>?1 ORDER BY rowid LIMIT ?2")?
        .query_map(params![cursor, batch.clamp(1, 64) as i64], |r| {
            r.get::<_, i64>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut report = ResultCompactionReport {
        cursor: if ids.is_empty() { 0 } else { cursor },
        ..Default::default()
    };
    let started = std::time::Instant::now();
    for id in ids {
        if started.elapsed() > std::time::Duration::from_millis(250) {
            break;
        }
        let (task_id, status, raw): (String, String, Option<String>) = tx.query_row(
            "SELECT task_id,status,result_json FROM tasks WHERE rowid=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        report.scanned_rows += 1;
        report.cursor = id;
        if !matches!(
            status.as_str(),
            "succeeded" | "failed" | "canceled" | "timeout"
        ) {
            continue;
        }
        let Some(raw) = raw else {
            continue;
        };
        if raw.len() < MIN_SNAPSHOT_BYTES {
            continue;
        }
        let Ok(mut value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let saved = compact_projection(&tx, &task_id, &mut value)?;
        if saved == 0 {
            continue;
        }
        let changed = tx.execute(
            "UPDATE tasks SET result_json=?2 WHERE task_id=?1 AND status=?3 AND result_json=?4",
            params![task_id, value.to_string(), status, raw],
        )?;
        ensure!(changed == 1, "result_object_migration_conflict");
        report.compacted_rows += 1;
        report.saved_logical_bytes += saved;
    }
    tx.execute("INSERT INTO runtime_result_object_maintenance(singleton,last_rowid,saved_bytes,compacted_rows) VALUES(1,?1,?2,?3) ON CONFLICT(singleton) DO UPDATE SET last_rowid=excluded.last_rowid,saved_bytes=saved_bytes+excluded.saved_bytes,compacted_rows=compacted_rows+excluded.compacted_rows",params![report.cursor,report.saved_logical_bytes,report.compacted_rows])?;
    tx.commit()?;
    Ok(report)
}

#[cfg(test)]
#[path = "task_result_objects_tests.rs"]
mod tests;
