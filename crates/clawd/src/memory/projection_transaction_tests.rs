use super::*;

fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE effects(value TEXT);INSERT INTO effects VALUES('original');")
        .unwrap();
    db
}

fn values(db: &Connection) -> Vec<String> {
    let mut statement = db
        .prepare("SELECT value FROM effects ORDER BY rowid")
        .unwrap();
    let values = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    values
}

#[test]
fn standalone_failure_rolls_back_and_releases_the_writer() {
    let db = fixture();
    let result: anyhow::Result<()> = atomic(&db, |db| {
        db.execute("INSERT INTO effects VALUES('partial')", [])?;
        anyhow::bail!("projection_fixture_failure");
    });
    assert_eq!(
        result.unwrap_err().to_string(),
        "projection_fixture_failure"
    );
    assert!(db.is_autocommit());
    assert_eq!(values(&db), ["original"]);
    db.execute("INSERT INTO effects VALUES('later')", [])
        .unwrap();
    assert_eq!(values(&db), ["original", "later"]);
}

#[test]
fn nested_failure_caught_by_parent_rolls_back_only_its_local_effects() {
    let db = fixture();
    let tx = db.unchecked_transaction().unwrap();
    tx.execute("INSERT INTO effects VALUES('parent')", [])
        .unwrap();
    let result: anyhow::Result<()> = atomic(&tx, |db| {
        db.execute("INSERT INTO effects VALUES('partial')", [])?;
        anyhow::bail!("projection_fixture_failure");
    });
    assert!(result.is_err());
    assert!(
        !tx.is_autocommit(),
        "index failure must not finish the parent transaction"
    );
    tx.execute("INSERT INTO effects VALUES('parent-after')", [])
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(values(&db), ["original", "parent", "parent-after"]);
}

#[test]
fn nested_success_does_not_commit_its_parent_transaction() {
    let db = fixture();
    let tx = db.unchecked_transaction().unwrap();
    tx.execute("INSERT INTO effects VALUES('parent')", [])
        .unwrap();
    atomic(&tx, |db| {
        db.execute("INSERT INTO effects VALUES('projection')", [])?;
        Ok(())
    })
    .unwrap();
    assert!(!tx.is_autocommit());
    assert_eq!(values(&tx), ["original", "parent", "projection"]);
    tx.rollback().unwrap();
    assert_eq!(values(&db), ["original"]);
}

#[test]
fn nested_panic_restores_local_state_without_ending_the_parent() {
    let db = fixture();
    let tx = db.unchecked_transaction().unwrap();
    tx.execute("INSERT INTO effects VALUES('parent')", [])
        .unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: anyhow::Result<()> = atomic(&tx, |db| {
            db.execute("INSERT INTO effects VALUES('partial')", [])?;
            panic!("projection_fixture_panic");
        });
    }));
    assert!(panic.is_err());
    assert!(!tx.is_autocommit());
    tx.execute("INSERT INTO effects VALUES('parent-after')", [])
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(values(&db), ["original", "parent", "parent-after"]);
}
