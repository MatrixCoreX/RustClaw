#[test]
fn direct_answer_gate_promotes_chat_to_clarify_when_blocker_is_missing() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("clarify", gate_contract(false, "none", "none"));
    gate.clarify_question = "要创建的文件夹叫什么名字？".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "create a folder", gate);

    assert!(
        matches!(outcome, DirectAnswerPreflight::Clarify(question) if question == "要创建的文件夹叫什么名字？")
    );
    let route = ctx.route_result.expect("route");
    assert_eq!(route.ask_mode, crate::AskMode::clarify());
    assert!(route.is_clarify_gate());
    assert!(route.needs_clarify);
    assert_eq!(route.clarify_question, "要创建的文件夹叫什么名字？");
    assert!(route.route_reason.contains("direct_answer_gate_clarify"));
}

#[test]
fn direct_answer_gate_clarify_preserves_existing_file_delivery_contract() {
    let mut route = chat_route_for_gate();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("clarify", gate_contract(false, "none", "none"));
    gate.clarify_question = "Which file should I send?".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "send that file", gate);

    assert!(
        matches!(outcome, DirectAnswerPreflight::Clarify(question) if question == "Which file should I send?")
    );
    let route = ctx.route_result.expect("route");
    assert!(route.is_clarify_gate());
    assert!(route.needs_clarify);
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
}

#[test]
fn chat_prompt_context_appends_authoritative_route_resolution() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "上一个和上上个哪个更多，只回答目录名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason:
            "'上一个'=assistant[-1](document,17), '上上个'=assistant[-2](scripts,48); scripts 更多"
                .to_string(),
        route_confidence: Some(0.94),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            locator_hint: "scripts".to_string(),
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let rendered = chat_prompt_context_with_route_resolution(
        "### MEMORY_CONTEXT\nRECENT_ASSISTANT_RESULTS\n- old summary",
        Some(&ctx),
    );
    assert!(rendered.contains("### ROUTE_RESOLUTION"));
    assert!(rendered.contains("resolved_user_intent: 上一个和上上个哪个更多，只回答目录名"));
    assert!(rendered.contains("locator_hint: scripts"));
    assert!(!rendered.contains("scripts 更多"));
    assert!(!rendered.contains("route_reason:"));
}

#[test]
fn chat_prompt_context_replaces_empty_placeholder_with_route_resolution() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "client-like-continuous-20260428_144029".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(0.94),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let rendered = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));
    assert!(!rendered.contains("<none>"));
    assert!(rendered.contains("### ROUTE_RESOLUTION"));
    assert!(rendered.contains("client-like-continuous-20260428_144029"));
}

#[test]
fn chat_prompt_context_includes_recent_execution_when_contract_requires_evidence() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "Summarize the observed README excerpt in one sentence".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "prior observed content is available".to_string(),
        route_confidence: Some(0.94),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(
            "read_range path=/tmp/README.md\n# RustClaw\nlocal Rust agent runtime".to_string(),
        ),
        ..Default::default()
    };

    let rendered = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));

    assert!(rendered.contains("### ROUTE_RESOLUTION"));
    assert!(rendered.contains("### RECENT_EXECUTION_CONTEXT"));
    assert!(rendered.contains("local Rust agent runtime"));
}

#[test]
fn chat_prompt_context_includes_recent_execution_for_repaired_evidence_route() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "Compare two previously observed file excerpts.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "semantic_contract_requires_evidence; active_text_followup_route_repair"
            .to_string(),
        route_confidence: Some(0.94),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(
            "first read_range result: README fixture\nsecond read_range result: service notes"
                .to_string(),
        ),
        ..Default::default()
    };

    let rendered = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));

    assert!(rendered.contains("### RECENT_EXECUTION_CONTEXT"));
    assert!(rendered.contains("README fixture"));
    assert!(rendered.contains("service notes"));
}

#[test]
fn chat_prompt_context_omits_recent_execution_for_pure_active_text_rewrite() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "Current task:\nRestyle the previous assistant reply.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "active_text_followup_route_repair".to_string(),
        route_confidence: Some(0.94),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        cross_turn_recent_execution_context: Some(
            "current_process_cwd: /home/guagua/rustclaw".to_string(),
        ),
        ..Default::default()
    };

    let rendered = chat_prompt_context_with_route_resolution(
        "### RUNTIME_CONTEXT\ncurrent_process_cwd: /home/guagua/rustclaw",
        Some(&ctx),
    );

    assert!(rendered.contains("### ROUTE_RESOLUTION"));
    assert!(!rendered.contains("### RECENT_EXECUTION_CONTEXT"));
    assert!(!rendered.contains("current_process_cwd"));
}

