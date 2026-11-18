use rusqlite::{types::Value as SqlValue, Connection};
use sha2::{Digest, Sha256};

use super::super::boundary_test_fixtures as setup;
use super::super::{export_memory, MemoryImportPreviewRequest};
use crate::memory::capacity_test_fixtures::Fixture;

pub(super) fn fixture(file: bool) -> Fixture {
    let fixture = if file {
        Fixture::file()
    } else {
        setup::fixture()
    };
    {
        let db = fixture.state().core.db.get().unwrap();
        super::super::ensure_memory_ux_schema(&db).unwrap();
        setup::fact(
            &db,
            &fixture.key,
            &fixture.principal,
            "witness",
            "Preserved fact",
            100,
        );
    }
    fixture
}

pub(super) fn request(db: &Connection, owner: &str) -> MemoryImportPreviewRequest {
    let mut export = export_memory(db, owner, 100).unwrap();
    assert_eq!(export.items.len(), 1);
    export.items[0].content = "A distinct imported fact".into();
    export.checksum = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&export.items).unwrap())
    );
    MemoryImportPreviewRequest { export }
}

pub(super) fn snapshot(
    db: &Connection,
    owner: &str,
) -> (u64, Vec<(&'static str, Vec<Vec<SqlValue>>)>) {
    let state = [
        "memory_facts",
        "user_preferences",
        "memory_import_sessions",
        "memory_revisions",
        "memory_principal_quotas",
    ]
    .into_iter()
    .map(|table| {
        let mut statement = db
            .prepare(&format!(
                "SELECT * FROM {table} WHERE principal_id=?1 ORDER BY rowid"
            ))
            .unwrap();
        let count = statement.column_count();
        let rows = statement
            .query_map([owner], |row| {
                (0..count)
                    .map(|column| row.get::<_, SqlValue>(column))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (table, rows)
    })
    .collect();
    (db.total_changes(), state)
}
