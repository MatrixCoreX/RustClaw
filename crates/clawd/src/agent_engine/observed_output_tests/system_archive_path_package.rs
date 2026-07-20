#[test]
fn direct_answer_defers_system_basic_info_summary_to_llm_for_brief_request() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"info","hostname":"rustclaw-test-host.local","os":"macos","arch":"x86_64","cwd":"/tmp/rustclaw-workspace"}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("command_output".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
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
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "scripts/skill_calls".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
fn direct_scalar_reads_unique_scalar_from_multi_read_fields_with_container_noise() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "config_basic",
        r#"{"action":"read_fields","path":"/repo/package.json","format":"json","results":[{"exists":true,"field_path":"scripts","resolved_field_path":"scripts","value":{"build":"echo build","dev":"echo dev","lint":"echo lint"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}","value_type":"object"},{"exists":true,"field_path":"name","resolved_field_path":"name","value":"rustclaw-nl-fixture","value_text":"rustclaw-nl-fixture","value_type":"string"}]}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.requires_content_evidence = true;
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "/repo/package.json".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
    route_result.requires_content_evidence = true;
    route_result.locator_kind = OutputLocatorKind::Path;
    route_result.locator_hint = "/repo/package.json".to_string();
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
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
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("command_output".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("command_output".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_system_basic_info_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"extra":{"arch":"x86_64","current_user":"guagua","cwd":"/home/guagua/rustclaw","hostname":"ThinkPad-X1","os":"linux","pid":2488573,"process_rss_bytes":3055616,"uptime_seconds":"894677.25","workspace_root":"/home/guagua/rustclaw"},"text":"{\"arch\":\"x86_64\",\"current_user\":\"guagua\",\"cwd\":\"/home/guagua/rustclaw\",\"hostname\":\"ThinkPad-X1\",\"os\":\"linux\",\"pid\":2488573,\"process_rss_bytes\":3055616,\"uptime_seconds\":\"894677.25\",\"workspace_root\":\"/home/guagua/rustclaw\"}"}"#,
    ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("resolved_path".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("path".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "pwd_line.txt".to_string(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("path".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/home/guagua/rustclaw/document/pwd_line.txt")
    );
}

#[test]
fn generic_workspace_summary_is_not_hard_summarized_by_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "Cargo.toml\ncrates/\nUI/\nconfigs/\nREADME.md\nREADME.zh-CN.md\nprompts/\nrustclaw.service\ncomponent_start/start-telegramd.sh\ncomponent_start/start-wechatd.sh\ncomponent_start/start-whatsappd.sh\n",
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
fn exact_path_selector_uses_auto_locator_full_path_for_unique_list_dir_match() {
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
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "report.md".to_string(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("path".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
fn exact_path_selector_uses_rooted_full_path_for_unique_find_name_match() {
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
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "report.md".to_string(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("path".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
fn exact_path_selector_prefers_resolved_path_from_path_batch_facts() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD","resolved_path":"/tmp/case_only/Report.MD","size_bytes":33},"path":"/tmp/case_only/report.md","resolved_from_case_insensitive":true}],"include_missing":true}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "report.md".to_string(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("resolved_path".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/case_only/Report.MD")
    );
}

#[test]
fn path_fact_without_exact_selector_defers_to_model_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"Report.MD","resolved_path":"/tmp/case_only/Report.MD","size_bytes":33},"path":"Report.MD"}],"include_missing":true}"#,
    ));
    let route_result = IntentOutputContract {
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "Report.MD".to_string(),
        ..Default::default()
    };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn path_fact_without_exact_path_selector_does_not_direct_render() {
    let mut loop_state = LoopState::new(2);
    loop_state.last_user_visible_respond = Some("/tmp/case_only/Report.MD".to_string());
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD","resolved_path":"/tmp/case_only/Report.MD","size_bytes":33},"path":"/tmp/case_only/Report.MD"}],"include_missing":true}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "report.md".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
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
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("count".to_string()),
                ..Default::default()
            },
            locator_hint: "scripts".to_string(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("count".to_string()),
                ..Default::default()
            },
            locator_hint: "scripts".to_string(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("3")
    );
}

#[test]
fn non_scalar_count_observation_waits_for_model_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","path":"document","resolved_path":"/tmp/document","names_only":true,"names":["a","b","c","d"],"counts":{"total":4,"files":4,"dirs":0},"recursive":false}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("count".to_string()),
                ..Default::default()
            },
            locator_hint: "document".to_string(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn exact_path_selector_projects_inventory_directory_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/stem_multi","resolved_path":"/tmp/stem_multi","names_only":true,"names":["abcd.cpp","abcd.txt"],"counts":{"total":2}}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "/tmp/stem_multi".to_string(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("path".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/stem_multi")
    );
}
