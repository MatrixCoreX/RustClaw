#[test]
fn direct_scalar_prefers_unique_exact_fs_search_match_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"README.md","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("README.md")
    );
}
#[test]
fn direct_scalar_uses_locator_hint_when_fs_search_output_omits_pattern() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output_with_locator_hint(
            &loop_state,
            Some("README.md"),
            None,
            false,
        )
        .as_deref(),
        Some("README.md")
    );
}

#[test]
fn direct_scalar_does_not_collapse_ambiguous_fs_search_to_count() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"README","count":2,"results":["README.md","README.txt"],"root":""}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None),
        None
    );
}

#[test]
fn direct_scalar_prefers_locator_extension_when_fs_search_pattern_is_broad() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"execution_intent","count":2,"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output_with_locator_hint(
            &loop_state,
            Some("plan/extra_missing_repair_probe.md"),
            None,
            false,
        )
        .as_deref(),
        Some("plan/execution_intent_routing_repair_plan_20260509.md")
    );
}

#[test]
fn fs_search_file_paths_contract_filters_with_structured_pattern() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"execution_intent","count":8,"results":["crates/clawd/src/agent_engine/planning.rs","docs/planning_deterministic_guardrails_audit.md","plan/agent_intelligence_architecture_plan_20260511_已完成.md","plan/builtin_skill_capability_governance_plan_20260510.md","plan/codex_style_agent_architecture_refactor_plan_20260511.md","plan/execution_intent_routing_repair_plan_20260509_已完成.md","plan/llm_first_agent_convergence_plan_20260511.md","prompts/layers/overlays/plan_repair_prompt.md"],"root":""}"#,
        ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.selection.structured_field_selector = Some("path".to_string());
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/home/guagua/rustclaw/plan".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("plan/execution_intent_routing_repair_plan_20260509_已完成.md")
    );
}

#[test]
fn fs_search_file_paths_contract_uses_planner_semantic_kind() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"execution_intent","count":8,"results":["crates/clawd/src/agent_engine/planning.rs","docs/planning_deterministic_guardrails_audit.md","plan/execution_intent_routing_repair_plan_20260509_已完成.md","prompts/layers/overlays/plan_repair_prompt.md"],"root":""}"#,
        ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.selection.structured_field_selector = Some("path".to_string());
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/home/guagua/rustclaw/plan".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("plan/execution_intent_routing_repair_plan_20260509_已完成.md")
    );
}

#[test]
fn fs_search_file_paths_contract_preserves_multi_candidates_when_not_decisive() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"find_name","pattern":"README","count":5,"results":["README.md","README.zh-CN.md","UI/README.md","data/vendor/whisper.cpp/examples/whisper.android.java/README_files","data/vendor/whisper.cpp/README.md"],"root":""}"#,
        ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.selection.structured_field_selector = Some("path".to_string());
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = "/home/guagua/rustclaw".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("multi-candidate search should produce a direct candidate list");

    assert!(answer.contains("README.md"));
    assert!(answer.contains("README.zh-CN.md"));
    assert!(
        answer.contains('\n'),
        "answer should not collapse to one path: {answer}"
    );
    assert_ne!(
        answer.trim(),
        "data/vendor/whisper.cpp/examples/whisper.android.java/README_files"
    );
}

#[test]
fn fs_search_file_paths_contract_i18n_expands_to_five_full_paths() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"find_name","count":6,"results":["README.md","README.zh-CN.md","README_cn.md","RUSTCLAW_SERVICE_README.md","UI/README.md","Cargo.toml"],"root":""}"#,
    ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.selection.structured_field_selector = Some("path".to_string());
    route.selection.list_selector.limit = Some(5);
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = String::new();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let state = AppState::test_default_with_fixture_provider();
    let answer = extract_direct_answer_from_generic_output_i18n(
        &loop_state,
        &state,
        Some(&agent_run_context),
    )
    .expect("exact path selector should produce a full path list");
    let lines = answer.lines().collect::<Vec<_>>();
    let root = state.skill_rt.workspace_root.display().to_string();

    assert_eq!(lines.len(), 5, "answer: {answer}");
    assert!(lines.iter().all(|line| line.starts_with(&root)));
    assert!(answer.contains("/README.md"));
    assert!(answer.contains("/UI/README.md"));
    assert!(!answer.contains("Cargo.toml"));
}

