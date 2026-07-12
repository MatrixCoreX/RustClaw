#[test]
fn observed_fallback_prompt_renders_language_and_response_style_hints() {
    let prompt = crate::render_prompt_template(
            OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE,
            &[
                ("__USER_REQUEST__", "读一下 README 开头，然后用一句话总结"),
                (
                    "__RESOLVED_USER_INTENT__",
                    "读一下 README 开头，然后用一句话总结",
                ),
                (
                    "__OUTPUT_CONTRACT__",
                    r#"{"response_shape":"one_sentence","contract_marker":"content_excerpt_summary"}"#,
                ),
                (
                    "__OBSERVED_OUTPUTS__",
                    "### step_1 skill(read_file)\n# RustClaw",
                ),
                ("__CONFIG_RESPONSE_LANGUAGE__", "zh-CN"),
                ("__REQUEST_LANGUAGE_HINT__", "mixed"),
                (
                    "__RESPONSE_STYLE_HINT__",
                    "style_policy=one_sentence include_all_deliverables=true",
                ),
            ],
        );
    assert!(prompt.contains("Request language hint:\nmixed"));
    assert!(prompt.contains("Response style hint:"));
    assert!(prompt.contains("style_policy=one_sentence"));
    assert!(prompt.contains("include_all_deliverables=true"));
    assert!(prompt.contains("Do not collapse multi-dimensional structured evidence"));
    assert!(prompt.contains("combine the deliverables into one grammatical sentence"));
    assert!(prompt.contains("final_answer_shape` is `status_with_source"));
    assert!(prompt.contains("do not answer with only a raw machine status field"));
}

#[test]
fn observed_fallback_prompt_uses_compact_template_for_terminal_status_contracts() {
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.delivery_intent = OutputDeliveryIntent::None;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let path = observed_answer_fallback_prompt_logical_path(
        Some(&agent_run_context),
        "status=ok\nprocess=clawd\nport=8787",
    );

    assert_eq!(path, "prompts/observed_answer_fallback_compact_prompt.md");
}

#[test]
fn observed_fallback_prompt_uses_compact_template_for_docker_capability_ref() {
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::None;
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.delivery_intent = OutputDeliveryIntent::None;
    route_result.resolved_intent = "capability_ref=docker.version".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let path = observed_answer_fallback_prompt_logical_path(
        Some(&agent_run_context),
        "field_value=24.0\nstatus=ok",
    );

    assert_eq!(path, "prompts/observed_answer_fallback_compact_prompt.md");
}

#[test]
fn observed_answer_language_compatibility_rejects_clear_request_language_mismatch() {
    assert!(!observed_answer_language_compatible(
        "当前工作目录是 /home/guagua/rustclaw；进程 clawd 正在监听 8787。",
        "en",
    ));
    assert!(observed_answer_language_compatible(
        "The working directory is /home/guagua/rustclaw; process clawd is listening on 8787.",
        "en",
    ));
    assert!(observed_answer_language_compatible(
        "/home/guagua/rustclaw",
        "en",
    ));
    assert!(observed_answer_language_compatible(
        "model_io.log 122500780\nmodel_io.log.2026-07-07 110353651\nmodel_io.log.2026-07-02 70101903",
        "zh-CN",
    ));
}

#[test]
fn observed_answer_language_compatibility_accepts_structured_json_machine_output() {
    let answer = r#"{"changed_files":["calc_core.py","test_calc_core.py"],"test_command":"python -m unittest test_calc_core.py","test_status":"OK","functions":["add","sub","mul"]}"#;

    assert!(observed_answer_language_compatible(answer, "zh-CN"));
    assert!(observed_answer_language_compatible(answer, "en"));
}

#[test]
fn observed_answer_language_compatibility_accepts_grounded_strict_path_list_machine_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","counts":{"files":4,"total":4},"entries":[{"kind":"file","name":"x_abcd_log.txt","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt"},{"kind":"file","name":"zz_abcd_backup.log","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"},{"kind":"file","name":"abcd_report.md","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md"},{"kind":"file","name":"my_abcd.txt","path":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt"}],"path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"}"#,
    ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/fuzzy_top3".to_string();
    route.output_contract.self_extension.list_selector.limit = Some(3);
    let answer = "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md";

    assert!(observed_answer_language_compatible_for_route(
        Some(&route),
        &loop_state,
        None,
        answer,
        "zh-CN"
    ));
}

