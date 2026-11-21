//! Reference accounting follows the existing archive retention/deletion transaction.
use rusqlite::Connection;

pub(super) fn ensure_schema(db: &Connection) -> rusqlite::Result<()> {
    // A savepoint also works when schema initialization is already inside a transaction.
    db.execute_batch("SAVEPOINT event_chunk_lifecycle")?;
    let result = initialize(db);
    if result.is_ok() {
        db.execute_batch("RELEASE event_chunk_lifecycle")?;
    } else {
        db.execute_batch("ROLLBACK TO event_chunk_lifecycle; RELEASE event_chunk_lifecycle")?;
    }
    result
}

fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS task_event_chunk_state(key TEXT PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS task_event_chunk_references(
            chunk_id BLOB PRIMARY KEY, reference_count INTEGER NOT NULL CHECK(reference_count>=0));")?;
    let ready: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_event_chunk_state WHERE key='references_v1_ready')",
        [],
        |row| row.get(0),
    )?;
    if !ready {
        // One grouped pass, only for initial installation or a pre-accounting codec database.
        db.execute_batch(
            "DELETE FROM task_event_chunk_references;
            INSERT INTO task_event_chunk_references(chunk_id,reference_count)
                SELECT substr(storage_blob,1,32),COUNT(*) FROM task_event_archive
                WHERE storage_version=3 GROUP BY substr(storage_blob,1,32);
            INSERT OR IGNORE INTO task_event_chunk_references(chunk_id,reference_count)
                SELECT chunk_id,0 FROM task_event_chunks;
            INSERT OR IGNORE INTO task_event_chunk_state(key)
                SELECT 'codec_v3_enabled' WHERE EXISTS(SELECT 1 FROM task_event_chunks);
            INSERT INTO task_event_chunk_state(key) VALUES('references_v1_ready');",
        )?;
    }
    db.execute_batch("CREATE INDEX IF NOT EXISTS idx_task_event_chunks_unreferenced
            ON task_event_chunk_references(reference_count) WHERE reference_count=0;
        CREATE TRIGGER IF NOT EXISTS task_event_chunk_created AFTER INSERT ON task_event_chunks BEGIN
            INSERT OR IGNORE INTO task_event_chunk_references(chunk_id,reference_count) VALUES(NEW.chunk_id,0);
        END;
        CREATE TRIGGER IF NOT EXISTS task_event_chunk_reference_inserted AFTER INSERT ON task_event_archive
        WHEN NEW.storage_version=3 BEGIN
            UPDATE task_event_chunk_references SET reference_count=reference_count+1
                WHERE chunk_id=substr(NEW.storage_blob,1,32);
        END;
        CREATE TRIGGER IF NOT EXISTS task_event_chunk_reference_deleted AFTER DELETE ON task_event_archive
        WHEN OLD.storage_version=3 BEGIN
            UPDATE task_event_chunk_references SET reference_count=reference_count-1
                WHERE chunk_id=substr(OLD.storage_blob,1,32);
        END;
        CREATE TRIGGER IF NOT EXISTS task_event_chunk_reference_updated
        AFTER UPDATE OF storage_version,storage_blob ON task_event_archive
        WHEN OLD.storage_version=3 OR NEW.storage_version=3 BEGIN
            UPDATE task_event_chunk_references SET reference_count=reference_count-1
                WHERE OLD.storage_version=3 AND chunk_id=substr(OLD.storage_blob,1,32);
            UPDATE task_event_chunk_references SET reference_count=reference_count+1
                WHERE NEW.storage_version=3 AND chunk_id=substr(NEW.storage_blob,1,32);
        END;")
}

pub(super) fn collect_unreferenced(db: &Connection, batch: usize) -> rusqlite::Result<usize> {
    let ids = db
        .prepare(
            "SELECT chunk_id FROM task_event_chunk_references WHERE reference_count=0 LIMIT ?1",
        )?
        .query_map([batch.clamp(1, 128) as i64], |row| row.get::<_, Vec<u8>>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in &ids {
        db.execute("DELETE FROM task_event_chunks WHERE chunk_id=?1", [id])?;
        db.execute(
            "DELETE FROM task_event_chunk_references WHERE chunk_id=?1 AND reference_count=0",
            [id],
        )?;
    }
    Ok(ids.len())
}
