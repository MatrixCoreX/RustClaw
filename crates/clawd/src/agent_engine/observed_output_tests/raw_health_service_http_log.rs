#[test]
fn direct_answer_preserves_run_cmd_directory_entry_names() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_output_test_{}_run_cmd_names",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&temp_dir);
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "act_plan.log\nclawd.log\nfeishud.log\n",
    ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "logs".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
fn direct_answer_preserves_run_cmd_directory_entry_names_without_request_text_limit() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_output_test_{}_run_cmd_limit",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&temp_dir);
    let mut loop_state = LoopState::new();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "a\nb\nc\nd\n"));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "logs".to_string(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
fn run_cmd_exists_token_is_not_interpreted_as_a_path_verdict() {
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
    let mut loop_state = LoopState::new();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "EXISTS\n"));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "rustclaw.service".to_string(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("exists,path".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        auto_locator_path: Some(resolved.clone()),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn run_cmd_not_found_token_is_not_interpreted_as_a_path_verdict() {
    let mut loop_state = LoopState::new();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "NOT_FOUND\n"));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: "rustclaw.service".to_string(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("exists,path".to_string()),
                ..Default::default()
            },
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
fn direct_answer_defers_health_check_json_for_act_free_shape() {
    let mut loop_state = LoopState::new();
    let body = r#"{"clawd_health_port_open":true,"telegramd_process_count":0}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
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
fn direct_answer_defers_health_check_contract_to_synthesis() {
    let mut loop_state = LoopState::new();
    let body = r#"{"clawd_process_count":1,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0}}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
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
fn direct_answer_defers_wrapped_health_check_free_shape() {
    let mut loop_state = LoopState::new();
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
    let mut loop_state = LoopState::new();
    let body = r#"{"clawd_process_count":1,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":43},"system_health":{"os_family":"linux","load_avg_1m":3.81,"memory_available_bytes":11270471680,"disk_root_available_bytes":18108059648,"warnings":[]}}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
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
fn direct_answer_defers_health_check_summary_for_act_free_shape() {
    let mut loop_state = LoopState::new();
    let body = r#"{"clawd_process_count":7,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
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
fn direct_answer_defers_health_check_summary_over_later_steps_to_llm() {
    let mut loop_state = LoopState::new();
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
    let route_result = IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
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
fn direct_answer_defers_health_check_one_sentence_summary_to_llm() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
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
fn direct_answer_defers_health_check_unhealthy_summary_to_llm() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":0,"telegramd_process_count":1,"clawd_health_port_open":false,"clawd_log":{"exists":true,"keyword_error_count":3},"telegramd_log":{"exists":true,"keyword_error_count":0}}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
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
fn direct_answer_defers_health_check_telegramd_stopped_summary_to_llm() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
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
fn direct_answer_defers_health_check_language_sensitive_summary_to_llm() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":12,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":1,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":true,"keyword_error_count":0},"system_health":{"os_family":"linux","warnings":["disk_root_low"]}}"#,
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract::default(),
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
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
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "process_basic",
            "exit=0\nCOMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nclawd 4498 testuser 12u IPv4 0x0 0t0 TCP *:8787 (LISTEN)\nnginx 51129 testuser 6u IPv4 0x0 0t0 TCP *:80 (LISTEN)\nss-local 424 testuser 6u IPv4 0x0 0t0 TCP 127.0.0.1:1086 (LISTEN)\n",
        ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
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
fn direct_answer_defers_wrapped_process_basic_port_status_to_synthesis() {
    let mut loop_state = LoopState::new();
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
                "all_interface_listener_count": 2,
                "localhost_listener_count": 1,
                "internet_reachability": "not_observed",
                "ports": ["80", "8787", "46225"],
                "all_interface_ports": ["80", "8787"],
                "all_interface_listeners": [
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

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context));

    assert_eq!(answer, None);
}

#[test]
fn observed_entries_compact_wrapped_process_basic_port_list() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "process_basic",
        &serde_json::json!({
            "extra": {
                "action": "port_list",
                "command_tool": "ss",
                "exit_code": 0,
                "listener_count": 3,
                "all_interface_listener_count": 2,
                "localhost_listener_count": 1,
                "internet_reachability": "not_observed",
                "ports": ["80", "8787", "46225"],
                "all_interface_ports": ["80", "8787"],
                "all_interface_listeners": [
                    {
                        "bind_scope": "all_interfaces",
                        "local_endpoint": "0.0.0.0:80",
                        "pid": null,
                        "port": "80",
                        "process_name": null
                    },
                    {
                        "bind_scope": "all_interfaces",
                        "local_endpoint": "0.0.0.0:8787",
                        "pid": 2308287,
                        "port": "8787",
                        "process_name": "clawd"
                    }
                ],
                "listeners": [],
                "output": "exit=0\nState Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\nLISTEN 0 4096 0.0.0.0:8787 0.0.0.0:* users:((\"clawd\",pid=2308287,fd=31))"
            },
            "text": "exit=0\nState Recv-Q Send-Q Local Address:Port Peer Address:PortProcess"
        })
        .to_string(),
    ));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(joined.contains("process_basic.port_list"));
    assert!(joined.contains("listener.2.port=8787"));
    assert!(joined.contains("listener.2.process=clawd"));
    assert!(joined.contains("listener.2.pid=2308287"));
    assert!(!joined.contains("State Recv-Q"));
    assert!(!joined.contains("users:((\"clawd\""));
}

#[test]
fn observed_entries_compact_wrapped_process_basic_ps() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "process_basic",
        &serde_json::json!({
            "extra": {
                "action": "ps",
                "exit_code": 0,
                "filter": null,
                "limit": 30,
                "match_count": 3,
                "process_count": 3,
                "running": true,
                "status": "running",
                "output": "exit=0\nPID PPID %CPU %MEM COMM\n111 1 9.1 0.2 chrome\n222 1 0.7 0.4 clawd\n333 1 0.1 0.1 helper",
                "platform": "linux"
            },
            "text": "exit=0\nPID PPID %CPU %MEM COMM\n111 1 9.1 0.2 chrome\n222 1 0.7 0.4 clawd\n333 1 0.1 0.1 helper"
        })
        .to_string(),
    ));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(joined.contains("process_basic.ps"));
    assert!(joined.contains("ps.match_count=3"));
    assert!(joined.contains("process.2.pid=222"));
    assert!(joined.contains("process.2.comm=clawd"));
    assert!(!joined.contains("PID PPID"));
    assert!(!joined.contains("exit=0"));
}

