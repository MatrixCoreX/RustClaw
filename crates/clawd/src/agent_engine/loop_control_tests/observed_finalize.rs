use super::*;

#[test]
fn observed_scalar_output_can_stop_loop_without_second_round() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
    ));
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"extract_field"}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route_result(OutputResponseShape::Scalar)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn observed_config_basic_strict_output_continues_for_synthesis() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "config_basic",
        r#"{"action":"extract_field","exists":true,"field_path":"run_cmd.planner_kind","value_text":"tool","value":"tool","value_type":"string"}"#,
    ));
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({"action":"read_field","path":"configs/skills_registry.toml","field_path":"run_cmd.planner_kind"}),
    }];
    let mut route = route_result(OutputResponseShape::Strict);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn observed_call_capability_inventory_names_continue_for_synthesis() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","names_by_kind":{"dirs":[],"files":["full_suite_trace_note.txt","gen-1778122040.png","hello.sh"],"other":[]},"path":"document","sort_by":"name"}}"#,
    ));
    let actions = vec![AgentAction::CallCapability {
        capability: "filesystem.list_file_names".to_string(),
        args: json!({"path":"/workspace/document","files_only":true,"names_only":true,"max_entries":5}),
    }];

    let mut route = route_result(OutputResponseShape::Strict);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn complete_structured_selector_stops_after_single_capability_only_plan() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    let mut route = route_result(OutputResponseShape::Strict);
    route.selection.structured_field_selector = Some(
        "checkpoint,diff,failed_verification,repair_attempt,passing_verification,rewind_references"
            .to_string(),
    );
    loop_state.output_contract = Some(route);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "task_control",
        r#"{"extra":{"checkpoint":{"status":"planned"},"diff":{"status":"planned"},"failed_verification":{"status":"failed"},"repair_attempt":{"attempt":1},"passing_verification":{"status":"passed"},"rewind_references":["checkpoint:1"]}}"#,
    ));
    loop_state
        .capability_results
        .push(claw_core::capability_result::CapabilityResultEnvelope::ok(
            "task_control",
            Some("preview_repair".to_string()),
            json!({
                "output": {
                    "extra": {
                        "checkpoint": {"status":"planned"},
                        "diff": {"status":"planned"},
                        "failed_verification": {"status":"failed"},
                        "repair_attempt": {"attempt":1},
                        "passing_verification": {"status":"passed"},
                        "rewind_references": ["checkpoint:1"]
                    }
                }
            }),
        ));
    let actions = vec![AgentAction::CallCapability {
        capability: "coding_workflow.preview_repair".to_string(),
        args: json!({}),
    }];

    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route_result(OutputResponseShape::Free)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn incomplete_structured_selector_does_not_trigger_shared_round_stop() {
    let mut loop_state = LoopState::new();
    let mut route = route_result(OutputResponseShape::Strict);
    route.selection.structured_field_selector = Some("checkpoint,diff".to_string());
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "task_control",
        r#"{"extra":{"checkpoint":{"status":"planned"}}}"#,
    ));

    assert!(!structured_field_selector_observation_can_finalize(
        &route,
        &loop_state,
    ));
}

