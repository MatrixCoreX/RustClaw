use rusqlite::{params, Connection};

pub(super) fn legacy() -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    {
        let db = state.core.db.get().unwrap();
        db.execute(
            "DELETE FROM runtime_schema_migrations WHERE migration_id=?1",
            [super::MIGRATION_ID],
        )
        .unwrap();
        db.execute_batch("DROP TABLE memory_quota_initializations")
            .unwrap();
    }
    state
}

pub(super) fn identity(state: &crate::AppState, role: &str) -> (String, String) {
    let key = crate::repo::auth::create_auth_key(state, role).unwrap();
    let principal = crate::resolve_auth_identity_by_key(state, &key)
        .unwrap()
        .unwrap()
        .principal_id;
    (key, principal)
}

pub(super) fn source(db: &Connection, key: &str, owner: &str, text: &str) {
    db.execute(
        "INSERT INTO memories(memory_id,user_id,chat_id,user_key,principal_id,
            scope_kind,scope_ref,channel,role,content,created_at,created_at_ts)
         VALUES (?1,1,7,?2,?3,'principal',?3,'ui','user',?4,'100',100)",
        params![uuid::Uuid::new_v4().to_string(), key, owner, text],
    )
    .unwrap();
}

pub(super) fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

pub(super) fn quota(db: &Connection, owner: &str) -> (i64, i64, i64, i64, i64, i64, i64, i64) {
    db.query_row(
        "SELECT max_rows,max_bytes,max_background_cost_microunits,used_rows,used_bytes,
            used_background_cost_microunits,revision,updated_at_ts FROM memory_principal_quotas WHERE principal_id=?1",
        [owner], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)),
    ).unwrap()
}
