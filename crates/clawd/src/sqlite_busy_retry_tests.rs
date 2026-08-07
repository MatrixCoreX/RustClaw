use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

fn busy_error() -> anyhow::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
        Some("fixture busy".to_string()),
    )
    .into()
}

#[test]
fn transient_busy_is_retried_and_eventually_commits_once() {
    let attempts = AtomicU32::new(0);
    let value = with_sqlite_busy_retry(
        SqliteBusyRetryPolicy {
            max_attempts: 4,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        },
        || {
            let current = attempts.fetch_add(1, Ordering::SeqCst);
            if current < 2 {
                Err(busy_error())
            } else {
                Ok("committed")
            }
        },
    )
    .expect("transient busy should recover");
    assert_eq!(value, "committed");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[test]
fn permanent_busy_stops_at_bounded_attempt_count() {
    let attempts = AtomicU32::new(0);
    let error = with_sqlite_busy_retry(
        SqliteBusyRetryPolicy {
            max_attempts: 3,
            base_delay: Duration::ZERO,
            max_delay: Duration::ZERO,
        },
        || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(busy_error())
        },
    )
    .expect_err("permanent busy must remain visible");
    assert!(is_sqlite_busy(&error));
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[test]
fn real_sqlite_writer_lock_recovers_after_owner_commits() {
    let path = std::env::temp_dir().join(format!(
        "agent-sqlite-busy-{}.db",
        uuid::Uuid::new_v4().simple()
    ));
    let mut owner = rusqlite::Connection::open(&path).unwrap();
    owner
        .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE events(id INTEGER PRIMARY KEY);")
        .unwrap();
    let owner_tx = owner
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    owner_tx
        .execute("INSERT INTO events(id) VALUES (1)", [])
        .unwrap();

    let contender_path = path.clone();
    let contender = std::thread::spawn(move || {
        let connection = rusqlite::Connection::open(contender_path).unwrap();
        connection.busy_timeout(Duration::ZERO).unwrap();
        with_sqlite_busy_retry(
            SqliteBusyRetryPolicy {
                max_attempts: 20,
                base_delay: Duration::from_millis(5),
                max_delay: Duration::from_millis(20),
            },
            || {
                connection
                    .execute("INSERT INTO events(id) VALUES (2)", [])
                    .map(|_| ())
                    .map_err(Into::into)
            },
        )
    });
    std::thread::sleep(Duration::from_millis(35));
    owner_tx.commit().unwrap();
    contender
        .join()
        .expect("contender thread")
        .expect("bounded retry should recover");

    let count: i64 = owner
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
    drop(owner);
    let _ = std::fs::remove_file(path);
}
