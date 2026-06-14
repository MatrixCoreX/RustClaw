#[test]
fn direct_answer_preserves_run_cmd_directory_entry_names() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_output_test_{}_run_cmd_names",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&temp_dir);
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "act_plan.log\nclawd.log\nfeishud.log\n",
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
            semantic_kind: Default::default(),
            locator_hint: "logs".to_string(),
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
        Some("act_plan.log\nclawd.log\nfeishud.log")
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_answer_preserves_run_cmd_semantic_directory_path_list() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            ".\n./scripts\n./scripts/nl_tests\n./crates/skills/browser_web/node_modules/playwright-core/bin\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "查找当前工作目录中哪些文件夹存放了 .sh 脚本文件，列出这些文件夹的名称"
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
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::DirectoryNames,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/home/guagua/rustclaw".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                ".\n./scripts\n./scripts/nl_tests\n./crates/skills/browser_web/node_modules/playwright-core/bin"
            )
        );
}

#[test]
fn direct_answer_preserves_run_cmd_directory_entry_names_without_request_text_limit() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_output_test_{}_run_cmd_limit",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&temp_dir);
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "a\nb\nc\nd\n"));
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
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("a\nb\nc\nd")
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_answer_formats_run_cmd_exists_probe_with_resolved_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_output_test_{}_run_cmd_exists",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&temp_dir);
    let file_path = temp_dir.join("rustclaw.service");
    std::fs::write(&file_path, "unit").expect("write fixture file");
    let resolved = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.clone())
        .to_string_lossy()
        .to_string();
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "EXISTS\n"));
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
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(resolved.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(format!("有，路径：{resolved}").as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_answer_formats_run_cmd_not_found_probe_as_no() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "NOT_FOUND\n"));
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
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
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
        Some("没有")
    );
}

#[test]
fn direct_answer_defers_health_check_json_for_act_free_shape() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"clawd_health_port_open":true,"telegramd_process_count":0}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "做一次 health check".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
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
fn direct_answer_defers_health_check_service_status_contract_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"clawd_process_count":1,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0}}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "检查 clawd 服务当前状态，并用一句话说明来源。".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
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
fn direct_answer_defers_wrapped_health_check_service_status_free_shape() {
    let mut loop_state = LoopState::new(2);
    let body = serde_json::json!({
        "extra": {
            "clawd_health_port_open": true,
            "clawd_log": {
                "exists": true,
                "keyword_error_count": 43
            },
            "clawd_process_count": 1,
            "system_health": {
                "os_family": "linux",
                "warnings": ["disk_root_low"]
            },
            "telegramd_log": {
                "exists": true,
                "keyword_error_count": 1
            },
            "telegramd_process_count": 0
        },
        "text": "{\"clawd_health_port_open\":true,\"clawd_process_count\":1}"
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", &body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Show system/service status".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some("show status".to_string()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_health_check_diagnostic_summary_for_system_health_fields() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"clawd_process_count":1,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":43},"system_health":{"os_family":"linux","load_avg_1m":3.81,"memory_available_bytes":11270471680,"disk_root_available_bytes":18108059648,"warnings":[]}}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "执行基础健康检查，列出最重要的诊断结论".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
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
fn direct_answer_defers_health_check_summary_for_act_free_shape() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"clawd_process_count":7,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "对系统做一次基础健康检查，只总结操作系统信息，RustClaw 自身不展开总结，仅返回其关键字段"
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
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
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
fn direct_answer_passes_health_check_json_only_for_raw_output_contract() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"clawd_health_port_open":true,"telegramd_process_count":0}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "run health_check and return the raw output".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(body)
    );
}

#[test]
fn direct_answer_defers_health_check_summary_over_later_steps_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":12,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "system_basic",
        r#"{"action":"info","os":"macos","hostname":"example"}"#,
    ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "Run a basic health check. Summarize only the host operating system, and for RustClaw itself just list the key fields.".to_string(),
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
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
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
fn direct_answer_defers_health_check_one_sentence_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
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
fn direct_answer_defers_health_check_unhealthy_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":0,"telegramd_process_count":1,"clawd_health_port_open":false,"clawd_log":{"exists":true,"keyword_error_count":3},"telegramd_log":{"exists":true,"keyword_error_count":0}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "run a basic health check here and summarize only the most important findings"
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
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
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
fn direct_answer_defers_health_check_telegramd_stopped_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
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
fn direct_answer_defers_health_check_language_sensitive_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some(
            "run a basic health check here and summarize only the most important findings"
                .to_string(),
        ),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_health_check_os_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":12,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "做一次基础健康检查，只返回操作系统层面的关键字段，不要包含 RustClaw 自身的状态摘要"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_failed_safe_clarify".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some(
            "做一次基础健康检查，只总结操作系统；RustClaw 自身不要总结，直接给我关键字段。"
                .to_string(),
        ),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_health_check_os_warning_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":1,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":true,"keyword_error_count":0},"system_health":{"os_family":"linux","warnings":["disk_root_low"]}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "run a basic health check here and summarize only the most important findings"
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
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some(
            "run a basic health check here and summarize only the most important findings"
                .to_string(),
        ),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_process_basic_port_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "process_basic",
            "exit=0\nCOMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nclawd 4498 testuser 12u IPv4 0x0 0t0 TCP *:8787 (LISTEN)\nnginx 51129 testuser 6u IPv4 0x0 0t0 TCP *:80 (LISTEN)\nss-local 424 testuser 6u IPv4 0x0 0t0 TCP 127.0.0.1:1086 (LISTEN)\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "看看这台机器现在有哪些端口在监听，然后挑最值得注意的几个简单说一下"
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
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
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
fn direct_answer_formats_process_basic_port_status_contract_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "process_basic",
            "exit=0\nState  Recv-Q Send-Q Local Address:Port  Peer Address:PortProcess\nLISTEN 0      4096   127.0.0.53%lo:53         0.0.0.0:*\nLISTEN 0      4096         0.0.0.0:8787       0.0.0.0:*    users:((\"clawd\",pid=706551,fd=31))\nLISTEN 0      4096         0.0.0.0:22         0.0.0.0:*\nLISTEN 0      511          0.0.0.0:80         0.0.0.0:*\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "查看当前机器监听的端口，列出最值得注意的端口并简单说明".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("service_status should use process_basic port evidence directly");

    assert!(answer.contains("port.count=4"));
    assert!(answer.contains("port[1].number=8787"));
    assert!(answer.contains("port[1].process=clawd"));
    assert!(!answer.contains("State  Recv-Q"));
}