#[test]
fn chat_user_request_preserves_inline_structured_prompt_when_resolution_dropped_payload() {
    let prompt = r#"sort this JSON array by score descending and render it as a markdown table: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let resolved =
        "Sort the provided JSON array by score in descending order and output as a markdown table";
    assert_eq!(chat_user_request(resolved, prompt), prompt);
}

#[test]
fn chat_request_for_prompt_keeps_original_constraints_and_semantic_anchor() {
    let request = chat_request_for_prompt(
        "刚才我让你记住的测试编号是什么？只回答编号。",
        "client-like-continuous-20260428_144029",
    );
    assert!(request.contains("Original user request:"));
    assert!(request.contains("只回答编号"));
    assert!(request.contains("Resolved semantic intent / answer candidate:"));
    assert!(request.contains("client-like-continuous-20260428_144029"));
    assert!(request.contains("output only the resolved value"));
}

#[test]
fn direct_answer_chat_user_request_strips_unapproved_answer_candidate() {
    let unapproved = direct_answer_chat_user_request(
        "get current hostname\nanswer_candidate: stale-user",
        "只输出当前机器 hostname，不要解释",
        false,
    );
    assert_eq!(unapproved, "get current hostname");

    let approved = direct_answer_chat_user_request(
        "recall stored id\nanswer_candidate: client-like-continuous-20260428_144029",
        "刚才我让你记住的测试编号是什么？只回答编号。",
        true,
    );
    assert!(approved.contains("answer_candidate: client-like-continuous-20260428_144029"));
}

#[test]
fn task_payload_text_preserves_raw_current_turn_for_chat_language_hint() {
    let task = crate::ClaimedTask {
        task_id: "task".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"先只看登录模块"}).to_string(),
    };
    assert_eq!(task_payload_text(&task).as_deref(), Some("先只看登录模块"));
}

#[test]
fn chat_reply_does_not_attach_context_process_message() {
    let reply = ask_reply_with_chat_process("RustClaw 是本地 agent 运行时。".to_string(), "zh-CN");

    assert_eq!(reply.text, "RustClaw 是本地 agent 运行时。");
    assert!(reply.messages.is_empty());
}

#[test]
fn english_chat_reply_does_not_attach_execution_process_message() {
    let reply = ask_reply_with_chat_process("RustClaw is a local agent runtime.".to_string(), "en");

    assert_eq!(reply.text, "RustClaw is a local agent runtime.");
    assert!(reply.messages.is_empty());
}

#[test]
fn structural_alias_ack_without_answer_candidate_defers_to_language_path() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.should_refresh_long_term_memory = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "再记一下“乙”指 /tmp/device/docs/service_notes.md",
        "record alias to /tmp/device/docs/service_notes.md",
        "zh-CN",
    )
    .is_none());
}

#[test]
fn structural_alias_ack_uses_unquoted_memory_alias_and_answer_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Acknowledge and retain that the note file path is ",
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md\n",
        "answer_candidate: confirmed"
    )
    .to_string();
    route.should_refresh_long_term_memory = false;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "Remember that the note file means scripts/nl_tests/fixtures/device_local/docs/service_notes.md. Reply only confirmed.",
        "Acknowledge and retain that the note file path is scripts/nl_tests/fixtures/device_local/docs/service_notes.md\nanswer_candidate: confirmed",
        "en",
    )
    .unwrap();

    assert_eq!(reply.text, "confirmed");
    assert!(reply.messages.is_empty());
}

#[test]
fn structural_alias_ack_uses_i18n_for_alias_state_patch() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.policy.schedule.i18n_dict.insert(
        "clawd.msg.memory.alias_remembered".to_string(),
        "alias remembered via i18n".to_string(),
    );
    let mut route = chat_route_for_gate();
    route.route_reason = "structured_alias_binding_fast_path".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [{
                    "alias": "那个文件",
                    "target": "/tmp/device/README.md"
                }]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "先记一下，后面我说“那个文件”就是 /tmp/device/README.md",
        "先记一下，后面我说“那个文件”就是 /tmp/device/README.md",
        "zh-CN",
    )
    .expect("alias state patch ack");

    assert_eq!(reply.text, "alias remembered via i18n");
    assert!(reply.messages.is_empty());
}

