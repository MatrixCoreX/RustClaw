#[test]
fn recent_artifacts_judgment_is_not_hard_classified_by_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "total 151792\n-rw-r--r--@ 1 testuser staff 76509771 Apr 12 16:30 model_io.log\n-rw-r--r--@ 1 testuser staff 906739 Apr 12 16:30 act_plan.log\n-rw-r--r--@ 1 testuser staff 191187 Apr 12 15:48 service_ops.log\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "列出 logs 目录最近修改的 3 个文件，再告诉我这更像是测试日志还是正式产物"
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
            semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
            locator_hint: "logs".to_string(),
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
fn direct_answer_defers_system_basic_info_summary_to_llm_for_brief_request() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"info","hostname":"rustclaw-test-host.local","os":"macos","arch":"x86_64","cwd":"/tmp/rustclaw-workspace"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent:
            "show me the basic machine info here like hostname and system, keep it brief"
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
            locator_kind: OutputLocatorKind::CurrentWorkspace,
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
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_archive_creation_success_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "archive_basic",
        "exit=0\nupdating: tmp/rustclaw-workspace/scripts/skill_calls/\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent:
            "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，然后告诉我是否成功"
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
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "scripts/skill_calls -> tmp/nl_archive_case.zip".to_string(),
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
    assert!(
        has_observed_answer_candidates(&loop_state),
        "archive output should remain available as observed facts for synthesis"
    );
}

#[test]
fn direct_answer_defers_archive_basic_output_destination_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "archive_basic",
            r#"{"action":"pack","format":"zip","source":"/tmp/rustclaw-workspace/scripts/skill_calls","archive":"/tmp/rustclaw-workspace/tmp/nl_archive_case.zip","output":"exit=0\nupdating: skill_calls/\n"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent:
            "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，然后告诉我是否成功"
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
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "scripts/skill_calls".to_string(),
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
    assert!(
        has_observed_answer_candidates(&loop_state),
        "archive json should remain available as observed facts for synthesis"
    );
}

#[test]
fn archive_read_direct_answer_projects_member_path_and_content_excerpt() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "archive_basic",
        r#"{"action":"read","archive":"/repo/tmp/test_bundle.zip","path":"notes.txt","member":"notes.txt","member_path":"notes.txt","content":"fixture archive notes\n","content_excerpt":"fixture archive notes"}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent:
            "Read member notes.txt from tmp/test_bundle.zip and return member_path and content_excerpt"
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
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ArchiveRead,
            locator_hint: "tmp/test_bundle.zip | notes.txt".to_string(),
            self_extension: crate::SelfExtensionContract {
                structured_field_selector: Some("member_path,content_excerpt".to_string()),
                ..crate::SelfExtensionContract::default()
            },
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(r#"{"content_excerpt":"fixture archive notes","member_path":"notes.txt"}"#)
    );
}

#[test]
fn archive_pack_scalar_contract_returns_created_archive_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "archive_basic",
            "archive_path=/tmp/rustclaw-workspace/tmp/nl_archive_case.zip\nexit=0\n  adding: /tmp/rustclaw-workspace/scripts/skill_calls/ (stored 0%)\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent:
            "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，只返回生成路径"
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
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ArchivePack,
            locator_hint: "scripts/skill_calls | tmp/nl_archive_case.zip".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/rustclaw-workspace/tmp/nl_archive_case.zip")
    );
}

#[test]
fn archive_pack_scalar_contract_accepts_route_marker_without_semantic_enum() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "archive_basic",
        "archive_path=/tmp/rustclaw-workspace/tmp/nl_archive_case.zip\nexit=0\n",
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.route_reason = "contract:archive_pack".to_string();
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case.zip".to_string();
    assert_eq!(
        route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/rustclaw-workspace/tmp/nl_archive_case.zip")
    );
}

#[test]
fn direct_scalar_reads_unique_scalar_from_multi_read_fields_with_container_noise() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "config_basic",
        r#"{"action":"read_fields","path":"/repo/package.json","format":"json","results":[{"exists":true,"field_path":"scripts","resolved_field_path":"scripts","value":{"build":"echo build","dev":"echo dev","lint":"echo lint"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}","value_type":"object"},{"exists":true,"field_path":"name","resolved_field_path":"name","value":"rustclaw-nl-fixture","value_text":"rustclaw-nl-fixture","value_type":"string"}]}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "/repo/package.json".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("rustclaw-nl-fixture")
    );
}

