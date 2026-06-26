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
        route_reason: "semantic_contract_requires_evidence".to_string(),
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
fn structural_alias_ack_without_safe_candidate_defers_for_alias_state_patch() {
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

    assert!(structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "先记一下，后面我说“那个文件”就是 /tmp/device/README.md",
        "先记一下，后面我说“那个文件”就是 /tmp/device/README.md",
        "zh-CN",
    )
    .is_none());
}

#[test]
fn structural_alias_ack_defers_alias_only_state_patch_without_turn_type() {
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

    assert!(structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "For this conversation, remember that ALPHA_DOC refers to scripts/nl_tests/fixtures/device_local/docs/service_notes.md.",
        "Establish ALPHA_DOC as a temporary alias for scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
        "zh-CN",
    )
    .is_none());
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

    assert!(structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "For this conversation, remember that ALPHA_DOC refers to scripts/nl_tests/fixtures/device_local/docs/service_notes.md.",
        "Set session alias ALPHA_DOC\nanswer_candidate: ALPHA_DOC -> scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
        "zh-CN",
    )
    .is_none());
}

#[test]
fn structural_alias_ack_defers_update_without_safe_candidate_when_alias_existed() {
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

    assert!(structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "不对，甲文件改成 /tmp/device/new.md。只回复已更新。",
        "不对，甲文件改成 /tmp/device/new.md。只回复已更新。",
        "zh-CN",
    )
    .is_none());
}

#[test]
fn structural_alias_ack_defers_memory_rule_when_machine_literal_would_be_dropped() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.policy.schedule.i18n_dict.insert(
        "clawd.msg.memory.alias_remembered".to_string(),
        "alias remembered via i18n".to_string(),
    );
    let mut route = chat_route_for_gate();
    route.should_refresh_long_term_memory = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [{
                    "alias": "provider_blocker",
                    "target": "external_blocker"
                }]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "Keep provider_blocker as the visible machine token for this memory rule.",
        "Remember this provider quota classification rule.",
        "en",
    )
    .is_none());
}

#[test]
fn structural_alias_ack_accepts_memory_rule_candidate_with_machine_literal() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Store provider quota classification rule\nanswer_candidate: provider_blocker"
            .to_string();
    route.should_refresh_long_term_memory = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [{
                    "alias": "provider_blocker",
                    "target": "external_blocker"
                }]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = structural_alias_binding_ack(
        &state,
        Some(&ctx),
        "Keep provider_blocker as the visible machine token for this memory rule.",
        "Store provider quota classification rule\nanswer_candidate: provider_blocker",
        "en",
    )
    .expect("machine-literal candidate should stay eligible");

    assert_eq!(reply.text, "provider_blocker");
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