#[test]
fn structural_alias_ack_accepts_alias_only_state_patch_without_turn_type() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.policy.schedule.i18n_dict.insert(
        "clawd.msg.memory.alias_remembered".to_string(),
        "alias remembered via i18n".to_string(),
    );
    let route = chat_route_for_gate();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: None,
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [{
                    "alias": "ALPHA_DOC",
                    "target": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
                }]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "For this conversation, remember that ALPHA_DOC refers to scripts/nl_tests/fixtures/device_local/docs/service_notes.md.",
        "Establish ALPHA_DOC as a temporary alias for scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
        "zh-CN",
    )
    .expect("alias-only state patch should be enough for structural ack");

    assert_eq!(reply.text, "alias remembered via i18n");
    assert!(reply.messages.is_empty());
}

#[test]
fn structural_alias_ack_prefers_safe_candidate_for_alias_state_patch() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.policy.schedule.i18n_dict.insert(
        "clawd.msg.memory.alias_remembered".to_string(),
        "alias remembered via i18n".to_string(),
    );
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Set session alias: 資料A -> scripts/nl_tests/fixtures/device_local/docs/service_notes.md\nanswer_candidate: 記憶しました"
            .to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: None,
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [{
                    "alias": "資料A",
                    "target": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
                }]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "この会話では、参照名「資料A」を scripts/nl_tests/fixtures/device_local/docs/service_notes.md として覚えてください。",
        "Set session alias: 資料A -> scripts/nl_tests/fixtures/device_local/docs/service_notes.md\nanswer_candidate: 記憶しました",
        "mixed",
    )
    .expect("safe model-rendered ack should win");

    assert_eq!(reply.text, "記憶しました");
    assert!(reply.messages.is_empty());
}

#[test]
fn structural_alias_ack_rejects_path_candidate_for_alias_state_patch() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.policy.schedule.i18n_dict.insert(
        "clawd.msg.memory.alias_remembered".to_string(),
        "alias remembered via i18n".to_string(),
    );
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Set session alias ALPHA_DOC\nanswer_candidate: ALPHA_DOC -> scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
            .to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: None,
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [{
                    "alias": "ALPHA_DOC",
                    "target": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
                }]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "For this conversation, remember that ALPHA_DOC refers to scripts/nl_tests/fixtures/device_local/docs/service_notes.md.",
        "Set session alias ALPHA_DOC\nanswer_candidate: ALPHA_DOC -> scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
        "zh-CN",
    )
    .expect("unsafe path candidate should fall back to i18n");

    assert_eq!(reply.text, "alias remembered via i18n");
    assert!(reply.messages.is_empty());
}

#[test]
fn structural_alias_ack_uses_update_key_when_alias_existed() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.policy.schedule.i18n_dict.insert(
        "clawd.msg.memory.alias_updated".to_string(),
        "alias updated via i18n".to_string(),
    );
    let route = chat_route_for_gate();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        session_alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
            alias: "甲文件".to_string(),
            target: "/tmp/device/old.md".to_string(),
            updated_at_ts: 1,
        }],
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [{
                    "alias": "甲文件",
                    "target": "/tmp/device/new.md"
                }]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "不对，甲文件改成 /tmp/device/new.md。只回复已更新。",
        "不对，甲文件改成 /tmp/device/new.md。只回复已更新。",
        "zh-CN",
    )
    .expect("alias update ack");

    assert_eq!(reply.text, "alias updated via i18n");
    assert!(reply.messages.is_empty());
}

#[test]
fn structural_alias_ack_rejects_recall_question_without_memory_update_contract() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Recall the first file and the directory inspected earlier.".to_string();
    route.should_refresh_long_term_memory = false;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "上一个结果里说的“第一个文件”是什么，上上个动作查的又是什么目录",
        "Recall the first file and the directory inspected earlier.",
        "zh-CN",
    )
    .is_none());
}

