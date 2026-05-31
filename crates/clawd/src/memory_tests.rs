use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use claw_core::config::{AgentConfig, ToolsConfig};

use rusqlite::{params, Connection};

use crate::runtime::policy::ToolsPolicy;
use crate::runtime::state::SkillViewsSnapshot;

use crate::{AgentRuntimeConfig, AppState};

use super::{
    build_last_turn_full_context, build_memory_intent_llm_prompt,
    build_recent_assistant_replies_context, clarify_assistant_placeholder,
    classify_assistant_context_reply_kind, extract_result_text_for_recent_turns, insert_memory,
    legacy_principal_chat_id, ordered_entries_from_assistant_reply,
    provider_unavailable_assistant_placeholder, recall_memories_since_id, recall_user_preferences,
    retrieval_source_ref_for_kb_chunk, retrieval_source_ref_for_memory,
    retrieval_source_ref_for_preference, upsert_user_preferences_from_route_hint,
    AssistantContextReplyKind, MemoryWriteKind, MEMORY_ROLE_ASSISTANT, MEMORY_ROLE_SYSTEM,
    MEMORY_ROLE_USER, MEMORY_TYPE_GENERIC, RETRIEVAL_PRODUCER_KB,
    RETRIEVAL_PRODUCER_MEMORY_PIPELINE, RETRIEVAL_SOURCE_MEMORY,
};
use serde_json::json;

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

fn create_tasks_table(db: &Connection) {
    db.execute_batch(
        "CREATE TABLE tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT,
            result_json TEXT,
            error_text TEXT,
            status TEXT NOT NULL,
            created_at TEXT,
            updated_at TEXT
        );",
    )
    .expect("create tasks table");
}

fn create_memories_table(db: &Connection) {
    db.execute_batch(
        "CREATE TABLE memories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT,
            channel TEXT NOT NULL DEFAULT 'telegram',
            external_chat_id TEXT,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL,
            created_at_ts INTEGER NOT NULL DEFAULT 0,
            memory_type TEXT NOT NULL DEFAULT 'generic',
            salience REAL NOT NULL DEFAULT 0.5,
            is_instructional INTEGER NOT NULL DEFAULT 0,
            safety_flag TEXT NOT NULL DEFAULT 'normal'
        );",
    )
    .expect("create memories table");
}

fn create_user_preferences_table(db: &Connection) {
    db.execute_batch(
        "CREATE TABLE user_preferences (
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT NOT NULL,
            pref_key TEXT NOT NULL,
            pref_value TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0,
            source TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL DEFAULT '',
            updated_at_ts INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (user_id, chat_id, user_key, pref_key)
        );",
    )
    .expect("create user_preferences table");
}

#[test]
fn clarify_task_reply_is_replaced_with_placeholder_for_recent_turn_context() {
    let parsed = json!({
        "text": "LOCATOR_CLARIFY_PROMPT",
        "task_journal": {
            "summary": {
                "final_status": "clarify",
                "route_result": {
                    "route_gate_kind": "clarify",
                    "needs_clarify": true
                }
            }
        }
    });
    // Keep phrase-based fallback matching out of this path; structured
    // routing metadata should be enough to identify a clarify placeholder.
    let never_fallback = |_: &str| false;
    assert_eq!(
        classify_assistant_context_reply_kind(
            Some(&parsed),
            "LOCATOR_CLARIFY_PROMPT",
            never_fallback,
        ),
        AssistantContextReplyKind::ClarifyPlaceholder
    );
    assert_eq!(clarify_assistant_placeholder(), "[clarification_requested]");
}

#[test]
fn provider_unavailable_reply_is_replaced_with_placeholder_for_recent_turn_context() {
    let parsed = json!({
        "text": "当前大模型服务暂时不可用（未加载成功或鉴权失败），我先无法进行语义理解与规划。请稍后重试，或让我改用可用模型继续。若你愿意，也可以先补充目标或上下文，模型恢复后我会优先处理。"
    });
    // §7.2: 模拟"is_fallback 集合命中"—— 文本本身被识别为 fallback 占位符。
    let target_text = "当前大模型服务暂时不可用（未加载成功或鉴权失败），我先无法进行语义理解与规划。请稍后重试，或让我改用可用模型继续。若你愿意，也可以先补充目标或上下文，模型恢复后我会优先处理。";
    let target = target_text.to_string();
    let is_target_fallback = move |t: &str| t.trim() == target.trim();
    assert_eq!(
        classify_assistant_context_reply_kind(Some(&parsed), target_text, is_target_fallback,),
        AssistantContextReplyKind::ProviderUnavailablePlaceholder
    );
    assert_eq!(
        provider_unavailable_assistant_placeholder(),
        "[provider_unavailable_reply_omitted]"
    );
}

