use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use uuid::Uuid;

use super::*;

fn temporary_database_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "conversation-input-{label}-{}.sqlite3",
        Uuid::new_v4().simple()
    ))
}

fn remove_database_files(path: &Path) {
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-shm", path.display())),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-journal", path.display())),
    ] {
        let _ = std::fs::remove_file(candidate);
    }
}

#[test]
fn writer_lock_cannot_publish_a_partial_input_receipt() {
    let path = temporary_database_path("busy");
    let mut owner = Connection::open(&path).expect("open owner database");
    ensure_conversation_input_schema(&owner).expect("initialize input schema");
    let owner_transaction = owner
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("hold writer lock");

    let mut contender = Connection::open(&path).expect("open contender database");
    contender
        .busy_timeout(Duration::ZERO)
        .expect("disable sqlite wait");
    let error = accept_conversation_input_in_db(&mut contender, &input("busy-input"), 100)
        .expect_err("locked database must reject acceptance");
    assert!(matches!(error, ConversationInputStoreError::Database(_)));

    owner_transaction.rollback().expect("release writer lock");
    let input_count: i64 = owner
        .query_row("SELECT COUNT(*) FROM conversation_inputs", [], |row| {
            row.get(0)
        })
        .expect("count inputs");
    let scope_count: i64 = owner
        .query_row(
            "SELECT COUNT(*) FROM conversation_input_scopes",
            [],
            |row| row.get(0),
        )
        .expect("count scopes");
    assert_eq!(input_count, 0);
    assert_eq!(scope_count, 0);
    drop(contender);
    drop(owner);
    remove_database_files(&path);
}

#[test]
fn sqlite_full_rolls_back_scope_input_and_event_together() {
    let path = temporary_database_path("full");
    let mut database = Connection::open(&path).expect("open database");
    database
        .execute_batch("PRAGMA page_size=512; PRAGMA journal_mode=DELETE;")
        .expect("configure small database pages");
    ensure_conversation_input_schema(&database).expect("initialize input schema");
    let page_count: i64 = database
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .expect("read page count");
    database
        .execute_batch(&format!("PRAGMA max_page_count={page_count};"))
        .expect("freeze database capacity");

    let mut request = input("full-input");
    request.submission.content = vec![ConversationInputContent::Text {
        text: "x".repeat(128 * 1024),
    }];
    let error = accept_conversation_input_in_db(&mut database, &request, 100)
        .expect_err("database capacity limit must reject acceptance");
    assert!(matches!(error, ConversationInputStoreError::Database(_)));

    for table in [
        "conversation_input_scopes",
        "conversation_inputs",
        "conversation_input_events",
    ] {
        let count: i64 = database
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count rows after rollback");
        assert_eq!(count, 0, "{table} must not contain a partial acceptance");
    }
    drop(database);
    remove_database_files(&path);
}