#[test]
fn response_language_hint_prefers_current_request_language() {
    assert_eq!(
        crate::language_policy::preferred_response_language_hint("写个两句短诗", None),
        "zh-CN"
    );
    assert_eq!(
        crate::language_policy::preferred_response_language_hint(
            "do not run anything, just tell me a very short joke",
            None
        ),
        "en"
    );
    assert_eq!(
        crate::language_policy::preferred_response_language_hint("用 English 解释 README", None),
        "mixed"
    );
    assert_eq!(
        crate::language_policy::preferred_response_language_hint("12345", None),
        "config_default"
    );
}

#[test]
fn normalizer_chat_direct_answer_does_not_bypass_gate_for_unverified_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "normalizer supplied candidate".to_string(),
        route_confidence: Some(0.95),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜",
            Some(&ctx),
        ),
        None
    );

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜",
            Some(&ctx),
        ),
        None
    );
}

#[test]
fn active_task_text_mutation_does_not_direct_return_normalizer_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "active_task_refinement\nanswer_candidate: - first\n- second\n- third\n- fourth"
            .to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "output_shape": {"bullet_count": 3}
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "active_task_refinement\nanswer_candidate: - first\n- second\n- third\n- fourth",
            Some(&ctx),
        ),
        None
    );
}

#[test]
fn direct_answer_gate_skips_boundary_clean_freeform_chat() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Draft a short release note for a calendar app.".to_string();
    route.route_confidence = Some(0.95);
    route.output_contract.response_shape = crate::OutputResponseShape::Free;

    assert!(direct_answer_gate_can_skip_for_boundary_clean_chat(
        &state,
        "Draft a short release note for a calendar app.",
        Some(&route)
    ));
}

#[test]
fn direct_answer_gate_boundary_clean_chat_does_not_skip_locator_surface() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Summarize README.md.".to_string();
    route.route_confidence = Some(0.95);
    route.output_contract.response_shape = crate::OutputResponseShape::Free;

    assert!(!direct_answer_gate_can_skip_for_boundary_clean_chat(
        &state,
        "Summarize README.md.",
        Some(&route)
    ));
}

#[test]
fn direct_answer_gate_boundary_clean_chat_does_not_skip_memory_write() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Remember the current marker.".to_string();
    route.route_confidence = Some(0.95);
    route.should_refresh_long_term_memory = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;

    assert!(!direct_answer_gate_can_skip_for_boundary_clean_chat(
        &state,
        "Remember the current marker.",
        Some(&route)
    ));
}

#[test]
fn direct_answer_gate_boundary_clean_chat_does_not_skip_workspace_identity() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = std::path::PathBuf::from("/tmp/rustclaw");
    let workspace_name = state
        .skill_rt
        .workspace_root
        .file_name()
        .and_then(|name| name.to_str())
        .expect("test workspace root should have a name");
    let mut route = chat_route_for_gate();
    route.resolved_intent = format!("Draft a short release note for {workspace_name}.");
    route.route_confidence = Some(0.95);
    route.output_contract.response_shape = crate::OutputResponseShape::Free;

    assert!(!direct_answer_gate_can_skip_for_boundary_clean_chat(
        &state,
        &format!("Draft a short release note for {workspace_name}."),
        Some(&route)
    ));
}

#[test]
fn normalizer_chat_direct_answer_preserves_current_turn_version_literal() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    let candidate = "Deploy after confirming Python 3.10 is installed.";
    route.resolved_intent = format!("deployment_sentence\nanswer_candidate: {candidate}");
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate_with_context_summary(
            &state,
            &format!("deployment_sentence\nanswer_candidate: {candidate}"),
            Some(&ctx),
            None,
            Some("Write one deployment sentence mentioning Python 3.10"),
        )
        .as_deref(),
        Some(candidate)
    );
}

#[test]
fn normalizer_chat_direct_answer_rejects_candidate_missing_current_turn_version_literal() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    let candidate = "Deploy after confirming Python 3.11 is installed.";
    route.resolved_intent = format!("deployment_sentence\nanswer_candidate: {candidate}");
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate_with_context_summary(
            &state,
            &format!("deployment_sentence\nanswer_candidate: {candidate}"),
            Some(&ctx),
            None,
            Some("Write one deployment sentence mentioning Python 3.10"),
        ),
        None
    );
}

