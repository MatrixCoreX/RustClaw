use super::*;

#[test]
fn requested_machine_kv_summary_replaces_raw_observed_delivery() {
    let task = claimed_task("task-machine-kv-summary-finalizer");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        &serde_json::json!({
            "extra": {
                "action": "read_range",
                "path": "AGENTS.md",
                "excerpt": "248|must run `python3 scripts/check_runtime_semantic_rewrite_boundary.py` after boundary changes"
            },
            "text": "{\"action\":\"read_range\",\"excerpt\":\"248|must run `python3 scripts/check_runtime_semantic_rewrite_boundary.py` after boundary changes\"}"
        })
        .to_string(),
    ));
    let mut delivery_messages = vec![
        "248|must run `python3 scripts/check_runtime_semantic_rewrite_boundary.py` after boundary changes"
            .to_string(),
    ];
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Use read_range only. Answer exactly as machine summary: required=yes script=check_runtime_semantic_rewrite_boundary.py.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["required=yes script=check_runtime_semantic_rewrite_boundary.py"]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("required=yes script=check_runtime_semantic_rewrite_boundary.py")
    );
    assert_eq!(
        finalizer_summary
            .as_ref()
            .and_then(|summary| summary.grounded_ok),
        Some(true)
    );
}

#[test]
fn requested_machine_kv_summary_uses_normalized_config_preview_selector() {
    let task = claimed_task("task-machine-kv-config-preview-selector");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "synthesize_answer",
        r#"{"after":"minimax","applied":false,"before":"minimax","dry_run":true,"field_path":"llm.selected_vendor","path":"configs/config.toml","would_change":false}"#,
    ));
    let mut delivery_messages = vec!["dry_run=true field_path=llm.selected_vendor".to_string()];
    let mut finalizer_summary = None;
    let mut route = free_route_result();
    route.selection.structured_field_selector = Some("dry_run,field_path,before,after".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "preview the config change",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec![
            "dry_run=true field_path=llm.selected_vendor before=minimax after=minimax".to_string()
        ]
    );
}

#[test]
fn requested_machine_kv_summary_preserves_policy_decision_selector() {
    let task = claimed_task("task-machine-kv-policy-decision-selector");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        r#"{"confirmation_required":false,"decision":"deny","reason_codes":["sudo_not_allowed"],"risk_level":"high","would_execute":false}"#,
    ));
    let mut delivery_messages = vec![
        r#"risk_level=high confirmation_required=false reason_codes=["sudo_not_allowed"]"#
            .to_string(),
    ];
    let mut finalizer_summary = None;
    let mut route = free_route_result();
    route.selection.structured_field_selector =
        Some("decision,risk_level,confirmation_required,reason_codes".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Preview the policy fields.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec![
            r#"decision=deny risk_level=high confirmation_required=false reason_codes=["sudo_not_allowed"]"#
                .to_string()
        ]
    );
}

#[test]
fn requested_machine_kv_summary_preserves_richer_required_evidence_delivery() {
    let task = claimed_task("task-machine-kv-summary-required-evidence-richer");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"path_batch_facts","facts":[{"path":"service_notes.md","exists":true},{"path":"release_checklist.md","exists":true}]}}"#,
    ));
    let mut delivery_messages = vec![
        "same_path=false\nservice_notes.md exists=true\nrelease_checklist.md exists=true"
            .to_string(),
    ];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::None;
    route.response_shape = OutputResponseShape::Strict;
    route.delivery_required = false;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    let _ = replace_delivery_with_requested_machine_kv_summary(
        &task,
        "return same_path and both exist fields",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    );

    assert_eq!(
        delivery_messages,
        vec![
            "same_path=false\nservice_notes.md exists=true\nrelease_checklist.md exists=true"
                .to_string()
        ]
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("same_path=false\nservice_notes.md exists=true\nrelease_checklist.md exists=true")
    );
}

#[test]
fn requested_machine_kv_summary_preserves_richer_colon_labeled_delivery() {
    let task = claimed_task("task-machine-kv-summary-colon-labeled-richer");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r##"{"extra":{"action":"read_range","path":"/workspace/README.md","line_count":1277,"first_line":"# RustClaw"}}"##,
    ));
    let current = "Observed fields:\n\n- path: /workspace/README.md\n- line_count: 1277\n- first_line: # RustClaw";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", current));
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return only machine fields path and line_count.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));
    assert_eq!(delivery_messages, vec![current.to_string()]);
}

