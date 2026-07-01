#[test]
fn direct_answer_formats_structured_keys_result_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"structured_keys","path":"/tmp/package.json","resolved_path":"/tmp/package.json","field_path":"scripts","exists":true,"container_type":"object","count":3,"keys":["build","dev","lint"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: "读 /tmp/package.json，告诉我 scripts 字段下都有哪些子键".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_explicit_path_structured_keys".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "/tmp/package.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("build\ndev\nlint")
    );
}

#[test]
fn direct_answer_formats_structured_keys_presence_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"structured_keys","path":"/tmp/en-US.toml","resolved_path":"/tmp/en-US.toml","field_path":"","exists":true,"container_type":"object","count":3,"keys":["execute_prefixes","locale","result_suffixes"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: "读取 /tmp/en-US.toml 并确认是否存在 negative_markers 字段".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "planner_locator_requires_evidence".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::StructuredKeys,
            locator_hint: "/tmp/en-US.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        original_user_request: Some(
            "读取 configs/command_intent/en-US.toml，只回答是否还有 negative_markers 字段"
                .to_string(),
        ),
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("不包含 negative_markers 字段")
    );
}

#[test]
fn direct_answer_formats_structured_array_identity_presence_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"structured_keys","path":"/tmp/skills_registry.toml","resolved_path":"/tmp/skills_registry.toml","field_path":"skills","exists":true,"container_type":"array","count":2,"identity_values":["fs_basic","config_basic"],"identity_omitted":0,"indices_preview":[{"index":0,"value_type":"object","keys":["name","planner_kind"],"identity_key":"name","identity_value":"fs_basic"},{"index":1,"value_type":"object","keys":["name","planner_kind"],"identity_key":"name","identity_value":"config_basic"}]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: "读取 /tmp/skills_registry.toml，回答 fs_basic 是否注册".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "planner_locator_requires_evidence".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::StructuredKeys,
            locator_hint: "/tmp/skills_registry.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        original_user_request: Some(
            "读取 docker/config/skills_registry.toml，回答 fs_basic 是否注册".to_string(),
        ),
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("包含 fs_basic")
    );
}

#[test]
fn structured_keys_one_sentence_defers_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"structured_keys","path":"/tmp/package.json","resolved_path":"/tmp/package.json","field_path":"scripts","exists":true,"container_type":"object","count":3,"keys":["build","dev","lint"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: "读 /tmp/package.json，用一句话告诉我 scripts 字段下有哪些子键"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_explicit_path_structured_keys".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "/tmp/package.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_formats_extract_fields_result_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_fields","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","count":2,"results":[{"field_path":"database.sqlite_path","exists":true,"value_type":"string","value_text":"data/rustclaw.db","value":"data/rustclaw.db"},{"field_path":"tools.allow_sudo","exists":true,"value_type":"bool","value_text":"true","value":true}]}"#,
        ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
            resolved_intent:
                "读取 /tmp/config.toml 里的 database.sqlite_path 和 tools.allow_sudo，告诉我两个字段的值"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:generic_explicit_path_extract_fields"
                .to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::None,
                locator_hint: "/tmp/config.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("database.sqlite_path: data/rustclaw.db\ntools.allow_sudo: true")
    );
}

#[test]
fn direct_answer_uses_inventory_dir_names_for_system_basic() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["act_plan.log","clawd.log","feishud.log"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下前 5 个文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::FileNames,
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("act_plan.log\nclawd.log\nfeishud.log")
    );
}

#[test]
fn direct_answer_uses_inventory_dir_names_for_fs_basic() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","path":"/tmp/document","resolved_path":"/tmp/document","files_only":true,"names_only":true,"names":["a.txt","b.md","c.png"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "List file names from a known directory.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::FileNames,
            locator_hint: "document".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("a.txt\nb.md\nc.png")
    );
}

#[test]
fn direct_answer_uses_inventory_dir_entry_sizes_when_names_only_is_false() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":false,"entries":[{"name":"act_plan.log","kind":"file","size_bytes":2467002},{"name":"clawd.run.log","kind":"file","size_bytes":397321},{"name":"clawd.log","kind":"file","size_bytes":2035}],"names":["act_plan.log","clawd.run.log","clawd.log"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下最大的 3 个文件，输出文件名和大小".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::FileNames,
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("act_plan.log 2467002\nclawd.run.log 397321\nclawd.log 2035")
    );
}

