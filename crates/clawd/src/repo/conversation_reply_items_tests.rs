use super::*;
use rusqlite::params;

fn reply_state(channel: &str) -> (AppState, ClaimedTask) {
    let state = AppState::test_default_with_fixture_provider();
    let task_id = uuid::Uuid::new_v4().to_string();
    let payload = serde_json::json!({
        "channel_ingress": {
            "schema_version": 1,
            "channel": channel,
            "adapter": "test_adapter",
            "account_id": "account-1",
            "external_user_id": "user-1",
            "external_chat_id": "chat-1",
            "reply_target": {"kind": "chat", "external_id": "chat-1"}
        }
    });
    let db = state.core.db.get().expect("db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT,
            principal_id TEXT,
            channel TEXT NOT NULL,
            external_user_id TEXT,
            external_chat_id TEXT,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL,
            result_json TEXT,
            error_text TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            lease_owner TEXT,
            claim_attempt INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("tasks");
    ensure_conversation_reply_item_schema(&db).expect("reply schema");
    crate::repo::conversation_inputs::ensure_conversation_input_schema(&db).expect("input schema");
    db.execute(
        "INSERT INTO tasks (
             task_id, user_id, chat_id, user_key, principal_id, channel,
             external_user_id, external_chat_id, kind, payload_json, status,
             created_at, updated_at, lease_owner, claim_attempt
         ) VALUES (?1, 1, 1, 'key-1', 'principal-1', ?2,
                   'user-1', 'chat-1', 'ask', ?3, 'running', '1', '1', ?4, 3)",
        params![
            task_id,
            channel,
            payload.to_string(),
            state.worker.worker_id
        ],
    )
    .expect("task");
    drop(db);
    let task = ClaimedTask {
        task_id,
        user_id: 1,
        chat_id: 1,
        user_key: Some("key-1".to_string()),
        channel: channel.to_string(),
        external_user_id: Some("user-1".to_string()),
        external_chat_id: Some("chat-1".to_string()),
        kind: "ask".to_string(),
        payload_json: payload.to_string(),
        claim_attempt: 3,
    };
    (state, task)
}

#[test]
fn nonterminal_reply_is_idempotent_and_channel_delivery_is_claimed() {
    let (state, task) = reply_state("telegram");
    let first = persist_nonterminal_reply_item(
        &state,
        &task,
        "side_reply",
        "accepted",
        "The main task is still running.",
        4,
        2,
    )
    .expect("persist");
    assert!(first.inserted);
    let replay = persist_nonterminal_reply_item(
        &state,
        &task,
        "side_reply",
        "accepted",
        "The main task is still running.",
        4,
        2,
    )
    .expect("replay");
    assert!(!replay.inserted);
    assert_eq!(replay.item.reply_id, first.item.reply_id);

    let claim = claim_due_conversation_reply_delivery(&state.core.db, 100, 30)
        .expect("claim")
        .expect("due");
    assert_eq!(claim.reply_id, first.item.reply_id);
    finish_conversation_reply_delivery(&state.core.db, &claim, true, None, None, 101)
        .expect("finish");
    assert!(
        claim_due_conversation_reply_delivery(&state.core.db, 200, 30)
            .expect("claim")
            .is_none()
    );
}

#[test]
fn ui_reply_does_not_create_channel_delivery_and_stale_claim_is_rejected() {
    let (state, mut task) = reply_state("ui");
    let outcome = persist_nonterminal_reply_item(
        &state,
        &task,
        "clarification",
        "accepted",
        "Which directory should I use?",
        1,
        1,
    )
    .expect("persist");
    assert!(outcome.inserted);
    assert!(
        claim_due_conversation_reply_delivery(&state.core.db, 100, 30)
            .expect("claim")
            .is_none()
    );

    task.claim_attempt = 2;
    let error = persist_nonterminal_reply_item(
        &state,
        &task,
        "side_reply",
        "accepted",
        "This must not be committed.",
        2,
        2,
    )
    .expect_err("stale claim");
    assert!(error.to_string().contains("worker_lease_lost"));
}

#[test]
fn machine_control_status_roundtrips_message_key_and_channel_outbox() {
    let (state, task) = reply_state("telegram");
    let db = state.core.db.get().expect("db");
    let persisted = persist_control_status_reply_item_in_db(
        &db,
        &task.task_id,
        "stop_requested",
        "channel.control.cancel_requested",
        &BTreeMap::new(),
        true,
    )
    .expect("persist machine status");
    drop(db);

    assert!(persisted.inserted);
    assert!(persisted.item.text.is_empty());
    assert_eq!(
        persisted.item.message_key.as_deref(),
        Some("channel.control.cancel_requested")
    );
    let loaded = get_conversation_reply_item(&state.core.db, &persisted.item.reply_id)
        .expect("load")
        .expect("reply");
    assert_eq!(loaded, persisted.item);
    assert_eq!(
        claim_due_conversation_reply_delivery(&state.core.db, 100, 30)
            .expect("claim")
            .expect("outbox")
            .reply_id,
        persisted.item.reply_id
    );
}

#[test]
fn clarification_reply_and_needs_user_checkpoint_commit_together() {
    let (state, task) = reply_state("ui");
    let mut loop_state = crate::agent_engine::LoopState::new();
    loop_state.round_no = 2;
    loop_state.conversation_input_revision = 7;
    loop_state.conversation_execution_epoch = 5;

    let persisted = crate::agent_engine::persist_agent_loop_clarification_checkpoint(
        &state,
        &task,
        &mut loop_state,
        "Which directory should I use?",
        serde_json::json!({
            "terminal_intent": "clarify",
            "clarify_reason_code": "missing_locator",
            "missing_slot": "locator",
            "field_path": "output_contract.locator_hint",
            "locator_kind": "path"
        }),
    )
    .expect("persist clarification checkpoint");

    assert_eq!(persisted.item.relation, "clarification");
    assert_eq!(persisted.item.lifecycle_stage, "accepted");
    assert_eq!(persisted.item.instruction_revision, 7);
    assert_eq!(persisted.item.execution_epoch, 5);
    assert_eq!(
        loop_state
            .task_lifecycle
            .as_ref()
            .and_then(|value| value.get("state"))
            .and_then(serde_json::Value::as_str),
        Some("needs_user")
    );
    assert!(crate::task_lifecycle::has_matching_nonterminal_checkpoint(
        loop_state.task_lifecycle.as_ref(),
        loop_state.task_checkpoint.as_ref(),
    ));

    let db = state.core.db.get().expect("db");
    let raw_result = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1",
            [&task.task_id],
            |row| row.get::<_, String>(0),
        )
        .expect("checkpointed task result");
    let result: serde_json::Value = serde_json::from_str(&raw_result).expect("result json");
    assert_eq!(result["task_lifecycle"]["state"], "needs_user");
    assert_eq!(
        result["task_lifecycle"]["reply_id"],
        persisted.item.reply_id
    );
    assert_eq!(
        result["task_checkpoint"]["boundary_context"]["clarification"]["missing_slot"],
        "locator"
    );
    drop(db);
    assert!(
        claim_due_conversation_reply_delivery(&state.core.db, 100, 30)
            .expect("claim")
            .is_none()
    );
}
