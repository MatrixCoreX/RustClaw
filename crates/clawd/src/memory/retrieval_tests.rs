use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use claw_core::config::{AgentConfig, ToolsConfig};

use super::{
    build_structured_memory_context_block, retrieve_indexed_memories, source_label_for_row,
    MemoryContextMode, RetrievalRow, RetrievedMemoryItem, StructuredMemoryContext,
};
use crate::db_init::ensure_memory_schema;
use crate::memory::facts::{upsert_memory_fact_card, MemoryFactUpsert};
use crate::memory::indexing::{ensure_retrieval_schema, upsert_knowledge_fact};
use crate::runtime::{AgentRuntimeConfig, AppState, SkillViewsSnapshot, ToolsPolicy};

fn item(text: &str) -> RetrievedMemoryItem {
    RetrievedMemoryItem {
        role: Some("assistant".to_string()),
        text: text.to_string(),
        score: 0.91,
        source_label: None,
    }
}

fn test_state() -> AppState {
    let agents_by_id = HashMap::from([(
        crate::DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(HashSet::new()),
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            locator_scan_max_depth: 3,
            locator_scan_max_files: 200,
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            ..crate::SkillRuntime::test_default()
        },
        policy: crate::PolicyConfig::test_default(),
        worker: crate::WorkerConfig::test_default(),
        metrics: crate::TaskMetricsRegistry::default(),
        channels: crate::ChannelConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
        ask_states: crate::AskStateRegistry::default(),
    }
}

#[test]
fn planner_memory_context_is_strictly_scoped() {
    let ctx = StructuredMemoryContext {
        long_term_summary: Some("legacy long term summary".to_string()),
        preferences: vec![("response_language".to_string(), "zh-CN".to_string())],
        similar_triggers: vec![item("similar trigger")],
        relevant_facts: vec![item("stable fact")],
        knowledge_docs: vec![item("kb fact")],
        recent_related_events: vec![item("recent event")],
        assistant_results: vec![item("assistant result")],
        unfinished_goals: vec![item("unfinished goal")],
        recalled_recent: vec![("assistant".to_string(), "recent snippet".to_string())],
    };

    let block = build_structured_memory_context_block(&ctx, MemoryContextMode::Planner, 2000);

    assert!(block.contains("PLANNER_MEMORY_CONTEXT"));
    assert!(block.contains("RECENT_UNFINISHED_GOALS"));
    assert!(block.contains("ACTIVE_PREFERENCES"));
    assert!(block.contains("STABLE_FACTS"));
    assert!(!block.contains("RECENT_ASSISTANT_RESULTS"));
    assert!(!block.contains("RECENT_RELATED_EVENTS"));
    assert!(!block.contains("FALLBACK_LONG_TERM_SUMMARY"));
}

#[test]
fn chat_memory_context_keeps_assistant_results_and_unfinished_goals() {
    let ctx = StructuredMemoryContext {
        assistant_results: vec![item("assistant result")],
        unfinished_goals: vec![item("unfinished goal")],
        ..Default::default()
    };

    let block = build_structured_memory_context_block(&ctx, MemoryContextMode::Chat, 2000);

    assert!(block.contains("RECENT_ASSISTANT_RESULTS"));
    assert!(block.contains("RECENT_UNFINISHED_GOALS"));
}

#[test]
fn chat_memory_context_includes_knowledge_base_section() {
    let ctx = StructuredMemoryContext {
        knowledge_docs: vec![RetrievedMemoryItem {
            role: None,
            text: "deployment steps live here".to_string(),
            score: 0.88,
            source_label: Some("docs:README.md".to_string()),
        }],
        ..Default::default()
    };

    let block = build_structured_memory_context_block(&ctx, MemoryContextMode::Chat, 2000);

    assert!(block.contains("KNOWLEDGE_BASE_CONTEXT"));
    assert!(block.contains("[docs:README.md]"));
    assert!(block.contains("deployment steps live here"));
}

#[test]
fn stable_fact_rendering_skips_cross_turn_deictic_locator_mapping() {
    let ctx = StructuredMemoryContext {
        relevant_facts: vec![
            item(
                r#"{"deictic_reference":{"target":"unresolved_prior_object"},"locator":"/tmp/device/app.log","reason":"stale cross-turn alias"}"#,
            ),
            item("项目别名：'那个服务' 代指 'clawd'"),
            item("那个配置文件 maps to /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/configs/app_config.toml"),
            item("那个配置文件 refers to /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/configs/app_config.toml"),
            item("默认用中文回复\nReason: durable preference"),
        ],
        ..Default::default()
    };

    let block = build_structured_memory_context_block(&ctx, MemoryContextMode::Planner, 2000);

    assert!(block.contains("STABLE_FACTS"));
    assert!(block.contains("默认用中文回复"));
    assert!(!block.contains("/tmp/device/app.log"));
    assert!(!block.contains("unresolved_prior_object"));
    assert!(!block.contains("app_config.toml"));
    assert!(!block.contains("clawd"));
}

#[test]
fn stable_fact_rendering_skips_internal_client_like_run_ids_from_project_facts() {
    let ctx = StructuredMemoryContext {
        relevant_facts: vec![
            RetrievedMemoryItem {
                role: None,
                text: "测试编号 client-like-continuous-20260520_031810 用于 RustClaw 客户端连续会话测试场".to_string(),
                score: 0.91,
                source_label: Some("project_facts".to_string()),
            },
            RetrievedMemoryItem {
                role: None,
                text: "当前连续测试标记为 RC-CONT-CN-0428-A".to_string(),
                score: 0.905,
                source_label: Some("project_facts".to_string()),
            },
            RetrievedMemoryItem {
                role: None,
                text: "用户保存的测试编号是 client-like-continuous-20260520_031810".to_string(),
                score: 0.9,
                source_label: Some("user_profile".to_string()),
            },
            RetrievedMemoryItem {
                role: None,
                text: "RustClaw workspace package version is 0.1.7".to_string(),
                score: 0.89,
                source_label: Some("project_facts".to_string()),
            },
        ],
        ..Default::default()
    };

    let block = build_structured_memory_context_block(&ctx, MemoryContextMode::Planner, 2000);

    assert!(!block.contains("[project_facts] 测试编号 client-like-continuous-20260520_031810"));
    assert!(!block.contains("[project_facts] 当前连续测试标记为 RC-CONT-CN-0428-A"));
    assert!(block
        .contains("[user_profile] 用户保存的测试编号是 client-like-continuous-20260520_031810"));
    assert!(block.contains("[project_facts] RustClaw workspace package version is 0.1.7"));
}

#[test]
fn knowledge_fact_source_label_uses_namespace_only() {
    let embedding_spec = crate::memory::embedding::local_hash_embedding_spec();
    let row = RetrievalRow {
        id: 1,
        source_kind: crate::memory::RETRIEVAL_SOURCE_KNOWLEDGE_FACT.to_string(),
        memory_kind: crate::memory::RETRIEVAL_KIND_SEMANTIC_FACT.to_string(),
        role: Some(crate::memory::MEMORY_ROLE_SYSTEM.to_string()),
        search_text: "用户长期偏好中文回复".to_string(),
        vector_json: "[]".to_string(),
        embedding_model: embedding_spec.model_id,
        embedding_dims: embedding_spec.dims,
        embedding_version: embedding_spec.version,
        metadata_json: r#"{"namespace":"user_profile","path":"conversation"}"#.to_string(),
        salience: 0.9,
        success_state: crate::memory::RETRIEVAL_SUCCESS_STATE_SUCCEEDED.to_string(),
        updated_at_ts: 1,
    };

    assert_eq!(source_label_for_row(&row).as_deref(), Some("user_profile"));
}

#[test]
fn knowledge_fact_rows_recall_into_relevant_facts() {
    let state = test_state();
    let user_id = 1001;
    let chat_id = 2002;
    let user_key = "user:test";
    {
        let db = state.core.db.get().expect("db lock");
        db.execute_batch(crate::INIT_SQL).expect("init base schema");
        ensure_memory_schema(&db).expect("ensure memory schema");
        crate::repo::auth::ensure_key_auth_schema(&db).expect("ensure key auth schema");
        ensure_retrieval_schema(&db).expect("ensure retrieval schema");
        upsert_knowledge_fact(
            &db,
            user_id,
            user_key,
            "user_profile",
            crate::memory::RETRIEVAL_KIND_SEMANTIC_FACT,
            "knowledge:user:test:demo",
            "以后默认用中文回复\nReason: explicit durable preference",
            1_775_301_800,
        )
        .expect("insert knowledge fact");

        let mut stmt = db
            .prepare(
                "SELECT source_kind, source_ref, memory_kind, tool_or_skill_name, metadata_json, search_text
                 FROM memory_retrieval_index
                 WHERE source_kind = 'knowledge_fact'",
            )
            .expect("prepare query");
        let row = stmt
            .query_row([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .expect("fetch inserted row");
        println!("inserted knowledge_fact row: {row:?}");
    }

    let recall =
        retrieve_indexed_memories(&state, Some(user_key), user_id, chat_id, "以后回复都用中文")
            .expect("retrieve indexed memories");
    println!("recalled relevant_facts: {:?}", recall.relevant_facts);
    let ctx = StructuredMemoryContext {
        relevant_facts: recall.relevant_facts.clone(),
        ..Default::default()
    };
    let block = build_structured_memory_context_block(&ctx, MemoryContextMode::Chat, 2000);
    println!("chat memory block:\n{block}");

    assert_eq!(recall.relevant_facts.len(), 1);
    assert!(recall.relevant_facts[0].text.contains("默认用中文回复"));
    assert_eq!(
        recall.relevant_facts[0].source_label.as_deref(),
        Some("user_profile")
    );
    assert!(block.contains("RELEVANT_FACTS"));
    assert!(block.contains("[user_profile]"));
}

#[test]
fn memory_fact_cards_recall_into_relevant_facts_without_reason_text() {
    let state = test_state();
    let user_id = 1001;
    let chat_id = 2002;
    let user_key = "user:test";
    {
        let db = state.core.db.get().expect("db lock");
        db.execute_batch(crate::INIT_SQL).expect("init base schema");
        ensure_memory_schema(&db).expect("ensure memory schema");
        ensure_retrieval_schema(&db).expect("ensure retrieval schema");
        let source_ids = [42_i64];
        let fact = MemoryFactUpsert::from_long_term_summary(
            "user_profile",
            "response_language",
            "zh-CN",
            "以后默认用中文回复",
            0.96,
            "long_term_summary:42",
            &source_ids,
            "explicit durable preference",
            Some("user_profile:response_language"),
        );
        upsert_memory_fact_card(&db, user_id, chat_id, user_key, &fact, 1_775_301_800)
            .expect("upsert fact card");
    }

    let recall =
        retrieve_indexed_memories(&state, Some(user_key), user_id, chat_id, "以后回复都用中文")
            .expect("retrieve indexed memories");

    assert_eq!(recall.relevant_facts.len(), 1);
    assert_eq!(recall.relevant_facts[0].text, "以后默认用中文回复");
    assert_eq!(
        recall.relevant_facts[0].source_label.as_deref(),
        Some("user_profile")
    );
    assert!(!recall.relevant_facts[0].text.contains("Reason:"));
}

#[test]
fn retrieval_multilingual_queries_keep_structured_preferences_and_facts() {
    let state = test_state();
    let user_id = 1001;
    let chat_id = 2002;
    let user_key = "user:test";
    {
        let db = state.core.db.get().expect("db lock");
        db.execute_batch(crate::INIT_SQL).expect("init base schema");
        ensure_memory_schema(&db).expect("ensure memory schema");
        crate::repo::auth::ensure_key_auth_schema(&db).expect("ensure key auth schema");
        ensure_retrieval_schema(&db).expect("ensure retrieval schema");
        db.execute(
            "INSERT INTO user_preferences
             (user_id, chat_id, user_key, pref_key, pref_value, confidence, source, updated_at, updated_at_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                user_id,
                chat_id,
                user_key,
                "response_language",
                "zh-CN",
                0.96_f32,
                "memory_extract",
                "1000",
                1000_i64,
            ],
        )
        .expect("insert preference");
        crate::memory::indexing::index_preference_entries(
            &db,
            user_id,
            chat_id,
            user_key,
            &[(
                "response_language".to_string(),
                "zh-CN".to_string(),
                0.96,
                "memory_extract".to_string(),
            )],
            1000,
        )
        .expect("index preference");
        let source_ids = [42_i64];
        let fact = MemoryFactUpsert::from_long_term_summary(
            "user_profile",
            "reply_language",
            "zh-CN",
            "用户希望默认使用中文回复",
            0.96,
            "long_term_summary:42",
            &source_ids,
            "stable multilingual preference",
            Some("user_profile:reply_language"),
        );
        upsert_memory_fact_card(&db, user_id, chat_id, user_key, &fact, 1_775_301_800)
            .expect("upsert fact card");
    }

    for prompt in [
        "以后回复都用中文",
        "Please answer in my saved language",
        "保存済みの返信言語を使って",
        "저장된 답변 언어를 사용해줘",
        "Utilise ma langue de réponse enregistrée",
    ] {
        let recall = retrieve_indexed_memories(&state, Some(user_key), user_id, chat_id, prompt)
            .expect("retrieve indexed memories");
        let joined = recall
            .relevant_facts
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("response_language"),
            "preference should remain recallable for prompt {prompt:?}; got {joined:?}"
        );
        assert!(
            joined.contains("默认使用中文回复"),
            "fact should remain recallable for prompt {prompt:?}; got {joined:?}"
        );
    }
}

#[test]
fn retrieval_index_rows_record_embedding_version() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db lock");
        db.execute_batch(crate::INIT_SQL).expect("init base schema");
        ensure_memory_schema(&db).expect("ensure memory schema");
        ensure_retrieval_schema(&db).expect("ensure retrieval schema");
        upsert_knowledge_fact(
            &db,
            1001,
            "user:test",
            "user_profile",
            crate::memory::RETRIEVAL_KIND_SEMANTIC_FACT,
            "knowledge:user:test:embedding-version",
            "User prefers concise Chinese replies.",
            1_775_301_800,
        )
        .expect("insert knowledge fact");

        let row = db
            .query_row(
                "SELECT embedding_model, embedding_dims, embedding_version
                 FROM memory_retrieval_index
                 WHERE source_ref = ?1",
                ["knowledge:user:test:embedding-version"],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .expect("embedding version row");
        let spec = crate::memory::embedding::local_hash_embedding_spec();
        assert_eq!(row.0, spec.model_id);
        assert_eq!(row.1, spec.dims as i64);
        assert_eq!(row.2, spec.version);
    }
}

#[test]
fn retrieval_excludes_legacy_safety_signal_index_rows() {
    let state = test_state();
    let user_id = 1001;
    let chat_id = 2002;
    let user_key = "user:test";
    let safety_memory_id = {
        let db = state.core.db.get().expect("db lock");
        db.execute_batch(crate::INIT_SQL).expect("init base schema");
        ensure_memory_schema(&db).expect("ensure memory schema");
        crate::repo::auth::ensure_key_auth_schema(&db).expect("ensure key auth schema");
        ensure_retrieval_schema(&db).expect("ensure retrieval schema");
        db.execute(
            "INSERT INTO memories
             (user_id, chat_id, user_key, channel, role, content, created_at, created_at_ts, memory_type, salience, is_instructional, safety_flag)
             VALUES (?1, ?2, ?3, 'ui', 'user', 'policy-sensitive legacy memory', '1000', ?4, ?5, 0.2, 0, ?6)",
            rusqlite::params![
                user_id,
                chat_id,
                user_key,
                1_775_301_800_i64,
                crate::memory::MEMORY_TYPE_SAFETY_SIGNAL,
                crate::memory::MEMORY_SAFETY_FLAG_INJECTION_LIKE
            ],
        )
        .expect("insert safety memory");
        let memory_id = db.last_insert_rowid();
        db.execute(
            "INSERT INTO memory_retrieval_index (
                source_kind, source_memory_id, source_pref_key, source_ref, user_id, chat_id, user_key,
                memory_kind, role, search_text, trigger_text, topic_tags, vector_json, metadata_json,
                salience, success_state, tool_or_skill_name, created_at_ts, updated_at_ts
             )
             VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, 'user', ?8, NULL, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
            rusqlite::params![
                crate::memory::RETRIEVAL_SOURCE_MEMORY,
                memory_id,
                crate::memory::retrieval_source_ref_for_memory(memory_id),
                user_id,
                chat_id,
                user_key,
                crate::memory::RETRIEVAL_KIND_EPISODIC_EVENT,
                "policy-sensitive legacy memory",
                crate::memory::retrieval::build_topic_tags("policy-sensitive legacy memory"),
                crate::memory::retrieval::vector_to_json(
                    &crate::memory::embedding::embed_text_locally(
                        "policy-sensitive legacy memory",
                    ),
                ),
                r#"{"scope_kind":"chat"}"#,
                0.9_f32,
                crate::memory::RETRIEVAL_SUCCESS_STATE_NEUTRAL,
                crate::memory::RETRIEVAL_PRODUCER_MEMORY_PIPELINE,
                1_775_301_800_i64,
            ],
        )
        .expect("insert legacy index row");
        memory_id
    };

    let recall = retrieve_indexed_memories(
        &state,
        Some(user_key),
        user_id,
        chat_id,
        "policy-sensitive legacy memory",
    )
    .expect("retrieve indexed memories");
    let joined = format!("{recall:?}");
    assert!(
        !joined.contains("policy-sensitive legacy memory"),
        "safety memory {safety_memory_id} must not be recalled: {joined}"
    );
}

