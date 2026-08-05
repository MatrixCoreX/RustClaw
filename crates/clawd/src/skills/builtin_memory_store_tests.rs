use serde_json::{json, Map, Value};

fn task(task_id: &str, conversation_id: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 1,
        task_id: task_id.to_string(),
        user_id: 41,
        chat_id: 51,
        user_key: Some("memory-capability-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({
            "text": "Please remember that concise answers are preferred.",
            "conversation_id": conversation_id,
        })
        .to_string(),
    }
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("object args")
}

fn setup() -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().expect("db");
    db.execute(
        "INSERT OR IGNORE INTO auth_keys(user_key, role, enabled, created_at)
         VALUES ('memory-capability-key', 'user', 1, '1')",
        [],
    )
    .expect("auth key");
    super::ensure_schema(&db).expect("memory capability schema");
    crate::repo::auth::ensure_principal_identity_schema(&db).expect("principal schema");
    let principal_id = crate::repo::auth::principal_id_for_user_key(&db, "memory-capability-key")
        .expect("resolve principal")
        .expect("principal");
    crate::memory::settings::update_memory_settings(
        &db,
        &claw_core::types::AuthIdentity {
            user_key: "memory-capability-key".to_string(),
            principal_id,
            role: "user".to_string(),
            user_id: 41,
            chat_id: 51,
        },
        &crate::memory::settings::MemorySettingsUpdateRequest {
            scope: crate::memory::settings::MemorySettingScope::Principal,
            target_principal_id: None,
            conversation_id: None,
            use_mode: Some(crate::memory::settings::MemorySettingMode::Enabled),
            generate_mode: Some(crate::memory::settings::MemorySettingMode::Enabled),
            external_context_policy: Some(crate::memory::settings::ExternalContextPolicy::Exclude),
            expected_revision: Some(0),
            long_term_enabled: None,
        },
        state.policy.memory.long_term_enabled,
    )
    .expect("authenticated memory choice");
    drop(db);
    state
}

#[tokio::test]
async fn save_is_effectively_once_and_search_returns_only_opaque_identity() {
    let state = setup();
    let task = task("memory-capability-task", "conversation-a");
    let save = args(json!({
        "action": "save",
        "scope": "current_principal",
        "kind": "preference",
        "key": "answer_style",
        "content": "Prefer concise answers",
        "idempotency_key": "fixture-save-0001",
    }));
    let first = super::execute(&state, Some(&task), &save)
        .await
        .expect("first save");
    let first: Value = serde_json::from_str(&first).expect("first JSON");
    assert_eq!(first["status"], "ok", "first save outcome: {first}");
    assert_eq!(first["data"]["write_status"], "created");
    let memory_id = first["data"]["memory_id"]
        .as_str()
        .expect("memory id")
        .to_string();
    assert!(memory_id.starts_with("memory_"));

    let second = super::execute(&state, Some(&task), &save)
        .await
        .expect("duplicate save");
    let second: Value = serde_json::from_str(&second).expect("second JSON");
    assert_eq!(second["data"]["write_status"], "existing");
    assert_eq!(second["data"]["memory_id"], memory_id);

    let search = args(json!({
        "action": "search",
        "scope": "current_principal",
        "query": "concise",
        "limit": 5,
    }));
    let result = super::execute(&state, Some(&task), &search)
        .await
        .expect("search");
    let result: Value = serde_json::from_str(&result).expect("search JSON");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["data"]["count"], 1);
    assert_eq!(result["data"]["items"][0]["memory_id"], memory_id);
    assert!(result.to_string().find("memory-capability-key").is_none());
}

#[tokio::test]
async fn conversation_scope_isolated_and_child_mutation_is_denied() {
    let state = setup();
    let first_task = task("memory-session-parent", "conversation-a");
    let save = args(json!({
        "action": "save",
        "scope": "current_conversation",
        "kind": "session_note",
        "content": "Temporary note visible only in conversation A",
        "idempotency_key": "fixture-note-0001",
    }));
    let saved = super::execute(&state, Some(&first_task), &save)
        .await
        .expect("save note");
    let saved: Value = serde_json::from_str(&saved).expect("save JSON");
    assert_eq!(saved["status"], "ok", "session save outcome: {saved}");

    let other_task = task("memory-session-other", "conversation-b");
    let search = args(json!({
        "action": "search",
        "scope": "current_conversation",
        "query": "Temporary note",
    }));
    let isolated = super::execute(&state, Some(&other_task), &search)
        .await
        .expect("isolated search");
    let isolated: Value = serde_json::from_str(&isolated).expect("isolated JSON");
    assert_eq!(isolated["data"]["count"], 0);

    let db = state.core.db.get().expect("db");
    db.execute(
        "INSERT INTO child_task_graph_nodes(
            parent_task_id, child_task_id, role, required, readiness,
            permission_profile, merge_policy, owned_paths_json, budget_json,
            model_policy_json, tool_policy_json, result_contract_json,
            steering_version, steering_json, created_at, updated_at
         ) VALUES ('parent', 'memory-child-task', 'worker', 1, 'ready',
                   'read_only', 'review', '[]', '{}', '{}', '{}', '{}', 0, '{}', '1', '1')",
        [],
    )
    .expect("child row");
    drop(db);
    let child_task = task("memory-child-task", "conversation-a");
    let child = super::execute(&state, Some(&child_task), &save)
        .await
        .expect("child outcome");
    let child: Value = serde_json::from_str(&child).expect("child JSON");
    assert_eq!(child["status"], "error");
    assert_eq!(child["error_code"], "memory_child_write_denied");
}