#[test]
fn normalizer_chat_direct_answer_allows_distinctive_candidate_bound_in_memory_context() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "recall_scalar\nanswer_candidate: RC-CONT-CN-0428-A".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        memory_context_for_execution: Some(
            "### MEMORY_CONTEXT (NOT CURRENT REQUEST)\n\
#### STABLE_FACTS\n\
- Current consecutive test ID: RC-CONT-CN-0428-A"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "recall_scalar\nanswer_candidate: RC-CONT-CN-0428-A",
            Some(&ctx),
        )
        .as_deref(),
        Some("RC-CONT-CN-0428-A")
    );
}

#[test]
fn normalizer_chat_direct_answer_allows_preference_ack_candidate_with_current_marker() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    let candidate = "Confirmed. Marker RC-CONT-EN-0428-B is remembered.";
    route.resolved_intent = format!("remember marker\nanswer_candidate: {candidate}");
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate_with_context_summary(
            &state,
            &format!("remember marker\nanswer_candidate: {candidate}"),
            Some(&ctx),
            None,
            Some("For this continuous test, remember marker RC-CONT-EN-0428-B.")
        )
        .as_deref(),
        Some(candidate)
    );
}

#[test]
fn normalizer_chat_direct_answer_adds_current_marker_for_memory_ack_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "remember marker\nanswer_candidate: Confirmed.".to_string();
    route.should_refresh_long_term_memory = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate_with_context_summary(
            &state,
            "remember marker\nanswer_candidate: Confirmed.",
            Some(&ctx),
            None,
            Some("For this continuous test, remember marker RC-CONT-EN-0428-B.")
        )
        .as_deref(),
        Some("Confirmed. RC-CONT-EN-0428-B")
    );
}

#[test]
fn normalizer_answer_candidate_strips_internal_context_sections() {
    let raw = concat!(
        "alias update\n",
        "answer_candidate: updated\n\n",
        "### SESSION_ALIAS_BINDINGS\n",
        "- alias: ALPHA_DOC\n",
        "  target: scripts/nl_tests/fixtures/device_local/docs/service_notes.md\n\n",
        "### ACTIVE_EXECUTION_ANCHOR\n",
        "followup_bound_target: scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );

    assert_eq!(
        normalizer_answer_candidate_from_resolved_prompt(raw).as_deref(),
        Some("updated")
    );
}

#[test]
fn normalizer_chat_direct_answer_alias_update_keeps_candidate_without_context_blocks() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Update ALPHA_DOC alias binding to scripts/nl_tests/fixtures/device_local/docs/release_checklist.md\n",
        "answer_candidate: updated\n\n",
        "### SESSION_ALIAS_BINDINGS\n",
        "- alias: ALPHA_DOC\n",
        "  target: scripts/nl_tests/fixtures/device_local/docs/service_notes.md\n\n",
        "### ACTIVE_EXECUTION_ANCHOR\n",
        "followup_bound_target: scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    )
    .to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [{
                    "alias": "ALPHA_DOC",
                    "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
                }]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate_with_context_summary(
            &state,
            "Update ALPHA_DOC alias binding\nanswer_candidate: updated\n\n### SESSION_ALIAS_BINDINGS\n- alias: ALPHA_DOC",
            Some(&ctx),
            None,
            Some("Correction: ALPHA_DOC now refers to scripts/nl_tests/fixtures/device_local/docs/release_checklist.md.")
        )
        .as_deref(),
        Some("updated")
    );
}

#[test]
fn normalizer_chat_direct_answer_rejects_preference_ack_candidate_with_pathlike_token() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    let candidate =
        "Confirmed. scripts/nl_tests/fixtures/device_local/docs/release_checklist.md remembered.";
    route.resolved_intent = format!("remember alias\nanswer_candidate: {candidate}");
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate_with_context_summary(
            &state,
            &format!("remember alias\nanswer_candidate: {candidate}"),
            Some(&ctx),
            None,
            Some("Remember scripts/nl_tests/fixtures/device_local/docs/release_checklist.md.")
        ),
        None
    );
}