#[test]
fn direct_scalar_count_uses_latest_fs_search_count() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":107,"files":101,"dirs":6},"path":"scripts/nl_tests/cases"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "fs_search",
            r#"{"action":"find_name","count":10,"patterns":["clarify"],"results":["a.txt","b.txt"],"root":"scripts/nl_tests/cases"}"#,
        ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.semantic_kind = OutputSemanticKind::ScalarCount;

    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("10")
    );
}

#[test]
fn fs_search_find_ext_direct_answer_returns_paths_list() {
    let value = serde_json::json!({
        "action": "find_ext",
        "ext": "toml",
        "count": 3,
        "results": ["Cargo.toml", "configs/config.toml", "configs/git_basic.toml"]
    });
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, true, false).as_deref(),
        Some("Cargo.toml\nconfigs/config.toml\nconfigs/git_basic.toml")
    );
}

#[test]
fn fs_search_grep_text_direct_answer_returns_unique_matching_paths() {
    let value = serde_json::json!({
        "action": "grep_text",
        "query": "FirstLayerDecision",
        "count": 1,
        "match_count": 2,
        "matches": [
            {"path": "README.md", "line": 45, "text": "FirstLayerDecision"},
            {"path": "README.md", "line": 95, "text": "FirstLayerDecision"}
        ]
    });

    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, false, false)
            .as_deref(),
        Some("README.md")
    );
}

#[test]
fn fs_search_grep_text_direct_answer_preserves_path_answer_when_requested() {
    let value = serde_json::json!({
        "action": "grep_text",
        "query": "FirstLayerDecision",
        "count": 4,
        "match_count": 5,
        "matches": [
            {"path": "README.md", "line": 45, "text": "FirstLayerDecision"},
            {"path": "README.md", "line": 95, "text": "FirstLayerDecision"},
            {"path": "crates/clawd/src/ask_flow.rs", "line": 10, "text": "FirstLayerDecision"},
            {"path": "crates/clawd/src/intent_router.rs", "line": 20, "text": "FirstLayerDecision"},
            {"path": "crates/clawd/src/main.rs", "line": 30, "text": "FirstLayerDecision"}
        ]
    });

    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, true, true).as_deref(),
        Some("README.md\ncrates/clawd/src/ask_flow.rs\ncrates/clawd/src/intent_router.rs")
    );
}

#[test]
fn fs_search_grep_text_direct_answer_returns_matching_lines_when_listing_allowed() {
    let value = serde_json::json!({
        "action": "grep_text",
        "query": "ERROR",
        "count": 1,
        "match_count": 1,
        "matches": [
            {
                "path": "logs/app.log",
                "line": 16,
                "text": "2026-04-01 10:08:44 ERROR provider timeout while fetching external metadata"
            }
        ]
    });

    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, true, false).as_deref(),
        Some("16: 2026-04-01 10:08:44 ERROR provider timeout while fetching external metadata")
    );
}

#[test]
fn raw_command_output_grep_text_direct_answer_returns_matching_lines() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"grep_text","query":"ERROR","count":1,"match_count":1,"matches":[{"path":"scripts/nl_tests/fixtures/device_local/logs/app.log","line":16,"text":"2026-04-01 10:08:44 ERROR provider timeout while fetching external metadata"}],"results":["scripts/nl_tests/fixtures/device_local/logs/app.log"]}"#,
    ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint =
        "scripts/nl_tests/fixtures/device_local/logs/app.log".to_string();
    route.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("16: 2026-04-01 10:08:44 ERROR provider timeout while fetching external metadata")
    );
}

