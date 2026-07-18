use super::*;

#[test]
fn observed_scalar_output_can_stop_loop_without_second_round() {
    let mut loop_state = LoopState::new(2);
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
fn observed_config_basic_scalar_output_can_stop_loop_without_second_round() {
    let mut loop_state = LoopState::new(2);
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
    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route_result(OutputResponseShape::Strict)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn observed_call_capability_inventory_names_can_stop_loop_without_second_round() {
    let mut loop_state = LoopState::new(2);
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

    assert!(should_stop_for_observed_finalize(
        Some(&AgentRunContext {
            output_contract: Some(route_result(OutputResponseShape::Strict)),
            ..Default::default()
        }),
        &loop_state,
        &actions,
    ));
}

#[test]
fn complete_structured_selector_stops_after_single_capability_only_plan() {
    let mut loop_state = LoopState::new(2);
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
    let mut loop_state = LoopState::new(2);
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
fn capability_inventory_names_can_stop_without_incremental_planner() {
    let mut loop_state = LoopState::new(2);
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
    let route = route_result(OutputResponseShape::Strict);

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
fn observed_wrapped_empty_config_basic_scalar_output_can_stop_loop_without_second_round() {
    let mut loop_state = LoopState::new(2);
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
fn bounded_read_range_observe_only_round_does_not_force_incremental_planner() {
    let mut loop_state = LoopState::new(2);
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
    route.semantic_kind = OutputSemanticKind::None;

    assert!(!observe_only_round_should_continue(
        &route,
        &loop_state,
        &actions,
    ));
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
fn summary_read_range_observe_only_round_still_uses_incremental_planner() {
    let mut loop_state = LoopState::new(2);
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
    route.semantic_kind = OutputSemanticKind::ContentExcerptSummary;

    assert!(observe_only_round_should_continue(
        &route,
        &loop_state,
        &actions,
    ));
}

#[test]
fn service_control_status_protocol_output_can_stop_strict_loop_without_synthesis_round() {
    let mut loop_state = LoopState::new(2);
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
    route.locator_kind = OutputLocatorKind::None;
    route.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "service_control".to_string(),
        args: json!({"action":"status","target":"clawd","manager_type":"rustclaw"}),
    }];

    let context = AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let direct_answer =
        crate::agent_engine::observed_output::extract_direct_answer_from_generic_output(
            &loop_state,
            Some(&context),
        )
        .expect("service status direct answer");
    assert!(direct_answer.contains("target=clawd"));
    assert!(direct_answer.contains("status=ok"));
    assert!(direct_answer.contains("manager_type=rustclaw"));
    assert!(should_stop_for_observed_finalize(
        Some(&context),
        &loop_state,
        &actions,
    ));
}

#[test]
fn raw_strict_model_language_output_does_not_stop_on_bare_observation() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "/home/guagua/rustclaw\n"));
    let actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: json!({"command":"pwd"}),
    }];
    let mut route = route_result(OutputResponseShape::Strict);
    route.locator_kind = OutputLocatorKind::None;
    route.semantic_kind = OutputSemanticKind::RawCommandOutput;

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
fn observed_structured_scalar_equality_pair_can_stop_without_synthesis_round() {
    let mut loop_state = LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "config_basic",
        r#"{"action":"read_field","path":"UI/package.json","resolved_path":"/repo/UI/package.json","field_path":"name","exists":true,"value_text":"react-example","value":"react-example","value_type":"string"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "config_basic",
        r#"{"action":"read_field","path":"crates/clawd/Cargo.toml","resolved_path":"/repo/crates/clawd/Cargo.toml","field_path":"package.name","exists":true,"value_text":"clawd","value":"clawd","value_type":"string"}"#,
    ));
    let mut route = route_result(OutputResponseShape::Strict);
    route.semantic_kind = OutputSemanticKind::RecentScalarEqualityCheck;
    let actions = vec![
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({"action":"read_field","path":"UI/package.json","field_path":"name"}),
        },
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({"action":"read_field","path":"crates/clawd/Cargo.toml","field_path":"package.name"}),
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
fn observation_only_freeform_round_can_stop_for_observed_fallback() {
    let mut loop_state = LoopState::new(2);
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
fn one_sentence_quantity_comparison_waits_for_model_language_followup() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"compare_paths","comparison":{"same_size":false,"size_delta_bytes":21724},"left":{"kind":"file","path":"README.md","resolved_path":"/repo/README.md","size_bytes":46905},"right":{"kind":"file","path":"AGENTS.md","resolved_path":"/repo/AGENTS.md","size_bytes":25181}}"#,
    ));
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.semantic_kind = OutputSemanticKind::QuantityComparison;
    route.locator_kind = OutputLocatorKind::Path;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({"action":"compare_paths","left_path":"README.md","right_path":"AGENTS.md"}),
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
fn service_status_port_observation_without_direct_candidate_does_not_stop() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "process_basic",
        "exit=0\nState  Recv-Q Send-Q Local Address:Port  Peer Address:PortProcess\nLISTEN 0      4096         0.0.0.0:8787       0.0.0.0:*    users:((\"clawd\",pid=706551,fd=31))\nLISTEN 0      4096         0.0.0.0:22         0.0.0.0:*\n",
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::None;
    route.semantic_kind = OutputSemanticKind::ServiceStatus;
    let actions = vec![AgentAction::CallSkill {
        skill: "process_basic".to_string(),
        args: json!({"action":"port_list"}),
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
fn unscoped_workspace_evidence_drafting_does_not_stop_on_search_only() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_search",
        r#"{"action":"find_name","count":2,"results":["README.md","USAGE.md"]}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint.clear();
    route.semantic_kind = OutputSemanticKind::None;
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
fn unscoped_workspace_evidence_drafting_can_stop_after_doc_read() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","path":"README.md","excerpt":"1|# RustClaw\n2|## Setup"}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.locator_hint.clear();
    route.semantic_kind = OutputSemanticKind::None;
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"read_range","path":"README.md","mode":"head","n":120}),
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
fn hidden_entries_scalar_output_can_stop_before_synthesis_followup() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        ".git\nREADME.md\n.env\nsrc\n",
    ));
    let mut route = route_result(OutputResponseShape::Scalar);
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route.locator_hint = ".".to_string();
    let actions = vec![
        AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path":"."}),
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
fn fs_basic_inventory_names_can_stop_before_synthesis_followup() {
    let mut loop_state = LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","path":"/tmp/document","resolved_path":"/tmp/document","files_only":true,"names_only":true,"names":["a.txt","b.md","c.png"]}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::Path;
    route.semantic_kind = OutputSemanticKind::FileNames;
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
fn recent_artifacts_inventory_can_stop_before_content_read_round() {
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"request_id":"req-1","status":"ok","text":"{\"action\":\"inventory_dir\"}","error_text":null,"extra":{"action":"inventory_dir","entries":[{"kind":"file","modified_ts":9,"name":"clawd.run.log","path":"logs/clawd.run.log","size_bytes":2300},{"kind":"file","modified_ts":8,"name":"model_io.log","path":"logs/model_io.log","size_bytes":900},{"kind":"file","modified_ts":7,"name":"act_plan.log","path":"logs/act_plan.log","size_bytes":300}],"names":["clawd.run.log","model_io.log","act_plan.log"],"path":"/repo/logs","resolved_path":"/repo/logs","sort_by":"mtime_desc"}}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "logs".to_string();
    route.selection.list_selector.limit = Some(2);
    route.selection.list_selector.target_kind = crate::OutputScalarCountTargetKind::File;
    route.selection.list_selector.target_kind_specified = true;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({"action":"list_dir","path":"logs","sort_by":"mtime_desc","files_only":true,"max_entries":2}),
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
fn recent_artifacts_inventory_stop_respects_file_selector() {
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","entries":[{"kind":"dir","modified_ts":9,"name":"bundle_unpack","path":"tmp/bundle_unpack"},{"kind":"dir","modified_ts":8,"name":"manual_unpack","path":"tmp/manual_unpack"}],"path":"/repo/tmp","sort_by":"mtime_desc"}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.semantic_kind = OutputSemanticKind::RecentArtifactsJudgment;
    route.locator_kind = OutputLocatorKind::Path;
    route.locator_hint = "tmp".to_string();
    route.selection.list_selector.target_kind = crate::OutputScalarCountTargetKind::File;
    route.selection.list_selector.target_kind_specified = true;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({"action":"list_dir","path":"tmp","sort_by":"mtime_desc","files_only":true}),
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
fn existence_with_path_free_output_can_stop_before_second_round() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/home/guagua/rustclaw/rustclaw.service","size_bytes":1190},"path":"/home/guagua/rustclaw/rustclaw.service"}],"include_missing":true}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.locator_hint = "rustclaw.service".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"path_batch_facts","paths":["/home/guagua/rustclaw/rustclaw.service"]}),
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
fn missing_path_batch_facts_existence_contract_stops_before_second_round() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"path":"plan/missing.md"}],"include_missing":true}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::Path;
    route.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.locator_hint = "plan/missing.md".to_string();
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: json!({"action":"path_batch_facts","paths":["plan/missing.md"]}),
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
fn missing_path_batch_facts_content_contract_continues_for_possible_fallback() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"path":"plan/missing.md"}],"include_missing":true}"#,
    ));
    let mut route = route_result(OutputResponseShape::Free);
    route.locator_kind = OutputLocatorKind::Path;
    route.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
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
    let mut loop_state = LoopState::new(2);
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
    let mut loop_state = LoopState::new(2);
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
    let mut loop_state = LoopState::new(2);
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
    let mut loop_state = LoopState::new(2);
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
    let mut loop_state = LoopState::new(2);
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
    let mut loop_state = LoopState::new(2);
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
    let mut loop_state = LoopState::new(2);
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
    let route = route_result(OutputResponseShape::Strict);
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
fn bounded_capability_observation_can_finalize_at_round_cap() {
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 2;
    loop_state.has_tool_or_skill_output = true;
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_text_range","path":"/workspace/project/calc_core.py","resolved_path":"/workspace/project/calc_core.py","excerpt":"1|def add(a,b): return a+b"}}"#,
    ));
    let route = route_result(OutputResponseShape::Strict);
    let actions = vec![AgentAction::CallCapability {
        capability: "filesystem.read_text_range".to_string(),
        args: json!({"path":"/workspace/project/calc_core.py","start_line":1,"end_line":16}),
    }];

    assert!(!observe_only_round_should_continue(
        &route,
        &loop_state,
        &actions,
    ));
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
fn fs_basic_capability_read_only_round_continues_planner() {
    let mut loop_state = LoopState::new(4);
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
    let mut loop_state = LoopState::new(2);
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
fn recoverable_recipe_failure_continues_next_round_and_keeps_repair_count() {
    let task = test_task();
    let policy = test_policy();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    loop_state.execution_recipe = ExecutionRecipeRuntimeState {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Repair,
        inspect_first: true,
        validation_required: true,
        max_repairs: 3,
        repair_count: 1,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: false,
        ..Default::default()
    };
    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: Some("repair sing-box".to_string()),
        no_progress: false,
    };
    assert!(!evaluate_round_outcome(
        &task,
        &mut loop_state,
        &policy,
        &outcome
    ));
    assert_eq!(loop_state.execution_recipe.repair_count, 1);
    assert_eq!(
        loop_state.execution_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Repair
    );
    assert_eq!(loop_state.consecutive_no_progress, 0);
}