#[test]
fn normal_assistant_task_reply_keeps_original_text_for_recent_turn_context() {
    let parsed = json!({
        "text": "README.md"
    });
    // §7.2: 普通答案 + 任意 fallback 集合都不命中 → Normal。
    let never_fallback = |_: &str| false;
    assert_eq!(
        classify_assistant_context_reply_kind(Some(&parsed), "README.md", never_fallback,),
        AssistantContextReplyKind::Normal
    );
}

#[test]
fn provider_unavailable_task_is_skipped_for_last_turn_context() {
    let state = test_state();
    let db = state.core.db.get().expect("db");
    create_tasks_table(&db);
    let provider_unavailable = crate::i18n_t_with_default(
        &state,
        "clawd.msg.clarify_question_fallback",
        "I need to clarify: what task is this message about? Please provide the target or context.",
    );
    db.execute(
        "INSERT INTO tasks (user_id, chat_id, user_key, kind, payload_json, result_json, error_text, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'ask', ?4, ?5, NULL, 'succeeded', '100', '100')",
        params![
            1_i64,
            2_i64,
            "test-user",
            r#"{"text":"先列出 logs 目录下前 5 个文件名"}"#,
            r#"{"text":"act_plan.log, clawd.log, feishud.log, install_ops.log, logs_directory_listing.txt"}"#,
        ],
    )
    .expect("insert older successful turn");
    db.execute(
        "INSERT INTO tasks (user_id, chat_id, user_key, kind, payload_json, result_json, error_text, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'ask', ?4, ?5, NULL, 'succeeded', '200', '200')",
        params![
            1_i64,
            2_i64,
            "test-user",
            r#"{"text":"一句话说有没有异常"}"#,
            json!({ "text": provider_unavailable }).to_string(),
        ],
    )
    .expect("insert provider unavailable turn");
    drop(db);

    let last_turn = build_last_turn_full_context(&state, Some("test-user"), 1, 2, 400, 1200);
    assert!(last_turn.contains("先列出 logs 目录下前 5 个文件名"));
    assert!(last_turn.contains("act_plan.log, clawd.log"));
    assert!(!last_turn.contains("一句话说有没有异常"));
    assert!(!last_turn.contains(provider_unavailable_assistant_placeholder()));
}

#[test]
fn last_turn_full_context_does_not_fallback_to_legacy_chat() {
    let state = test_state();
    let db = state.core.db.get().expect("db");
    create_tasks_table(&db);
    let legacy_chat_id = legacy_principal_chat_id("test-user", 2).expect("legacy chat id");
    db.execute(
        "INSERT INTO tasks (user_id, chat_id, user_key, kind, payload_json, result_json, error_text, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'ask', ?4, ?5, NULL, 'succeeded', '100', '100')",
        params![
            1_i64,
            legacy_chat_id,
            "test-user",
            r#"{"text":"旧 chat 的问题"}"#,
            r#"{"text":"旧 chat 的答案"}"#,
        ],
    )
    .expect("insert legacy chat task");
    drop(db);

    let last_turn = build_last_turn_full_context(&state, Some("test-user"), 1, 2, 400, 1200);
    assert_eq!(last_turn, "<none>");
}

#[test]
fn recent_assistant_replies_context_does_not_fallback_to_legacy_chat() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db");
        create_memories_table(&db);
        create_user_preferences_table(&db);
    }
    insert_memory(
        &state,
        1,
        legacy_principal_chat_id("test-user", 2).expect("legacy chat id"),
        Some("test-user"),
        "local",
        None,
        MEMORY_ROLE_ASSISTANT,
        "FILE:/tmp/legacy.txt",
        2000,
        MemoryWriteKind::AssistantOutcome,
    )
    .expect("insert legacy assistant memory");

    let recent = build_recent_assistant_replies_context(&state, Some("test-user"), 1, 2, 3, 220);
    assert_eq!(recent, "<none>");
}