#[test]
fn fs_search_grep_text_direct_answer_uses_name_matches_when_content_empty() {
    let value = serde_json::json!({
        "action": "grep_text",
        "query": "abcd",
        "count": 0,
        "match_count": 0,
        "matches": [],
        "name_count": 4,
        "name_results": [
            "abcd_report.md",
            "my_abcd.txt",
            "x_abcd_log.txt",
            "zz_abcd_backup.log"
        ]
    });

    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, true, false).as_deref(),
        Some("abcd_report.md\nmy_abcd.txt\nx_abcd_log.txt\nzz_abcd_backup.log")
    );
}

#[test]
fn virtual_fs_basic_grep_text_output_can_direct_answer_file_paths() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"grep_text","query":"FirstLayerDecision","count":4,"match_count":5,"matches":[{"path":"README.md","line":45,"text":"FirstLayerDecision"},{"path":"README.md","line":95,"text":"FirstLayerDecision"},{"path":"crates/clawd/src/ask_flow.rs","line":10,"text":"FirstLayerDecision"},{"path":"crates/clawd/src/intent_router.rs","line":20,"text":"FirstLayerDecision"},{"path":"crates/clawd/src/main.rs","line":30,"text":"FirstLayerDecision"}]}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.selection.structured_field_selector = Some("path".to_string());
    route_result.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("README.md\ncrates/clawd/src/ask_flow.rs\ncrates/clawd/src/intent_router.rs")
    );
}

#[test]
fn fs_search_grep_text_observed_body_keeps_line_evidence() {
    let body = r#"{"action":"grep_text","query":"run_cmd","patterns":["prompt_utils.rs"],"count":1,"match_count":2,"matches":[{"path":"crates/clawd/src/prompt_utils.rs","line":1275,"text":"if step_type == \"run_cmd\" {"},{"path":"crates/clawd/src/prompt_utils.rs","line":1276,"text":"return normalize_run_cmd_call(obj, obj.get(\"args\").and_then(|v| v.as_object()));"}]}"#;
    let observed = super::structured_observed_body("fs_search", body)
        .expect("grep_text should compact observed evidence");

    assert!(observed.contains("grep_text query=run_cmd"));
    assert!(observed.contains("file_patterns=prompt_utils.rs"));
    assert!(observed.contains("match path=crates/clawd/src/prompt_utils.rs line=1275"));
    assert!(observed.contains("step_type == \"run_cmd\""));
}

#[test]
fn fs_search_grep_text_observed_body_keeps_name_match_fallback() {
    let body = r#"{"action":"grep_text","query":"abcd","count":0,"match_count":0,"matches":[],"name_count":1,"name_results":["my_abcd.txt"]}"#;
    let observed = super::structured_observed_body("fs_search", body)
        .expect("grep_text should compact name fallback evidence");

    assert!(observed.contains("grep_text query=abcd"));
    assert!(observed.contains("name_count=1"));
    assert!(observed.contains("name_match path=my_abcd.txt"));
    assert!(observed.contains("matches: none"));
}

#[test]
fn fs_search_find_ext_unclassified_contract_keeps_observed_file_paths() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_ext","ext":"sh","count":4,"results":["system_report.sh","scripts/run.sh","scripts/dev/check.sh","component_start/start-clawd.sh"],"root":""}"#,
        ));
    let route_result = IntentOutputContract {
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::None,
            ..IntentOutputContract::default()
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        auto_locator_path: Some("/home/guagua/rustclaw".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("system_report.sh\nscripts/run.sh\nscripts/dev/check.sh\ncomponent_start/start-clawd.sh")
    );
}

#[test]
fn virtual_fs_basic_find_ext_unclassified_contract_keeps_observed_file_paths() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"find_ext","ext":"sh","count":4,"results":["system_report.sh","scripts/run.sh","scripts/dev/check.sh","component_start/start-clawd.sh"],"root":""}"#,
        ));
    let route_result = IntentOutputContract {
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::None,
            ..IntentOutputContract::default()
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        auto_locator_path: Some("/home/guagua/rustclaw".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("system_report.sh\nscripts/run.sh\nscripts/dev/check.sh\ncomponent_start/start-clawd.sh")
    );
}

