//! Lossless immutable cold-event chunks. Public JSON, sequence and hash chain are unchanged.
use crate::event_json_storage::{self, StoredJson};
use anyhow::{ensure, Context};
use rusqlite::{params, Connection, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
const MAX_CHUNK_BYTES: usize = 16 * 1024 * 1024;
const TARGET_CHUNK_BYTES: usize = 256 * 1024;
const MAX_CHUNK_ROWS: usize = 128;
thread_local! { static CACHE: RefCell<Vec<(Vec<u8>, Vec<String>)>> = const { RefCell::new(Vec::new()) }; }
pub(crate) fn ensure_schema(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS task_event_chunks (chunk_id BLOB PRIMARY KEY, payload_json TEXT NOT NULL, storage_version INTEGER NOT NULL, storage_blob BLOB);
        CREATE INDEX IF NOT EXISTS idx_task_event_archive_cold_pending ON task_event_archive(storage_version) WHERE storage_version<3;")?;
    lifecycle::ensure_schema(db)
}
pub(crate) fn decode(
    db: &Connection,
    projection: &str,
    version: i64,
    blob: Option<&[u8]>,
) -> anyhow::Result<String> {
    if version != 3 {
        return event_json_storage::decode(projection, version, blob);
    }
    let reference = blob.context("event_chunk_reference_missing")?;
    ensure!(reference.len() == 68, "event_chunk_reference_invalid");
    let index = u32::from_le_bytes(reference[32..36].try_into()?) as usize;
    let (raw, version, packed): (String, i64, Option<Vec<u8>>) = db.query_row(
        "SELECT payload_json,storage_version,storage_blob FROM task_event_chunks WHERE chunk_id=?1",
        [&reference[..32]],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    ensure!(matches!(version, 1 | 2), "event_chunk_codec_invalid");
    let mut key = Sha256::new();
    key.update(&reference[..32]);
    key.update(raw.as_bytes());
    key.update(version.to_le_bytes());
    key.update(packed.as_deref().unwrap_or_default());
    let key = key.finalize().to_vec();
    let decoded = CACHE.with(|cache| -> anyhow::Result<String> {
        let mut cache = cache.borrow_mut();
        let found = cache.iter().position(|(digest, _)| digest == &key);
        let slot = match found {
            Some(slot) => slot,
            None => {
                let decoded = event_json_storage::decode(&raw, version, packed.as_deref())?;
                ensure!(decoded.len() <= MAX_CHUNK_BYTES, "event_chunk_size_invalid");
                ensure!(
                    Sha256::digest(decoded.as_bytes()).as_slice() == &reference[..32],
                    "event_chunk_digest_mismatch"
                );
                let entries: Vec<String> = serde_json::from_str(&decoded)?;
                ensure!(entries.len() <= MAX_CHUNK_ROWS, "event_chunk_count_invalid");
                if cache.len() >= 2 {
                    cache.remove(0);
                }
                cache.push((key, entries));
                cache.len() - 1
            }
        };
        cache[slot]
            .1
            .get(index)
            .cloned()
            .context("event_chunk_index_invalid")
    })?;
    ensure!(
        Sha256::digest(decoded.as_bytes()).as_slice() == &reference[36..],
        "event_chunk_event_digest_mismatch"
    );
    ensure!(
        serde_json::from_str::<serde_json::Value>(projection)?
            == event_json_storage::projection(&decoded, true)?,
        "event_chunk_projection_mismatch"
    );
    Ok(decoded)
}
pub(crate) fn read(
    db: &Connection,
    raw: &str,
    version: i64,
    blob: Option<&[u8]>,
) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::from_str(&decode(db, raw, version, blob)?)?)
}
/// Committing the first operator-verified chunk enables bounded ongoing maintenance.
/// Merely booting the new reader leaves the old encoding unchanged for rollback safety.
pub(crate) fn maintenance_enabled(db: &Connection) -> rusqlite::Result<bool> {
    let installed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='task_event_chunk_state')",[],|row|row.get(0))?;
    if !installed {
        return Ok(false);
    }
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_event_chunk_state WHERE key='codec_v3_enabled')",
        [],
        |row| row.get(0),
    )
}

pub(crate) fn compact_batch(db: &mut Connection, batch: usize) -> anyhow::Result<usize> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let rows = tx.prepare("SELECT rowid,event_json,storage_version,storage_blob FROM task_event_archive WHERE storage_version<3 ORDER BY storage_version,rowid LIMIT ?1")?
        .query_map([batch.clamp(1,MAX_CHUNK_ROWS) as i64], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)?,row.get::<_,Option<Vec<u8>>>(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        lifecycle::collect_unreferenced(&tx, batch)?;
        tx.commit()?;
        return Ok(0);
    }
    let mut entries = Vec::new();
    let mut ids = Vec::new();
    let mut encoded_bytes = 2usize;
    for (id, raw, version, blob) in rows {
        let decoded = event_json_storage::decode(&raw, version, blob.as_deref())?;
        let next_bytes = encoded_bytes
            .saturating_add(serde_json::to_string(&decoded)?.len())
            .saturating_add(usize::from(!entries.is_empty()));
        if !entries.is_empty() && next_bytes > TARGET_CHUNK_BYTES {
            break;
        }
        ensure!(
            next_bytes <= MAX_CHUNK_BYTES,
            "event_chunk_single_event_too_large"
        );
        encoded_bytes = next_bytes;
        entries.push(decoded);
        ids.push(id);
    }
    if !entries.is_empty() {
        let raw = serde_json::to_string(&entries)?;
        let digest = Sha256::digest(raw.as_bytes()).to_vec();
        let StoredJson {
            json,
            version,
            blob,
        } = event_json_storage::encode(&raw, false)?;
        ensure!(
            event_json_storage::decode(&json, version, blob.as_deref())? == raw,
            "event_chunk_roundtrip_mismatch"
        );
        tx.execute("INSERT OR IGNORE INTO task_event_chunks(chunk_id,payload_json,storage_version,storage_blob) VALUES(?1,?2,?3,?4)",params![digest,json,version,blob])?;
        for (index, (id, entry)) in ids.iter().zip(&entries).enumerate() {
            let mut reference = digest.clone();
            reference.extend_from_slice(&(index as u32).to_le_bytes());
            reference.extend_from_slice(&Sha256::digest(entry.as_bytes()));
            let projection = event_json_storage::projection(entry, true)?.to_string();
            ensure!(
                decode(&tx, &projection, 3, Some(&reference))? == *entry,
                "event_chunk_reference_roundtrip_mismatch"
            );
            tx.execute("UPDATE task_event_archive SET event_json=?2,storage_version=3,storage_blob=?3 WHERE rowid=?1 AND storage_version<3", params![id,projection,reference])?;
        }
        tx.execute(
            "INSERT OR IGNORE INTO task_event_chunk_state(key) VALUES('codec_v3_enabled')",
            [],
        )?;
    }
    lifecycle::collect_unreferenced(&tx, batch)?;
    tx.commit()?;
    Ok(ids.len())
}

#[path = "event_json_chunk_lifecycle.rs"]
mod lifecycle;

#[cfg(test)]
#[path = "event_json_chunks_tests.rs"]
mod tests;
