use super::*;

#[test]
fn typed_sqlite_codes_survive_context_without_reading_error_prose() {
    let error = anyhow::Error::new(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY_SNAPSHOT),
        Some("unrelated provider prose".into()),
    ))
    .context("fixture storage context");
    assert_eq!(
        tick_error_detail(&error),
        TickErrorDetail {
            error_code: "background_database_busy",
            sqlite_extended_code: Some(rusqlite::ffi::SQLITE_BUSY_SNAPSHOT),
        }
    );
    let locked = anyhow::Error::new(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_LOCKED_SHAREDCACHE),
        None,
    ));
    assert_eq!(
        tick_error_detail(&locked).error_code,
        "background_database_locked"
    );
    let prose = anyhow::anyhow!("database is locked");
    assert_eq!(
        tick_error_detail(&prose),
        TickErrorDetail {
            error_code: "background_tick_failed",
            sqlite_extended_code: None,
        }
    );
}

#[tokio::test]
async fn real_writer_contention_is_classified_then_recovers_without_duplicate_write() {
    let directory = std::env::temp_dir().join(format!(
        "worker-contention-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("fixture.sqlite");
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer
        .execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE witness(value INTEGER); BEGIN IMMEDIATE;",
        )
        .unwrap();
    let monitor = WorkerMonitor::named("fixture_database_write".into());
    let completed = Arc::new(AtomicBool::new(false));
    let (target, done) = (path.clone(), completed.clone());
    let handle = start(monitor.clone(), move || {
        let (target, done) = (target.clone(), done.clone());
        async move {
            if done.load(Ordering::Acquire) {
                return Ok(());
            }
            super::super::super::background_workers::run_blocking_tick(
                "fixture_database_write",
                move || {
                    let connection = rusqlite::Connection::open(target)?;
                    connection.busy_timeout(Duration::from_millis(5))?;
                    connection.execute("INSERT INTO witness VALUES (1)", [])?;
                    done.store(true, Ordering::Release);
                    Ok(())
                },
            )
            .await
        }
    });
    wait_for(|| {
        monitor.last_error_code.lock().unwrap().as_deref() == Some("background_database_busy")
    })
    .await;
    assert_eq!(monitor.name, "fixture_database_write");
    assert_eq!(monitor.restarts.load(Ordering::Acquire), 0);
    writer.execute_batch("COMMIT").unwrap();
    wait_for(|| completed.load(Ordering::Acquire)).await;
    wait_for(|| monitor.last_error_code.lock().unwrap().is_none()).await;
    handle.abort();
    let _ = handle.await;
    let count: i64 = writer
        .query_row("SELECT count(*) FROM witness", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    drop(writer);
    std::fs::remove_dir_all(directory).unwrap();
}