#[test]
fn multi_status_json_direct_answer_keeps_all_observed_status_files() {
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"1|{\"kind\":\"telegram\",\"name\":\"primary\",\"scope\":\"telegram:primary\",\"healthy\":true,\"status\":\"running\",\"last_error\":null}","path":"/home/guagua/rustclaw/run/gateway-instance-status/telegram__primary.json","resolved_path":"/home/guagua/rustclaw/run/gateway-instance-status/telegram__primary.json"},"text":"{\"action\":\"read_range\",\"excerpt\":\"1|{\\\"kind\\\":\\\"telegram\\\",\\\"name\\\":\\\"primary\\\",\\\"scope\\\":\\\"telegram:primary\\\",\\\"healthy\\\":true,\\\"status\\\":\\\"running\\\",\\\"last_error\\\":null}\",\"path\":\"/home/guagua/rustclaw/run/gateway-instance-status/telegram__primary.json\",\"resolved_path\":\"/home/guagua/rustclaw/run/gateway-instance-status/telegram__primary.json\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"1|{\"name\":\"primary\",\"healthy\":true,\"status\":\"running\",\"last_error\":null}","path":"/home/guagua/rustclaw/run/telegram-bot-status/primary.json","resolved_path":"/home/guagua/rustclaw/run/telegram-bot-status/primary.json"},"text":"{\"action\":\"read_range\",\"excerpt\":\"1|{\\\"name\\\":\\\"primary\\\",\\\"healthy\\\":true,\\\"status\\\":\\\"running\\\",\\\"last_error\\\":null}\",\"path\":\"/home/guagua/rustclaw/run/telegram-bot-status/primary.json\",\"resolved_path\":\"/home/guagua/rustclaw/run/telegram-bot-status/primary.json\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"1|{\n2|  \"healthy\": true,\n3|  \"status\": \"login_required\",\n4|  \"last_error\": null,\n5|  \"account_label\": \"primary\"\n6|}","path":"/home/guagua/rustclaw/run/wechatd-status/primary.json","resolved_path":"/home/guagua/rustclaw/run/wechatd-status/primary.json"},"text":"{\"action\":\"read_range\",\"excerpt\":\"1|{\\n2|  \\\"healthy\\\": true,\\n3|  \\\"status\\\": \\\"login_required\\\",\\n4|  \\\"last_error\\\": null,\\n5|  \\\"account_label\\\": \\\"primary\\\"\\n6|}\",\"path\":\"/home/guagua/rustclaw/run/wechatd-status/primary.json\",\"resolved_path\":\"/home/guagua/rustclaw/run/wechatd-status/primary.json\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "synthesize_answer",
        r#"{"healthy":true,"status":"login_required","account_label":"primary"}"#,
    ));
    let route_result = chat_wrapped_unclassified_route(OutputResponseShape::Free);
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        auto_locator_path: Some("/home/guagua/rustclaw/run".to_string()),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("multi status observations should produce a combined direct answer");

    assert!(answer.contains("status_files.count=3"), "{answer}");
    assert!(
        answer.contains("gateway-instance-status/telegram__primary.json"),
        "{answer}"
    );
    assert!(
        answer.contains("telegram-bot-status/primary.json"),
        "{answer}"
    );
    assert!(answer.contains("wechatd-status/primary.json"), "{answer}");
    assert!(answer.contains("status=running"), "{answer}");
    assert!(answer.contains("status=login_required"), "{answer}");
    assert!(
        answer.contains("status_files.notable.status=login_required"),
        "{answer}"
    );
}

