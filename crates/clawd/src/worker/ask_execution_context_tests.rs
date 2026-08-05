use rusqlite::params;
use serde_json::json;

#[tokio::test]
async fn fifty_two_turn_context_compacts_at_real_pre_prompt_owner() {
    let mut state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let provider = std::sync::Arc::make_mut(
        state
            .core
            .llm_providers
            .first_mut()
            .expect("fixture provider"),
    );
    provider.config.context_window_tokens = Some(8_000);
    state.skill_rt.workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    {
        let db = state.core.db.get().expect("database");
        db.execute(
            "INSERT OR IGNORE INTO auth_keys(user_key, role, enabled, created_at)
             VALUES ('context-user', 'user', 1, '1')",
            [],
        )
        .expect("auth key");
        crate::repo::auth::ensure_principal_identity_schema(&db).expect("principal schema");
        crate::repo::ensure_principal_ownership_schema(&db).expect("ownership schema");
        let principal_id = crate::repo::auth::principal_id_for_user_key(&db, "context-user")
            .expect("principal query")
            .expect("principal id");
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
                    task_id, user_id, chat_id, user_key, principal_id, channel, kind, payload_json,
                    status, result_json, created_at, updated_at
                 ) VALUES (?1, 7, 9, 'context-user', ?2, 'ui', 'ask', ?3, 'succeeded', ?4, ?5, ?5)",
                params![
                    format!("context-history-{index}"),
                    principal_id,
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
                task_id, user_id, chat_id, user_key, principal_id, channel, kind, payload_json,
                status, result_json, created_at, updated_at
             ) VALUES (
                'task-live-context-compaction', 7, 9, 'context-user', ?1, 'ui', 'ask',
                '{}', 'running', ?2, 53, 53
             )",
            params![
                principal_id,
                json!({
                    "task_journal": {
                        "summary": {
                            "transcript_compaction_records": [{"generation": 7}]
                        }
                    }
                })
                .to_string()
            ],
        )
        .expect("insert resumable current task record");
    }
    let task = crate::ClaimedTask {
        claim_attempt: 0,
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
    assert_eq!(record["generation"], 1);
    assert_eq!(record["lifecycle"]["base_generation"], 0);
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

#[tokio::test]
async fn coding_execution_context_injects_workspace_instructions_and_attribution() {
    let mut state = crate::AppState::test_default_with_fixture_provider()
        .with_seeded_db_schema()
        .with_prompt_layers_installed();
    state.reload_ctx.workspace_instructions = claw_core::config::WorkspaceInstructionsConfig {
        enabled_for_coding: true,
        enabled_for_non_coding: false,
        filenames: vec!["AGENTS.md".to_string()],
        user_instruction_paths: Vec::new(),
        max_total_bytes: 4_096,
        max_file_bytes: 8_192,
        max_files: 8,
    };
    let payload = json!({
        "text": "inspect the workspace",
        "entrypoint": "exec",
        "source": "clawcli_machine",
        "execution_profile": "coding",
        "workspace_context": {
            "schema_version": 1,
            "current_working_directory": state.skill_rt.workspace_root,
        },
    });
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-workspace-instruction-context".to_string(),
        user_id: 7,
        chat_id: 9,
        user_key: Some("workspace-instruction-user".to_string()),
        channel: "cli".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: payload.to_string(),
    };

    let prepared =
        super::prepare_ask_execution_context(&state, &task, &payload, "inspect the workspace")
            .await
            .expect("prepare coding execution context");

    assert!(prepared
        .resolved_prompt_for_execution
        .contains("### WORKSPACE_INSTRUCTION_CONTEXT"));
    assert!(prepared
        .prompt_with_memory_for_execution
        .contains("### WORKSPACE_INSTRUCTION_CONTEXT"));
    let attribution = prepared
        .initial_task_observations
        .iter()
        .find(|item| {
            item.get("source_kind").and_then(serde_json::Value::as_str)
                == Some("workspace_instructions")
        })
        .expect("workspace instruction attribution");
    assert_eq!(attribution["working_directory_status"], "resolved");
    assert_eq!(attribution["instruction_authority"], "model_context_only");
    assert_eq!(attribution["permission_authority"], false);
    assert_eq!(attribution["sources"][0]["logical_path"], "AGENTS.md");
    assert!(attribution["injected_bytes_total"].as_u64().unwrap() <= 4_096);
    assert_eq!(attribution["sources"][0]["digest_scope"], "loaded_prefix");
}

#[test]
fn rewind_boundary_uses_authoritative_history_and_marks_side_effects_non_replayable() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let result = json!({
        "task_journal": {
            "summary": {"coding_workflow": {
                "completed_side_effect_refs": ["mutation:1"]
            }},
            "trace": {"event_stream": [
                {"seq":1,"event_type":"task_started","payload":{}},
                {"seq":2,"event_type":"tool_finished","payload":{"mutation_id":"mutation:1"}},
                {"seq":3,"event_type":"task_final","payload":{}}
            ]}
        }
    });
    {
        let db = state.core.db.get().unwrap();
        db.execute(
            "INSERT INTO tasks (task_id,user_id,chat_id,channel,kind,payload_json,status,result_json,created_at,updated_at) VALUES ('rewind-source',1,1,'ui','ask','{}','succeeded',?1,1,1)",
            params![result.to_string()],
        )
        .unwrap();
    }
    let payload = json!({"session_rewind":{
        "schema_version":1,
        "anchor":{
            "schema_version":1,
            "source_session_id":"session-source",
            "source_task_id":"rewind-source",
            "event_seq":2,
            "checkpoint_id":null
        },
        "completed_side_effect_refs":["client-value-is-not-authoritative"]
    }});

    let task = crate::ClaimedTask {
        claim_attempt: 1,
        task_id: "rewind-current".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let observation = super::session_rewind_observation(&state, &task, &payload)
        .unwrap()
        .expect("rewind observation");
    assert_eq!(observation["event_count"], 2);
    assert_eq!(observation["completed_side_effect_refs"][0], "mutation:1");
    assert_eq!(
        observation["side_effect_replay_policy"],
        "already_occurred_do_not_replay"
    );
    assert_eq!(observation["instruction_authority"], "none");
}