#[test]
fn normalizer_chat_direct_answer_allows_bound_anchor_basename_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "known_file_basename\nanswer_candidate: ABCD.txt".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "route_view=false resolved_prompt=known_file_basename\n\
answer_candidate: ABCD.txt\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: Delivery\n\
followup_bound_target: /tmp/rustclaw/stem_unique/ABCD.txt\n\
followup_ordered_entries: 1:/tmp/rustclaw/stem_unique/ABCD.txt\n\
observed_bound_target: /tmp/rustclaw/stem_unique/ABCD.txt"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "known_file_basename\nanswer_candidate: ABCD.txt",
            Some(&ctx),
        )
        .as_deref(),
        Some("ABCD.txt")
    );
}

#[test]
fn normalizer_chat_direct_answer_reads_route_answer_candidate_when_merged_prompt_lacks_it() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "previous_ordered_entry\nanswer_candidate: orders".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "route_view=false resolved_prompt=previous_ordered_entry\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: List\n\
followup_ordered_entries: 1:orders | 2:service_logs | 3:users"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "merged active task without candidate",
            Some(&ctx)
        )
        .as_deref(),
        Some("orders")
    );
}

#[test]
fn normalizer_chat_direct_answer_uses_repaired_turn_binding_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    let candidate = "上一个结果中的第一个文件是 app.log；上上个动作查询的目录是 /tmp/device/logs";
    route.resolved_intent = format!("recent_turn_binding\nanswer_candidate: {candidate}");
    route.route_reason =
        "recent turn binding resolved; llm_semantic_contract_repair:contract_structurally_valid_but_turn_binding_invalid_active_task_context; repaired target /tmp/device/logs"
            .to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "route_view=false resolved_prompt=recent_turn_binding\n\
answer_candidate: 上一个结果中的第一个文件是 app.log；上上个动作查询的目录是 /tmp/device/logs\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: Read\n\
followup_bound_target: /tmp/device/docs\n\
observed_bound_target: /tmp/device/docs"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(&state, "merged prompt", Some(&ctx)).as_deref(),
        Some(candidate)
    );
}

#[test]
fn normalizer_chat_direct_answer_uses_context_answer_candidate_with_route_path_support() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    let candidate = "第一个文件是 app.log，上上个动作查询的目录是 /tmp/device/logs";
    route.resolved_intent = "recent_turn_binding".to_string();
    route.route_reason = "recent context resolved app.log and the logs directory".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "route_view=false resolved_prompt=recent_turn_binding\n\
answer_candidate: 第一个文件是 app.log，上上个动作查询的目录是 /tmp/device/logs\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: Read\n\
followup_bound_target: /tmp/device/docs\n\
observed_bound_target: /tmp/device/docs"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(&state, "merged prompt", Some(&ctx)).as_deref(),
        Some(candidate)
    );
}

#[test]
fn normalizer_chat_direct_answer_uses_context_answer_candidate_with_existing_workspace_path() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    let target = state
        .skill_rt
        .workspace_root
        .join("rustclaw_context_candidate_marker");
    std::fs::create_dir_all(&target).expect("create workspace child");
    let target = target.to_string_lossy().to_string();
    let candidate = format!("resolved directory: {target}");
    route.resolved_intent = "recent_turn_binding".to_string();
    route.route_reason = "recent context resolved the requested directory".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(format!(
            "route_view=false resolved_prompt=recent_turn_binding\n\
answer_candidate: {candidate}\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: Read\n\
followup_bound_target: /tmp/device/docs\n\
observed_bound_target: /tmp/device/docs"
        )),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            &format!("recent_turn_binding\nanswer_candidate: {candidate}"),
            Some(&ctx),
        )
        .as_deref(),
        Some(candidate.as_str())
    );
}

#[test]
fn normalizer_chat_direct_answer_uses_rendered_context_override_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    let target = state
        .skill_rt
        .workspace_root
        .join("rustclaw_context_candidate_marker");
    std::fs::create_dir_all(&target).expect("create workspace child");
    let target = target.to_string_lossy().to_string();
    let candidate = format!("resolved directory: {target}");
    route.resolved_intent = "recent_turn_binding".to_string();
    route.route_reason = "recent context resolved the requested directory".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let rendered_context = format!(
        "route_view=false resolved_prompt=recent_turn_binding\n\
answer_candidate: {candidate}\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: Read\n\
followup_bound_target: /tmp/device/docs\n\
observed_bound_target: /tmp/device/docs"
    );

    assert_eq!(
        normalizer_chat_direct_answer_candidate_with_context_summary(
            &state,
            &format!("recent_turn_binding\nanswer_candidate: {candidate}"),
            Some(&ctx),
            Some(&rendered_context),
            None,
        )
        .as_deref(),
        Some(candidate.as_str())
    );
}