#[test]
fn direct_answer_defers_wrapped_process_basic_port_status_to_synthesis() {
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"extra":{"hostname":"ThinkPad-X1","os":"linux","pid":2304396},"text":"{\"hostname\":\"ThinkPad-X1\",\"os\":\"linux\",\"pid\":2304396}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "process_basic",
        &serde_json::json!({
            "extra": {
                "action": "port_list",
                "command_tool": "ss",
                "exit_code": 0,
                "listener_count": 3,
                "public_listener_count": 2,
                "localhost_listener_count": 1,
                "ports": ["80", "8787", "46225"],
                "public_ports": ["80", "8787"],
                "public_listeners": [
                    {
                        "bind_scope": "all_interfaces",
                        "is_loopback": false,
                        "is_wildcard": true,
                        "local_address": "0.0.0.0",
                        "local_endpoint": "0.0.0.0:80",
                        "pid": null,
                        "port": "80",
                        "process_name": null
                    },
                    {
                        "bind_scope": "all_interfaces",
                        "is_loopback": false,
                        "is_wildcard": true,
                        "local_address": "0.0.0.0",
                        "local_endpoint": "0.0.0.0:8787",
                        "pid": 2308287,
                        "port": "8787",
                        "process_name": "clawd"
                    }
                ],
                "listeners": [],
                "output": "exit=0\nState Recv-Q Send-Q Local Address:Port Peer Address:PortProcess"
            },
            "text": "exit=0\nState Recv-Q Send-Q Local Address:Port Peer Address:PortProcess"
        })
        .to_string(),
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "查看当前机器监听的端口，列出最值得注意的端口并简单说明".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context));

    assert_eq!(answer, None);
}