#[test]
fn requested_machine_kv_summary_repairs_conflicting_colon_value_without_dropping_fields() {
    let task = claimed_task("task-machine-kv-summary-colon-conflict");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r##"{"extra":{"action":"read_range","path":"/workspace/README.md","line_count":1277,"first_line":"# RustClaw"}}"##,
    ));
    let current = "path: /workspace/README.md\nline_count: 20\nfirst_line: # RustClaw";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", current));
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return only machine fields path and line_count.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));
    assert_eq!(
        delivery_messages,
        vec!["path: /workspace/README.md\nline_count: 1277\nfirst_line: # RustClaw".to_string()]
    );
}

#[test]
fn hook_runtime_surface_json_can_replace_short_token_delivery() {
    let mut route = free_route_result();
    route.requires_content_evidence = true;
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"action":"extract_fields","path":"configs/agent_guard.toml"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","path":"plan/current.md","excerpt":"Track I"}"#,
    ));
    let synthesis = serde_json::json!({
        "message_key": "clawd.msg.agent_hooks.runtime_surface",
        "reason_code": "agent_hooks_runtime_surface",
        "owner_layer": "agent_hooks",
        "handler_field_path": "agent.hooks.handlers",
        "hook_stages": ["session_start", "user_prompt_submit", "pre_tool_use", "permission_request", "post_tool_use", "pre_compact", "post_compact", "subagent_start", "subagent_stop", "stop", "session_end"],
        "decision_tokens": ["allow", "deny", "require_confirmation", "background_wait"],
        "configured_handler_count": 0
    })
    .to_string();

    assert!(structured_compound_synthesis_can_replace_current_delivery(
        &route,
        &loop_state,
        "require_confirmation background_wait stage=pre_tool_use",
        &synthesis,
    ));
}

#[test]
fn grounded_compound_delivery_preserves_latest_terminal_language_over_observed_projection() {
    let task = claimed_task("task-grounded-compound-terminal");
    let mut route = free_route_result();
    route.requires_content_evidence = false;
    route.semantic_kind = OutputSemanticKind::None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "log_analyze",
        r#"{"keyword_counts":{"error":1,"warn":2},"path":"logs/app.log"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r##"{"action":"read_range","path":"docs/service_notes.md","excerpt":"# Service Notes\nrestart guidance"}"##,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "transform",
        r#"{"formatted":"| name | score |\n| beta | 12 |"}"#,
    ));
    let terminal_answer = concat!(
        "1) log evidence: error=1 warn=2\n",
        "2) document evidence: Service Notes restart guidance\n",
        "3) table:\n",
        "| name | score |\n",
        "| beta | 12 |"
    );
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "synthesize_answer",
        terminal_answer,
    ));
    let mut delivery_messages = vec!["| name | score |\n| beta | 12 |".to_string()];
    loop_state.delivery_messages = delivery_messages.clone();
    loop_state.last_user_visible_respond = delivery_messages.first().cloned();
    let mut finalizer_summary = None;

    assert!(prefer_latest_synthesis_for_compound_observation_delivery(
        &task,
        &mut loop_state,
        Some(&agent_run_context),
        &mut delivery_messages,
        &mut finalizer_summary,
    ));

    assert_eq!(delivery_messages, vec![terminal_answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(terminal_answer)
    );
    assert_eq!(
        finalizer_summary.as_ref().and_then(|summary| summary.stage),
        Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric)
    );
}

#[test]
fn requested_machine_kv_summary_preserves_publishable_summary_over_marker_only_summary() {
    let task = claimed_task("task-machine-kv-marker-only-summary");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        "fs_basic planner_kind",
    ));
    let table = "| 检查项 | 结果 |\n|---|---|\n| README.md 是否存在 | 是 |\n| docs 文件名 | release_checklist.md、service_notes.md |\n| logs 直接子项数量 | 2 |\n| fs_basic 的 planner_kind | tool |";
    let tagged_table = format!("markdown\n{table}");
    let mut delivery_messages = vec![tagged_table.clone()];
    loop_state.last_user_visible_respond = Some(tagged_table.clone());
    loop_state.last_publishable_synthesis_output = Some(tagged_table);
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Strict;
    route.delivery_required = false;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "检查 README.md、列出 docs 文件名、统计 logs 直接子项数量，并读取 fs_basic 的 planner_kind，最后用表格回答。",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![table.to_string()]);
    assert_eq!(loop_state.last_user_visible_respond.as_deref(), Some(table));
}

