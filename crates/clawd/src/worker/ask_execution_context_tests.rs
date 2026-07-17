use rusqlite::params;
use serde_json::json;

#[tokio::test]
async fn fifty_two_turn_context_compacts_at_real_pre_prompt_owner() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    {
        let db = state.core.db.get().expect("database");
        for index in 0..52_i64 {
            db.execute(
                "INSERT INTO tasks (
                    task_id, user_id, chat_id, user_key, channel, kind, payload_json,
                    status, result_json, created_at, updated_at
                 ) VALUES (?1, 7, 9, 'context-user', 'ui', 'ask', ?2, 'succeeded', ?3, ?4, ?4)",
                params![
                    format!("context-history-{index}"),
                    json!({"text": format!("request-{index}-{}", "u".repeat(300))}).to_string(),
                    json!({"text": format!("result-{index}-{}", "a".repeat(300))}).to_string(),
                    index + 1,
                ],
            )
            .expect("insert historical turn");
        }
    }
    let task = crate::ClaimedTask {
        task_id: "task-live-context-compaction".to_string(),
        user_id: 7,
        chat_id: 9,
        user_key: Some("context-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({
            "text": "continue",
            "goal": {"objective_ref": "goal:context-compaction", "constraint_refs": ["constraint:no_duplicate_write"]}
        })
        .to_string(),
    };

    let prepared = super::prepare_ask_execution_context(
        &state,
        &task,
        &serde_json::from_str(&task.payload_json).unwrap(),
        "continue",
    )
    .await
    .expect("prepare compacted execution context");
    let view = prepared.context_bundle.execution_view.as_ref().unwrap();
    let record = prepared
        .context_bundle
        .compaction_records
        .first()
        .expect("compaction record");

    assert_eq!(view.budget_tier.as_str(), "light");
    assert_eq!(view.recent_turns_full, "<none>");
    assert_eq!(view.recent_execution_context, "<none>");
    assert!(view.goal_context.contains("goal:context-compaction"));
    assert!(
        record["after_char_count"].as_u64().unwrap() * 2
            < record["before_char_count"].as_u64().unwrap()
    );
    assert!(prepared.initial_task_observations.iter().any(|item| {
        item.get("stage").and_then(serde_json::Value::as_str) == Some("pre_compact")
    }));
    assert!(prepared.initial_task_observations.iter().any(|item| {
        item.get("stage").and_then(serde_json::Value::as_str) == Some("post_compact")
    }));
    assert!(prepared
        .context_bundle
        .summary()
        .contains("deterministic_context_budget"));
}
