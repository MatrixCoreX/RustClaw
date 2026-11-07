use rusqlite::{params, Connection};

use super::MemoryAvailability;
use crate::memory::capacity_test_fixtures::Fixture;

pub(super) fn observe(fixture: &Fixture, db: &Connection, admin: bool) -> MemoryAvailability {
    let settings = crate::memory::settings::resolve_principal_memory_settings(
        db,
        &fixture.principal,
        fixture.state().policy.memory.long_term_enabled,
    )
    .unwrap();
    super::observe_sqlite(db, settings, admin, 123456, 123)
        .unwrap()
        .0
}

pub(super) fn recent(db: &Connection, fixture: &Fixture, id: &str, content: &str) {
    recent_as(db, &fixture.key, &fixture.principal, id, content);
}

pub(super) fn recent_as(db: &Connection, key: &str, principal: &str, id: &str, content: &str) {
    db.execute(
        "INSERT INTO memories(memory_id,user_id,chat_id,user_key,principal_id,
            scope_kind,scope_ref,channel,role,content,created_at,created_at_ts,
            memory_type,safety_flag,origin,row_revision,legacy_scope_inferred)
         VALUES (?1,1,7,?2,?3,'principal',?3,'ui','user',?4,'100',100,
            'episodic_event','normal','user_confirmed',1,0)",
        params![id, key, principal, content],
    )
    .unwrap();
}

pub(super) fn measured(value: &super::ByteMeasurement) -> u64 {
    match value {
        super::ByteMeasurement::Measured { bytes } => *bytes,
        other => panic!("expected actual byte measurement, got {other:?}"),
    }
}

pub(super) fn derived_fact(db: &Connection, fixture: &Fixture) {
    db.execute(
        "INSERT INTO memory_facts(memory_id,user_id,chat_id,user_key,principal_id,
            scope_kind,scope_ref,namespace,fact_key,fact_value,fact_text,source_kind,
            source_ref,created_at_ts,updated_at_ts,status,origin,row_revision,
            legacy_scope_inferred,source_refs_json,evidence_refs_json,trust_tier,
            sensitivity,last_verified_at_ts,modified_at_ts)
         VALUES ('derived',1,7,?1,?2,'principal',?2,'fixture_profile','derived',?3,?3,
            'user_message','fixture',100,100,'active','user_confirmed',1,0,
            '[]','[]','user_confirmed','normal',100,100)",
        params![fixture.key, fixture.principal, "derived".repeat(1000)],
    )
    .unwrap();
}