#[test]
fn requested_machine_kv_summary_preserves_structured_media_dry_run_projection() {
    let task = claimed_task("task-machine-kv-media-dry-run-projection");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    let output_path = "/home/guagua/rustclaw/document/media_dry_run/image_status_card.png";
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "image_generate",
        &serde_json::json!({
            "text": "IMAGE_GENERATE_DRY_RUN",
            "extra": {
                "dry_run": true,
                "provider": "minimax",
                "model": "image-01",
                "model_kind": "dry_run",
                "output_path": output_path,
                "planned_outputs": [{
                    "path": output_path,
                    "type": "image_file"
                }]
            }
        })
        .to_string(),
    ));
    let current = concat!(
        "dry_run=true\n",
        "provider=minimax\n",
        "model=image-01\n",
        "model_kind=dry_run\n",
        "output_path=/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\n",
        "planned_outputs=[{\"path\":\"/home/guagua/rustclaw/document/media_dry_run/image_status_card.png\",\"type\":\"image_file\"}]"
    );
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Free;
    route.delivery_required = false;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "use image.generate dry_run=true and return provider/model planned_outputs output_path",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![current.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(current)
    );
}

#[test]
fn requested_machine_kv_summary_preserves_async_cancel_adapter_projection() {
    let task = claimed_task("task-machine-kv-async-cancel-projection");
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "image_generate",
        r#"{"text":"IMAGE_CANCEL_DRY_RUN","extra":{"async_cancel_adapter_result":{"adapter_kind":"media_job_poll","job_id":"image-job-001","status":"cancelled","cancellation_result_json":{"task_id":"image-task-001","job_id":"image-job-001","status":"cancelled","dry_run":true}}}}"#,
    ));
    let current = concat!(
        "task_id=image-task-001\n",
        "job_id=image-job-001\n",
        "status=cancelled\n",
        "async_cancel_adapter_result={\"adapter_kind\":\"media_job_poll\",\"job_id\":\"image-job-001\",\"status\":\"cancelled\"}"
    );
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut route = free_route_result();
    route.response_shape = OutputResponseShape::Free;
    route.delivery_required = false;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "return task_id job_id cancelled status and async_cancel_adapter_result",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![current.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(current)
    );
}

#[test]
fn requested_machine_kv_summary_preserves_publishable_command_summary() {
    let task = claimed_task("task-machine-kv-summary-command-summary");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        r#"{"extra":{"action":"run_cmd","command":"pwd","command_output":"/home/guagua/rustclaw"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "process_basic",
        r#"{"extra":{"action":"port_list","port":8787,"process":"clawd","pid":892143}}"#,
    ));
    let full_answer = "The working directory is /home/guagua/rustclaw. A clawd-related process is running, and port 8787 is visible.";
    let mut delivery_messages = vec![full_answer.to_string()];
    loop_state.last_user_visible_respond = Some(full_answer.to_string());
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.response_shape = OutputResponseShape::Scalar;
    route.delivery_required = false;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        original_user_request: Some(
            "Run pwd, inspect the local port, and answer with the working directory and whether a port is visible."
                .to_string(),
        ),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Run pwd, inspect the local port, and answer with the working directory and whether a port is visible.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![full_answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(full_answer)
    );
    assert_ne!(delivery_messages, vec!["port=8787".to_string()]);
}

#[test]
fn requested_machine_kv_summary_preserves_agent_hook_runtime_surface_delivery() {
    let task = claimed_task("task-machine-kv-agent-hook-surface");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"extract_fields","path":"configs/agent_guard.toml","results":[{"field_path":"agent.hooks.handlers","value":[]}]}}"#,
    ));
    let full_answer = "agent.hooks.handlers=[]\nhook_stages=session_start,user_prompt_submit,pre_tool_use,permission_request,post_tool_use,pre_compact,post_compact,subagent_start,subagent_stop,stop,session_end\nhook_decisions=allow,deny,require_confirmation,background_wait";
    let mut delivery_messages = vec![full_answer.to_string()];
    loop_state.last_user_visible_respond = Some(full_answer.to_string());
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "最终输出必须包含机器字段 stage=pre_tool_use",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![full_answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(full_answer)
    );
}