#[test]
fn kb_docs_are_scoped_by_user_key() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db lock");
        db.execute_batch(crate::INIT_SQL).expect("init base schema");
        ensure_memory_schema(&db).expect("ensure memory schema");
        ensure_retrieval_schema(&db).expect("ensure retrieval schema");
        db.execute(
            "INSERT INTO memory_retrieval_index (
                source_kind, source_memory_id, source_pref_key, source_ref, user_id, chat_id, user_key,
                memory_kind, role, search_text, trigger_text, topic_tags, vector_json, metadata_json,
                salience, success_state, tool_or_skill_name, created_at_ts, updated_at_ts
             )
             VALUES (?1, NULL, NULL, ?2, 0, 0, ?3, ?4, NULL, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
            rusqlite::params![
                crate::memory::RETRIEVAL_SOURCE_KB_DOC,
                "kb:user-a:docs:chunk-1",
                "user-a",
                crate::memory::RETRIEVAL_KIND_KNOWLEDGE_DOC,
                "deployment runbook for user a",
                crate::memory::retrieval::build_topic_tags("deployment runbook for user a"),
                crate::memory::retrieval::vector_to_json(
                    &crate::memory::embedding::embed_text_locally(
                        "deployment runbook for user a",
                    ),
                ),
                r#"{"scope_kind":"user","namespace":"docs","path":"README.md"}"#,
                0.78_f32,
                crate::memory::RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
                crate::memory::RETRIEVAL_PRODUCER_KB,
                1_775_301_800_i64,
            ],
        )
        .expect("insert kb row");
    }

    let recall_for_owner =
        retrieve_indexed_memories(&state, Some("user-a"), 1, 2, "deployment runbook")
            .expect("owner recall");
    assert_eq!(recall_for_owner.knowledge_docs.len(), 1);

    let recall_for_other =
        retrieve_indexed_memories(&state, Some("user-b"), 1, 2, "deployment runbook")
            .expect("other recall");
    assert!(recall_for_other.knowledge_docs.is_empty());
}