#[test]
fn recent_assistant_replies_context_skips_execution_summary_noise() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db");
        create_memories_table(&db);
        create_user_preferences_table(&db);
    }
    insert_memory(
        &state,
        1,
        2,
        Some("test-user"),
        "local",
        None,
        MEMORY_ROLE_ASSISTANT,
        &format!(
            "{}**执行过程**\n1. 调用命令 `pwd`\n   输出：\n```text\n/tmp\n```",
            crate::memory::LLM_SHORT_TERM_MEMORY_PREFIX
        ),
        2000,
        MemoryWriteKind::AssistantOutcome,
    )
    .expect("insert execution summary memory");

    let recent = build_recent_assistant_replies_context(&state, Some("test-user"), 1, 2, 3, 220);
    assert_eq!(recent, "<none>");
}

#[test]
fn long_term_source_recall_skips_unfinished_goal_runtime_memory() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db");
        create_memories_table(&db);
    }
    insert_memory(
        &state,
        1,
        2,
        Some("test-user"),
        "local",
        None,
        MEMORY_ROLE_USER,
        "帮我写一个 RustClaw 生产环境部署方案，包含启动、日志和回滚",
        2000,
        MemoryWriteKind::Default,
    )
    .expect("insert user memory");
    insert_memory(
        &state,
        1,
        2,
        Some("test-user"),
        "local",
        None,
        MEMORY_ROLE_SYSTEM,
        "Unfinished goal\nUser request: 帮我写一个 RustClaw 生产环境部署方案，包含启动、日志和回滚\nCurrent blocker: provider timeout",
        2000,
        MemoryWriteKind::UnfinishedGoal,
    )
    .expect("insert unfinished runtime memory");

    let recalled =
        recall_memories_since_id(&state, Some("test-user"), 1, 2, 0, 10).expect("recall memories");
    assert_eq!(recalled.len(), 1);
    assert!(recalled[0].2.contains("部署方案"));
    assert!(!recalled
        .iter()
        .any(|(_, _, content, _)| content.contains("Unfinished goal")));
}

#[test]
fn long_term_source_recall_skips_transient_assistant_runtime_noise() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db");
        create_memories_table(&db);
        create_user_preferences_table(&db);
    }
    insert_memory(
        &state,
        1,
        2,
        Some("test-user"),
        "local",
        None,
        MEMORY_ROLE_USER,
        "记住这个长期偏好：回复要简短。",
        2000,
        MemoryWriteKind::Default,
    )
    .expect("insert user memory");
    insert_memory(
        &state,
        1,
        2,
        Some("test-user"),
        "local",
        None,
        MEMORY_ROLE_ASSISTANT,
        "**执行过程**\n1. 调用命令 `pwd`\n   输出：\n```text\n/tmp\n```",
        2000,
        MemoryWriteKind::AssistantOutcome,
    )
    .expect("insert transient assistant runtime memory");
    insert_memory(
        &state,
        1,
        2,
        Some("test-user"),
        "local",
        None,
        MEMORY_ROLE_ASSISTANT,
        "已记住：后续回复保持简短。",
        2000,
        MemoryWriteKind::AssistantOutcome,
    )
    .expect("insert assistant answer memory");

    let recalled =
        recall_memories_since_id(&state, Some("test-user"), 1, 2, 0, 10).expect("recall memories");
    assert_eq!(recalled.len(), 2);
    assert!(recalled
        .iter()
        .any(|(_, _, content, _)| content.contains("回复要简短")));
    assert!(recalled
        .iter()
        .any(|(_, _, content, _)| content.contains("已记住")));
    assert!(!recalled
        .iter()
        .any(|(_, _, content, _)| content.contains("执行过程")));
}

#[test]
fn current_turn_constraints_do_not_become_persistent_preferences() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db");
        create_memories_table(&db);
        create_user_preferences_table(&db);
    }

    insert_memory(
        &state,
        1,
        2,
        Some("test-user"),
        "local",
        None,
        MEMORY_ROLE_USER,
        "do not run anything, just tell me a very short joke",
        2000,
        MemoryWriteKind::Default,
    )
    .expect("insert user memory");

    let db = state.core.db.get().expect("db");
    let (memory_type, is_instructional): (String, i64) = db
        .query_row(
            "SELECT memory_type, is_instructional FROM memories ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("latest memory");
    assert_eq!(memory_type, MEMORY_TYPE_GENERIC);
    assert_eq!(is_instructional, 0);
    drop(db);

    let prefs = recall_user_preferences(&state, Some("test-user"), 1, 2, 8).expect("recall prefs");
    assert!(prefs.is_empty());
}