#[test]
fn recoverable_failure_at_round_cap_extends_loop_once() {
    let task = test_task();
    let mut policy = test_policy();
    policy.max_rounds = 2;
    policy.recoverable_failure_extra_rounds = 1;
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 2;
    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: Some("try alternate locator".to_string()),
        no_progress: false,
    };

    assert!(!evaluate_round_outcome(
        &task,
        &mut loop_state,
        &policy,
        &outcome
    ));
    assert_eq!(loop_state.max_rounds, 3);
    assert_eq!(loop_state.recoverable_failure_extra_rounds_used, 1);
}

#[test]
fn recoverable_failure_extra_round_exhaustion_stops() {
    let task = test_task();
    let mut policy = test_policy();
    policy.max_rounds = 2;
    policy.recoverable_failure_extra_rounds = 1;
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 2;
    loop_state.recoverable_failure_extra_rounds_used = 1;
    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("recoverable_failure_continue_round".to_string()),
        next_goal_hint: Some("try alternate locator".to_string()),
        no_progress: false,
    };

    assert!(evaluate_round_outcome(
        &task,
        &mut loop_state,
        &policy,
        &outcome
    ));
    assert_eq!(loop_state.max_rounds, 2);
    assert_eq!(loop_state.recoverable_failure_extra_rounds_used, 1);
}

#[test]
fn exhausted_recipe_budget_stops_next_round() {
    let task = test_task();
    let policy = test_policy();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 2;
    loop_state.execution_recipe = ExecutionRecipeRuntimeState {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Repair,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        repair_count: 3,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: false,
        ..Default::default()
    };
    let outcome = RoundOutcome {
        executed_actions: 1,
        had_error: false,
        stop_signal: Some("recipe_repair_budget_exhausted".to_string()),
        next_goal_hint: None,
        no_progress: false,
    };
    assert!(evaluate_round_outcome(
        &task,
        &mut loop_state,
        &policy,
        &outcome
    ));
    assert_eq!(loop_state.execution_recipe.repair_count, 3);
    assert_eq!(
        loop_state.execution_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Repair
    );
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