#[test]
fn direct_answer_keeps_wrapped_process_basic_port_status_scalar_count() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "process_basic",
        &serde_json::json!({
            "extra": {
                "action": "port_list",
                "listener_count": 3,
                "public_listener_count": 2,
                "ports": ["80", "8787", "46225"],
                "public_ports": ["80", "8787"],
                "public_listeners": [],
                "listeners": []
            },
            "text": "exit=0"
        })
        .to_string(),
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "count listening ports".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context));

    assert_eq!(answer.as_deref(), Some("3"));
}

#[test]
fn direct_answer_formats_process_basic_service_status_contract_without_json_passthrough() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "process_basic",
        "exit=0\nPID PPID %CPU %MEM COMM\n413590 7620 1.0 0.2 clawd",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "检查 clawd 服务当前状态，并用一句话说明来源。".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("service_status should use process_basic evidence directly");

    assert!(
        serde_json::from_str::<serde_json::Value>(&answer).is_err(),
        "service_status must not expose raw structured JSON"
    );
    assert!(answer.contains("clawd"));
    assert!(answer.contains("process_basic"));
}

#[test]
fn direct_answer_formats_process_basic_multi_row_cpu_inventory() {
    let answer = super::process_basic_service_status_direct_answer_candidate(
        None,
        "exit=0\nPID PPID %CPU %MEM COMM\n1713539 8057 6.4 2.7 WebKitWebProces\n8923 7620 6.1 0.3 ptyxis\n7886 7620 3.5 1.8 gnome-shell\n9127 9116 3.5 4.2 codex\n1100416 83086 1.2 1.7 chrome",
        Some(OutputResponseShape::OneSentence),
        false,
    )
    .expect("multi-row process inventory should produce a data-grounded summary");

    assert!(answer.contains("WebKitWebProces"));
    assert!(answer.contains("ptyxis"));
    assert!(answer.contains("gnome-shell"));
    assert!(answer.contains("codex"));
    assert!(answer.contains("chrome"));
    assert!(answer.contains("6.4"));
    assert!(answer.contains("最值得注意的是 WebKitWebProces"));
    assert!(!answer.contains("PID PPID"));
}

#[test]
fn direct_answer_formats_process_basic_no_match_as_not_running() {
    let answer = super::process_basic_service_status_direct_answer_candidate(
        None,
        "exit=0\nPID PPID %CPU %MEM COMM\nno matching processes for filter: telegramd",
        Some(OutputResponseShape::OneSentence),
        true,
    )
    .expect("no-match process output should produce a status answer");

    assert!(answer.contains("telegramd"));
    assert!(answer.contains("not running"));
    assert!(!answer.contains("1 process record"));
    assert!(!answer.contains("COMM=telegramd"));
}

#[test]
fn direct_answer_prefers_process_basic_status_over_later_system_info() {
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"extra":{"hostname":"ThinkPad-X1","os":"linux","pid":2304396},"text":"{\"hostname\":\"ThinkPad-X1\",\"os\":\"linux\",\"pid\":2304396}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "process_basic",
        &serde_json::json!({
            "extra": {
                "action": "ps",
                "exit_code": 0,
                "filter": "telegramd",
                "limit": 20,
                "match_count": 0,
                "process_count": 0,
                "running": false,
                "status": "not_running",
                "output": "exit=0\nPID PPID %CPU %MEM COMM\nno matching processes for filter: telegramd",
                "platform": "linux"
            },
            "text": "exit=0\nPID PPID %CPU %MEM COMM\nno matching processes for filter: telegramd"
        })
        .to_string(),
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "check whether telegramd is running right now".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: "telegramd".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("process status should override generic system info");
    let value: serde_json::Value = serde_json::from_str(&answer).expect("structured status json");

    assert_eq!(
        value
            .get("status_source")
            .and_then(serde_json::Value::as_str),
        Some("process_basic.ps")
    );
    assert_eq!(
        value.get("target").and_then(serde_json::Value::as_str),
        Some("telegramd")
    );
    assert_eq!(
        value
            .get("process_filter")
            .and_then(serde_json::Value::as_str),
        Some("telegramd")
    );
    assert_eq!(
        value.get("status").and_then(serde_json::Value::as_str),
        Some("not_running")
    );
    assert_eq!(
        value.get("running").and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        value.get("match_count").and_then(serde_json::Value::as_u64),
        Some(0)
    );
}
