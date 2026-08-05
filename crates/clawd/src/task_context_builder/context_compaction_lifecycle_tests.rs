use serde_json::json;

use super::{
    begin_context_compaction, complete_context_compaction,
    ensure_context_compaction_lifecycle_schema, invalidate_compactions_after_rewind,
};

fn fixture() -> (
    crate::AppState,
    crate::ClaimedTask,
    super::super::ContextCompactionPlan,
) {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().expect("db");
    crate::repo::ensure_principal_ownership_schema(&db).expect("ownership");
    db.execute(
        "INSERT OR IGNORE INTO auth_keys(user_key, role, enabled, created_at)
         VALUES ('compact-key', 'user', 1, '1')",
        [],
    )
    .expect("auth key");
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("principal");
    let principal_id = crate::repo::auth::principal_id_for_user_key(&db, "compact-key")
        .expect("principal query")
        .expect("principal id");
    let payload = json!({"conversation_id":"conversation-compact"}).to_string();
    db.execute(
        "INSERT INTO tasks(task_id,user_id,chat_id,user_key,principal_id,channel,kind,payload_json,status,created_at,updated_at)
         VALUES ('compact-task',7,8,'compact-key',?1,'ui','ask',?2,'running',1,1)",
        rusqlite::params![principal_id, payload],
    )
    .expect("task");
    drop(db);
    let task = crate::ClaimedTask {
        claim_attempt: 1,
        task_id: "compact-task".to_string(),
        user_id: 7,
        chat_id: 8,
        user_key: Some("compact-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: payload,
    };
    let bundle =
        crate::task_context_builder::build_agent_loop_task_context_bundle(&state, &task, "", 1_024);
    let mut plan =
        crate::task_context_builder::force_agent_loop_context_compaction_plan(&bundle, Some(1024))
            .expect("plan");
    crate::task_context_builder::hydrate_agent_loop_context_compaction_plan(
        &state, &task, &mut plan,
    );
    (state, task, plan)
}

#[test]
fn lease_and_generation_cas_are_single_writer_and_tail_is_preserved() {
    let (state, task, mut plan) = fixture();
    let lease = begin_context_compaction(&state, &task, &mut plan).expect("first lease");
    let mut competing_plan = plan.clone();
    let error = begin_context_compaction(&state, &task, &mut competing_plan)
        .expect_err("second writer blocked");
    assert_eq!(error.to_string(), "context_compaction_lease_busy");

    let db = state.core.db.get().expect("db");
    let principal_id = crate::repo::auth::principal_id_for_user_key(&db, "compact-key")
        .expect("principal query")
        .expect("principal id");
    db.execute(
        "INSERT INTO tasks(task_id,user_id,chat_id,user_key,principal_id,channel,kind,payload_json,status,created_at,updated_at)
         VALUES ('tail-task',7,8,'compact-key',?1,'ui','ask',?2,'queued',2,2)",
        rusqlite::params![principal_id, task.payload_json],
    )
    .expect("tail task");
    drop(db);
    let commit = complete_context_compaction(
        &state,
        &task,
        &lease,
        &json!({"compaction_id":"context_compaction:test","generation":plan.generation}),
    )
    .expect("commit");
    assert_eq!(commit.uncovered_tail_task_count, 1);
    assert_eq!(
        commit.record["lifecycle"]["tail_policy"],
        "preserved_for_next_turn"
    );
}

#[test]
fn rewind_invalidates_only_records_beyond_the_anchor_head() {
    let (state, task, mut plan) = fixture();
    let lease = begin_context_compaction(&state, &task, &mut plan).expect("lease");
    complete_context_compaction(
        &state,
        &task,
        &lease,
        &json!({"compaction_id":"context_compaction:rewind","generation":plan.generation}),
    )
    .expect("commit");
    let db = state.core.db.get().expect("db");
    ensure_context_compaction_lifecycle_schema(&db).expect("schema repeat");
    db.execute(
        "UPDATE context_compaction_records SET snapshot_task_row_id = snapshot_task_row_id + 1",
        [],
    )
    .expect("move record beyond anchor");
    drop(db);
    let changed =
        invalidate_compactions_after_rewind(&state, &task, "compact-task").expect("invalidate");
    assert_eq!(changed, 1);
}
