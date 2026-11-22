use super::*;
use serde_json::json;

fn database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE tasks(task_id TEXT PRIMARY KEY,kind TEXT,status TEXT,
        updated_at TEXT,result_json TEXT);
        CREATE TABLE child_task_graph_nodes(child_task_id TEXT PRIMARY KEY);",
    )
    .unwrap();
    db
}
fn result() -> Value {
    json!({"text":"final reply", "task_lifecycle":{"state":"completed"},
        "task_journal":{"summary":{"verified":true},"trace":{
        "rounds":[{"observation":"历史🌿evidence".repeat(4000)}],
        "event_stream":{"events":["event".repeat(5000)]},
        "step_results":[{"step_id":"step_1","status":"ok"}],
        "task_checkpoint":{"checkpoint_id":"preserved","completed_side_effect_refs":["write_1"]},
        "cost_budget":{"spent":17},"answer_verification":{"status":"passed"}}}})
}
fn insert(db: &Connection, id: &str, kind: &str, status: &str, at: i64, value: &Value) {
    db.execute(
        "INSERT INTO tasks VALUES(?1,?2,?3,?4,?5)",
        params![id, kind, status, at.to_string(), value.to_string()],
    )
    .unwrap();
}
fn stored(db: &Connection, id: &str) -> Value {
    serde_json::from_str(
        &db.query_row(
            "SELECT result_json FROM tasks WHERE task_id=?1",
            [id],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap()
}
fn archived() -> (Connection, Value) {
    let mut db = database();
    insert(&db, "owner", "ask", "succeeded", 10, &result());
    assert_eq!(
        compact_batch(&mut db, 64, 100_000).unwrap().compacted_rows,
        1
    );
    let value = stored(&db, "owner");
    (db, value)
}

#[test]
fn cold_fields_roundtrip_and_operational_evidence_stays_inline() {
    let (mut db, mut value) = archived();
    assert!(value.pointer("/task_journal/trace/rounds").is_none());
    for field in [
        "step_results",
        "task_checkpoint",
        "cost_budget",
        "answer_verification",
    ] {
        assert_eq!(
            value.pointer(TRACE).unwrap()[field],
            result().pointer(TRACE).unwrap()[field]
        );
    }
    assert_eq!(value["text"], result()["text"]);
    assert_eq!(
        value["task_journal"]["summary"],
        result()["task_journal"]["summary"]
    );
    assert!(hydrate(&db, "owner", &mut value).unwrap());
    assert_eq!(value, result());
    for _ in 0..3 {
        assert_eq!(
            compact_batch(&mut db, 64, 100_000).unwrap().compacted_rows,
            0
        );
    }
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM runtime_journal_archives", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT updated_at FROM tasks", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "10"
    );
}

#[test]
fn only_old_successful_root_asks_are_eligible() {
    let mut db = database();
    for (id, kind, status, at) in [
        ("running", "ask", "running", 10),
        ("failed", "ask", "failed", 10),
        ("recent", "ask", "succeeded", 101),
        ("skill", "run_skill", "succeeded", 10),
        ("child", "ask", "succeeded", 10),
        ("resume", "ask", "succeeded", 10),
    ] {
        let mut value = result();
        if id == "resume" {
            value["task_lifecycle"]["state"] = json!("background");
        }
        insert(&db, id, kind, status, at, &value);
    }
    db.execute("INSERT INTO child_task_graph_nodes VALUES('child')", [])
        .unwrap();
    let original = db
        .prepare("SELECT task_id,result_json FROM tasks ORDER BY task_id")
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        compact_batch(&mut db, 64, 100_000).unwrap().compacted_rows,
        0
    );
    for (id, raw) in original {
        assert_eq!(
            stored(&db, &id),
            serde_json::from_str::<Value>(&raw).unwrap()
        );
    }
}

