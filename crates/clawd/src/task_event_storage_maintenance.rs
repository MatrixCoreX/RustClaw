use anyhow::ensure;
use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;

use crate::event_json_storage;

pub(crate) const DATASETS: [(&str, &str); 2] = [
    ("task_event_archive", "event_json"),
    ("task_event_artifacts", "payload_json"),
];

pub(crate) fn ensure_schema(db: &Connection) -> rusqlite::Result<()> {
    for (table, _) in DATASETS {
        let columns = db
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !columns.iter().any(|c| c == "storage_version") {
            db.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN storage_version INTEGER NOT NULL DEFAULT 0"
            ))?;
        }
        if !columns.iter().any(|c| c == "storage_blob") {
            db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN storage_blob BLOB"))?;
        }
        db.execute_batch(&format!("CREATE INDEX IF NOT EXISTS idx_{table}_unpacked ON {table}(storage_version) WHERE storage_version=0"))?;
    }
    Ok(())
}

#[derive(Default, Serialize)]
pub(crate) struct CompactionReport {
    pub rows: usize,
    pub compressed_rows: usize,
    pub logical_bytes: u64,
    pub stored_bytes: u64,
}

/// Every replacement is round-trip verified before the original is overwritten.
/// The transaction's storage_version change is also the durable, resumable cursor.
pub(crate) fn compact_batch(db: &mut Connection, batch: usize) -> anyhow::Result<CompactionReport> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut report = CompactionReport::default();
    for (table, column) in DATASETS {
        let ids = tx
            .prepare(&format!(
                "SELECT rowid FROM {table} WHERE storage_version=0 ORDER BY rowid LIMIT ?1"
            ))?
            .query_map([batch.clamp(1, 512) as i64], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in ids {
            let raw: String = tx.query_row(
                &format!("SELECT {column} FROM {table} WHERE rowid=?1"),
                [id],
                |r| r.get(0),
            )?;
            let stored = event_json_storage::encode(&raw, table == "task_event_archive")?;
            ensure!(
                event_json_storage::decode(&stored.json, stored.version, stored.blob.as_deref())?
                    == raw,
                "event_storage_roundtrip_mismatch"
            );
            tx.execute(&format!("UPDATE {table} SET {column}=?2,storage_version=?3,storage_blob=?4 WHERE rowid=?1 AND storage_version=0"), params![id, stored.json, stored.version, stored.blob])?;
            report.rows += 1;
            report.compressed_rows += usize::from(stored.version == 2);
            report.logical_bytes += raw.len() as u64;
            report.stored_bytes +=
                (stored.json.len() + stored.blob.as_ref().map_or(0, Vec::len)) as u64;
        }
    }
    tx.commit()?;
    Ok(report)
}

/// Discard only old terminal-task cache copies proven present, intact, in the archive.
pub(crate) fn prune_hot_batch(
    db: &mut Connection,
    older_than_ms: i64,
    batch: usize,
) -> anyhow::Result<usize> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ids = tx.prepare("SELECT s.rowid FROM task_event_stream s JOIN tasks t ON t.task_id=s.task_id
        JOIN task_event_archive a ON a.task_id=s.task_id AND a.seq=s.seq AND a.event_hash=s.event_hash
        WHERE t.status IN ('succeeded','failed','canceled','timeout') AND s.created_at_ms<?1
        ORDER BY s.rowid LIMIT ?2")?.query_map(params![older_than_ms,batch.clamp(1,512) as i64], |r| r.get::<_,i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in &ids {
        let (hot, archive, version, blob): (String,String,i64,Option<Vec<u8>>) = tx.query_row(
            "SELECT s.event_json,a.event_json,a.storage_version,a.storage_blob FROM task_event_stream s
             JOIN task_event_archive a ON a.task_id=s.task_id AND a.seq=s.seq AND a.event_hash=s.event_hash WHERE s.rowid=?1", [id],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        let hot: Value = serde_json::from_str(&hot)?;
        let archive = event_json_storage::read(&archive, version, blob.as_deref())?;
        let object = hot
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("event_hot_json_invalid"))?;
        ensure!(
            object
                .iter()
                .all(|(key, value)| archive.get(key) == Some(value)),
            "event_hot_archive_mismatch"
        );
        tx.execute("DELETE FROM task_event_stream WHERE rowid=?1", [id])?;
    }
    tx.commit()?;
    Ok(ids.len())
}

pub(crate) fn pending_rows(db: &Connection) -> rusqlite::Result<u64> {
    let mut count = 0;
    for (table, _) in DATASETS {
        count += db.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE storage_version=0"),
            [],
            |r| r.get::<_, u64>(0),
        )?;
    }
    Ok(count)
}

#[cfg(test)]
#[path = "task_event_storage_maintenance_tests.rs"]
mod tests;