#[test]
fn direct_answer_does_not_apply_listing_limit_from_resolved_intent_text() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["a","b","c","d"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下前 2 个文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("a\nb\nc\nd")
    );
}

#[test]
fn direct_answer_does_not_apply_listing_limit_from_current_turn_request_text() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["a","b","c","d"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下的文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "normalizer:planner_execute_with_chat_finalizer".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some("列出 logs 目录下前 2 个文件名".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("a\nb\nc\nd")
    );
}

#[test]
fn scalar_listing_gate_does_not_repair_count_from_request_text_limit() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "a\nb\nc\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下的文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ScalarCount,
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some("列出 logs 目录下前 2 个文件名，只输出文件名".to_string()),
        ..AgentRunContext::default()
    };
    let route = agent_run_context.route_result.as_ref().unwrap();
    assert!(
        !super::scalar_route_prefers_structured_observed_answer(route, &loop_state,),
        "scalar/listing gate must not infer bounded listing from current-turn request text"
    );
}

#[test]
fn direct_answer_uses_latest_list_dir_entries_for_act_free_shape() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "README.txt\nnotes.md\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 archive 目录下有什么".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "archive".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("README.txt\nnotes.md")
    );
}

#[test]
fn direct_answer_uses_latest_list_dir_even_after_synthesis_step() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "alpha.md\nbeta.md\n"));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "synthesize_answer",
        "document 目录下有 alpha.md 和 beta.md。",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 document 目录下有哪些文件，只输出文件名列表".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::FileNames,
            locator_hint: "document".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some("列出 document 目录下有哪些文件，只输出文件名列表".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("alpha.md\nbeta.md")
    );
}

#[test]
fn direct_answer_preserves_list_dir_entries_without_request_text_limit() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "a\nb\nc\nd\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下前 2 个文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("a\nb\nc\nd")
    );
}

#[test]
fn direct_answer_defers_hidden_entries_explanation_shape_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        ".git\nREADME.md\n.env\nsrc\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: "检查当前目录是否存在隐藏文件，然后用一句话解释隐藏文件的常见用途"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_formats_hidden_entries_check_scalar_from_listing() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        ".git\nREADME.md\n.env\nsrc\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("2")
    );
}

#[test]
fn direct_answer_formats_hidden_entries_check_strict_shape_from_listing() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        ".\n..\n.codex\n.git/\n.gitignore\nREADME.md\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(".codex\n.git/\n.gitignore")
    );
}

#[test]
fn direct_answer_formats_hidden_entries_check_strict_shape_from_wrapped_inventory_with_limit() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":3,"files":2,"hidden":5,"total":5},"entries":[],"include_hidden":true,"names":[".agents",".codex",".git",".gitignore",".pids","README.md"],"names_only":true,"path":"/tmp/workspace"},"text":"{\"action\":\"inventory_dir\"}"}"#,
    ));
    let mut route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: "hidden entries selector_limit=3".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    route_result
        .output_contract
        .self_extension
        .list_selector
        .limit = Some(3);
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(".agents\n.codex\n.git")
    );
}

#[test]
fn direct_answer_formats_hidden_entries_check_from_names_when_entries_empty() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"dirs":33,"files":43,"hidden":5,"total":76},"entries":[],"include_hidden":true,"names":[".agents",".codex",".git",".gitignore",".pids","README.md"],"path":"/tmp/workspace"}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(".agents\n.codex\n.git\n.gitignore\n.pids")
    );
}

#[test]
fn direct_answer_formats_hidden_entries_check_empty_inventory_without_followup() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","counts":{"dirs":1,"files":1,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","name":"README.md","path":"README.md"},{"hidden":false,"kind":"dir","name":"src","path":"src"}],"include_hidden":true,"names":["README.md","src"],"path":"/tmp/workspace"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.route_reason =
        "structured_contract_hint_fast_path; contract_hint_fast_path".to_string();
    route_result.resolved_intent = "检查当前目录有没有隐藏文件，如果有就列出几个例子。".to_string();
    route_result.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("hidden entries strict contract should answer from inventory");

    assert!(answer.contains("未发现隐藏文件"));
    assert!(!answer.contains("要继续"));
}