#[test]
fn requested_machine_kv_summary_restores_service_status_terminal_delivery() {
    let task = claimed_task("task-machine-kv-summary-service-status-terminal");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "docker_basic",
        r#"{"extra":{"action":"version","available":false,"command_succeeded":false,"output":"docker unavailable: No such file or directory (os error 2)"},"text":"docker unavailable: No such file or directory (os error 2)"}"#,
    ));
    let terminal = "Docker version (read-only check)\n- status: unavailable\n- source: docker_basic (action=version)\n- command_succeeded: false\n- field_value: unavailable";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", terminal));
    let mut delivery_messages = vec!["docker.version".to_string()];
    loop_state.last_user_visible_respond = Some("docker.version".to_string());
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("docker.version".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Check Docker version read-only.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![terminal.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(terminal)
    );
}

#[test]
fn requested_machine_kv_summary_restores_service_contract_terminal_delivery() {
    let task = claimed_task("task-machine-kv-summary-service-capability-terminal");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "service_control",
        r#"{"extra":{"action":"status","target":"clawd","status":"ok","manager_type":"rustclaw","verified":true},"text":"{}"}"#,
    ));
    let terminal = "target=clawd\nstatus=ok\nmanager_type=rustclaw\nverified=true";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", terminal));
    let mut delivery_messages = vec!["service.status".to_string()];
    loop_state.last_user_visible_respond = Some("service.status".to_string());
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.response_shape = OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.selection.structured_field_selector = Some("service.status".to_string());
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Check clawd service status.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![terminal.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(terminal)
    );
}

#[test]
fn requested_machine_kv_summary_preserves_service_control_observed_field_projection() {
    let task = claimed_task("task-machine-kv-preserve-service-control-observed-fields");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "service_control",
        r#"{"extra":{"service_name":"telegramd","target":"telegramd","post_state":"telegramd=running","pre_state":"telegramd=running","status":"ok","verified":true,"manager_type":"rustclaw","summary":"Status: telegramd=running"}}"#,
    ));
    let current = concat!(
        "target=telegramd service_name=telegramd post_state=telegramd=running ",
        "pre_state=telegramd=running status=ok verified=true manager_type=rustclaw ",
        "source=service_control"
    )
    .to_string();
    let mut delivery_messages = vec![current.clone()];
    loop_state.delivery_messages = delivery_messages.clone();
    loop_state.last_user_visible_respond = Some(current.clone());
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "check whether telegramd is running right now and briefly explain the status",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![current.clone()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(current.as_str())
    );
}

#[test]
fn requested_machine_kv_summary_preserves_colon_field_value_delivery() {
    let task = claimed_task("task-machine-kv-summary-colon-fields");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"1|Archive fixtures for NL tests.\n2|This subdirectory exists so the docs directory has a nested child for directory-count and names-only prompts.","path":"/tmp/README.txt"},"text":"{\"action\":\"read_range\",\"excerpt\":\"1|Archive fixtures for NL tests.\\n2|This subdirectory exists so the docs directory has a nested child for directory-count and names-only prompts.\",\"path\":\"/tmp/README.txt\"}"}"#,
    ));
    let answer =
        "text_excerpt: \"Archive fixtures for NL tests.\"\ndetected_format: plain text (.txt)";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "synthesize_answer", answer));
    loop_state.delivery_messages.push(answer.to_string());
    loop_state.last_user_visible_respond = Some(answer.to_string());
    let mut delivery_messages = vec![answer.to_string()];
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route.response_shape = OutputResponseShape::Free;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return text_excerpt and detected_format.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![answer.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(answer)
    );
}

#[test]
fn requested_machine_kv_summary_requires_observed_non_flag_value() {
    let task = claimed_task("task-machine-kv-summary-finalizer-missing");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"extra":{"action":"read_range","excerpt":"248|must run another_guard.py"}}"#,
    ));
    let mut delivery_messages = vec!["248|must run another_guard.py".to_string()];
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Answer exactly as machine summary: required=yes script=missing_guard.py.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec!["248|must run another_guard.py"]);
    assert!(finalizer_summary.is_none());
}