#[test]
fn direct_answer_uses_generic_selector_for_process_listener_count() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "process_basic",
        &serde_json::json!({
            "extra": {
                "action": "port_list",
                "listener_count": 3,
                "all_interface_listener_count": 2,
                "internet_reachability": "not_observed",
                "ports": ["80", "8787", "46225"],
                "all_interface_ports": ["80", "8787"],
                "all_interface_listeners": [],
                "listeners": []
            },
            "text": "exit=0"
        })
        .to_string(),
    ));
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            locator_hint: String::new(),
            selection: crate::OutputSelectionContract {
                structured_field_selector: Some("listener_count".to_string()),
                ..Default::default()
            },
        };
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result.clone()),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context));

    assert_eq!(answer.as_deref(), Some("3"));
}

#[test]
fn direct_answer_defers_process_basic_observation_to_synthesis() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "process_basic",
        "exit=0\nPID PPID %CPU %MEM COMM\n413590 7620 1.0 0.2 clawd",
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

    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none(),
        "non-scalar process observations should be rendered by synthesis, not a runtime reply template"
    );
}

#[test]
fn direct_answer_prefers_process_basic_status_over_later_system_info() {
    let mut loop_state = LoopState::new();
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
    let route_result = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
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

    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none(),
        "one-sentence process status should not override synthesis with a fixed runtime template"
    );
}