#[test]
fn structured_memory_intent_preferences_are_applied_without_phrase_rules() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db");
        create_user_preferences_table(&db);
        let action = crate::memory::intent::MemoryAction {
            action: crate::memory::intent::MemoryActionOp::Upsert,
            kind: crate::memory::intent::MemoryActionKind::Preference,
            scope: crate::memory::intent::MemoryScope::User,
            key: "response_language".to_string(),
            value: "한국어".to_string(),
            normalized_value: Some("ko-KR".to_string()),
            confidence: 0.93,
            ttl_policy: crate::memory::intent::MemoryTtlPolicy::LongTerm,
            expires_at_ts: None,
            source: crate::memory::intent::MemoryActionSource {
                source_kind: crate::memory::intent::MemorySourceKind::LlmMemoryExtract,
                source_ref: Some("task:test:user".to_string()),
                source_text: "앞으로 한국어로 답해줘.".to_string(),
                memory_ids: Vec::new(),
            },
            reason: "durable response language preference".to_string(),
            risk: crate::memory::intent::MemoryActionRisk {
                sensitive: false,
                injection_like: false,
            },
        };
        let stats = crate::memory::apply::apply_memory_actions(
            &state,
            &db,
            1,
            2,
            "test-user",
            &[action],
            "2026-05-19T00:00:00Z",
            1,
        )
        .expect("apply memory action");
        assert_eq!(stats.upserted_preferences, 1);
    }

    let prefs = recall_user_preferences(&state, Some("test-user"), 1, 2, 8).expect("recall prefs");
    assert_eq!(
        prefs,
        vec![("response_language".to_string(), "ko-KR".to_string())]
    );
}

#[test]
fn structured_memory_intent_multilingual_overwrite_uses_schema_key_only() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db");
        create_user_preferences_table(&db);
        let ja_action = crate::memory::intent::MemoryAction {
            action: crate::memory::intent::MemoryActionOp::Upsert,
            kind: crate::memory::intent::MemoryActionKind::Preference,
            scope: crate::memory::intent::MemoryScope::User,
            key: "response_language".to_string(),
            value: "日本語".to_string(),
            normalized_value: Some("ja-JP".to_string()),
            confidence: 0.9,
            ttl_policy: crate::memory::intent::MemoryTtlPolicy::LongTerm,
            expires_at_ts: None,
            source: crate::memory::intent::MemoryActionSource {
                source_kind: crate::memory::intent::MemorySourceKind::LlmMemoryExtract,
                source_ref: Some("task:test-ja:user".to_string()),
                source_text: "これからは日本語で返事してください。".to_string(),
                memory_ids: Vec::new(),
            },
            reason: "durable response language preference".to_string(),
            risk: crate::memory::intent::MemoryActionRisk {
                sensitive: false,
                injection_like: false,
            },
        };
        let fr_action = crate::memory::intent::MemoryAction {
            action: crate::memory::intent::MemoryActionOp::Upsert,
            kind: crate::memory::intent::MemoryActionKind::Preference,
            scope: crate::memory::intent::MemoryScope::User,
            key: "response_language".to_string(),
            value: "français".to_string(),
            normalized_value: Some("fr-FR".to_string()),
            confidence: 0.92,
            ttl_policy: crate::memory::intent::MemoryTtlPolicy::LongTerm,
            expires_at_ts: None,
            source: crate::memory::intent::MemoryActionSource {
                source_kind: crate::memory::intent::MemorySourceKind::LlmMemoryExtract,
                source_ref: Some("task:test-fr:user".to_string()),
                source_text: "Réponds toujours en français.".to_string(),
                memory_ids: Vec::new(),
            },
            reason: "durable response language preference update".to_string(),
            risk: crate::memory::intent::MemoryActionRisk {
                sensitive: false,
                injection_like: false,
            },
        };
        crate::memory::apply::apply_memory_actions(
            &state,
            &db,
            1,
            2,
            "test-user",
            &[ja_action],
            "2026-05-19T00:00:00Z",
            1,
        )
        .expect("apply Japanese action");
        crate::memory::apply::apply_memory_actions(
            &state,
            &db,
            1,
            2,
            "test-user",
            &[fr_action],
            "2026-05-19T00:00:01Z",
            2,
        )
        .expect("apply French action");
    }

    let prefs = recall_user_preferences(&state, Some("test-user"), 1, 2, 8).expect("recall prefs");
    assert_eq!(
        prefs,
        vec![("response_language".to_string(), "fr-FR".to_string())]
    );
}

