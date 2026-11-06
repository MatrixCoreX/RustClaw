use super::*;

fn database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(include_str!(
        "../../../../migrations/012_memory_ux_audit.sql"
    ))
    .unwrap();
    db
}

fn insert(db: &Connection, id: &str, owner: &str, digest: &str, payload: &str) {
    db.execute(
        "INSERT INTO memory_import_sessions(import_id,principal_id,scope_kind,scope_ref,
        payload_digest,preview_json,status,created_at_ts)
        VALUES (?1,?2,'principal',?2,?3,?4,'preview',1)",
        params![id, owner, digest, payload],
    )
    .unwrap();
}

fn snapshot(db: &Connection, owner: &str) -> Vec<(String, String, String, String)> {
    db.prepare(
        "SELECT import_id,payload_digest,preview_json,status FROM memory_import_sessions
         WHERE principal_id=?1 ORDER BY import_id",
    )
    .unwrap()
    .query_map([owner], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })
    .unwrap()
    .collect::<Result<Vec<_>, _>>()
    .unwrap()
}

fn assert_redacted(rows: &[(String, String, String, String)]) {
    for (id, digest, payload, status) in rows {
        let value: Value = serde_json::from_str(payload).unwrap();
        assert_eq!(value["purged"], true);
        assert_eq!(value["import_id"].as_str(), Some(id.as_str()));
        assert!(!payload.contains("private source"));
        assert_eq!(
            *digest,
            format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
        );
        assert_eq!(status, "cancelled");
    }
}

#[test]
fn cursor_clears_empty_legacy_and_forward_backward_moving_keys() {
    let db = database();
    for index in 0..257 {
        let digest = match index {
            0 => String::new(),
            1..=128 => format!("{index:04}-legacy"),
            _ => format!("z-{index:04}-legacy"),
        };
        insert(
            &db,
            &format!("owned-{index}"),
            "owner",
            &digest,
            "private source",
        );
        insert(
            &db,
            &format!("foreign-{index}"),
            "foreign",
            &digest,
            "foreign source",
        );
    }
    let foreign = snapshot(&db, "foreign");
    db.execute_batch(
        "CREATE TABLE updates(import_id TEXT, old_digest TEXT, new_digest TEXT);
         CREATE TRIGGER trace_import_update AFTER UPDATE ON memory_import_sessions
         BEGIN INSERT INTO updates VALUES (NEW.import_id,OLD.payload_digest,NEW.payload_digest); END;
         BEGIN IMMEDIATE;",
    )
    .unwrap();
    purge_import_sessions(&db, "owner", true).unwrap();
    db.execute_batch("COMMIT").unwrap();
    let rows = snapshot(&db, "owner");
    assert_eq!(rows.len(), 257);
    assert_redacted(&rows);
    assert_eq!(snapshot(&db, "foreign"), foreign);
    for direction in ["old_digest < new_digest", "old_digest > new_digest"] {
        let count: i64 = db
            .query_row(
                &format!("SELECT COUNT(*) FROM updates WHERE {direction}"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count > 0, "fixture must exercise {direction}");
    }
    let most_updates: i64 = db
        .query_row(
            "SELECT MAX(n) FROM (SELECT COUNT(*) AS n FROM updates GROUP BY import_id)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(most_updates <= 2, "idempotent normalization must not loop");
}

#[test]
fn transcript_cursor_preserves_unaffected_staging_and_foreign_owner() {
    let db = database();
    for (id, owner, digest, payload) in [
        (
            "recent-empty",
            "owner",
            "",
            r#"{"items":[{"kind":"recent","content":"private source"}]}"#,
        ),
        (
            "recent-high",
            "owner",
            "z-high",
            r#"{"items":[{"kind":"recent"},{"kind":"fact"}]}"#,
        ),
        (
            "derived",
            "owner",
            "a-derived",
            r#"{"items":[{"kind":"fact","content":"keep"}]}"#,
        ),
        (
            "unknown",
            "owner",
            "b-malformed",
            "malformed legacy payload",
        ),
        ("foreign", "foreign", "", r#"{"items":[{"kind":"recent"}]}"#),
    ] {
        insert(&db, id, owner, digest, payload);
    }
    let before = snapshot(&db, "owner");
    let foreign = snapshot(&db, "foreign");
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    purge_import_sessions(&db, "owner", false).unwrap();
    db.execute_batch("COMMIT").unwrap();
    let after = snapshot(&db, "owner");
    for original in before {
        let current = after.iter().find(|row| row.0 == original.0).unwrap();
        if original.0.starts_with("recent-") {
            assert_redacted(std::slice::from_ref(current));
        } else {
            assert_eq!(current, &original);
        }
    }
    assert_eq!(snapshot(&db, "foreign"), foreign);
}

#[test]
fn repeated_cursor_clear_has_stable_tombstones_and_payload_bindings() {
    let db = database();
    let original = format!("sha256:{}", "a".repeat(64));
    insert(&db, "valid", "owner", &original, "private source");
    insert(
        &db,
        "legacy",
        "owner",
        "z-invalid",
        r#"{"purged":true,"original_payload_digest":"not a digest"}"#,
    );
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    purge_import_sessions(&db, "owner", true).unwrap();
    let first = snapshot(&db, "owner");
    assert_redacted(&first);
    purge_import_sessions(&db, "owner", true).unwrap();
    assert_eq!(snapshot(&db, "owner"), first);
    let valid = first.iter().find(|row| row.0 == "valid").unwrap();
    let payload: Value = serde_json::from_str(&valid.2).unwrap();
    assert_eq!(payload["original_payload_digest"], original);
    insert(
        &db,
        "explicit-fresh-upload",
        "owner",
        &original,
        "fresh source",
    );
    db.execute_batch("COMMIT").unwrap();
}

#[test]
fn cursor_failure_after_partial_progress_rolls_back_all_staging() {
    let db = database();
    insert(&db, "first", "owner", "a-low", "private source");
    insert(&db, "last", "owner", "z-high", "private source");
    let before = snapshot(&db, "owner");
    db.execute_batch(
        "CREATE TRIGGER reject_last BEFORE UPDATE ON memory_import_sessions
         WHEN OLD.import_id='last' BEGIN SELECT RAISE(ABORT,'fixture'); END;
         BEGIN IMMEDIATE;",
    )
    .unwrap();
    assert!(purge_import_sessions(&db, "owner", true).is_err());
    assert_ne!(
        snapshot(&db, "owner"),
        before,
        "failure must follow a real update"
    );
    db.execute_batch("ROLLBACK").unwrap();
    assert_eq!(snapshot(&db, "owner"), before);
}