#[test]
fn requested_machine_kv_summary_uses_state_patch_required_field() {
    let task = claimed_task("task-machine-kv-summary-state-patch");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.last_user_visible_respond = Some(
        "After boundary changes, run `python3 scripts/check_runtime_semantic_rewrite_boundary.py`."
            .to_string(),
    );
    let mut delivery_messages = Vec::new();
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskRequest),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "output_format": "machine_summary",
                "required_field": "required=yes script=check_runtime_semantic_rewrite_boundary.py"
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Read AGENTS.md lines 248-249.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["required=yes script=check_runtime_semantic_rewrite_boundary.py"]
    );
    assert_eq!(
        loop_state.delivery_messages,
        vec!["required=yes script=check_runtime_semantic_rewrite_boundary.py"]
    );
}

#[test]
fn requested_machine_kv_summary_replaces_prose_when_state_patch_requires_machine_fields() {
    let task = claimed_task("task-machine-kv-strict-state-patch-prose");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "git_basic",
        r#"{"extra":{"action":"repository_state","branch":"main","remotes":["origin","backup"]}}"#,
    ));
    let current = "Current repository state: branch=main, remotes include origin and backup.";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskRequest),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "output_format": "machine_summary",
                "required_machine_fields": ["branch", "remotes"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return repository machine fields.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec![r#"branch=main remotes=["origin","backup"]"#]
    );
}

#[test]
fn requested_machine_kv_summary_replaces_partial_machine_delivery_for_required_fields() {
    let task = claimed_task("task-machine-kv-strict-state-patch-partial");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"write_text","changed_count":2,"paths":["tmp/a.txt","tmp/b.txt"]}}"#,
    ));
    let mut delivery_messages = vec!["changed_count=2".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskRequest),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "output_format": "machine_summary",
                "required_machine_fields": ["changed_count", "paths"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return mutation machine fields.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec![r#"changed_count=2 paths=["tmp/a.txt","tmp/b.txt"]"#]
    );
}

#[test]
fn requested_machine_kv_summary_repairs_array_field_without_truncating_json() {
    let task = claimed_task("task-machine-kv-array-conflict");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "kb",
        r#"{"extra":{"count":2,"names":["alpha","beta"]}}"#,
    ));
    let mut delivery_messages = vec!["count: 2\nnames: alpha, beta".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return count and names only.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec![
            r#"count: 2
names: ["alpha","beta"]"#
        ]
    );
}

#[test]
fn requested_machine_kv_summary_projects_git_status_fields_from_user_request() {
    let task = claimed_task("task-git-status-machine-kv-user-request");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "git_basic",
        r#"{"extra":{"action":"status","branch":"main","changed_count":0,"field_value":{"branch":"main","changed_count":0,"paths":[],"worktree_state":"clean"},"paths":[],"worktree_state":"clean"},"text":"exit=0\n## main...origin/main\n"}"#,
    ));
    let current = "状态检查已完成，但还需要重新整理字段。";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "只返回 branch、worktree_state、changed_count 三个字段。",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["branch=main worktree_state=clean changed_count=0"]
    );
}

#[test]
fn requested_machine_kv_summary_overrides_scalar_path_when_explicit_pair_is_observed() {
    let task = claimed_task("task-machine-kv-explicit-pair-over-scalar-path");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"grep_text","matches":[{"line":242,"path":"AGENTS.md","text":"run `python3 scripts/check_no_nl_hardmatch.py` after boundary changes"}],"query":"check_no_nl_hardmatch.py","results":["AGENTS.md"],"root":"AGENTS.md"},"text":"AGENTS.md"}"#,
    ));
    let current = "AGENTS.md";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(scalar_route_result()),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Only keep no_hardmatch_guard=check_no_nl_hardmatch.py.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["no_hardmatch_guard=check_no_nl_hardmatch.py"]
    );
}