#[test]
fn structured_memory_intent_delete_removes_preference_by_schema_key() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db");
        create_user_preferences_table(&db);
        db.execute(
            "INSERT INTO user_preferences (user_id, chat_id, user_key, pref_key, pref_value, confidence, source, updated_at, updated_at_ts)
             VALUES (1, 2, 'test-user', 'response_language', 'fr-FR', 0.91, 'test', 'now', 1)",
            [],
        )
        .expect("insert preference");
        let action = crate::memory::intent::MemoryAction {
            action: crate::memory::intent::MemoryActionOp::Delete,
            kind: crate::memory::intent::MemoryActionKind::Preference,
            scope: crate::memory::intent::MemoryScope::User,
            key: "response_language".to_string(),
            value: String::new(),
            normalized_value: None,
            confidence: 0.9,
            ttl_policy: crate::memory::intent::MemoryTtlPolicy::LongTerm,
            expires_at_ts: None,
            source: crate::memory::intent::MemoryActionSource {
                source_kind: crate::memory::intent::MemorySourceKind::LlmMemoryExtract,
                source_ref: Some("task:test:user".to_string()),
                source_text: "Forget my default response language preference.".to_string(),
                memory_ids: Vec::new(),
            },
            reason: "user asked to remove stored response language preference".to_string(),
            risk: crate::memory::intent::MemoryActionRisk {
                sensitive: false,
                injection_like: false,
            },
        };
        let stats = crate::memory::apply::apply_memory_actions(
            &state,
            &db,
            1,
            2,
            "test-user",
            &[action],
            "2026-05-19T00:00:00Z",
            2,
        )
        .expect("apply delete action");
        assert_eq!(stats.deleted_preferences, 1);
    }

    let prefs = recall_user_preferences(&state, Some("test-user"), 1, 2, 8).expect("recall prefs");
    assert!(prefs.is_empty());
}

#[test]
fn memory_intent_prompt_avoids_fixed_natural_language_trigger_examples() {
    let prompt = build_memory_intent_llm_prompt("plain request", "task:test:user");
    for forbidden in [
        "以后",
        "默认中文",
        "from now on",
        "going forward",
        "reply in english",
    ] {
        assert!(
            !prompt.contains(forbidden),
            "prompt should not contain fixed natural-language trigger example: {forbidden}"
        );
    }
    assert!(prompt.contains("task:test:user"));
    assert!(prompt.contains("\"memory_actions\""));
}

#[test]
fn ordered_entries_from_candidate_confirmation_reply_extracts_in_order() {
    let reply = "我找到 3 个最接近的候选，请确认要哪一个：scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt；scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md；scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt";
    assert_eq!(
        ordered_entries_from_assistant_reply(reply, 10),
        vec![
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt".to_string(),
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md".to_string(),
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt".to_string(),
        ]
    );
}

#[test]
fn recent_assistant_replies_context_includes_ordered_entries_for_candidate_list() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db");
        create_memories_table(&db);
    }
    insert_memory(
        &state,
        1,
        2,
        Some("test-user"),
        "local",
        None,
        MEMORY_ROLE_ASSISTANT,
        "我找到 3 个最接近的候选，请确认要哪一个：scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt；scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md；scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt",
        2000,
        MemoryWriteKind::AssistantOutcome,
    )
    .expect("insert assistant memory");

    let recent = build_recent_assistant_replies_context(&state, Some("test-user"), 1, 2, 3, 220);
    assert!(recent.contains(
        "ordered_entries=1:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt"
    ));
    assert!(recent.contains("2:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md"));
    assert!(recent.contains("3:scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt"));
}

#[test]
fn result_text_prefers_read_range_excerpt_over_linewise_json_text() {
    let parsed = json!({
        "text": "{\"a\":1}\n{\"b\":2}",
        "task_journal": {
            "trace": {
                "step_results": [{
                    "skill": "system_basic",
                    "output_excerpt": "{\"action\":\"read_range\",\"resolved_path\":\"/tmp/logs/act_plan.log\",\"excerpt\":\"10|first line\\n11|second line\\n12|third line\"}"
                }]
            }
        }
    });
    assert_eq!(
        extract_result_text_for_recent_turns(&parsed).as_deref(),
        Some("read_range path=/tmp/logs/act_plan.log\nfirst line\nsecond line\nthird line")
    );
}