#[test]
fn direct_scalar_keeps_multi_read_fields_ambiguous_when_multiple_scalars_exist() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "config_basic",
        r#"{"action":"read_fields","path":"/repo/package.json","format":"json","results":[{"exists":true,"field_path":"name","resolved_field_path":"name","value":"rustclaw-nl-fixture","value_text":"rustclaw-nl-fixture","value_type":"string"},{"exists":true,"field_path":"version","resolved_field_path":"version","value":"0.1.0","value_text":"0.1.0","value_type":"string"}]}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "/repo/package.json".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn archive_unpack_contract_returns_one_sentence_destination_summary() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "archive_basic",
            "dest_path=/tmp/rustclaw-workspace/tmp/contract_matrix_unpacked\nexit=0\nArchive: /tmp/test_bundle.zip\n inflating: /tmp/rustclaw-workspace/tmp/contract_matrix_unpacked/notes.txt\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "把 test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果"
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
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ArchiveUnpack,
            locator_hint: "/tmp/test_bundle.zip | tmp/contract_matrix_unpacked".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("已解压到 /tmp/rustclaw-workspace/tmp/contract_matrix_unpacked，包含 notes.txt。")
    );
}

#[test]
fn direct_answer_defers_system_basic_info_summary_without_action_field() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"hostname":"rustclaw-test-host.local","os":"macos","arch":"x86_64","cwd":"/tmp/rustclaw-workspace"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent:
            "show me the basic machine info here like hostname and system, keep it brief"
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
            locator_kind: OutputLocatorKind::CurrentWorkspace,
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
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_system_basic_info_for_free_shape_request() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"info","hostname":"ThinkPad-X1","os":"linux","arch":"x86_64","cwd":"/home/guagua/rustclaw"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent:
            "show me the basic machine info here like hostname and system, keep it brief"
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
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_system_basic_info_service_status_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"extra":{"arch":"x86_64","current_user":"guagua","cwd":"/home/guagua/rustclaw","hostname":"ThinkPad-X1","os":"linux","pid":2488573,"process_rss_bytes":3055616,"uptime_seconds":"894677.25","workspace_root":"/home/guagua/rustclaw"},"text":"{\"arch\":\"x86_64\",\"current_user\":\"guagua\",\"cwd\":\"/home/guagua/rustclaw\",\"hostname\":\"ThinkPad-X1\",\"os\":\"linux\",\"pid\":2488573,\"process_rss_bytes\":3055616,\"uptime_seconds\":\"894677.25\",\"workspace_root\":\"/home/guagua/rustclaw\"}"}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "show status".to_string(),
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
fn direct_answer_extracts_cwd_from_system_basic_info_for_scalar_path_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"info","hostname":"ThinkPad-X1","os":"linux","arch":"x86_64","cwd":"/home/guagua/rustclaw","workspace_root":"/home/guagua/rustclaw"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "获取当前工作目录的绝对路径".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "contract:scalar_path_only".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
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
        Some("/home/guagua/rustclaw")
    );
}

#[test]
fn direct_scalar_extracts_cwd_from_system_basic_info_without_action_field() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"hostname":"ThinkPad-X1","os":"linux","arch":"x86_64","cwd":"/home/guagua/rustclaw","workspace_root":"/home/guagua/rustclaw"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "获取当前工作目录的绝对路径".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "contract:scalar_path_only".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/home/guagua/rustclaw")
    );
}

#[test]
fn direct_scalar_path_contract_prefers_recorded_write_file_path() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "/home/guagua/rustclaw"));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "write_file",
        "written 40 bytes to /home/guagua/rustclaw/document/pwd_line.txt",
    ));
    loop_state.output_vars.insert(
        "last_file_path".to_string(),
        "/home/guagua/rustclaw/document/pwd_line.txt".to_string(),
    );
    loop_state.last_written_file_path =
        Some("/home/guagua/rustclaw/document/pwd_line.txt".to_string());
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "create the file and send me the file path only".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "contract:scalar_path_only".to_string(),
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
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            locator_hint: "pwd_line.txt".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/home/guagua/rustclaw/document/pwd_line.txt")
    );
}

#[test]
fn workspace_project_summary_is_not_hard_summarized_by_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "Cargo.toml\ncrates/\nUI/\nconfigs/\nREADME.md\nREADME.zh-CN.md\nprompts/\nrustclaw.service\ncomponent_start/start-telegramd.sh\ncomponent_start/start-wechatd.sh\ncomponent_start/start-whatsappd.sh\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "用非技术用户能听懂的话，简短解释这个仓库主要是干什么的".to_string(),
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
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
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
fn direct_scalar_uses_latest_list_dir_entries_when_listing_is_latest_step() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "README.txt\n"));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("README.txt")
    );
}