#[test]
fn foreign_task_cannot_read_another_archive() {
    let (db, mut value) = archived();
    let before = value.clone();
    assert!(hydrate(&db, "foreign", &mut value).is_err());
    assert_eq!(value, before);
}

#[test]
fn corrupt_archive_fails_without_partial_hydration() {
    let (db, mut value) = archived();
    let before = value.clone();
    db.execute("UPDATE runtime_journal_archives SET storage_blob=x'00'", [])
        .unwrap();
    assert!(hydrate(&db, "owner", &mut value).is_err());
    assert_eq!(value, before);
}

#[test]
fn malformed_reference_and_inline_conflict_fail_closed() {
    let (db, value) = archived();
    for bad in [
        json!({"schema_version":2}),
        Value::Null,
        json!({"schema_version":1,"kind":"other"}),
    ] {
        let mut candidate = value.clone();
        candidate.pointer_mut(TRACE).unwrap()[REFERENCE] = bad;
        assert!(hydrate(&db, "owner", &mut candidate).is_err());
    }
    let mut candidate = value;
    candidate.pointer_mut(TRACE).unwrap()["rounds"] = json!([]);
    let before = candidate.clone();
    assert!(hydrate(&db, "owner", &mut candidate).is_err());
    assert_eq!(candidate, before);
}

#[test]
fn archive_cannot_inject_execution_or_checkpoint_fields() {
    let (db, mut value) = archived();
    let raw = json!({"task_checkpoint":"forged".repeat(5000)}).to_string();
    let stored = crate::event_json_storage::encode(&raw, false).unwrap();
    let digest = hex::encode(Sha256::digest(raw.as_bytes()));
    db.execute(
        "UPDATE runtime_journal_archives SET storage_json=?1,storage_version=?2,
        storage_blob=?3,payload_bytes=?4,sha256=?5",
        params![
            stored.json,
            stored.version,
            stored.blob,
            raw.len() as i64,
            digest
        ],
    )
    .unwrap();
    value.pointer_mut(TRACE).unwrap()[REFERENCE] = json!({"schema_version":1,"kind":"task_journal_fields","payload_bytes":raw.len(),"sha256":digest});
    let before = value.clone();
    assert!(hydrate(&db, "owner", &mut value).is_err());
    assert_eq!(value, before);
}

#[test]
fn failed_publication_rolls_back_payload_and_cursor() {
    let mut db = database();
    insert(&db, "owner", "ask", "succeeded", 10, &result());
    db.execute_batch("CREATE TRIGGER reject_write BEFORE UPDATE ON tasks BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(compact_batch(&mut db, 64, 100_000).is_err());
    assert_eq!(stored(&db, "owner"), result());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM runtime_journal_archives", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT last_rowid FROM runtime_journal_archive_state",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER reject_write;").unwrap();
    assert_eq!(
        compact_batch(&mut db, 64, 100_000).unwrap().compacted_rows,
        1
    );
}

#[test]
fn deleting_task_removes_its_archived_payload_even_without_foreign_key_pragma() {
    let (db, _) = archived();
    db.execute("DELETE FROM tasks WHERE task_id='owner'", [])
        .unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM runtime_journal_archives", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}

#[test]
fn storage_is_opt_in_and_logical_fingerprint_is_encoding_independent() {
    let mut db = database();
    assert!(!enabled(&db).unwrap());
    let original = serde_json::to_string_pretty(&result()).unwrap();
    insert(&db, "owner", "ask", "succeeded", 10, &result());
    let expected = logical_result(&db, "owner", &original).unwrap();
    compact_batch(&mut db, 64, 100_000).unwrap();
    assert!(enabled(&db).unwrap());
    assert_eq!(
        logical_result(&db, "owner", &stored(&db, "owner").to_string()).unwrap(),
        expected
    );
    db.execute(
        "UPDATE runtime_journal_archive_state SET schema_version=2",
        [],
    )
    .unwrap();
    assert!(enabled(&db).is_err());
}