#[test]
fn result_text_prefers_read_range_excerpt_when_messages_and_final_answer_are_machine_json() {
    let machine_text = "{\"a\":1}\n{\"b\":2}";
    let parsed = json!({
        "text": machine_text,
        "messages": [machine_text],
        "task_journal": {
            "summary": {
                "final_answer": machine_text
            },
            "trace": {
                "step_results": [{
                    "skill": "system_basic",
                    "output_excerpt": "{\"action\":\"read_range\",\"resolved_path\":\"/tmp/logs/act_plan.log\",\"excerpt\":\"2283|alpha\\n2284|beta\\n2285|gamma\"}"
                }]
            }
        }
    });
    assert_eq!(
        extract_result_text_for_recent_turns(&parsed).as_deref(),
        Some("read_range path=/tmp/logs/act_plan.log\nalpha\nbeta\ngamma")
    );
}

#[test]
fn result_text_prefers_read_range_observed_excerpt_even_when_final_text_is_plain_summary() {
    let parsed = json!({
        "text": "共计 9 个日志文件、2 个 Markdown 文档和 9 个子目录。",
        "task_journal": {
            "trace": {
                "step_results": [{
                    "skill": "system_basic",
                    "output_excerpt": "{\"action\":\"read_range\",\"resolved_path\":\"/tmp/logs/logs_directory_listing.txt\",\"excerpt\":\"28|\\n29|共计 9 个日志文件、2 个 Markdown 文档和 9 个子目录。\"}"
                }]
            }
        }
    });
    assert_eq!(
        extract_result_text_for_recent_turns(&parsed).as_deref(),
        Some("read_range path=/tmp/logs/logs_directory_listing.txt\n\n共计 9 个日志文件、2 个 Markdown 文档和 9 个子目录。")
    );
}

#[test]
fn result_text_prefers_observed_listing_over_wrapped_numbered_answer() {
    let parsed = json!({
        "text": "logs 目录下前 5 个文件名：\n1. act_plan.log\n2. clawd.log\n3. feishud.log\n4. install_ops.log\n5. logs_directory_listing.txt",
        "task_journal": {
            "trace": {
                "step_results": [{
                    "skill": "run_cmd",
                    "output_excerpt": "act_plan.log\nclawd.log\nfeishud.log\ninstall_ops.log\nlogs_directory_listing.txt"
                }]
            }
        }
    });
    assert_eq!(
        extract_result_text_for_recent_turns(&parsed).as_deref(),
        Some("act_plan.log\nclawd.log\nfeishud.log\ninstall_ops.log\nlogs_directory_listing.txt")
    );
}

#[test]
fn retrieval_source_ref_for_memory_is_stable_id_string() {
    assert_eq!(retrieval_source_ref_for_memory(42), "42");
}

#[test]
fn retrieval_source_ref_for_preference_uses_trimmed_pref_key() {
    assert_eq!(
        retrieval_source_ref_for_preference(" response_language "),
        "response_language"
    );
}

#[test]
fn route_hint_upserts_agent_display_name_preference() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db");
        create_user_preferences_table(&db);
    }

    upsert_user_preferences_from_route_hint(&state, 1, 2, Some("test-user"), "小爪")
        .expect("upsert route hint preference");

    let prefs = recall_user_preferences(&state, Some("test-user"), 1, 2, 8).expect("recall prefs");
    assert!(prefs
        .iter()
        .any(|(key, value)| key == "agent_display_name" && value == "小爪"));
}

#[test]
fn route_hint_rejects_invalid_agent_display_name() {
    let state = test_state();
    {
        let db = state.core.db.get().expect("db");
        create_user_preferences_table(&db);
    }

    upsert_user_preferences_from_route_hint(&state, 1, 2, Some("test-user"), "assistant")
        .expect("reject invalid route hint");

    let prefs = recall_user_preferences(&state, Some("test-user"), 1, 2, 8).expect("recall prefs");
    assert!(!prefs.iter().any(|(key, _)| key == "agent_display_name"));
}

#[test]
fn retrieval_source_ref_for_kb_chunk_is_chunk_scoped() {
    assert_eq!(
        retrieval_source_ref_for_kb_chunk("user:test", "docs", "chunk-001"),
        "kb:user:test:docs:chunk-001"
    );
}

#[test]
fn retrieval_producer_constants_match_pipeline_intent() {
    assert_eq!(RETRIEVAL_PRODUCER_KB, "kb");
    assert_eq!(RETRIEVAL_PRODUCER_MEMORY_PIPELINE, RETRIEVAL_SOURCE_MEMORY);
}