#[test]
fn observed_fallback_prompt_keeps_full_template_for_complex_or_large_contracts() {
    let mut content_route = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
    content_route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    content_route.output_contract.requires_content_evidence = true;
    let content_context = AgentRunContext {
        route_result: Some(content_route),
        ..AgentRunContext::default()
    };
    assert_eq!(
        observed_answer_fallback_prompt_logical_path(Some(&content_context), "excerpt=..."),
        "prompts/observed_answer_fallback_prompt.md"
    );

    let mut delivery_route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    delivery_route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    delivery_route.output_contract.delivery_required = true;
    let delivery_context = AgentRunContext {
        route_result: Some(delivery_route),
        ..AgentRunContext::default()
    };
    assert_eq!(
        observed_answer_fallback_prompt_logical_path(Some(&delivery_context), "status=ok"),
        "prompts/observed_answer_fallback_prompt.md"
    );

    let mut terminal_route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    terminal_route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let terminal_context = AgentRunContext {
        route_result: Some(terminal_route),
        ..AgentRunContext::default()
    };
    let large_observed = "x".repeat(12_001);
    assert_eq!(
        observed_answer_fallback_prompt_logical_path(Some(&terminal_context), &large_observed),
        "prompts/observed_answer_fallback_prompt.md"
    );
}

#[test]
fn content_excerpt_summary_is_not_hard_summarized_by_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","excerpt":"12|# timeout note\n13|task_timeout_seconds = 3600\n14|# end"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "读取 /tmp/config.toml 最后 3 行，然后用一句话总结".to_string(),
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
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "/tmp/config.toml".to_string(),
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

#[tokio::test]
async fn observed_fallback_keeps_strict_raw_tail_read_before_composer() {
    let state = AppState::test_default_with_fixture_provider();
    let task = claimed_task("task-observed-strict-raw-tail");
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":2,"excerpt":"98|WARN provider failed: http 401: credential_missing\n99|WARN memory preference fallback failed: http 401","path":"/tmp/clawd-dev.log"}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.ask_mode = crate::AskMode::act_plain();
    route_result.resolved_intent = "Read the last two lines of the selected log file.".to_string();
    route_result.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route_result.output_contract.response_shape = OutputResponseShape::Strict;
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.delivery_required = false;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "/tmp/clawd-dev.log".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let (answer, summary) = try_synthesize_answer_from_observed_output(
        &state,
        &task,
        "read tail lines",
        &loop_state,
        Some(&agent_run_context),
    )
    .await
    .expect("observed fallback should not fail")
    .expect("strict raw tail read should direct-return observed output");

    assert_eq!(
        answer,
        "WARN provider failed: http 401: credential_missing\nWARN memory preference fallback failed: http 401"
    );
    assert!(matches!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    ));
    assert!(summary.contract_ok);
    assert_eq!(summary.completion_ok, Some(true));
    assert_eq!(summary.grounded_ok, Some(true));
}

