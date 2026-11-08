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