#[test]
fn direct_answer_defers_hidden_entries_check_when_inventory_did_not_include_hidden() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","counts":{"dirs":1,"files":1,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","name":"README.md","path":"README.md"},{"hidden":false,"kind":"dir","name":"src","path":"src"}],"include_hidden":false,"names":["README.md","src"],"path":"/tmp/workspace"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_defers_hidden_entries_check_free_shape_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        ".cargo/\nREADME.md\n.dockerignore\n.env.example\nsrc\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_defers_hidden_entries_check_one_sentence_from_system_basic_inventory_dir() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/workspace","resolved_path":"/tmp/workspace","names_only":true,"include_hidden":true,"names":[".cargo",".dockerignore",".env.example","README.md","src"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_formats_existence_with_path_from_system_basic_path_batch_facts() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/tmp/rustclaw-workspace/rustclaw.service","size_bytes":1190},"path":"/tmp/rustclaw-workspace/rustclaw.service"}],"include_missing":true}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("有，路径：/tmp/rustclaw-workspace/rustclaw.service")
    );
}

#[test]
fn direct_answer_formats_strict_path_kind_from_fs_basic_path_batch_facts() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"dir","path":"configs/channels","resolved_path":"/tmp/repo/configs/channels","size_bytes":4096},"path":"/tmp/repo/configs/channels"}],"include_missing":true}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.resolved_intent = "查看 configs 目录下最后一个条目的路径和类型信息".to_string();
    route_result.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "/tmp/repo/configs/channels".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/repo/configs/channels | 目录")
    );
    assert!(observed_output_entries(&loop_state)
        .join("\n")
        .contains("kind=dir"));
}

#[test]
fn direct_answer_formats_multi_path_facts_without_llm_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"package.json","resolved_path":"/tmp/repo/package.json","size_bytes":120},"path":"package.json"},{"exists":false,"path":"nope.json","error":"not found"}],"include_missing":true}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.resolved_intent =
        "Inspect explicit file paths and answer with existence and type".to_string();
    route_result.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route_result.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route_result.output_contract.locator_hint = "/tmp/repo".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("multi path facts answer");
    assert!(answer.contains("/tmp/repo/package.json: exists, type file"));
    assert!(answer.contains("nope.json: not found"));
}

#[test]
fn direct_answer_formats_scalar_existence_without_path_from_system_basic_path_batch_facts() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"configs/config.toml","resolved_path":"/tmp/repo/configs/config.toml","size_bytes":1190},"path":"/tmp/repo/configs/config.toml"}],"include_missing":true}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查 configs/config.toml 是否存在，只回答有或没有".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "configs/config.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("有")
    );
}

#[test]
fn direct_answer_formats_path_batch_facts_requested_size() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"fields":["exists","size"],"facts":[{"exists":true,"fact":{"kind":"file","path":"data/rustclaw.db","resolved_path":"/tmp/repo/data/rustclaw.db","size_bytes":55226368},"path":"/tmp/repo/data/rustclaw.db"}],"include_missing":true}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.ask_mode = crate::AskMode::planner_execute_plain();
    route_result.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "data/rustclaw.db".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("yes, path: /tmp/repo/data/rustclaw.db, size: 55226368 bytes")
    );
}

#[test]
fn direct_answer_formats_missing_path_batch_facts_with_reason() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"/tmp/missing.txt","error":"not found"}],"include_missing":true}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查文件 /tmp/missing.txt 是否存在，如果不存在，简短说明原因。"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "/tmp/missing.txt".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("missing path answer");

    assert!(answer.contains("路径不存在"));
    assert!(answer.contains("/tmp/missing.txt"));
}

#[test]
fn direct_answer_formats_existence_with_path_from_run_cmd_yes_output() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_exists_yes_{}_{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let target = temp_dir.join("rustclaw.service");
    std::fs::write(&target, "ok").expect("write target");
    let expected = format!(
        "有，路径：{}",
        target
            .canonicalize()
            .unwrap_or(target.clone())
            .to_string_lossy()
    );

    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "yes\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(expected.as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_answer_formats_existence_with_path_from_run_cmd_exists_output() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_exists_lower_{}_{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let target = temp_dir.join("rustclaw.service");
    std::fs::write(&target, "ok").expect("write target");
    let expected = format!(
        "有，路径：{}",
        target
            .canonicalize()
            .unwrap_or(target.clone())
            .to_string_lossy()
    );

    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "exists\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(expected.as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_answer_formats_existence_with_path_from_system_basic_find_name_output() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_exists_find_name_{}_{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let target = temp_dir.join("rustclaw.service");
    std::fs::write(&target, "ok").expect("write target");
    let resolved = target
        .canonicalize()
        .unwrap_or(target.clone())
        .to_string_lossy()
        .to_string();

    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    let expected = format!("有，路径：{resolved}");
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(expected.as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}