#[test]
fn content_excerpt_with_summary_composes_observed_slice_and_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"range","start_line":6,"end_line":8,"excerpt":"6|{\"status\":\"ok\",\"prompt_source\":\"clarify\"}\n7|{\"status\":\"ok\",\"prompt_source\":\"dynamic_guard\"}\n8|{\"status\":\"ok\",\"prompt_source\":\"context\"}"}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route_result.output_contract.response_shape = OutputResponseShape::Strict;
    route_result.output_contract.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = super::compose_content_excerpt_with_summary_answer(
        "All observed records are ok.",
        &loop_state,
        true,
        Some(&agent_run_context),
    );

    assert!(answer.contains(r#""prompt_source":"clarify""#));
    assert!(answer.contains(r#""prompt_source":"dynamic_guard""#));
    assert!(answer.contains(r#""prompt_source":"context""#));
    assert!(answer.contains("All observed records are ok."));
}

#[test]
fn content_excerpt_with_summary_does_not_prepend_log_excerpt() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":5,"path":"logs/clawd.run.log","resolved_path":"/workspace/logs/clawd.run.log","excerpt":"1700|2026-05-27T08:04:44Z INFO task_call\n1701|2026-05-27T08:04:45Z INFO task_journal_summary {\"kind\":\"ask\"}\n1702|2026-05-27T08:04:46Z WARN memory_intent"}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route_result.output_contract.response_shape = OutputResponseShape::Strict;
    route_result.output_contract.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/workspace/logs/clawd.run.log".to_string()),
        ..AgentRunContext::default()
    };

    let answer = super::compose_content_excerpt_with_summary_answer(
        "没有 ERROR 行",
        &loop_state,
        false,
        Some(&agent_run_context),
    );

    assert_eq!(answer, "没有 ERROR 行");
}

#[test]
fn content_excerpt_with_summary_strips_log_excerpt_prefix() {
    let mut loop_state = LoopState::new(2);
    let excerpt = "2026-05-27T08:04:44Z INFO task_call\n2026-05-27T08:04:45Z WARN memory_intent";
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        &format!(
            r#"{{"action":"read_range","mode":"tail","requested_n":2,"path":"logs/clawd.run.log","resolved_path":"/workspace/logs/clawd.run.log","excerpt":"1|{}"}}"#,
            excerpt.replace('\n', r"\n2|")
        ),
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route_result.output_contract.response_shape = OutputResponseShape::Strict;
    route_result.output_contract.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/workspace/logs/clawd.run.log".to_string()),
        ..AgentRunContext::default()
    };

    let answer = super::compose_content_excerpt_with_summary_answer(
        &format!("{excerpt}\n\n最后 2 行中没有 ERROR 行。"),
        &loop_state,
        false,
        Some(&agent_run_context),
    );

    assert_eq!(answer, "最后 2 行中没有 ERROR 行。");
}

#[test]
fn content_excerpt_with_summary_prefers_auto_locator_slice_over_latest_read() {
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"head","requested_n":3,"resolved_path":"/tmp/service_notes.md","excerpt":"1|# Service Notes\n2|Runtime status lives here.\n3|Use this for service checks."}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","mode":"head","requested_n":3,"resolved_path":"/tmp/README.md","excerpt":"1|# Device Local Fixture\n2|This repository contains the sample project.\n3|It is used for filesystem tests."}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route_result.output_contract.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/service_notes.md".to_string()),
        ..AgentRunContext::default()
    };

    let answer = super::compose_content_excerpt_with_summary_answer(
        "README.md describes the sample project.",
        &loop_state,
        true,
        Some(&agent_run_context),
    );

    assert!(answer.starts_with("# Service Notes"), "answer: {answer}");
    assert!(answer.contains("README.md describes the sample project."));
    assert!(
        !answer.starts_with("# Device Local Fixture"),
        "answer: {answer}"
    );
}

#[test]
fn direct_answer_keeps_fallback_for_unstructured_content_excerpt_summary() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "read_file",
        "RustClaw is deployed locally and keeps task state in sqlite.",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "看一下 /tmp/README.txt，然后用一句话总结".to_string(),
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
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "/tmp/README.txt".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.txt".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_summarizes_doc_parse_content_excerpt_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "doc_parse",
            r##"{"text":"# RustClaw\n\n<img src=\"./RustClaw.png\" width=\"420\" />\n\nRustClaw is a local Rust agent runtime centered on clawd and designed for multi-channel task execution.\n\n## Overview\nMore text."}"##,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "Read README.md and summarize it in one line.".to_string(),
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
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "RustClaw is a local Rust agent runtime centered on clawd and designed for multi-channel task execution."
            )
        );
}

#[test]
fn direct_doc_parse_summary_defers_when_language_conflicts_with_request() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "doc_parse",
            r##"{"text":"# RustClaw\n\nRustClaw is a local Rust agent runtime centered on clawd and designed for multi-channel task execution."}"##,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "读取 README.md 并用一句中文总结".to_string(),
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
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
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
fn direct_answer_passthroughs_contract_filename_read_range_excerpt_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|<img src=\"./RustClaw.png\" width=\"420\" />\n4|"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "先读一下 README.md 前 4 行".to_string(),
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
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.md".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("# RustClaw\n\n<img src=\"./RustClaw.png\" width=\"420\" />\n")
    );
}

#[test]
fn direct_answer_preserves_blank_lines_for_explicit_read_range() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","mode":"range","start_line":1,"end_line":4,"path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|<img src=\"./RustClaw.png\" width=\"420\" />\n4|"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "Show exactly the first 4 raw lines of README.md.".to_string(),
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
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.md".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("# RustClaw\n\n<img src=\"./RustClaw.png\" width=\"420\" />\n")
    );
}

#[test]
fn raw_command_output_read_range_direct_answer_preserves_visible_blank_line() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","mode":"head","requested_n":2,"path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "读取 README.md 前 2 行".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "semantic_contract_requires_evidence".to_string(),
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
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.md".to_string()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("# RustClaw\n")
    );
}