#[test]
fn fs_search_direct_answer_does_not_confirm_ambiguous_matches_when_direct_list_disallowed() {
    let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","pattern":"abcd","count":4,"results":["abcd_report.md","my_abcd.txt","x_abcd_log.txt","zz_abcd_backup.log"],"root":""}"#,
        )
        .expect("json");
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, false, false)
            .as_deref(),
        None
    );
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, true, false).as_deref(),
        Some("abcd_report.md\nmy_abcd.txt\nx_abcd_log.txt")
    );
}

#[test]
fn fs_search_direct_answer_prefers_exact_match_before_confirmation() {
    let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","pattern":"README.md","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        )
        .expect("json");
    let answer = super::fs_search_direct_answer_candidate(None, &value, None, false, false, false)
        .expect("exact match path fact");
    assert!(answer.contains("message_key=clawd.msg.path_fact.observed"));
    assert!(answer.contains("reason_code=path_fact_observed"));
    assert!(answer.contains("exists=true"));
    assert!(answer.contains("path=README.md"));
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, false, true).as_deref(),
        Some("README.md")
    );
}

#[test]
fn direct_answer_for_strict_file_names_fs_search_uses_plain_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","count":1,"results":["scripts/nl_tests/fixtures/locator_smart/stem_unique/ABCD.txt"],"root":"scripts/nl_tests/fixtures/locator_smart/stem_unique"}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "scripts/nl_tests/fixtures/locator_smart/stem_unique".to_string(),
            selection: crate::OutputSelectionContract {
                list_selector: crate::pipeline_types::OutputListSelector {
                    target_kind: crate::OutputScalarCountTargetKind::File,
                    target_kind_specified: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("scripts/nl_tests/fixtures/locator_smart/stem_unique/ABCD.txt")
    );
}

#[test]
fn fs_search_direct_answer_uses_locator_hint_for_ambiguous_list_when_allowed() {
    let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","count":4,"results":["scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt"],"root":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"}"#,
        )
        .expect("json");
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, Some("abcd"), false, false, false)
            .as_deref(),
        None
    );
    assert_eq!(
            super::fs_search_direct_answer_candidate(
                None,
                &value,
                Some("abcd"),
                false,
                true,
                false
            )
            .as_deref(),
            Some(
                "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt"
            )
        );
}

#[test]
fn fs_search_path_only_direct_answer_keeps_ambiguous_ranked_list_when_allowed() {
    let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","count":4,"results":["scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt"],"root":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"}"#,
        )
        .expect("json");
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, Some("abcd"), false, true, true)
            .as_deref(),
        Some(
            "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt"
        )
    );
}

#[test]
fn observed_entries_keep_latest_listing_plus_recent_non_listing_steps() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "a.md\nb.md\nc.md\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "read_file", "# A\nalpha\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "read_file", "# B\nbeta\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_4", "read_file", "# C\ngamma\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_5", "read_file", "# D\ndelta\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_6", "read_file", "# E\nepsilon\n"));

    let entries = observed_output_entries(&loop_state);
    assert_eq!(entries.len(), 5);
    assert!(entries
        .iter()
        .any(|entry| entry.contains("step_1 skill(list_dir)")));
    assert!(entries
        .iter()
        .any(|entry| entry.contains("step_6 skill(read_file)")));
    assert!(!entries
        .iter()
        .any(|entry| entry.contains("step_2 skill(read_file)")));
}

#[test]
fn normalized_listing_trims_blank_lines() {
    assert_eq!(
        normalized_observed_listing("\nfoo\n\nbar\n").as_deref(),
        Some("foo\nbar")
    );
}

#[test]
fn observed_entries_use_read_range_excerpt_body_instead_of_raw_json() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|Hello"}"#,
    ));
    let entries = observed_output_entries(&loop_state);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("read_range path=/tmp/README.md"));
    assert!(entries[0].contains("# RustClaw"));
    assert!(entries[0].contains("# RustClaw\n\nHello"));
    assert!(entries[0].contains("Hello"));
    assert!(!entries[0].contains(r#""action":"read_range""#));
}

#[test]
fn observed_entries_keep_read_range_runtime_log_excerpt_for_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","path":"/tmp/clawd.run.log","excerpt":"10|2026-06-19T12:39:31Z INFO task_call: verifier_result task_id=abc\n11|2026-06-19T12:39:32Z INFO task_call: executor_step_start step=read_range"}"#,
    ));

    let entries = observed_output_entries(&loop_state);

    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("read_range path=/tmp/clawd.run.log"));
    assert!(entries[0].contains("task_call: verifier_result"));
    assert!(entries[0].contains("executor_step_start"));
}