#[test]
fn capability_inventory_names_continue_to_incremental_planner() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","names_by_kind":{"dirs":[],"files":["full_suite_trace_note.txt","gen-1778122040.png","hello.sh"],"other":[]},"path":"document","sort_by":"name"}}"#,
    ));
    let actions = vec![AgentAction::CallCapability {
        capability: "filesystem.list_file_names".to_string(),
        args: json!({"path":"/workspace/document","files_only":true,"names_only":true,"max_entries":5}),
    }];
    let mut route = route_result(OutputResponseShape::Strict);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;

    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn observed_wrapped_empty_config_basic_scalar_output_can_stop_loop_without_second_round() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"extract_field","exists":true,"field_path":"workspace.package.repository","value_text":"","value":"","value_type":"string"},"text":"{\"action\":\"extract_field\",\"exists\":true,\"field_path\":\"workspace.package.repository\",\"value_text\":\"\",\"value\":\"\",\"value_type\":\"string\"}"}"#,
    ));
    let actions = vec![AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: json!({"action":"read_field","path":"Cargo.toml","field_path":"workspace.package.repository"}),
    }];
    let agent_run_context = AgentRunContext {
        output_contract: Some(route_result(OutputResponseShape::Scalar)),
        ..Default::default()
    };
    assert_eq!(
        crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output(
            &loop_state,
            Some(&agent_run_context),
        )
        .as_deref(),
        Some("\"\"")
    );
    assert_eq!(
        crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &crate::AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context),
        )
        .as_deref(),
        Some("\"\"")
    );
    assert!(should_stop_for_observed_finalize(
        Some(&agent_run_context),
        &loop_state,
        &actions,
    ));

    let mut path_route = route_result(OutputResponseShape::Scalar);
    path_route.requires_content_evidence = true;
    path_route.delivery_required = false;
    path_route.locator_kind = OutputLocatorKind::Path;
    path_route.locator_hint = "Cargo.toml".to_string();
    let path_agent_run_context = AgentRunContext {
        output_contract: Some(path_route.clone()),
        ..Default::default()
    };
    assert_eq!(
        crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &crate::AppState::test_default_with_fixture_provider(),
            Some(&path_agent_run_context),
        )
        .as_deref(),
        Some("\"\"")
    );
}

#[test]
fn bounded_read_range_observe_only_round_uses_incremental_planner() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r##"{"extra":{"action":"read_range","mode":"head","requested_n":4,"start_line":1,"end_line":4,"excerpt":"1|# Device Local Fixture\n2|\n3|This directory contains stable local files for RustClaw NL regression tests.\n4|","path":"/tmp/README.md"}}"##,
    ));
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({"action":"read_text_range","path":"/tmp/README.md","mode":"head","n":4}),
    }];
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = false;
    route.locator_kind = OutputLocatorKind::Path;
    assert!(observe_only_round_should_continue(
        &route,
        &loop_state,
        &actions,
    ));
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn summary_read_range_observe_only_round_still_uses_incremental_planner() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r##"{"extra":{"action":"read_range","mode":"head","requested_n":3,"excerpt":"1|# Service Notes\n2|\n3|Operators should check logs first.","path":"/tmp/service_notes.md"}}"##,
    ));
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({"action":"read_text_range","path":"/tmp/service_notes.md","mode":"head","n":3}),
    }];
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    assert!(observe_only_round_should_continue(
        &route,
        &loop_state,
        &actions,
    ));
}

#[test]
fn service_control_status_protocol_output_continues_for_model_synthesis() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    let service_payload = json!({
        "status": "ok",
        "target": "clawd",
        "service_name": "clawd",
        "manager_type": "rustclaw",
        "requested_action": "status",
        "executed_actions": ["status"],
        "pre_state": "clawd=running",
        "post_state": "clawd=running",
        "verified": true,
        "summary": "Status: clawd=running"
    });
    let protocol_output = json!({
        "request_id": "direct-44",
        "status": "ok",
        "text": "service_control status",
        "extra": service_payload,
        "error_text": null
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "service_control", &protocol_output));

    let mut route = route_result(OutputResponseShape::Strict);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::None;
    let actions = vec![AgentAction::CallSkill {
        skill: "service_control".to_string(),
        args: json!({"action":"status","target":"clawd","manager_type":"rustclaw"}),
    }];

    let context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    assert_eq!(
        crate::agent_engine::observed_output::extract_direct_answer_from_generic_output(
            &loop_state,
            Some(&context),
        ),
        None
    );
    assert!(!should_stop_for_observed_finalize(
        Some(&context),
        &loop_state,
        &actions,
    ));
}

