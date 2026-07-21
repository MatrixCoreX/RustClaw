use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const ARCHIVE_SNAPSHOT_INTERVAL: u64 = 256;

pub(crate) struct ArchivedReplay {
    pub(crate) events: Vec<Value>,
    pub(crate) oldest_seq: Option<u64>,
    pub(crate) newest_seq: Option<u64>,
    pub(crate) latest_snapshot: Option<Value>,
}

pub(crate) fn previous_event_hash(
    tx: &Transaction<'_>,
    task_id: &str,
) -> anyhow::Result<Option<String>> {
    Ok(tx
        .query_row(
            "SELECT event_hash
             FROM task_event_archive
             WHERE task_id = ?1
             ORDER BY seq DESC
             LIMIT 1",
            params![task_id],
            |row| row.get(0),
        )
        .optional()?)
}

pub(crate) fn insert_event(
    tx: &Transaction<'_>,
    task_id: &str,
    seq: u64,
    event_hash: &str,
    previous_event_hash: Option<&str>,
    event_kind: &str,
    event_json: &str,
    created_at_ms: u64,
) -> anyhow::Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO task_event_archive (
            task_id, seq, event_hash, previous_event_hash, event_json,
            payload_schema_version, redaction_policy, created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, 1, 'task_event_redaction_v1', ?6)",
        params![
            task_id,
            seq,
            event_hash,
            previous_event_hash,
            event_json,
            created_at_ms
        ],
    )?;
    if seq % ARCHIVE_SNAPSHOT_INTERVAL == 0 || event_kind == "task_final" {
        persist_snapshot(tx, task_id, seq, created_at_ms)?;
    }
    Ok(())
}

pub(crate) fn backfill_hot_suffix(tx: &Transaction<'_>, task_id: &str) -> anyhow::Result<()> {
    let archived_count = tx.query_row(
        "SELECT COUNT(*) FROM task_event_archive WHERE task_id = ?1",
        params![task_id],
        |row| row.get::<_, u64>(0),
    )?;
    if archived_count > 0 {
        return Ok(());
    }
    let rows = {
        let mut statement = tx.prepare(
            "SELECT seq, event_hash, event_json, created_at_ms
             FROM task_event_stream
             WHERE task_id = ?1
             ORDER BY seq ASC",
        )?;
        let rows = statement
            .query_map(params![task_id], |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let mut previous_hash: Option<String> = None;
    let mut newest = None;
    for (seq, event_hash, event_json, created_at_ms) in rows {
        let archived_event_json =
            normalize_archived_event_json(&event_json, &event_hash, previous_hash.as_deref())?;
        tx.execute(
            "INSERT OR IGNORE INTO task_event_archive (
                task_id, seq, event_hash, previous_event_hash, event_json,
                payload_schema_version, redaction_policy, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 1, 'task_event_redaction_v1', ?6)",
            params![
                task_id,
                seq,
                event_hash,
                previous_hash.as_deref(),
                archived_event_json,
                created_at_ms
            ],
        )?;
        previous_hash = Some(event_hash);
        newest = Some((seq, created_at_ms));
    }
    if let Some((seq, created_at_ms)) = newest {
        persist_snapshot(tx, task_id, seq, created_at_ms)?;
    }
    Ok(())
}

pub(crate) fn replay_after(
    db: &Connection,
    task_id: &str,
    cursor: u64,
    limit: u64,
) -> anyhow::Result<ArchivedReplay> {
    let (oldest_seq, newest_seq) = db.query_row(
        "SELECT MIN(seq), MAX(seq)
         FROM task_event_archive
         WHERE task_id = ?1",
        params![task_id],
        |row| Ok((row.get::<_, Option<u64>>(0)?, row.get::<_, Option<u64>>(1)?)),
    )?;
    let mut statement = db.prepare(
        "SELECT event_json
         FROM task_event_archive
         WHERE task_id = ?1 AND seq > ?2
         ORDER BY seq ASC
         LIMIT ?3",
    )?;
    let rows = statement.query_map(params![task_id, cursor, limit], |row| {
        row.get::<_, String>(0)
    })?;
    let mut events = Vec::new();
    for row in rows {
        if let Ok(event) = serde_json::from_str::<Value>(&row?) {
            events.push(event);
        }
    }
    Ok(ArchivedReplay {
        events,
        oldest_seq,
        newest_seq,
        latest_snapshot: latest_snapshot(db, task_id)?,
    })
}

pub(crate) fn delete_orphaned_records(db: &Connection) -> anyhow::Result<usize> {
    let mut deleted = 0;
    for table in [
        "task_event_stream",
        "task_event_archive",
        "task_event_snapshots",
        "task_event_artifacts",
    ] {
        deleted += db.execute(
            &format!(
                "DELETE FROM {table}
                 WHERE NOT EXISTS (
                     SELECT 1 FROM tasks WHERE tasks.task_id = {table}.task_id
                 )"
            ),
            [],
        )?;
    }
    Ok(deleted)
}

fn normalize_archived_event_json(
    raw: &str,
    event_hash: &str,
    previous_event_hash: Option<&str>,
) -> anyhow::Result<String> {
    let mut event = serde_json::from_str::<Value>(raw)?;
    let object = event
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("task_event_archive_event_invalid"))?;
    object
        .entry("schema_version".to_string())
        .or_insert_with(|| json!(1));
    object
        .entry("payload_schema_version".to_string())
        .or_insert_with(|| json!(1));
    object
        .entry("event_hash".to_string())
        .or_insert_with(|| json!(event_hash));
    object
        .entry("previous_event_hash".to_string())
        .or_insert_with(|| json!(previous_event_hash));
    Ok(serde_json::to_string(&event)?)
}