#[test]
fn observed_entries_preserve_full_find_name_results_for_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"find_name","pattern":"clawd","count":10,"root":"logs","results":["logs/clawd-codex-current.log","logs/clawd-dev.log","logs/clawd-runtime.log","logs/clawd.codex.minimax.log","logs/clawd.codex.nltest.log","logs/clawd.log","logs/clawd.nl-focus.log","logs/clawd.nl_missing_ja_20260526_221223.log","logs/clawd.nl_missing_ja_20260526_221700.log","logs/clawd.run.log"]}"#,
    ));

    let entries = observed_output_entries(&loop_state);

    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("find_name count=10"));
    assert!(entries[0].contains("root=logs"));
    assert!(entries[0].contains("pattern=clawd"));
    assert!(entries[0].contains("result.10.path=logs/clawd.run.log"));
}

#[test]
fn observed_contract_json_includes_final_answer_shape_and_locator_hint() {
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "README.md".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    let contract = observed_contract_json(Some(&agent_run_context));
    let parsed: serde_json::Value =
        serde_json::from_str(&contract).expect("observed contract json");
    assert!(parsed.get("route_gate_kind").is_none());
    assert!(parsed.get("ask_mode").is_none());
    assert!(parsed.get("derived_route_label").is_none());
    assert!(parsed.get("contract_marker").is_none());
    assert_eq!(
        parsed
            .get("final_answer_shape")
            .and_then(serde_json::Value::as_str),
        Some("summary_with_evidence")
    );
    assert_eq!(
        parsed
            .get("final_answer_shape_class")
            .and_then(serde_json::Value::as_str),
        Some("grounded_summary")
    );
    assert!(contract.contains(r#""locator_hint":"README.md""#));
}

#[test]
fn observed_request_language_hint_follows_current_user_text() {
    assert_eq!(
        observed_request_language_hint("读一下 README 开头，三句话总结"),
        "zh-CN"
    );
    assert_eq!(
        observed_request_language_hint("Summarize the README in one sentence."),
        "en"
    );
    assert_eq!(observed_request_language_hint("只输出路径"), "zh-CN");
    assert_eq!(observed_request_language_hint("12345"), "config_default");
}

#[test]
fn observed_bilingual_templates_defer_non_bilingual_missing_field_answers() {
    assert!(!observed_language_supports_bilingual_template("ja"));
    assert!(observed_request_prefers_english_template(None, "ja"));
    let missing = serde_json::json!({
        "action": "extract_field",
        "field_path": "package.no_such_key_100_matrix",
        "exists": false,
    });

    assert_eq!(
        extract_field_direct_answer_candidate(
            None,
            &missing,
            Some(OutputResponseShape::OneSentence),
            false,
            true,
        )
        .as_deref(),
        Some("message_key=clawd.msg.extract_field_missing\nreason_code=extract_field_missing\nfinal_answer_shape=missing_structured_field\nexists=false\nfield_path=package.no_such_key_100_matrix")
    );
    assert!(extract_field_direct_answer_candidate(
        None,
        &missing,
        Some(OutputResponseShape::OneSentence),
        true,
        false,
    )
    .is_none());
}

#[test]
fn observed_direct_answer_defers_non_bilingual_existence_with_path_template() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"kind":"missing","path":"/tmp/rustclaw-missing-ja.txt"}],"include_missing":true}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "/tmp/rustclaw-missing-ja.txt".to_string();
    let agent_run_context = AgentRunContext {
            original_user_request: Some(
                "/tmp/rustclaw-missing-ja.txt が存在するか確認してください。存在しない場合は日本語で短く答えてください。"
                    .to_string(),
            ),
            output_contract: Some(route_result.clone()),
            ..AgentRunContext::default()
        };

    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
    assert!(extract_direct_answer_from_generic_output_i18n(
        &loop_state,
        &AppState::test_default_with_fixture_provider(),
        Some(&agent_run_context)
    )
    .is_none());
    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
    assert!(extract_direct_scalar_from_generic_output_i18n(
        &loop_state,
        &AppState::test_default_with_fixture_provider(),
        Some(&agent_run_context)
    )
    .is_none());
}

