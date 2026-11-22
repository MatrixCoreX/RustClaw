//! Lossless cold journal storage. Execution and recovery fields stay inline.
use anyhow::{ensure, Context};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const TRACE: &str = "/task_journal/trace";
const REFERENCE: &str = "archived_fields_ref";
const FIELDS: [&str; 2] = ["rounds", "event_stream"];
const MIN_BYTES: usize = 16 * 1024;
const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    schema_version: u32,
    kind: String,
    payload_bytes: usize,
    sha256: String,
}

pub(crate) fn enabled(db: &Connection) -> anyhow::Result<bool> {
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
         ('runtime_journal_archives','runtime_journal_archive_state')",
        [],
        |r| r.get(0),
    )?;
    if count != 2 {
        return Ok(false);
    }
    let version: i64 = db.query_row(
        "SELECT schema_version FROM runtime_journal_archive_state WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    ensure!(version == 1, "journal_archive_schema_unsupported");
    Ok(true)
}

fn ensure_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS runtime_journal_archives(
        task_id TEXT PRIMARY KEY REFERENCES tasks(task_id) ON DELETE CASCADE,
        storage_json TEXT NOT NULL, storage_version INTEGER NOT NULL CHECK(storage_version=2),
        storage_blob BLOB NOT NULL, payload_bytes INTEGER NOT NULL, sha256 TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS runtime_journal_archive_state(
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),schema_version INTEGER NOT NULL,
        last_rowid INTEGER NOT NULL DEFAULT 0);
        INSERT OR IGNORE INTO runtime_journal_archive_state(singleton,schema_version,last_rowid) VALUES(1,1,0);
        CREATE TRIGGER IF NOT EXISTS runtime_journal_archive_task_deleted AFTER DELETE ON tasks
        BEGIN DELETE FROM runtime_journal_archives WHERE task_id=old.task_id; END;")?;
    ensure!(enabled(db)?, "journal_archive_schema_unavailable");
    Ok(())
}

