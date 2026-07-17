use rusqlite::params;
use serde_json::json;

#[tokio::test]
async fn fifty_two_turn_context_compacts_at_real_pre_prompt_owner() {
    let mut state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.skill_rt.workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    {
        let db = state.core.db.get().expect("database");
        for index in 0..52_i64 {
            let result_json = if index == 24 {
                json!({
                    "text": format!("result-{index}-{}", "a".repeat(300)),
                    "task_journal": {
                        "summary": {
                            "transcript_compaction_records": [{"generation": 3}]
                        }
                    }
                })
            } else {
                json!({"text": format!("result-{index}-{}", "a".repeat(300))})
            };
            db.execute(
                "INSERT INTO tasks (
                    task_id, user_id, chat_id, user_key, channel, kind, payload_json,
                    status, result_json, created_at, updated_at
                 ) VALUES (?1, 7, 9, 'context-user', 'ui', 'ask', ?2, 'succeeded', ?3, ?4, ?4)",
                params![
                    format!("context-history-{index}"),
                    json!({"text": format!("request-{index}-{}", "u".repeat(300))}).to_string(),
                    result_json.to_string(),
                    index + 1,
                ],
            )
            .expect("insert historical turn");
        }
        for (task_id, seq) in [("context-history-0", 2_i64), ("context-history-51", 9_i64)] {
            db.execute(
                "INSERT INTO task_event_stream (
                    task_id, seq, event_hash, event_json, created_at_ms
                 ) VALUES (?1, ?2, ?3, '{}', ?2)",
                params![task_id, seq, format!("fixture-{task_id}-{seq}")],
            )
            .expect("insert source event range");
        }
        db.execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, user_key, channel, kind, payload_json,
                status, result_json, created_at, updated_at
             ) VALUES (
                'task-live-context-compaction', 7, 9, 'context-user', 'ui', 'ask',
                '{}', 'running', ?1, 53, 53
             )",
            params![json!({
                "task_journal": {
                    "summary": {
                        "transcript_compaction_records": [{"generation": 7}]
                    }
                }
            })
            .to_string()],
        )
        .expect("insert resumable current task record");
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
    assert!(prepared.initial_task_observations.iter().any(|item| {
        item.get("observation_kind")
            .and_then(serde_json::Value::as_str)
            == Some("context_compaction_record")
            && item
                .get("record")
                .and_then(|record| record.get("compaction_id"))
                .is_some()
    }));
    let prompt_attribution = prepared
        .initial_task_observations
        .iter()
        .find(|item| {
            item.get("observation_kind")
                .and_then(serde_json::Value::as_str)
                == Some("context_prompt_attribution")
        })
        .expect("context prompt attribution");
    assert!(prompt_attribution["prompt_count"].as_u64().unwrap() >= 1);
    assert!(prompt_attribution["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item.get("logical_path").is_some()
            && item["template_char_count"].as_u64().unwrap() <= 2_000
            && item["overhead_char_count"].as_u64().unwrap() <= 1_800));
    assert!(!prepared
        .context_bundle
        .summary()
        .contains("transcript_compaction_records="));
    assert_eq!(record["generation"], 8);
    assert_eq!(record["source_task_ids"].as_array().unwrap().len(), 52);
    assert_eq!(record["source_task_ids"][0], "context-history-0");
    assert_eq!(record["source_task_ids"][51], "context-history-51");
    assert_eq!(
        record["source_event_range"]["start"]["task_id"],
        "context-history-0"
    );
    assert_eq!(record["source_event_range"]["start"]["event_seq"], 2);
    assert_eq!(
        record["source_event_range"]["end"]["task_id"],
        "context-history-51"
    );
    assert_eq!(record["source_event_range"]["end"]["event_seq"], 9);
    assert_eq!(record["source_event_ranges"].as_array().unwrap().len(), 52);
    assert_ne!(
        record["model_status_code"],
        "context_compaction_model_completed"
    );
    assert!(record["model_status_code"]
        .as_str()
        .unwrap()
        .starts_with("context_compaction_"));
    assert_eq!(
        record["compaction_source"],
        "deterministic_machine_reference_fallback"
    );
    assert_eq!(record["model_summary_attached"], false);
    assert_eq!(record["continuity_summary_attached"], true);
}
