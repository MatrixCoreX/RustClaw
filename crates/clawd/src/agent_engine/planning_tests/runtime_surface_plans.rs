use super::*;

#[test]
fn hook_permission_surface_returns_pre_tool_use_machine_projection() {
    let state = test_state_with_enabled_skills(&["config_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.resolved_intent =
        "Inspect agent_hooks PreToolUse decision surface without mutation".to_string();
    let loop_state = LoopState::new(1);

    let plan = hook_permission_surface_deterministic_plan_result(
        &state,
        "inspect hook surface",
        Some(&route),
        &loop_state,
        "PreToolUse agent_hooks",
    )
    .expect("machine hook token should use deterministic surface plan");

    assert_eq!(plan.steps.len(), 2);
    let read_action = plan.steps[0].to_agent_action().expect("agent action");
    let read_args = expect_planned_call(&read_action, "config_basic", "read_fields");
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some("configs/agent_guard.toml")
    );
    let reply_action = plan.steps[1].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = reply_action else {
        panic!("expected respond action, got {reply_action:?}");
    };
    assert!(content.contains("\"stage\":\"pre_tool_use\""), "{content}");
    assert!(content.contains("\"field_value\""), "{content}");
}

#[test]
fn hook_permission_surface_collects_fields_and_valid_for_config_validation_contract() {
    let state = test_state_with_enabled_skills(&["config_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.resolved_intent =
        "Inspect agent_hooks PreToolUse decision surface without mutation".to_string();
    let loop_state = LoopState::new(1);

    let plan = hook_permission_surface_deterministic_plan_result(
        &state,
        "inspect hook surface",
        Some(&route),
        &loop_state,
        "PreToolUse agent_hooks",
    )
    .expect("config validation contract should still collect hook field evidence");

    assert_eq!(plan.steps.len(), 3);
    let fields_action = plan.steps[0].to_agent_action().expect("agent action");
    expect_planned_call(&fields_action, "config_basic", "read_fields");
    let validate_action = plan.steps[1].to_agent_action().expect("agent action");
    expect_planned_call(&validate_action, "config_basic", "validate");
}

#[test]
fn clawcli_resume_surface_reports_resume_task_id_field_token() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.resolved_intent = "Inspect clawcli resume surface".to_string();
    let loop_state = LoopState::new(1);

    let plan = clawcli_resume_surface_deterministic_plan_result(
        &state,
        "inspect clawcli resume",
        Some(&route),
        &loop_state,
        "clawcli resume",
    )
    .expect("clawcli resume machine tokens should use deterministic surface plan");

    assert_eq!(plan.steps.len(), 2);
    let cmd_action = plan.steps[0].to_agent_action().expect("agent action");
    let (tool, cmd_args) = planned_call(&cmd_action).expect("planned call");
    assert_eq!(tool, "run_cmd");
    let command = cmd_args
        .get("command")
        .and_then(Value::as_str)
        .expect("command");
    assert!(command.contains("clawcli resume --help"), "{command}");
    let reply_action = plan.steps[1].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = reply_action else {
        panic!("expected respond action, got {reply_action:?}");
    };
    assert!(content.contains("\"resume_task_id\""), "{content}");
}

#[test]
fn subagent_review_boundary_surface_uses_readonly_machine_envelope() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let plan_dir = state.skill_rt.workspace_root.join("plan");
    fs::create_dir_all(&plan_dir).expect("create plan dir");
    fs::write(plan_dir.join("current_runtime_plan.md"), "# Current Plan\n")
        .expect("write plan file");
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "AGENTS.md".to_string();
    route.resolved_intent =
        "Review AGENTS.md and plan boundary with read-only subagent".to_string();
    let loop_state = LoopState::new(1);

    let plan = subagent_review_boundary_surface_deterministic_plan_result(
        &state,
        "review runtime boundary",
        Some(&route),
        &loop_state,
        "review AGENTS.md plan",
    )
    .expect("review role token with AGENTS.md and plan should use subagent boundary surface");

    assert_eq!(plan.steps.len(), 4);
    let subagent_action = plan.steps[0].to_agent_action().expect("agent action");
    let (subagent_tool, subagent_args) = planned_call(&subagent_action).expect("planned call");
    assert_eq!(subagent_tool, "subagent");
    assert_eq!(
        subagent_args.get("role").and_then(Value::as_str),
        Some("review")
    );
    assert_eq!(
        subagent_args
            .get("context_refs")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str),
        Some("AGENTS.md")
    );
    let agents_read = plan.steps[1].to_agent_action().expect("agent action");
    let agents_args = expect_planned_call(&agents_read, "fs_basic", "read_text_range");
    assert_eq!(
        agents_args.get("path").and_then(Value::as_str),
        Some("AGENTS.md")
    );
    let plan_read = plan.steps[2].to_agent_action().expect("agent action");
    let plan_args = expect_planned_call(&plan_read, "fs_basic", "read_text_range");
    assert!(
        plan_args
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| path.starts_with("plan/") && path.ends_with(".md")),
        "{plan_args}"
    );
    let reply_action = plan.steps[3].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = reply_action else {
        panic!("expected respond action, got {reply_action:?}");
    };
    assert!(content.contains("\"boundary\""), "{content}");
    assert!(content.contains("\"write_enabled\":false"), "{content}");
    assert!(
        content.contains("\"external_publish_enabled\":false"),
        "{content}"
    );
    assert!(
        content.contains("\"execution_mode\":\"inline_readonly_child_run\""),
        "{content}"
    );
    assert!(
        content.contains("\"child_worker_status\":\"inline_completed\""),
        "{content}"
    );
}