#[test]
fn requested_machine_kv_summary_projects_empty_git_paths() {
    let task = claimed_task("task-git-status-empty-paths");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "git_basic",
        r#"{"extra":{"action":"status","changed_count":0,"field_value":{"changed_count":0,"paths":[]},"paths":[]},"text":"exit=0\n## main...origin/main\n"}"#,
    ));
    let mut delivery_messages = vec!["exit=0 command=git status --porcelain".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "只返回 changed_count 和 paths。",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![r#"changed_count=0 paths=[]"#]);
}

#[test]
fn requested_machine_kv_summary_replaces_conflicting_machine_values_for_required_field() {
    let task = claimed_task("task-machine-kv-strict-conflicting-values");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"grep_text","contains_rustclaw":true}}"#,
    ));
    let mut delivery_messages = vec!["contains_rustclaw=true contains_rustclaw=false".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskRequest),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "output_format": "machine_summary",
                "required_machine_fields": ["contains_rustclaw"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return content check machine fields.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec!["contains_rustclaw=true"]);
}

#[test]
fn requested_machine_kv_summary_patches_empty_machine_field_in_rich_answer() {
    let task = claimed_task("task-machine-kv-patch-empty-field");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Usage: clawcli resume --text <TEXT> <TASK_ID>\n\nArguments:\n  <TASK_ID>  Existing task id to continue",
    ));
    let current =
        "clawcli resume is available.\n\nFields:\n- <TASK_ID>\n- --text <TEXT>\n\nresume_task_id=";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;
    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return required machine field resume_task_id.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec![
            "clawcli resume is available.\n\nFields:\n- <TASK_ID>\n- --text <TEXT>\n\nresume_task_id=<TASK_ID>"
        ]
    );
}

#[test]
fn requested_machine_kv_summary_patches_none_machine_field_in_rich_answer() {
    let task = claimed_task("task-machine-kv-patch-none-field");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Usage: clawcli resume --text <TEXT> <TASK_ID>\n\nArguments:\n  <TASK_ID>  Existing task id to continue",
    ));
    let current = "clawcli resume is available.\n\nresume_task_id=<none>";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return required machine field resume_task_id.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["clawcli resume is available.\n\nresume_task_id=<TASK_ID>"]
    );
}

#[test]
fn requested_machine_kv_summary_preserves_rich_answer_with_requested_machine_line() {
    let task = claimed_task("task-machine-kv-preserve-rich-field");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Usage: clawcli resume --text <TEXT> <TASK_ID>\n\nArguments:\n  <TASK_ID>  Existing task id to continue",
    ));
    let current = "clawcli resume is available.\n\nFields:\n- <TASK_ID>\n- --text <TEXT>\n\nresume_task_id=<TASK_ID>";
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return required machine field resume_task_id.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![current.to_string()]);
}

#[test]
fn requested_machine_kv_summary_projects_observed_field_value_over_marker_only_delivery() {
    let task = claimed_task("task-machine-kv-project-field-over-marker");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"extract_field","field_path":"llm.selected_vendor","path":"configs/config.toml","value":"minimax","value_text":"minimax","value_type":"string"}}"#,
    ));
    let rich_answer = "仅预览不会写入 configs/config.toml；llm.selected_vendor 当前值为 minimax，目标值也是 minimax，因此本次预览无实际变更、无明显风险。";
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        &serde_json::json!({
            "answer": rich_answer,
            "qualified": true,
            "publishable": true
        })
        .to_string(),
    ));
    loop_state.last_publishable_synthesis_output = Some(rich_answer.to_string());
    let mut delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "读取 configs/config.toml 的 llm.selected_vendor 当前值，并回答预览是否改变、当前值和风险。",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec!["minimax".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("minimax")
    );
}

#[test]
fn requested_machine_kv_summary_projects_exact_config_field_pair_across_languages() {
    let task = claimed_task("task-machine-kv-config-field-pair-ko");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"extract_field","exists":true,"field_path":"llm.selected_vendor","path":"configs/config.toml","resolved_field_path":"llm.selected_vendor","value":"minimax","value_text":"minimax"}}"#,
    ));
    let mut delivery_messages =
        vec!["llm.selected_vendor field_path=llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "configs/config.toml에서 llm.selected_vendor 값을 읽고 field_path와 value만 반환하세요.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec!["field_path=llm.selected_vendor value=minimax"]
    );
}

