use super::*;
use crate::memory::availability::{self, ByteMeasurement, MemoryAvailability};
pub(super) use crate::memory::capacity_test_fixtures::Fixture;
use rusqlite::{types::Value as SqlValue, Connection};
use std::collections::BTreeMap;

pub(super) const ORIGINAL: &str = "Original retained fact";
pub(super) fn fixture(file: bool) -> Fixture {
    let fixture = if file {
        Fixture::file()
    } else {
        Fixture::memory()
    };
    let (key, owner) = setup::another_owner(&fixture);
    {
        let db = fixture.state().core.db.get().unwrap();
        ensure_memory_ux_schema(&db).unwrap();
        setup::fact(
            &db,
            &fixture.key,
            &fixture.principal,
            "witness",
            ORIGINAL,
            100,
        );
        setup::fact(
            &db,
            &key,
            &owner,
            "other-witness",
            "Other owner retained fact",
            100,
        );
        crate::memory::retention::initialize_storage_pressure(
            &db,
            &fixture.state().policy.memory,
            100,
        )
        .unwrap();
    }
    fixture
}
pub(super) fn report(db: &Connection, fixture: &Fixture) -> (MemoryAvailability, Option<String>) {
    let settings = crate::memory::settings::resolve_principal_memory_settings(
        db,
        &fixture.principal,
        fixture.state().policy.memory.long_term_enabled,
    )
    .unwrap();
    availability::observe_sqlite(
        db,
        settings,
        true,
        fixture.state().policy.memory.storage_soft_limit_bytes,
        102,
    )
    .unwrap()
}
pub(super) fn bytes(value: &ByteMeasurement) -> u64 {
    match value {
        ByteMeasurement::Measured { bytes } => *bytes,
        other => panic!("actual measurement required: {other:?}"),
    }
}
pub(super) fn domain(db: &Connection, fixture: &Fixture) -> u64 {
    bytes(
        &report(db, fixture)
            .0
            .measurements
            .memory_domain_assigned_btree_bytes,
    )
}
pub(super) fn limit(fixture: &mut Fixture, value: u64) {
    fixture.state_mut().policy.memory.storage_soft_limit_bytes = value;
    let db = fixture.state().core.db.get().unwrap();
    crate::memory::retention::initialize_storage_pressure(&db, &fixture.state().policy.memory, 102)
        .unwrap();
}
pub(super) fn hard(fixture: &mut Fixture) {
    let value = {
        let db = fixture.state().core.db.get().unwrap();
        domain(&db, fixture)
    };
    limit(fixture, value);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        crate::memory::capacity::StoragePressure::load(&db).unwrap(),
        crate::memory::capacity::StoragePressure::ExplicitWriteBlocked
    );
    assert!(domain(&db, fixture) >= value);
}
pub(super) fn large_correction() -> MemoryCorrectionRequest {
    MemoryCorrectionRequest {
        expected_revision: 1,
        content: "capacity correction data ".repeat(2048),
    }
}
pub(super) fn request(db: &Connection, owner: &str) -> MemoryImportPreviewRequest {
    let mut export = export_memory(db, owner, 101).unwrap();
    export.items.retain(|item| item.id == "witness");
    assert_eq!(export.items.len(), 1);
    export.items[0].id = "incoming-budget-fact".into();
    export.items[0].content = "capacity import text ".repeat(1200);
    assert!(export.items[0].content.chars().count() <= 32768);
    checksum(&mut export);
    MemoryImportPreviewRequest { export }
}
pub(super) fn checksum(export: &mut MemoryExport) {
    export.checksum = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&export.items).unwrap())
    );
}
pub(super) fn state(db: &Connection) -> BTreeMap<String, Vec<Vec<SqlValue>>> {
    let mut names=db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND
        (substr(name,1,7)='memory_' OR name IN ('memories','long_term_memories','user_preferences')) ORDER BY name").unwrap();
    let tables = names
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let quoted = table.replace('"', "\"\"");
            let statement = db.prepare(&format!("SELECT * FROM \"{quoted}\"")).unwrap();
            let columns = statement.column_count();
            let order = (1..=columns)
                .map(|number| number.to_string())
                .collect::<Vec<_>>()
                .join(",");
            drop(statement);
            let mut statement = db
                .prepare(&format!("SELECT * FROM \"{quoted}\" ORDER BY {order}"))
                .unwrap();
            let rows = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|column| row.get(column))
                        .collect::<rusqlite::Result<Vec<SqlValue>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}
pub(super) fn without_sessions(
    mut value: BTreeMap<String, Vec<Vec<SqlValue>>>,
) -> BTreeMap<String, Vec<Vec<SqlValue>>> {
    value.remove("memory_import_sessions");
    value
}
pub(super) fn preference(fixture: &Fixture, content: &str) {
    let db = fixture.state().core.db.get().unwrap();
    setup::preference(&db, &fixture.key, &fixture.principal, "budget-pref");
    db.execute(
        "UPDATE user_preferences SET pref_value=?1 WHERE memory_id='budget-pref'",
        [content],
    )
    .unwrap();
}
pub(super) fn correct_preference(fixture: &Fixture, content: &str) -> MemoryUndoRequest {
    let db = fixture.state().core.db.get().unwrap();
    let correction = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "budget-pref",
        &MemoryCorrectionRequest {
            expected_revision: 1,
            content: content.into(),
        },
        101,
    )
    .unwrap();
    MemoryUndoRequest {
        revision_id: correction.revision_id.unwrap(),
    }
}