#[test]
fn direct_scalar_path_only_uses_auto_locator_full_path_for_unique_list_dir_match() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-observed-output-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let file_path = temp_dir.join("Report.MD");
    std::fs::write(&file_path, "hello").unwrap();

    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "Report.MD\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "去 case_only 找 report.md，只输出路径".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_hint: "report.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(file_path.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    let resolved = file_path
        .canonicalize()
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(resolved.as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_scalar_path_only_uses_rooted_full_path_for_unique_find_name_match() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-observed-output-find-name-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let file_path = temp_dir.join("Report.MD");
    std::fs::write(&file_path, "hello").unwrap();

    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            &format!(
                r#"{{"action":"find_name","pattern":"report.md","count":1,"results":["Report.MD"],"root":"{}"}}"#,
                temp_dir.display()
            ),
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "去 case_only 找 report.md，只输出路径".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_hint: "report.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    let resolved = file_path
        .canonicalize()
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(resolved.as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn system_basic_find_path_normalization_prefers_existing_relative_path() {
    let rel_dir = Path::new("target").join(format!(
        "clawd-observed-output-find-path-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&rel_dir).unwrap();
    let file_path = rel_dir.join("Report.MD");
    std::fs::write(&file_path, "hello").unwrap();
    let cwd = std::env::current_dir().unwrap();
    let resolved_root = cwd.join(&rel_dir).to_string_lossy().to_string();
    let expected = file_path
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();

    assert_eq!(
        normalize_system_basic_match_path(
            Some(&resolved_root),
            Some(file_path.to_string_lossy().as_ref())
        )
        .as_deref(),
        Some(expected.as_str())
    );
    let _ = std::fs::remove_dir_all(rel_dir);
}

#[test]
fn direct_scalar_path_only_prefers_resolved_path_from_path_batch_facts() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD","resolved_path":"/tmp/case_only/Report.MD","size_bytes":33},"path":"/tmp/case_only/report.md","resolved_from_case_insensitive":true}],"include_missing":true}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "去 case_only 目录里找 report.md，只输出路径".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_hint: "report.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/case_only/Report.MD")
    );
}

#[test]
fn direct_answer_keeps_plain_path_terminal_format_for_observed_path_fact() {
    let mut loop_state = LoopState::new(2);
    loop_state.last_user_visible_respond = Some("/tmp/case_only/Report.MD".to_string());
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD","resolved_path":"/tmp/case_only/Report.MD","size_bytes":33},"path":"/tmp/case_only/Report.MD"}],"include_missing":true}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "去 case_only 目录里找 report.md，只输出路径".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
            locator_hint: "report.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/case_only/Report.MD")
    );
}

#[test]
fn direct_scalar_does_not_passthrough_multiline_list_dir_listing() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "README.txt\nnotes.md\n"));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None),
        None
    );
}

#[test]
fn direct_scalar_counts_multiline_list_dir_when_route_requests_count() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "a\nb\nc\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "数一下 scripts 目录直接有多少个子项".to_string(),
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
            locator_hint: "scripts".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("3")
    );
}

#[test]
fn direct_scalar_uses_inventory_dir_count_for_scalar_count() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"scripts","resolved_path":"/tmp/scripts","names_only":true,"names":["a","b","c"],"counts":{"total":3}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "数一下 scripts 目录直接子项有多少个，只输出数字".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:current_workspace_scalar_count".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::ScalarCount,
            locator_hint: "scripts".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("3")
    );
}

#[test]
fn direct_count_uses_inventory_dir_total_for_non_scalar_shape() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","path":"document","resolved_path":"/tmp/document","names_only":true,"names":["a","b","c","d"],"counts":{"total":4,"files":4,"dirs":0},"recursive":false}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "再数一下 document 目录直接有多少个子项".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "scalar count with free-form response shape".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::ScalarCount,
            locator_hint: "document".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("4")
    );
}

#[test]
fn direct_scalar_path_lists_inventory_dir_candidates_without_choosing_first() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/stem_multi","resolved_path":"/tmp/stem_multi","names_only":true,"names":["abcd.cpp","abcd.txt"],"counts":{"total":2}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "find matching paths".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "structured scalar path request".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_hint: "/tmp/stem_multi".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/stem_multi/abcd.cpp\n/tmp/stem_multi/abcd.txt")
    );
}

#[test]
fn direct_scalar_uses_inventory_dir_hidden_count_for_hidden_entries_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":".","resolved_path":"/tmp/workspace","include_hidden":true,"names_only":true,"names":[".git",".env","README.md"],"counts":{"total":3,"hidden":2}}"#,
        ));
    let route_result = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "数一下当前目录里以点开头的隐藏文件有几个，只输出数字".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:hidden_entries_check".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("2")
    );
}

#[test]
fn direct_answer_formats_package_manager_detect_summary() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "package_manager",
        "package_manager=brew",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "看看当前机器识别到的包管理器，再一句话说最可能日常会用哪个".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:package_manager_detect_summary".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::None,
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
        Some("检测到的包管理器是 brew，依据是 package_manager 返回了 package_manager=brew。")
    );
}

#[test]
fn direct_answer_formats_package_manager_matrix_basis_summary() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "package_manager",
        "package_manager=apt-get",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "检测这台机器可用的包管理器，并说明依据。".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:package_manager_detection".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::PackageManagerDetection,
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
        Some("检测到的包管理器是 apt-get，依据是 package_manager 返回了 package_manager=apt-get。")
    );
}

#[test]
fn direct_scalar_extracts_package_manager_detect_value() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "package_manager",
        "package_manager=brew",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "只输出当前机器识别到的包管理器名称".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:package_manager_detect_scalar".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("brew")
    );
}