/// Restore only the archived cold fields, using the owning task as the lookup key.
pub(crate) fn hydrate(db: &Connection, task_id: &str, result: &mut Value) -> anyhow::Result<bool> {
    let Some(raw_reference) = result
        .pointer(TRACE)
        .and_then(|v| v.get(REFERENCE))
        .cloned()
    else {
        return Ok(false);
    };
    let reference: Reference =
        serde_json::from_value(raw_reference).context("journal_archive_reference_invalid")?;
    ensure!(
        reference.schema_version == 1
            && reference.kind == "task_journal_fields"
            && (MIN_BYTES..=MAX_INPUT_BYTES).contains(&reference.payload_bytes)
            && reference.sha256.len() == 64
            && reference.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "journal_archive_reference_invalid"
    );
    let row: Option<(String, i64, Vec<u8>, i64, String)> = db
        .query_row(
            "SELECT storage_json,storage_version,storage_blob,payload_bytes,sha256
         FROM runtime_journal_archives WHERE task_id=?1",
            [task_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let (projection, version, blob, bytes, digest) = row.context("journal_archive_missing")?;
    ensure!(
        bytes >= 0 && bytes as usize == reference.payload_bytes && digest == reference.sha256,
        "journal_archive_reference_mismatch"
    );
    let decoded = crate::event_json_storage::decode(&projection, version, Some(&blob))?;
    ensure!(
        decoded.len() == reference.payload_bytes
            && hex::encode(Sha256::digest(decoded.as_bytes())) == reference.sha256,
        "journal_archive_digest_mismatch"
    );
    let fields: Map<String, Value> = serde_json::from_str(&decoded)?;
    ensure!(
        !fields.is_empty() && fields.keys().all(|k| FIELDS.contains(&k.as_str())),
        "journal_archive_fields_invalid"
    );
    let trace = result
        .pointer_mut(TRACE)
        .and_then(Value::as_object_mut)
        .context("journal_archive_trace_missing")?;
    ensure!(
        fields.keys().all(|k| !trace.contains_key(k)),
        "journal_archive_inline_conflict"
    );
    trace.remove(REFERENCE);
    trace.extend(fields);
    Ok(true)
}

/// Canonical logical evidence, independent of physical JSON whitespace or storage.
pub(crate) fn logical_result(db: &Connection, task_id: &str, raw: &str) -> anyhow::Result<String> {
    let Ok(mut value) = serde_json::from_str::<Value>(raw) else {
        return Ok(raw.to_string());
    };
    hydrate(db, task_id, &mut value)?;
    Ok(serde_json::to_string(&canonical(value))?)
}

fn canonical(value: Value) -> Value {
    match value {
        Value::Object(fields) => {
            let ordered: std::collections::BTreeMap<_, _> = fields
                .into_iter()
                .map(|(key, value)| (key, canonical(value)))
                .collect();
            Value::Object(ordered.into_iter().collect())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonical).collect()),
        value => value,
    }
}

fn compact_result(db: &Connection, task_id: &str, result: &mut Value) -> anyhow::Result<usize> {
    let Some(trace) = result.pointer(TRACE).and_then(Value::as_object) else {
        return Ok(0);
    };
    if trace.contains_key(REFERENCE) {
        return Ok(0);
    }
    let fields: Map<String, Value> = FIELDS
        .iter()
        .filter_map(|k| trace.get(*k).cloned().map(|v| (k.to_string(), v)))
        .collect();
    let raw = serde_json::to_string(&fields)?;
    if !(MIN_BYTES..=MAX_INPUT_BYTES).contains(&raw.len()) {
        return Ok(0);
    }
    let stored = crate::event_json_storage::encode(&raw, false)?;
    if stored.version != 2 {
        return Ok(0);
    }
    let digest = hex::encode(Sha256::digest(raw.as_bytes()));
    let reference = serde_json::to_value(Reference {
        schema_version: 1,
        kind: "task_journal_fields".into(),
        payload_bytes: raw.len(),
        sha256: digest.clone(),
    })?;
    let cost = stored.json.len()
        + stored.blob.as_ref().map_or(0, Vec::len)
        + reference.to_string().len()
        + task_id.len()
        + 64;
    if cost >= raw.len() {
        return Ok(0);
    }
    db.execute("INSERT OR IGNORE INTO runtime_journal_archives
        (task_id,storage_json,storage_version,storage_blob,payload_bytes,sha256) VALUES(?1,?2,?3,?4,?5,?6)",
        params![task_id,stored.json,stored.version,stored.blob,raw.len() as i64,digest])?;
    let mut candidate = result.clone();
    let trace = candidate
        .pointer_mut(TRACE)
        .and_then(Value::as_object_mut)
        .unwrap();
    for key in FIELDS {
        trace.remove(key);
    }
    trace.insert(REFERENCE.into(), reference);
    let mut restored = candidate.clone();
    hydrate(db, task_id, &mut restored)?;
    ensure!(restored == *result, "journal_archive_roundtrip_failed");
    *result = candidate;
    Ok(raw.len() - cost)
}

#[derive(Default, Serialize)]
pub(crate) struct CompactionReport {
    pub scanned_rows: u64,
    pub compacted_rows: u64,
    pub saved_logical_bytes: u64,
    pub cursor: i64,
    pub scan_complete: bool,
}

/// Explicit first use enables future bounded maintenance. Active, child, failed,
/// recent and resumable tasks remain unchanged. The row and archive commit together.
pub(crate) fn compact_batch(
    db: &mut Connection,
    batch: usize,
    before_ms: i64,
) -> anyhow::Result<CompactionReport> {
    ensure_schema(db)?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let cursor: i64 = tx.query_row(
        "SELECT last_rowid FROM runtime_journal_archive_state WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let ids = tx
        .prepare("SELECT rowid FROM tasks WHERE rowid>?1 ORDER BY rowid LIMIT ?2")?
        .query_map(params![cursor, batch.clamp(1, 64) as i64], |r| {
            r.get::<_, i64>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut report = CompactionReport {
        cursor: if ids.is_empty() { 0 } else { cursor },
        scan_complete: ids.is_empty(),
        ..Default::default()
    };
    let start = std::time::Instant::now();
    for id in ids {
        if start.elapsed() >= std::time::Duration::from_millis(250) {
            break;
        }
        report.scanned_rows += 1;
        report.cursor = id;
        let row: Option<(String,String)> = tx.query_row(
            "SELECT task_id,result_json FROM tasks WHERE rowid=?1 AND kind='ask' AND status='succeeded'
             AND CAST(updated_at AS INTEGER)>0 AND CAST(updated_at AS INTEGER)<=?2
             AND length(CAST(result_json AS BLOB))<=?3
             AND NOT EXISTS(SELECT 1 FROM child_task_graph_nodes WHERE child_task_id=tasks.task_id)",
            params![id,before_ms/1000,MAX_INPUT_BYTES as i64],|r| Ok((r.get(0)?,r.get(1)?)),
        ).optional()?;
        let Some((task_id, raw)) = row else {
            continue;
        };
        let Ok(mut value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if value
            .pointer("/task_lifecycle/state")
            .and_then(Value::as_str)
            .is_some_and(|s| {
                matches!(
                    s,
                    "waiting" | "background" | "needs_user" | "running" | "queued"
                )
            })
        {
            continue;
        }
        let saved = compact_result(&tx, &task_id, &mut value)?;
        if saved == 0 {
            continue;
        }
        let changed = tx.execute("UPDATE tasks SET result_json=?2 WHERE task_id=?1 AND result_json=?3 AND status='succeeded'",
            params![task_id,value.to_string(),raw])?;
        ensure!(changed == 1, "journal_archive_migration_conflict");
        report.compacted_rows += 1;
        report.saved_logical_bytes += saved as u64;
    }
    tx.execute(
        "UPDATE runtime_journal_archive_state SET last_rowid=?1 WHERE singleton=1",
        [report.cursor],
    )?;
    tx.commit()?;
    Ok(report)
}

#[cfg(test)]
#[path = "task_journal_archive_tests.rs"]
mod tests;