#[test]
fn requested_machine_kv_summary_prefers_observed_field_value_over_marker_only_payload() {
    let task = claimed_task("task-machine-kv-project-field-over-payload-marker");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"extract_field","exists":true,"field_path":"llm.selected_vendor","path":"configs/config.toml","value":"minimax","value_text":"minimax","value_type":"string"}}"#,
    ));
    let structured_answer = serde_json::json!({
        "current_value": "minimax",
        "field_path": "llm.selected_vendor",
        "path": "configs/config.toml",
        "status": "ok",
        "risk_count": 0,
        "risks": []
    })
    .to_string();
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        &structured_answer,
    ));
    loop_state.last_publishable_synthesis_output = Some(structured_answer.clone());
    let mut delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = delivery_messages.last().cloned();
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "读取 configs/config.toml 的 llm.selected_vendor 当前值，并回答预览是否改变、当前值和风险。",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec!["minimax".to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("minimax")
    );
}

#[test]
fn requested_machine_kv_summary_preserves_latest_rich_answer_over_stale_machine_value() {
    let task = claimed_task("task-machine-kv-preserve-latest-rich-field");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "run_cmd",
        "Usage: clawcli resume --text <TEXT> <TASK_ID>\n\nArguments:\n  <TASK_ID>  Existing task id to continue",
    ));
    let latest = "clawcli resume is available.\n\nFields:\n- task_id: <TASK_ID>\n- text: <TEXT>\n\nresume_task_id=<TASK_ID>";
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", latest));
    loop_state.last_user_visible_respond = Some(latest.to_string());
    let mut delivery_messages = vec![
        "resume_task_id=null".to_string(),
        "resume_task_id=not_applicable".to_string(),
    ];
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return required machine field resume_task_id.",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(delivery_messages, vec![latest.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(latest)
    );
}

#[test]
fn requested_machine_kv_summary_ignores_context_summary_machine_tokens() {
    let task = claimed_task("task-machine-kv-context-token");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "respond", "false"));
    let mut delivery_messages = vec!["false".to_string()];
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        context_bundle_summary: Some(
            "current_workspace_scope_from_current_request=false".to_string(),
        ),
        ..Default::default()
    };

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "return the async timeout policy fields",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));
    assert_eq!(delivery_messages, vec!["false"]);
}

#[test]
fn requested_machine_kv_summary_ignores_internal_user_request_machine_tokens() {
    let task = claimed_task("task-machine-kv-internal-user-request-token");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":0,"files":2,"total":2},"files_only":true,"names_by_kind":{"dirs":[],"files":["release_checklist.md","service_notes.md"],"other":[]},"path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs","resolved_path":"/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs","sort_by":"name"}}"#,
    ));
    let answer = "release_checklist.md\nservice_notes.md";
    loop_state.last_user_visible_respond = Some(answer.to_string());
    let mut delivery_messages = vec![answer.to_string()];
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        user_request: Some(
            "list workspace=/home/guagua/rustclaw child file names from docs".to_string(),
        ),
        ..Default::default()
    };

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "List the docs directory file names only.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));
    assert_eq!(delivery_messages, vec![answer]);
}

#[test]
fn requested_machine_kv_summary_restores_count_after_grouped_listing_render() {
    let task = claimed_task("task-machine-kv-structured-listing");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"names_by_kind":{"dirs":[],"files":["release_checklist.md","service_notes.md"],"other":[]}}}"#,
    ));
    let grouped = "files:\n- release_checklist.md\n- service_notes.md";
    loop_state.last_user_visible_respond = Some(grouped.to_string());
    let mut delivery_messages = vec![grouped.to_string()];
    let mut finalizer_summary = None;
    let mut route = free_route_result();
    route.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route),
        ..Default::default()
    };

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return names and count only.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));

    assert_eq!(
        delivery_messages,
        vec![r#"names=["release_checklist.md","service_notes.md"] count=2"#]
    );
}

#[test]
fn requested_machine_kv_summary_preserves_full_structured_contract_json() {
    let task = claimed_task("task-machine-kv-structured-contract");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    let contract = serde_json::json!({
        "schema_version": 1,
        "contract_marker": "async_job_poll_contract_dry_run",
        "adapter_result": {"type": "pending_async_job"},
        "async_timeout_policy": {"effective_deadline_ts": "min(deadline_ts,max_runtime_deadline_ts)"}
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "respond", &contract));
    loop_state.delivery_messages.push(contract.clone());
    loop_state.last_user_visible_respond = Some(contract.clone());
    let mut delivery_messages = vec![contract.clone()];
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "current_workspace_scope_from_current_request=false",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));
    assert_eq!(delivery_messages, vec![contract]);
}
