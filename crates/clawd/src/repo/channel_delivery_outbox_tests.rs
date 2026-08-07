use super::*;
use rusqlite::params;

#[test]
fn terminal_channel_task_is_durably_claimed_and_retried() {
    let manager = r2d2_sqlite::SqliteConnectionManager::memory();
    let pool = r2d2::Pool::builder()
        .max_size(1)
        .build(manager)
        .expect("pool");
    let db = pool.get().expect("db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY, user_key TEXT, channel TEXT NOT NULL,
            payload_json TEXT NOT NULL, status TEXT NOT NULL
         );",
    )
    .expect("tasks");
    ensure_channel_delivery_outbox_schema(&db).expect("schema");
    db.execute(
        "INSERT INTO tasks VALUES (?1, 'key', 'telegram', ?2, 'running')",
        params![
            "task-1",
            serde_json::json!({"channel_ingress":{"schema_version":1}}).to_string()
        ],
    )
    .expect("insert");
    db.execute(
        "UPDATE tasks SET status = 'succeeded' WHERE task_id = 'task-1'",
        [],
    )
    .expect("terminal");
    drop(db);

    let first = claim_due_channel_terminal_delivery(&pool, 100, 30)
        .expect("claim")
        .expect("due");
    assert_eq!(first.task_id, "task-1");
    finish_channel_terminal_delivery(&pool, &first, false, Some(10), Some("temporary"), 101)
        .expect("retry");
    assert!(claim_due_channel_terminal_delivery(&pool, 110, 30)
        .expect("claim retry")
        .is_none());
    let second = claim_due_channel_terminal_delivery(&pool, 111, 30)
        .expect("claim retry")
        .expect("due retry");
    assert_eq!(second.attempt_count, 2);
    finish_channel_terminal_delivery(&pool, &second, true, None, None, 112).expect("complete");
    assert!(claim_due_channel_terminal_delivery(&pool, 1_000, 30)
        .expect("claim complete")
        .is_none());
}

#[test]
fn ui_and_non_ingress_tasks_do_not_enter_channel_outbox() {
    let db = rusqlite::Connection::open_in_memory().expect("db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY, user_key TEXT, channel TEXT NOT NULL,
            payload_json TEXT NOT NULL, status TEXT NOT NULL
         );",
    )
    .expect("tasks");
    ensure_channel_delivery_outbox_schema(&db).expect("schema");
    db.execute(
        "INSERT INTO tasks VALUES ('ui-1', 'key', 'ui', ?1, 'succeeded')",
        params![serde_json::json!({"channel_ingress":{"schema_version":1}}).to_string()],
    )
    .expect("ui");
    db.execute(
        "INSERT INTO tasks VALUES ('schedule-1', 'key', 'telegram', '{}', 'succeeded')",
        [],
    )
    .expect("schedule");
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM channel_terminal_delivery_outbox",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(count, 0);
}
