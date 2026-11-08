use rusqlite::Connection;

// Security floor documented by SQLite's WAL-reset advisory. This observes the
// library actually linked into the native program, not the host sqlite3 tool.
#[test]
fn bundled_sqlite_includes_wal_reset_fix() {
    let db = Connection::open_in_memory().unwrap();
    let (version, source_id): (String, String) = db
        .query_row("SELECT sqlite_version(), sqlite_source_id()", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(version, rusqlite::version());
    assert!(
        rusqlite::version_number() >= 3_051_003,
        "bundled_sqlite_without_wal_reset_fix:{}",
        rusqlite::version_number()
    );
    let source_hash = source_id.split_ascii_whitespace().last().unwrap();
    assert_eq!(source_hash.len(), 64);
    assert!(source_hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let options = db
        .prepare("PRAGMA compile_options")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    println!(
        "BUNDLED_SQLITE_SECURITY {}",
        serde_json::json!({
            "version": version,
            "version_number": rusqlite::version_number(),
            "source_id": source_id,
            "compile_options": options,
        })
    );
}

// Preserve the checked conversion contract used by counters and delivery
// cursors; enabling the upstream feature must not introduce wrapping casts.
#[test]
fn unsigned_sqlite_conversions_remain_checked() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE witness(value INTEGER NOT NULL CHECK(value>=0));")
        .unwrap();
    let expected = vec![0_u64, 7, i64::MAX as u64];
    for value in &expected {
        db.execute("INSERT INTO witness VALUES(?1)", [value])
            .unwrap();
    }
    let actual = db
        .prepare("SELECT value FROM witness ORDER BY value")
        .unwrap()
        .query_map([], |row| row.get::<_, u64>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(actual, expected);
    assert!(matches!(
        db.execute("INSERT INTO witness VALUES(?1)", [u64::MAX]),
        Err(rusqlite::Error::ToSqlConversionFailure(_))
    ));
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM witness", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 3);
    assert!(matches!(
        db.query_row("SELECT -1", [], |row| row.get::<_, u64>(0)),
        Err(rusqlite::Error::IntegralValueOutOfRange(_, -1))
    ));
    let count: usize = db
        .query_row("SELECT ?1", [3_usize], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 3);
    assert!(db
        .query_row("SELECT -1", [], |row| row.get::<_, usize>(0))
        .is_err());
    if usize::BITS == 64 {
        assert!(matches!(
            db.query_row("SELECT ?1", [usize::MAX], |row| row.get::<_, i64>(0)),
            Err(rusqlite::Error::ToSqlConversionFailure(_))
        ));
    }
    let absent: Option<u64> = db.query_row("SELECT NULL", [], |row| row.get(0)).unwrap();
    assert_eq!(absent, None);
}
