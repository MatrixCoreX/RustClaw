use super::*;
#[test]
fn chunks_preserve_original_json_and_detect_tampering_and_missing_references() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE task_event_archive(event_json TEXT,storage_version INTEGER,storage_blob BLOB);").unwrap();
    ensure_schema(&db).unwrap();
    assert!(!maintenance_enabled(&db).unwrap());
    let originals=(0..100).map(|n|serde_json::json!({"event_kind":"model_event","seq":n,"payload":{"text":"repeat evidence ".repeat(40)}}).to_string()).collect::<Vec<_>>();
    for raw in &originals {
        db.execute("INSERT INTO task_event_archive VALUES(?1,1,NULL)", [raw])
            .unwrap();
    }
    assert_eq!(compact_batch(&mut db, 128).unwrap(), 100);
    assert!(maintenance_enabled(&db).unwrap());
    let stored:u64=db.query_row("SELECT (SELECT COALESCE(SUM(length(event_json)+length(storage_blob)),0) FROM task_event_archive)+(SELECT COALESCE(SUM(length(payload_json)+COALESCE(length(storage_blob),0)),0) FROM task_event_chunks)",[],|row|row.get(0)).unwrap();
    assert!(stored < originals.iter().map(String::len).sum::<usize>() as u64 / 2);
    assert_eq!(compact_batch(&mut db, 128).unwrap(), 0);
    let rows = db
        .prepare(
            "SELECT event_json,storage_version,storage_blob FROM task_event_archive ORDER BY rowid",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    for ((raw, version, blob), expected) in rows.iter().zip(&originals) {
        assert_eq!(decode(&db, raw, *version, Some(blob)).unwrap(), *expected);
    }
    let (raw, version, mut blob) = rows[0].clone();
    blob[36] ^= 1;
    assert!(decode(&db, &raw, version, Some(&blob)).is_err());
    db.execute("DELETE FROM task_event_chunks", []).unwrap();
    assert!(decode(&db, &raw, version, Some(&rows[0].2)).is_err());
}

#[test]
fn retention_reclaims_only_unreferenced_chunks_and_keeps_codec_enabled() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE task_event_archive(event_json TEXT,storage_version INTEGER,storage_blob BLOB);").unwrap();
    ensure_schema(&db).unwrap();
    for seq in 0..2 {
        let raw = serde_json::json!({"event_kind":"test", "seq":seq}).to_string();
        db.execute("INSERT INTO task_event_archive VALUES(?1,1,NULL)", [raw])
            .unwrap();
    }
    assert_eq!(compact_batch(&mut db, 128).unwrap(), 2);
    let count = |db: &Connection| {
        db.query_row("SELECT COUNT(*) FROM task_event_chunks", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
    };
    db.execute("DELETE FROM task_event_archive WHERE rowid=1", [])
        .unwrap();
    assert_eq!(compact_batch(&mut db, 128).unwrap(), 0);
    assert_eq!(count(&db), 1);
    let (raw, version, blob): (String, i64, Vec<u8>) = db
        .query_row(
            "SELECT event_json,storage_version,storage_blob FROM task_event_archive",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(decode(&db, &raw, version, Some(&blob))
        .unwrap()
        .contains("test"));
    db.execute("DELETE FROM task_event_archive", []).unwrap();
    assert_eq!(compact_batch(&mut db, 128).unwrap(), 0);
    assert_eq!(count(&db), 0);
    ensure_schema(&db).unwrap();
    assert!(maintenance_enabled(&db).unwrap());
    db.execute(
        "INSERT INTO task_event_archive VALUES(?1,1,NULL)",
        [serde_json::json!({"event_kind":"later"}).to_string()],
    )
    .unwrap();
    assert_eq!(compact_batch(&mut db, 128).unwrap(), 1);
    assert_eq!(count(&db), 1);
}