#[tokio::test]
async fn pagination_scope_revision_forget_and_disabled_generation_are_enforced() {
    let state = setup();
    let task = task("memory-capability-lifecycle", "conversation-a");
    for (index, content) in ["First stable fact", "Second stable fact"]
        .into_iter()
        .enumerate()
    {
        let save = args(json!({
            "action": "save",
            "scope": "current_principal",
            "kind": "fact",
            "content": content,
            "idempotency_key": format!("fixture-lifecycle-{index}"),
        }));
        let outcome: Value =
            serde_json::from_str(&super::execute(&state, Some(&task), &save).await.unwrap())
                .unwrap();
        assert_eq!(outcome["status"], "ok", "save outcome: {outcome}");
    }

    let first_page: Value = serde_json::from_str(
        &super::execute(
            &state,
            Some(&task),
            &args(json!({
                "action": "list_recent",
                "scope": "current_principal",
                "limit": 1,
            })),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(first_page["data"]["count"], 1);
    assert_eq!(first_page["data"]["data_only"], true);
    assert_eq!(first_page["data"]["instruction_authority"], "none");
    let cursor = first_page["data"]["continuation_token"]
        .as_str()
        .expect("continuation token");
    let first_memory_id = first_page["data"]["items"][0]["memory_id"]
        .as_str()
        .unwrap();
    let revision = first_page["data"]["items"][0]["revision"].as_i64().unwrap();
    let second_page: Value = serde_json::from_str(
        &super::execute(
            &state,
            Some(&task),
            &args(json!({
                "action": "list_recent",
                "scope": "current_principal",
                "limit": 1,
                "cursor": cursor,
            })),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(second_page["data"]["count"], 1);
    assert_ne!(
        second_page["data"]["items"][0]["memory_id"],
        first_memory_id
    );

    let wrong_scope: Value = serde_json::from_str(
        &super::execute(
            &state,
            Some(&task),
            &args(json!({
                "action": "forget",
                "scope": "current_conversation",
                "memory_id": first_memory_id,
                "expected_revision": revision,
            })),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(wrong_scope["error_code"], "memory_item_not_found_in_scope");

    let stale: Value = serde_json::from_str(
        &super::execute(
            &state,
            Some(&task),
            &args(json!({
                "action": "correct",
                "scope": "current_principal",
                "memory_id": first_memory_id,
                "expected_revision": revision + 1,
                "content": "Corrected stable fact",
            })),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(stale["status"], "error");
    assert!(stale["error_code"].as_str().unwrap().contains("revision"));

    let forgotten: Value = serde_json::from_str(
        &super::execute(
            &state,
            Some(&task),
            &args(json!({
                "action": "forget",
                "scope": "current_principal",
                "memory_id": first_memory_id,
                "expected_revision": revision,
            })),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(forgotten["status"], "ok", "forget outcome: {forgotten}");

    let db = state.core.db.get().unwrap();
    let principal_id = crate::repo::auth::principal_id_for_user_key(&db, "memory-capability-key")
        .unwrap()
        .unwrap();
    let identity = claw_core::types::AuthIdentity {
        user_key: "memory-capability-key".to_string(),
        principal_id,
        role: "user".to_string(),
        user_id: 41,
        chat_id: 51,
    };
    crate::memory::settings::update_memory_settings(
        &db,
        &identity,
        &crate::memory::settings::MemorySettingsUpdateRequest {
            scope: crate::memory::settings::MemorySettingScope::Principal,
            target_principal_id: None,
            conversation_id: None,
            use_mode: None,
            generate_mode: Some(crate::memory::settings::MemorySettingMode::Disabled),
            external_context_policy: None,
            expected_revision: Some(1),
            long_term_enabled: None,
        },
        state.policy.memory.long_term_enabled,
    )
    .unwrap();
    drop(db);
    let disabled: Value = serde_json::from_str(
        &super::execute(
            &state,
            Some(&task),
            &args(json!({
                "action": "save",
                "scope": "current_principal",
                "kind": "fact",
                "content": "Must not be written",
                "idempotency_key": "fixture-disabled-generation",
            })),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(disabled["error_code"], "memory_generate_disabled");
}
