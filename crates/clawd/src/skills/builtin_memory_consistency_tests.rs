use super::{args, setup, task};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

async fn save_fact(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    content: &str,
    key: &str,
) -> String {
    let saved: Value = serde_json::from_str(
        &super::super::execute(
            state,
            Some(task),
            &args(json!({
                "action": "save",
                "scope": "current_principal",
                "kind": "fact",
                "content": content,
                "idempotency_key": format!("fixture-consistency-{key}"),
            })),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["status"], "ok", "normal save: {saved}");
    saved["data"]["memory_id"].as_str().unwrap().to_owned()
}

fn owner(state: &crate::AppState, key: &str) -> String {
    crate::resolve_auth_identity_by_key(state, key)
        .unwrap()
        .unwrap()
        .principal_id
}

fn index_texts(db: &Connection, principal: &str) -> Vec<String> {
    let mut statement = db.prepare(
        "SELECT search_text FROM memory_retrieval_index WHERE principal_id=?1 ORDER BY search_text",
    ).unwrap();
    let rows = statement
        .query_map([principal], |row| row.get::<_, String>(0))
        .unwrap();
    rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
}

fn another_account(state: &crate::AppState) -> (crate::ClaimedTask, String) {
    let key = crate::repo::auth::create_auth_key(state, "user").unwrap();
    let identity = crate::resolve_auth_identity_by_key(state, &key)
        .unwrap()
        .unwrap();
    {
        let db = state.core.db.get().unwrap();
        crate::memory::settings::update_memory_settings(
            &db,
            &identity,
            &crate::memory::settings::MemorySettingsUpdateRequest {
                scope: crate::memory::settings::MemorySettingScope::Principal,
                target_principal_id: None,
                conversation_id: None,
                use_mode: Some(crate::memory::settings::MemorySettingMode::Enabled),
                generate_mode: Some(crate::memory::settings::MemorySettingMode::Enabled),
                external_context_policy: Some(
                    crate::memory::settings::ExternalContextPolicy::Exclude,
                ),
                expected_revision: Some(0),
                long_term_enabled: None,
            },
            state.policy.memory.long_term_enabled,
        )
        .unwrap();
    }
    let mut task = task("memory-consistency-other", "other-conversation");
    task.user_key = Some(key);
    task.user_id = identity.user_id;
    task.chat_id = identity.chat_id;
    (task, identity.principal_id)
}

#[tokio::test]
async fn healthy_fact_is_not_reported_or_deleted_as_orphan() {
    let state = setup();
    let task = task("memory-consistency-healthy", "conversation-a");
    let id = save_fact(&state, &task, "A healthy indexed fact", "healthy").await;
    let principal = owner(&state, task.user_key.as_deref().unwrap());
    let db = state.core.db.get().unwrap();
    let before = index_texts(&db, &principal);
    assert_eq!(before, ["A healthy indexed fact"]);
    db.pragma_update(None, "query_only", "ON").unwrap();
    let changes = db.total_changes();
    let report = crate::memory::ux::check_memory_consistency(&db, &principal, false, 200).unwrap();
    assert_eq!(
        db.total_changes(),
        changes,
        "an observation never repairs or initializes"
    );
    assert_eq!(
        report.orphan_retrieval_rows, 0,
        "canonical prefixed references are valid"
    );
    db.pragma_update(None, "query_only", "OFF").unwrap();
    let repaired = crate::memory::ux::check_memory_consistency(&db, &principal, true, 201).unwrap();
    assert_eq!(repaired.repaired_rows, 0);
    assert_eq!(index_texts(&db, &principal), before);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_facts WHERE memory_id=?1 AND principal_id=?2",
            params![id, principal],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1
    );
    assert_eq!(
        crate::memory::ux::check_memory_consistency(&db, &principal, true, 202)
            .unwrap()
            .repaired_rows,
        0
    );
}

#[tokio::test]
async fn repair_removes_only_the_actual_orphan_and_preserves_other_owners() {
    let state = setup();
    let task = task("memory-consistency-mixed", "conversation-a");
    save_fact(&state, &task, "A retained valid fact", "retained").await;
    let missing = save_fact(&state, &task, "An orphaned retrieval fact", "orphan").await;
    let principal = owner(&state, task.user_key.as_deref().unwrap());
    let (other_task, other_owner) = another_account(&state);
    let other_id = save_fact(&state, &other_task, "Other account private fact", "other").await;
    let db = state.core.db.get().unwrap();
    let other_before = index_texts(&db, &other_owner);
    assert_eq!(other_before, ["Other account private fact"]);
    assert_eq!(
        db.execute(
            "DELETE FROM memory_facts WHERE memory_id=?1 AND principal_id=?2",
            params![missing, principal],
        )
        .unwrap(),
        1,
        "simulate a missing canonical row only in the disposable fixture"
    );
    let report = crate::memory::ux::check_memory_consistency(&db, &principal, false, 200).unwrap();
    assert_eq!(
        report.orphan_retrieval_rows, 1,
        "only the genuinely missing fact is orphaned"
    );
    let repaired = crate::memory::ux::check_memory_consistency(&db, &principal, true, 201).unwrap();
    assert_eq!(repaired.repaired_rows, 1);
    assert_eq!(index_texts(&db, &principal), ["A retained valid fact"]);
    assert_eq!(index_texts(&db, &other_owner), other_before);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_facts WHERE memory_id=?1 AND principal_id=?2",
            params![other_id, other_owner],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1
    );
    assert_eq!(
        crate::memory::ux::check_memory_consistency(&db, &principal, true, 202)
            .unwrap()
            .repaired_rows,
        0
    );
}