#[test]
fn normalizer_chat_direct_answer_rejects_repaired_turn_binding_without_path_proof() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "recent_turn_binding\nanswer_candidate: 查询目录是 /tmp/device/logs".to_string();
    route.route_reason =
        "recent turn binding resolved; llm_semantic_contract_repair:contract_structurally_valid_but_turn_binding_invalid_active_task_context; repaired target /tmp/device/docs"
            .to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(&state, "merged prompt", Some(&ctx)),
        None
    );
}

#[test]
fn normalizer_chat_direct_answer_allows_active_observation_synthesis_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.resolved_intent =
        "classify the current observed excerpt\nanswer_candidate: It is a runtime log.".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "route_view=false resolved_prompt=classify active excerpt\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: Read\n\
followup_bound_target: /tmp/rustclaw/app.log\n\
observed_bound_target: /tmp/rustclaw/app.log"
                .to_string(),
        ),
        cross_turn_recent_execution_context: Some(
            "### RECENT_EXECUTION_EVENTS\n\
1 request=read target result=2026-05-30T00:00:00Z INFO clawd listening"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "classify the current observed excerpt\nanswer_candidate: It is a runtime log.",
            Some(&ctx),
        )
        .as_deref(),
        Some("It is a runtime log.")
    );
}

#[test]
fn normalizer_chat_direct_answer_does_not_self_ground_answer_candidate_from_context_summary() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "known_file_basename\nanswer_candidate: ABCD.txt".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "route_view=false resolved_prompt=known_file_basename\n\
answer_candidate: ABCD.txt\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: Delivery"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "known_file_basename\nanswer_candidate: ABCD.txt",
            Some(&ctx),
        ),
        None
    );
}

#[test]
fn normalizer_chat_direct_answer_does_not_bypass_evidence_contract() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent:
            "检查当前目录是否有隐藏文件\nanswer_candidate: 有，例如 .git、.gitignore、.pids"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "needs local evidence".to_string(),
        route_confidence: Some(0.95),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Medium,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            semantic_kind: crate::OutputSemanticKind::HiddenEntriesCheck,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "检查当前目录是否有隐藏文件\nanswer_candidate: 有，例如 .git、.gitignore、.pids",
            Some(&ctx),
        ),
        None
    );
}

#[test]
fn normalizer_chat_direct_answer_uses_runtime_fact_candidate_without_budget_fallback() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let runtime_path = state.skill_rt.workspace_root.to_string_lossy().to_string();
    let route = crate::RouteResult {
            ask_mode: crate::AskMode::direct_answer(),
            resolved_intent: format!(
                "User request: output absolute path of current working directory\nanswer_candidate: {runtime_path}"
            ),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer supplied runtime fact".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
            normalizer_chat_direct_answer_candidate(
                &state,
                &format!(
                    "User request: output absolute path of current working directory\nanswer_candidate: {runtime_path}"
                ),
                Some(&ctx),
            )
            .as_deref(),
            Some(runtime_path.as_str())
        );
}

#[test]
fn normalizer_chat_direct_answer_uses_runtime_identity_candidate() {
    let Some(runtime_user) = ["USER", "LOGNAME", "USERNAME"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
    else {
        return;
    };
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = format!("runtime_scalar\nanswer_candidate: {runtime_user}");
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            &format!("runtime_scalar\nanswer_candidate: {runtime_user}"),
            Some(&ctx),
        )
        .as_deref(),
        Some(runtime_user.as_str())
    );
}

#[test]
fn normalizer_runtime_fact_direct_answer_allows_scalar_clarify_guard_output() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let runtime_path = state.skill_rt.workspace_root.to_string_lossy().to_string();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: format!("runtime_scalar\nanswer_candidate: {runtime_path}"),
        needs_clarify: true,
        clarify_question: "Please provide the target path.".to_string(),
        route_reason: "background_locator_requires_clarify".to_string(),
        route_confidence: Some(0.95),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_runtime_fact_direct_answer_candidate(
            &state,
            &format!("runtime_scalar\nanswer_candidate: {runtime_path}"),
            Some(&ctx),
        )
        .as_deref(),
        Some(runtime_path.as_str())
    );
}