#[test]
fn observed_response_style_hint_reflects_output_contract_shape() {
    let mut route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "README.md".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    let mut agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("style_policy=evidence_synthesis"));
    assert!(
        observed_response_style_hint(Some(&agent_run_context))
            .contains("response_shape=one_sentence")
    );
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("include_all_deliverables=true"));

    route_result.exact_sentence_count = Some(3);
    agent_run_context.output_contract = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("sentence_count=3"));
    route_result.exact_sentence_count = None;

    route_result.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route_result.response_shape = OutputResponseShape::Strict;
    route_result.exact_sentence_count = Some(1);
    agent_run_context.output_contract = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("style_policy=exact_observed_value"));
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("requested_format=preserve"));
    route_result.exact_sentence_count = None;
    route_result.semantic_kind = OutputSemanticKind::None;

    route_result.response_shape = OutputResponseShape::Scalar;
    route_result.selection.structured_field_selector = Some("value".to_string());
    agent_run_context.output_contract = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("style_policy=scalar"));
    assert!(observed_response_style_hint(Some(&agent_run_context)).contains("bare_value=true"));

    route_result.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route_result.selection.structured_field_selector = None;
    agent_run_context.output_contract = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("style_policy=existence_with_path"));
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("scalar_override=path_required"));

    route_result.semantic_kind = OutputSemanticKind::ScalarCount;
    route_result.response_shape = OutputResponseShape::OneSentence;
    agent_run_context.output_contract = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("style_policy=scalar_count"));
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("aggregate_only=explicit_request_only"));

    route_result.semantic_kind = OutputSemanticKind::None;
    route_result.response_shape = OutputResponseShape::Free;
    agent_run_context.output_contract = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("passthrough=disallowed"));
    assert!(route_disallows_direct_observation_passthrough(
        agent_run_context.output_contract.as_ref().unwrap()
    ));
    assert!(observed_contract_json(Some(&agent_run_context))
        .contains(r#""direct_observation_passthrough_allowed":false"#));

    route_result.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route_result.response_shape = OutputResponseShape::Strict;
    route_result.locator_kind = OutputLocatorKind::None;
    route_result.locator_hint.clear();
    agent_run_context.output_contract = Some(route_result.clone());
    assert!(route_disallows_direct_observation_passthrough(
        agent_run_context.output_contract.as_ref().unwrap()
    ));
    assert!(observed_contract_json(Some(&agent_run_context))
        .contains(r#""direct_observation_passthrough_allowed":false"#));

    route_result.response_shape = OutputResponseShape::FileToken;
    agent_run_context.output_contract = Some(route_result);
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("style_policy=file_token"));
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("bare_delivery_token=true"));
}

#[test]
fn chat_wrapped_free_content_contract_requires_model_synthesis() {
    let route = chat_wrapped_unclassified_route(OutputResponseShape::Free);
    assert!(route_requires_synthesized_delivery(&route));

    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let contract = observed_contract_json(Some(&agent_run_context));
    assert!(contract.contains(r#""direct_observation_passthrough_allowed":false"#));
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("style_policy=evidence_synthesis"));
}

#[test]
fn single_file_delivery_uses_path_batch_fact_as_file_token() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-observed-file-delivery-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let file = root.join("release_checklist.md");
    std::fs::write(&file, "release checklist").expect("write temp file");

    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::FileToken);
    route.delivery_required = true;
    route.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = file.display().to_string();

    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "path_batch_facts",
            "count": 1,
            "facts": [
                {
                    "exists": true,
                    "fact": {
                        "kind": "file",
                        "path": file.display().to_string(),
                        "resolved_path": file.display().to_string(),
                        "size_bytes": 17
                    },
                    "path": file.display().to_string()
                }
            ],
            "include_missing": true
        })
        .to_string(),
    ));
    loop_state.has_tool_or_skill_output = true;

    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("file token candidate");

    assert_eq!(answer, format!("FILE:{}", file.display()));

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn single_file_delivery_ignores_prior_read_range_rejections_after_path_fact() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-observed-file-delivery-after-reject-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let file = root.join("release_checklist.md");
    std::fs::write(&file, "release checklist").expect("write temp file");

    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::FileToken);
    route.requires_content_evidence = false;
    route.delivery_required = true;
    route.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = file.display().to_string();

    let contract_error = "__RC_SKILL_ERROR__:{\"error_kind\":\"contract_action_rejected\",\"error_text\":\"action `system_basic.read_range` is rejected by contract `generic_delivery` (rejected_not_allowed)\",\"extra\":{\"action\":\"system_basic.read_range\",\"contract_match\":\"generic_delivery\",\"decision\":\"rejected_not_allowed\"},\"skill\":\"system_basic\"}";
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(error_step("step_1", "system_basic", contract_error));
    loop_state
        .executed_step_results
        .push(error_step("step_2", "system_basic", contract_error));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "fs_basic",
        &serde_json::json!({
            "action": "path_batch_facts",
            "count": 1,
            "facts": [
                {
                    "exists": true,
                    "fact": {
                        "kind": "file",
                        "path": file.display().to_string(),
                        "resolved_path": file.display().to_string(),
                        "size_bytes": 17
                    },
                    "path": file.display().to_string()
                }
            ],
            "include_missing": true
        })
        .to_string(),
    ));
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;

    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("file token candidate after rejected reads");

    assert_eq!(answer, format!("FILE:{}", file.display()));

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn chat_wrapped_one_sentence_unclassified_contract_requires_synthesized_delivery() {
    let route = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
    assert!(route_requires_synthesized_delivery(&route));

    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let contract = observed_contract_json(Some(&agent_run_context));
    assert!(contract.contains(r#""direct_observation_passthrough_allowed":false"#));
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("passthrough=disallowed"));
}

#[test]
fn chat_wrapped_strict_exact_sentence_contract_requires_synthesized_delivery() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.exact_sentence_count = Some(1);
    assert!(route_requires_synthesized_delivery(&route));

    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let contract = observed_contract_json(Some(&agent_run_context));
    assert!(contract.contains(r#""direct_observation_passthrough_allowed":false"#));
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("passthrough=disallowed"));
}

#[test]
fn strict_plain_observation_contract_requires_synthesis() {
    let route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    assert!(route_requires_synthesized_delivery(&route));

    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "model_io.log.2026-05-14 215M\nmodel_io.log.2026-05-11 149M\n",
    ));
    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn raw_command_contract_allows_observation_passthrough() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.semantic_kind = OutputSemanticKind::RawCommandOutput;
    assert!(!route_requires_synthesized_delivery(&route));

    let agent_run_context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let contract = observed_contract_json(Some(&agent_run_context));
    assert!(contract.contains(r#""direct_observation_passthrough_allowed":true"#));
}

#[test]
fn direct_observation_passthrough_detector_matches_raw_output() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "/home/guagua/rustclaw\n"));

    assert!(answer_is_direct_observation_passthrough(
        "/home/guagua/rustclaw",
        &loop_state
    ));
    assert!(!answer_is_direct_observation_passthrough(
        "Working directory: /home/guagua/rustclaw",
        &loop_state
    ));
}
