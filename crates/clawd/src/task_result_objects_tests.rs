use super::*;
fn result() -> Value {
    json!({"text":"visible reply", "task_lifecycle":{"resume_executor_result_projection":{"executor_result_status":"ok","final_result_json":{"text":"完整结果🌿".repeat(5000)}}}})
}
fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE tasks(task_id TEXT PRIMARY KEY,status TEXT,result_json TEXT,updated_at TEXT)",
    )
    .unwrap();
    db
}
#[test]
fn immutable_snapshot_roundtrips_without_changing_visible_result() {
    let db = db();
    let mut value = result();
    let snapshot = value.pointer(PROJECTION).unwrap()["final_result_json"].clone();
    assert!(compact_projection(&db, "one", &mut value).unwrap() > 16000);
    assert_eq!(value["text"], "visible reply");
    let reference = &value.pointer(PROJECTION).unwrap()["final_result_ref"];
    assert_eq!(read_snapshot(&db, "one", reference).unwrap(), snapshot);
    assert!(read_snapshot(&db, "other", reference).is_err());
    assert_eq!(compact_projection(&db, "one", &mut value).unwrap(), 0);
    let mut same = result();
    compact_projection(&db, "one", &mut same).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM task_event_artifacts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn corrupt_snapshot_cannot_replace_inline_evidence() {
    let db = db();
    let mut value = result();
    compact_projection(&db, "one", &mut value).unwrap();
    db.execute("UPDATE task_event_artifacts SET storage_blob=x'00'", [])
        .unwrap();
    let original = result();
    let mut unchanged = original.clone();
    assert!(compact_projection(&db, "one", &mut unchanged).is_err());
    assert_eq!(unchanged, original);
}
#[test]
fn bounded_backfill_preserves_running_and_resumes_idempotently() {
    let mut db = db();
    for (id, status) in [
        ("running", "running"),
        ("done", "succeeded"),
        ("bad", "failed"),
    ] {
        db.execute(
            "INSERT INTO tasks VALUES(?1,?2,?3,'old')",
            params![
                id,
                status,
                if id == "bad" {
                    "{".into()
                } else {
                    result().to_string()
                }
            ],
        )
        .unwrap();
    }
    assert_eq!(compact_batch(&mut db, 1).unwrap().compacted_rows, 0);
    let report = compact_batch(&mut db, 1).unwrap();
    assert_eq!(report.scanned_rows, 1);
    assert_eq!(report.compacted_rows, 1);
    assert_eq!(compact_batch(&mut db, 64).unwrap().compacted_rows, 0);
    assert_eq!(compact_batch(&mut db, 64).unwrap().scanned_rows, 0);
    let raw: String = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id='running'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw, result().to_string());
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM tasks WHERE updated_at='old'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
}
#[test]
fn small_results_and_invalid_references_are_not_rewritten() {
    let db = db();
    let mut small = json!({"task_lifecycle":{"resume_executor_result_projection":{"final_result_json":{"text":"small"}}}});
    let copy = small.clone();
    assert_eq!(compact_projection(&db, "small", &mut small).unwrap(), 0);
    assert_eq!(copy, small);
    for reference in [
        json!({}),
        json!({"schema_version":1,"kind":"task_event_payload","sha256":"a".repeat(64),"artifact_id":"other"}),
    ] {
        assert!(!valid_reference(&reference));
    }
}