#[test]
fn direct_answer_sanitizes_read_range_log_excerpt_without_llm() {
    let mut loop_state = LoopState::new(2);
    let skill_output = serde_json::json!({
            "action": "read_range",
            "path": "/tmp/feishud.log",
            "resolved_path": "/tmp/feishud.log",
            "excerpt": "1|\u{1b}[32mconnected\u{1b}[0m to wss://host/ws?device_id=123&access_key=abc123&service_id=7&ticket=deadbeef"
        })
        .to_string();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "system_basic", &skill_output));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "看日志最后 1 行".to_string(),
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
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "feishud.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/feishud.log".to_string()),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("read_range direct answer");

    assert!(answer.contains("access_key=[REDACTED]"));
    assert!(answer.contains("ticket=[REDACTED]"));
    assert!(!answer.contains('\u{1b}'));
    assert!(!answer.contains("abc123"));
    assert!(!answer.contains("deadbeef"));
}

#[test]
fn scalar_route_fs_basic_tail_read_range_prefers_structured_excerpt() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "older output mentioning scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
    ));
    let skill_output = serde_json::json!({
            "action": "read_range",
            "path": "/home/guagua/rustclaw/logs/clawd.log",
            "resolved_path": "/home/guagua/rustclaw/logs/clawd.log",
            "mode": "tail",
            "requested_n": 2,
            "excerpt": "1858|2026-05-13T18:29:58Z finalize_ok\n1859|2026-05-13T18:29:59Z prior task mentioned release_checklist.md"
        })
        .to_string();
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "fs_basic", &skill_output));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "查看 logs 目录下第二个文件（clawd.log）的最后2行内容".to_string(),
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
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    assert!(scalar_route_prefers_structured_observed_answer(
        &route_result,
        &loop_state
    ));
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("fs_basic read_range direct answer");

    assert!(answer.contains("finalize_ok"));
    assert!(answer.contains("release_checklist.md"));
    assert!(!answer.contains(r#""action":"read_range""#));
    assert!(!answer.contains("older output mentioning"));
}

#[test]
fn direct_answer_passthroughs_chat_wrapped_execution_path_read_range_when_no_transform_is_requested(
) {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","excerpt":"1|[app]\n2|name = \"fixture\"\n3|mode = \"test\""}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "用户提供了文件路径 /tmp/config.toml，但未说明要对该文件执行什么操作"
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
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "/tmp/config.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/config.toml".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("[app]\nname = \"fixture\"\nmode = \"test\"")
    );
}

#[test]
fn direct_answer_does_not_passthrough_read_range_when_summary_is_requested() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|A tool runtime\n4|"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "先读一下 README.md 前 4 行，再用三句话总结".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_filename_read_range".to_string(),
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
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.md".to_string()),
        ..AgentRunContext::default()
    };
    assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none(),
            "summary-style read_range requests should fall back to synthesis instead of raw passthrough"
        );
}

#[test]
fn direct_answer_defers_read_range_passthrough_when_language_conflicts() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","path":"/tmp/service_notes.md","resolved_path":"/tmp/service_notes.md","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes."}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "service_notes.md 를 읽고 핵심만 요약해.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_filename_read_range".to_string(),
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
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "service_notes.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/service_notes.md".to_string()),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none(),
        "language-conflicting read_range evidence should be synthesized instead of raw passthrough"
    );
}

#[test]
fn direct_answer_does_not_passthrough_read_range_for_existence_with_path_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/rustclaw.service","resolved_path":"/tmp/rustclaw.service","excerpt":"1|[Unit]\n2|Description=RustClaw Service\n3|[Service]\n4|ExecStart=/bin/bash start-all-bin.sh"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "检查 rustclaw.service 是否存在，若存在返回路径并解释用途".to_string(),
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
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/rustclaw.service".to_string()),
        ..AgentRunContext::default()
    };

    assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none(),
            "existence/path contracts with read_range evidence need synthesis, not raw file passthrough"
        );
}

#[test]
fn direct_answer_prefers_current_turn_excerpt_summary_request_over_resolved_intent_drift() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|A tool runtime\n4|"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "先读一下 README.md 前 4 行".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_filename_read_range".to_string(),
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
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        user_request: Some("先读一下 README.md 前 4 行，再用三句话总结".to_string()),
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.md".to_string()),
        ..AgentRunContext::default()
    };
    assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none(),
            "current-turn summary/read-range request should still block raw passthrough even if resolved_intent drifted"
        );
}