#[test]
fn raw_strict_model_language_output_does_not_stop_on_bare_observation() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "/home/guagua/rustclaw\n"));
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command":"pwd"}),
    }];
    let mut route = route_result(OutputResponseShape::Strict);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::None;
    route.configure_exact_command_output();
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn observation_only_freeform_round_can_stop_for_observed_fallback() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        "README.md\ndocs/\ncrates/\n",
    ));
    let actions = vec![AgentAction::CallSkill {
        skill: "list_dir".to_string(),
        args: json!({"path":"."}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route_result(OutputResponseShape::Free)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn unscoped_workspace_evidence_drafting_does_not_stop_on_search_only() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_search",
        r#"{"action":"find_name","count":2,"results":["README.md","USAGE.md"]}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint.clear();
    let actions = vec![AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: json!({"action":"find_name","pattern":"README"}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn unscoped_workspace_evidence_drafting_continues_after_doc_read() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","path":"README.md","excerpt":"1|# RustClaw\n2|## Setup"}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint.clear();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"read_range","path":"README.md","mode":"head","n":120}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn fs_basic_inventory_names_can_stop_before_synthesis_followup() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","path":"/tmp/document","resolved_path":"/tmp/document","files_only":true,"names_only":true,"names":["a.txt","b.md","c.png"]}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "document".to_string();
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action":"list_dir","path":"/tmp/document","files_only":true,"names_only":true}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
    ];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn path_inspection_waits_for_model_synthesis_after_observation() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/home/guagua/rustclaw/rustclaw.service","size_bytes":1190},"path":"/home/guagua/rustclaw/rustclaw.service"}],"include_missing":true}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint = "rustclaw.service".to_string();
    route.selection.structured_field_selector = Some("exists,path".to_string());
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"path_batch_facts","paths":["/home/guagua/rustclaw/rustclaw.service"]}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn missing_path_inspection_waits_for_model_synthesis() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"path":"plan/missing.md"}],"include_missing":true}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "plan/missing.md".to_string();
    route.selection.structured_field_selector = Some("exists,path".to_string());
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"path_batch_facts","paths":["plan/missing.md"]}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn missing_path_batch_facts_content_contract_continues_for_possible_fallback() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"path":"plan/missing.md"}],"include_missing":true}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "plan/missing.md".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"path_batch_facts","paths":["plan/missing.md"]}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn structured_keys_free_output_can_stop_before_second_round() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"structured_keys","path":"/tmp/package.json","resolved_path":"/tmp/package.json","field_path":"scripts","exists":true,"container_type":"object","count":3,"keys":["build","dev","lint"]}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/tmp/package.json".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"structured_keys","path":"/tmp/package.json","field_path":"scripts"}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn extract_fields_free_output_can_stop_before_second_round() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"extract_fields","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","count":2,"results":[{"field_path":"database.sqlite_path","exists":true,"value_type":"string","value_text":"data/rustclaw.db","value":"data/rustclaw.db"},{"field_path":"tools.allow_sudo","exists":true,"value_type":"bool","value_text":"true","value":true}]}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "/tmp/config.toml".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"extract_fields","path":"/tmp/config.toml","field_paths":["database.sqlite_path","tools.allow_sudo"]}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn health_check_scalar_summary_continues_to_synthesis() {
    let mut loop_state = LoopState::new();
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "health_check",
        r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
    ));
    let route = route_result(OutputResponseShape::Scalar);
    let actions = vec![AgentAction::CallSkill {
        skill: "health_check".to_string(),
        args: json!({}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn recipe_waiting_for_validation_does_not_stop_on_observed_output() {
    let mut loop_state = LoopState::new();
    loop_state.execution_recipe = ExecutionRecipeRuntimeState {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        validation_required: true,
        saw_mutation: true,
        ..Default::default()
    };
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "configuration updated\n"));
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command":"cat ./config.json"}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route_result(OutputResponseShape::Free)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn recipe_inspect_stage_does_not_stop_on_observed_output() {
    let mut loop_state = LoopState::new();
    loop_state.execution_recipe = ExecutionRecipeRuntimeState {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Inspect,
        inspect_first: true,
        validation_required: true,
        ..Default::default()
    };
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "index.html\n"));
    let actions = vec![AgentAction::CallSkill {
        skill: "list_dir".to_string(),
        args: json!({"path":"document/nl_ops_http_demo"}),
    }];
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route_result(OutputResponseShape::Scalar)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn read_only_round_continues_planner_without_runtime_recipe() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"list_dir","path":"/tmp/demo","entries":["calc_core.py"]}}"#,
    ));
    let route = route_result(OutputResponseShape::Free);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action":"list_dir","path":"/tmp/demo"}),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action":"read_text_range","path":"/tmp/demo/calc_core.py"}),
        },
    ];

    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn strict_json_read_only_round_continues_planner_for_live_code_workspace() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"list_dir","path":"/workspace/project","names":["calc_core.py","test_calc_core.py"]}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/project/calc_core.py","excerpt":"1|def add(a,b): return a+b\n2|def sub(a,b): return a-b"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/project/test_calc_core.py","excerpt":"1|from calc_core import add, sub"}}"#,
    ));
    let mut route = route_result(OutputResponseShape::Strict);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action":"list_dir","path":"/workspace/project","names_only":true}),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action":"read_text_range","path":"/workspace/project/calc_core.py","mode":"full"}),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({"action":"read_text_range","path":"/workspace/project/test_calc_core.py","mode":"full"}),
        },
    ];

    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn bounded_capability_observation_continues_without_round_cap() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/project/calc_core.py","resolved_path":"/workspace/project/calc_core.py","excerpt":"1|def add(a,b): return a+b"}}"#,
    ));
    let mut route = route_result(OutputResponseShape::Strict);
    route.requires_content_evidence = true;
    route.locator_kind = OutputLocatorKind::Path;
    let actions = vec![AgentAction::CallCapability {
        capability: "filesystem.read_text_range".to_string(),
        args: json!({"path":"/workspace/project/calc_core.py","start_line":1,"end_line":16}),
    }];

    assert!(observe_only_round_should_continue(
        &route,
        &loop_state,
        &actions,
    ));
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn fs_basic_capability_read_only_round_continues_planner() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/project/calc_core.py","resolved_path":"/workspace/project/calc_core.py","excerpt":"1|def add(a,b): return a+b\n2|def sub(a,b): return a-b"}}"#,
    ));
    let route = route_result(OutputResponseShape::FileToken);
    let actions = vec![AgentAction::CallCapability {
        capability: "fs_basic.read_text_range".to_string(),
        args: json!({"path":"/workspace/project/calc_core.py"}),
    }];

    assert!(observe_only_round_should_continue(
        &route,
        &loop_state,
        &actions,
    ));
    assert!(!should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route.clone()),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn recipe_done_does_not_scan_user_text_for_success_marker() {
    let mut loop_state = LoopState::new();
    loop_state.execution_recipe = ExecutionRecipeRuntimeState {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Done,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: true,
        ..Default::default()
    };
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "ops-demo-ok\n"));
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command":"curl -s http://127.0.0.1:52752/ | grep -o ops-demo-ok"}),
    }];
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route_result(OutputResponseShape::Scalar)),
            user_request: Some(
                "验证通过时请明确输出 VALIDATION_PASSED，然后直接结束。".to_string()
            ),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn explicit_execution_recipe_hint_takes_priority_over_local_detection() {
    let spec = initial_execution_recipe_spec(
        "configure sing-box and verify the proxy works",
        "configure sing-box and verify the proxy works",
        Some(&AgentRunContext {
            execution_recipe_hint: Some(ExecutionRecipeSpec {
                kind: ExecutionRecipeKind::OpsClosedLoop,
                profile: ExecutionRecipeProfile::CodeChange,
                target_scope: ExecutionRecipeTargetScope::Greenfield,
                inspect_first: true,
                validation_required: true,
                max_repairs: 2,
            }),
            output_contract: Some(route_result(OutputResponseShape::Free)),
            user_request: Some("configure sing-box and verify the proxy works".to_string()),
            ..Default::default()
        }),
    );
    assert_eq!(spec.profile, ExecutionRecipeProfile::CodeChange);
    assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::Greenfield);
}