fn persist_snapshot(
    tx: &Transaction<'_>,
    task_id: &str,
    snapshot_seq: u64,
    created_at_ms: u64,
) -> anyhow::Result<()> {
    let source_seq_start = tx.query_row(
        "SELECT COALESCE(MAX(source_seq_end), 0) + 1
         FROM task_event_snapshots
         WHERE task_id = ?1",
        params![task_id],
        |row| row.get::<_, u64>(0),
    )?;
    if source_seq_start > snapshot_seq {
        return Ok(());
    }
    let rows = {
        let mut statement = tx.prepare(
            "SELECT seq, event_hash, event_json
             FROM task_event_archive
             WHERE task_id = ?1 AND seq BETWEEN ?2 AND ?3
             ORDER BY seq ASC",
        )?;
        let rows = statement
            .query_map(params![task_id, source_seq_start, snapshot_seq], |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    if rows.is_empty() {
        return Ok(());
    }
    let mut range_hasher = Sha256::new();
    let mut event_type_counts = BTreeMap::<String, u64>::new();
    let mut latest_event_kind = None;
    let mut latest_timestamp_ms = None;
    let mut latest_task_status = None;
    let mut latest_execution_state = None;
    for (_, event_hash, event_json) in &rows {
        range_hasher.update(event_hash.as_bytes());
        range_hasher.update(b"\n");
        if let Ok(event) = serde_json::from_str::<Value>(event_json) {
            if let Some(kind) = event
                .get("event_kind")
                .or_else(|| event.get("event_type"))
                .and_then(Value::as_str)
            {
                *event_type_counts.entry(kind.to_string()).or_default() += 1;
                latest_event_kind = Some(kind.to_string());
            }
            latest_timestamp_ms = event.get("timestamp_ms").and_then(Value::as_u64);
            if let Some(status) = event.pointer("/payload/status").and_then(Value::as_str) {
                latest_task_status = Some(status.to_string());
            }
            if let Some(state) = event
                .pointer("/payload/execution_state")
                .and_then(Value::as_str)
            {
                latest_execution_state = Some(state.to_string());
            }
        }
    }
    let source_hash = format!("{:x}", range_hasher.finalize());
    let source_seq_end = rows.last().map(|row| row.0).unwrap_or(snapshot_seq);
    let snapshot = json!({
        "schema_version": 1,
        "snapshot_kind": "task_event_archive",
        "task_id": task_id,
        "snapshot_seq": snapshot_seq,
        "source_event_range": {
            "start_seq": source_seq_start,
            "end_seq": source_seq_end,
            "event_count": rows.len(),
            "sha256": source_hash,
        },
        "projection": {
            "latest_event_kind": latest_event_kind,
            "latest_timestamp_ms": latest_timestamp_ms,
            "task_status": latest_task_status,
            "execution_state": latest_execution_state,
            "event_type_counts": event_type_counts,
        },
        "redaction_policy": "task_event_redaction_v1",
    });
    let snapshot_json = serde_json::to_string(&snapshot)?;
    let snapshot_hash = bytes_hash(snapshot_json.as_bytes());
    tx.execute(
        "INSERT OR IGNORE INTO task_event_snapshots (
            task_id, snapshot_seq, source_seq_start, source_seq_end,
            source_event_count, source_hash, snapshot_hash, snapshot_json,
            created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            task_id,
            snapshot_seq,
            source_seq_start,
            source_seq_end,
            rows.len() as u64,
            source_hash,
            snapshot_hash,
            snapshot_json,
            created_at_ms
        ],
    )?;
    Ok(())
}

fn latest_snapshot(db: &Connection, task_id: &str) -> anyhow::Result<Option<Value>> {
    let value = db
        .query_row(
            "SELECT snapshot_json
             FROM task_event_snapshots
             WHERE task_id = ?1
             ORDER BY snapshot_seq DESC
             LIMIT 1",
            params![task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(value.and_then(|raw| serde_json::from_str(&raw).ok()))
}

fn bytes_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
