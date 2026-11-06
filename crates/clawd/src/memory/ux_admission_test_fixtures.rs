use rusqlite::{types::Value as SqlValue, Connection};
use sha2::{Digest, Sha256};

use super::super::boundary_test_fixtures as setup;
use super::super::{export_memory, MemoryCorrectionRequest, MemoryImportPreviewRequest};
use crate::memory::capacity_test_fixtures::Fixture;

pub(super) fn fixture() -> Fixture {
    let fixture = setup::fixture();
    {
        let db = fixture.state().core.db.get().unwrap();
        setup::fact(
            &db,
            &fixture.key,
            &fixture.principal,
            "witness",
            "Keep the original fact",
            100,
        );
    }
    fixture
}

pub(super) fn private_state(
    db: &Connection,
    owner: &str,
) -> Vec<(&'static str, Vec<Vec<SqlValue>>)> {
    [
        "memories",
        "long_term_memories",
        "memory_facts",
        "user_preferences",
        "memory_revisions",
        "memory_import_sessions",
        "memory_privacy_purge_queue",
        "memory_retrieval_index",
        "memory_principal_quotas",
        "memory_jobs",
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
    .collect()
}

pub(super) fn import_request(db: &Connection, owner: &str) -> MemoryImportPreviewRequest {
    let mut export = export_memory(db, owner, 101).unwrap();
    assert_eq!(export.items.len(), 1);
    export.items[0].id = "incoming-fact".into();
    export.items[0].content = "A distinct imported fact".into();
    export.checksum = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&export.items).unwrap())
    );
    MemoryImportPreviewRequest { export }
}

pub(super) fn correction() -> MemoryCorrectionRequest {
    MemoryCorrectionRequest {
        expected_revision: 1,
        content: "A corrected fact with new content".into(),
    }
}
