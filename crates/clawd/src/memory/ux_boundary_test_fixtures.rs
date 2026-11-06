use super::*;
use crate::memory::capacity_test_fixtures::Fixture;

pub(super) fn fixture() -> Fixture {
    let fixture = Fixture::memory();
    {
        let db = fixture.state().core.db.get().unwrap();
        ensure_memory_ux_schema(&db).expect("normal UX schema initialization");
    }
    fixture
}

pub(super) fn another_owner(fixture: &Fixture) -> (String, String) {
    let key = crate::repo::auth::create_auth_key(fixture.state(), "user").unwrap();
    let owner = crate::resolve_auth_identity_by_key(fixture.state(), &key)
        .unwrap()
        .expect("normal active identity")
        .principal_id;
    (key, owner)
}

pub(super) fn fact(db: &Connection, key: &str, owner: &str, id: &str, content: &str, ts: i64) {
    db.execute(
        "INSERT INTO memory_facts(memory_id,user_id,chat_id,user_key,principal_id,
            scope_kind,scope_ref,namespace,fact_key,fact_value,fact_text,source_kind,
            source_ref,created_at_ts,updated_at_ts,status,origin,row_revision,
            legacy_scope_inferred,source_refs_json,evidence_refs_json,trust_tier,
            sensitivity,last_verified_at_ts,modified_at_ts)
         VALUES (?1,1,7,?2,?3,'principal',?3,'fixture_profile',?1,?4,?4,
            'user_message','fixture',?5,?5,'active','user_confirmed',1,0,
            '[]','[]','user_confirmed','normal',?5,?5)",
        params![id, key, owner, content, ts],
    )
    .unwrap();
}

pub(super) fn recent(db: &Connection, key: &str, owner: &str, id: &str, content: &str) {
    db.execute(
        "INSERT INTO memories(memory_id,user_id,chat_id,user_key,principal_id,
            scope_kind,scope_ref,channel,role,content,created_at,created_at_ts,
            memory_type,safety_flag,origin,row_revision,legacy_scope_inferred)
         VALUES (?1,1,7,?2,?3,'principal',?3,'ui','user',?4,'100',100,
            'episodic_event','normal','user_confirmed',1,0)",
        params![id, key, owner, content],
    )
    .unwrap();
}

pub(super) fn preference(db: &Connection, key: &str, owner: &str, id: &str) {
    db.execute(
        "INSERT INTO user_preferences(memory_id,user_id,chat_id,user_key,principal_id,
            scope_kind,scope_ref,pref_key,pref_value,confidence,source,updated_at,
            updated_at_ts,origin,row_revision,legacy_scope_inferred,source_refs_json,
            evidence_refs_json,trust_tier,sensitivity,last_verified_at_ts,modified_at_ts)
         VALUES (?1,1,7,?2,?3,'principal',?3,'response_format','markdown',1.0,
            'user_message','100',100,'user_confirmed',1,0,'[]','[]',
            'user_confirmed','normal',100,100)",
        params![id, key, owner],
    )
    .unwrap();
}

pub(super) fn clear(db: &Connection, owner: &str, mode: &str) {
    let preview = clear_preview(db, owner, mode).unwrap();
    clear_with_mode(
        db,
        owner,
        &MemoryBulkClearRequest {
            mode: mode.into(),
            expected_transcript_rows: preview.transcript_rows,
            expected_derived_rows: preview.derived_rows,
        },
        101,
    )
    .expect("normal confirmed clear");
}

pub(super) fn delete(db: &Connection, owner: &str, id: &str) -> MemoryUndoRequest {
    let deleted = delete_memory_with_revision(
        db,
        owner,
        id,
        &MemoryMutationRequest {
            expected_revision: 1,
        },
        100,
    )
    .expect("normal reversible deletion");
    MemoryUndoRequest {
        revision_id: deleted.revision_id.expect("delete revision"),
    }
}

pub(super) fn preview(db: &Connection, owner: &str, content: &str) -> MemoryImportPreview {
    let items = vec![MemoryListItem {
        id: "source-memory".into(),
        revision: 1,
        kind: "fact".into(),
        scope_kind: "principal".into(),
        origin: "user_confirmed".into(),
        status: "active".into(),
        content: content.into(),
        source: "user_message".into(),
        evidence_available: false,
        trust_tier: "user_confirmed".into(),
        updated_at_ts: 100,
        expires_at_ts: None,
        supersedes_memory_id: None,
        last_recalled_at_ts: None,
        freshness: "fresh".into(),
    }];
    let checksum = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&items).unwrap())
    );
    preview_memory_import(
        db,
        owner,
        &MemoryImportPreviewRequest {
            export: MemoryExport {
                schema_version: 1,
                exported_at_ts: 100,
                scope_kind: "principal".into(),
                items,
                checksum,
            },
        },
        100,
    )
    .expect("normal verified import preview")
}

pub(super) fn confirmation(preview: &MemoryImportPreview) -> MemoryImportConfirmRequest {
    MemoryImportConfirmRequest {
        import_id: preview.import_id.clone(),
        expected_payload_digest: preview.payload_digest.clone(),
    }
}
