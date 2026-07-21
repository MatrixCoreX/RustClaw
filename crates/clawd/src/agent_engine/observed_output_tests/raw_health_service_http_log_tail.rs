#[test]
fn direct_answer_defers_http_basic_one_sentence_summary_to_llm() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "http_basic",
        "status=200\n{\"ok\":true}\n",
    ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Url,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
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
fn direct_answer_preserves_http_basic_raw_scalar_for_free_shape() {
    let mut loop_state = LoopState::new();
    let body = "status=200\n{\"ok\":true}\n";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "http_basic", body));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Url,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("status=200")
    );
}

#[test]
fn direct_answer_preserves_http_status_machine_value_without_domain_rendering() {
    let mut loop_state = LoopState::new();
    let body = "status=200\n{\"ok\":true,\"data\":{\"version\":\"0.1.7\",\"worker_state\":\"running\",\"queue_length\":0,\"bound_channel_count\":3}}\n";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "http_basic", body));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
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
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("status=200")
    );
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("status=200")
    );
}

#[test]
fn direct_answer_defers_service_control_status_summary_for_chinese_request() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "service_control",
            r#"{"status":"ok","service_name":"telegramd","manager_type":"rustclaw","requested_action":"status","executed_actions":["status"],"pre_state":"telegramd=stopped","post_state":"telegramd=stopped","verified":true,"key_evidence":["telegramd process_count=0 memory_rss_bytes=Some(0)"],"failure_reason":"","next_step":"","summary":"Status: telegramd=stopped"}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "telegramd".to_string(),
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
fn direct_answer_defers_service_control_status_summary_for_english_request() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "service_control",
            r#"{"status":"ok","service_name":"telegramd","manager_type":"rustclaw","requested_action":"status","executed_actions":["status"],"pre_state":"telegramd=running","post_state":"telegramd=running","verified":true,"key_evidence":["telegramd process_count=1 memory_rss_bytes=Some(1024)"],"failure_reason":"","next_step":"","summary":"Status: telegramd=running"}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "telegramd".to_string(),
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
fn direct_answer_uses_generic_selector_for_service_control_machine_value() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "service_control",
            r#"{"status":"ok","service_name":"telegramd","manager_type":"rustclaw","requested_action":"status","executed_actions":["status"],"pre_state":"telegramd=running","post_state":"telegramd=running","verified":true,"summary":"Status: telegramd=running"}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "telegramd".to_string(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("post_state".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("telegramd=running")
    );
}

#[test]
fn observed_entries_compact_log_analyze_json_into_summary() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "log_analyze",
            r#"{"path":"/tmp/test.log","total_lines":120,"keyword_counts":{"error":9,"panic":1},"recent_matches":["10: error one","20: panic two"]}"#,
        ));
    let entries = observed_output_entries(&loop_state);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("log_analyze path=/tmp/test.log total_lines=120"));
    assert!(entries[0].contains("keyword_counts: error=9, panic=1"));
    assert!(entries[0].contains("recent_matches:\n- 10: error one\n- 20: panic two"));
    assert!(!entries[0].contains(r#""keyword_counts""#));
}

#[test]
fn observed_entries_keep_log_analyze_tail_evidence() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "log_analyze",
        r#"{"path":"/tmp/test.log","total_lines":120,"keyword_counts":{},"tail_lines":["118|phase=loop_done no_progress_count=0","119|phase=loop_done tool_calls=1"],"tail_excerpt":"118|phase=loop_done no_progress_count=0\n119|phase=loop_done tool_calls=1"}"#,
    ));
    let entries = observed_output_entries(&loop_state);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("log_analyze path=/tmp/test.log total_lines=120"));
    assert!(entries[0].contains("tail_lines:\n- 118|phase=loop_done no_progress_count=0"));
    assert!(entries[0].contains("119|phase=loop_done tool_calls=1"));
    assert!(!entries[0].contains(r#""tail_lines""#));
}
