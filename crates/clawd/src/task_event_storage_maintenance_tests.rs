use super::*;
use serde_json::json;

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE tasks(task_id TEXT PRIMARY KEY,status TEXT);
        CREATE TABLE task_event_stream(task_id TEXT,seq INTEGER,event_hash TEXT,event_json TEXT,created_at_ms INTEGER);
        CREATE TABLE task_event_archive(task_id TEXT,seq INTEGER,event_hash TEXT,event_json TEXT,created_at_ms INTEGER);
        CREATE TABLE task_event_artifacts(task_id TEXT,artifact_id TEXT,payload_json TEXT,payload_bytes INTEGER);
        INSERT INTO tasks VALUES('done','succeeded'),('active','running'),('gap','succeeded');").unwrap();
    ensure_schema(&db).unwrap();
    db
}

fn event(db: &Connection, task: &str, seq: i64, archived: bool) {
    let raw=json!({"task_id":task,"seq":seq,"event_kind":"tool_finished","payload":{"skill":"example","requested_action_ref":"example.read","data":"repeat".repeat(2000)}}).to_string();
    db.execute(
        "INSERT INTO task_event_stream VALUES(?1,?2,?3,?4,1)",
        params![task, seq, format!("hash-{seq}"), raw],
    )
    .unwrap();
    if archived {
        db.execute("INSERT INTO task_event_archive(task_id,seq,event_hash,event_json,created_at_ms) VALUES(?1,?2,?3,?4,1)",params![task,seq,format!("hash-{seq}"),raw]).unwrap();
    }
}

#[test]
fn migration_resumes_and_index_queries_remain_identical() {
    let mut db = fixture();
    event(&db, "done", 1, true);
    event(&db, "done", 2, true);
    db.execute(
        "INSERT INTO task_event_artifacts(task_id,artifact_id,payload_json,payload_bytes,storage_version) VALUES('done','payload',?1,10000,0)",
        [json!({"text":"example".repeat(2000)}).to_string()],
    )
    .unwrap();
    assert_eq!(compact_batch(&mut db, 1).unwrap().rows, 2);
    assert_eq!(pending_rows(&db).unwrap(), 1);
    assert_eq!(compact_batch(&mut db, 1).unwrap().rows, 1);
    assert_eq!(pending_rows(&db).unwrap(), 0);
    assert_eq!(compact_batch(&mut db, 1).unwrap().rows, 0);
    let count:i64=db.query_row("SELECT count(*) FROM task_event_archive WHERE json_extract(event_json,'$.event_kind')='tool_finished' AND json_extract(event_json,'$.payload.skill')='example' AND json_extract(event_json,'$.payload.requested_action_ref')='example.read'",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 2);
}

#[test]
fn only_verified_old_terminal_cache_is_pruned_and_archive_is_preserved() {
    let mut db = fixture();
    event(&db, "done", 1, true);
    event(&db, "active", 1, true);
    event(&db, "gap", 1, false);
    compact_batch(&mut db, 128).unwrap();
    assert_eq!(prune_hot_batch(&mut db, 0, 128).unwrap(), 0);
    assert_eq!(prune_hot_batch(&mut db, 100, 128).unwrap(), 1);
    let hot: i64 = db
        .query_row("SELECT count(*) FROM task_event_stream", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hot, 2);
    let archived: i64 = db
        .query_row("SELECT count(*) FROM task_event_archive", [], |r| r.get(0))
        .unwrap();
    assert_eq!(archived, 2);
}

#[test]
fn corrupted_archive_never_removes_last_good_hot_copy() {
    let mut db = fixture();
    event(&db, "done", 1, true);
    compact_batch(&mut db, 1).unwrap();
    db.execute(
        "UPDATE task_event_archive SET storage_blob=zeroblob(length(storage_blob))",
        [],
    )
    .unwrap();
    assert!(prune_hot_batch(&mut db, 100, 128).is_err());
    assert_eq!(
        db.query_row("SELECT count(*) FROM task_event_stream", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn old_schema_upgrade_is_idempotent_and_does_not_rewrite_evidence() {
    let db = fixture();
    event(&db, "done", 1, true);
    let before: String = db
        .query_row("SELECT event_json FROM task_event_archive", [], |r| {
            r.get(0)
        })
        .unwrap();
    ensure_schema(&db).unwrap();
    ensure_schema(&db).unwrap();
    let after: String = db
        .query_row("SELECT event_json FROM task_event_archive", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(before, after);
}
