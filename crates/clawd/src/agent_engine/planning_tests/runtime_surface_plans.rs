use super::*;

fn normalize_runtime_surface_actions(
    state: &AppState,
    route: &RouteResult,
    loop_state: &LoopState,
    goal: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    normalize_planned_actions(state, Some(route), loop_state, goal, None, actions)
}

fn subagent_review_surface_actions(plan_path: &str) -> Vec<AgentAction> {
    vec![
        AgentAction::CallTool {
            tool: "subagent".to_string(),
            args: json!({
                "role": "review",
                "objective": "runtime_boundary_alignment_audit",
                "context_refs": ["AGENTS.md", plan_path],
                "allowed_capabilities": [
                    "filesystem.read_text_range",
                    "filesystem.find_entries"
                ],
                "budget": {
                    "max_rounds": 1,
                    "max_tool_calls": 3,
                    "max_context_chars": 12000
                }
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "AGENTS.md",
                "start_line": 1,
                "end_line": 260,
                "max_bytes": 24000
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": plan_path,
                "start_line": 1,
                "end_line": 260,
                "max_bytes": 24000
            }),
        },
        AgentAction::Respond {
            content: json!({
                "boundary": {
                    "write_enabled": false,
                    "external_publish_enabled": false,
                    "execution_mode": "inline_readonly_child_run",
                    "child_worker_status": "inline_completed"
                },
                "evidence_refs": ["step_1", "step_2", "step_3"]
            })
            .to_string(),
        },
    ]
}

fn subagent_bounded_batch_surface_actions(plan_path: &str) -> Vec<AgentAction> {
    vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "AGENTS.md",
                "start_line": 1,
                "end_line": 260,
                "max_bytes": 24000
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": plan_path,
                "start_line": 1,
                "end_line": 260,
                "max_bytes": 24000
            }),
        },
        AgentAction::CallTool {
            tool: "subagent".to_string(),
            args: json!({
                "children": [
                    {"role": "explorer", "objective": "collect_boundary_context_refs"},
                    {"role": "verifier", "objective": "verify_boundary_contract_fields"}
                ]
            }),
        },
        AgentAction::Respond {
            content: json!({
                "execution_mode": "bounded_parallel_readonly_child_runs",
                "finding_refs": ["step_1:evidence", "step_2:evidence"],
                "external_publish_enabled": false
            })
            .to_string(),
        },
    ]
}

#[test]
fn open_planning_tool_spec_includes_runtime_protocols() {
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    state.skill_rt.workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task = test_task();
    let library =
        PlannerToolLibrary::new(&state, &task, PlanningPromptClass::OpenPlanning, None, None);

    let spec = library.tool_spec().expect("open planning tool spec");

    assert!(spec.starts_with("runtime_capability_map_v1"));
    assert!(spec.contains("agent_runtime_protocols=subagent_roles:"));
    assert!(spec.contains("subagent_write_enabled:false"));
    assert!(spec.contains("### Agent runtime protocols"));
}

#[test]
fn lightweight_tool_spec_includes_runtime_protocols() {
    let spec = build_lightweight_tool_spec(None, None);

    assert!(spec.contains("agent_runtime_protocols=subagent_roles:"));
    assert!(spec.contains("subagent_write_enabled:false"));
    assert!(spec.contains("async_job_protocol="));
}

#[test]
fn subagent_internal_runtime_tool_does_not_require_skill_switch() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let actions = vec![AgentAction::CallTool {
        tool: "subagent".to_string(),
        args: json!({
            "children": [
                {"role": "explorer", "objective": "collect_runtime_surface"},
                {"role": "verifier", "objective": "verify_runtime_surface"}
            ]
        }),
    }];

    assert!(!contains_unavailable_skill_action(&state, &actions));
}

#[test]
fn unknown_runtime_tool_still_requires_skill_switch() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let actions = vec![AgentAction::CallTool {
        tool: "unknown_runtime_tool".to_string(),
        args: json!({}),
    }];

    assert!(contains_unavailable_skill_action(&state, &actions));
}

fn find_planned_call<'a>(actions: &'a [AgentAction], name: &str, action_name: &str) -> &'a Value {
    actions
        .iter()
        .find(|action| planned_call_is(action, name, action_name))
        .map(|action| expect_planned_call(action, name, action_name))
        .unwrap_or_else(|| panic!("expected {name}.{action_name} in {actions:?}"))
}

fn find_tool_call<'a>(actions: &'a [AgentAction], name: &str) -> &'a Value {
    actions
        .iter()
        .find_map(|action| {
            let (tool, args) = planned_call(action)?;
            (tool == name).then_some(args)
        })
        .unwrap_or_else(|| panic!("expected {name} call in {actions:?}"))
}

#[test]
fn hook_permission_surface_returns_pre_tool_use_machine_projection() {
    let state = test_state_with_enabled_skills(&["config_basic"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::respond_trace();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.route_reason = "surface=agent_hooks stage=pre_tool_use".to_string();
    let loop_state = LoopState::new(1);

    let normalized = normalize_runtime_surface_actions(
        &state,
        &route,
        &loop_state,
        "inspect hook surface",
        vec![
            AgentAction::CallTool {
                tool: "config_basic".to_string(),
                args: json!({
                    "action": "read_fields",
                    "path": "configs/agent_guard.toml",
                    "format": "toml",
                    "field_paths": [
                        "agent.hooks.blocked_action_refs",
                        "agent.hooks.blocked_tools",
                        "agent.hooks.require_confirmation_action_refs",
                        "agent.hooks.background_wait_action_refs"
                    ]
                }),
            },
            AgentAction::Respond {
                content: json!({
                    "stage": "pre_tool_use",
                    "field_value": {
                        "allow": "default_allow",
                        "deny": ["blocked_action_refs", "blocked_tools"],
                        "require_confirmation": "require_confirmation_action_refs",
                        "background_wait": "background_wait_action_refs"
                    },
                    "evidence_refs": ["step_1"]
                })
                .to_string(),
            },
        ],
    );

    let read_args = find_planned_call(&normalized, "config_basic", "read_fields");
    assert_eq!(
        read_args.get("path").and_then(Value::as_str),
        Some("configs/agent_guard.toml")
    );
}

#[test]
fn hook_permission_surface_collects_fields_and_valid_for_config_validation_contract() {
    let state = test_state_with_enabled_skills(&["config_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route.route_reason = "surface=agent_hooks stage=pre_tool_use".to_string();
    let loop_state = LoopState::new(1);

    let normalized = normalize_runtime_surface_actions(
        &state,
        &route,
        &loop_state,
        "inspect hook surface",
        vec![
            AgentAction::CallTool {
                tool: "config_basic".to_string(),
                args: json!({
                    "action": "read_fields",
                    "path": "configs/agent_guard.toml",
                    "format": "toml",
                    "field_paths": [
                        "agent.hooks.blocked_action_refs",
                        "agent.hooks.blocked_tools",
                        "agent.hooks.require_confirmation_action_refs",
                        "agent.hooks.background_wait_action_refs"
                    ]
                }),
            },
            AgentAction::CallTool {
                tool: "config_basic".to_string(),
                args: json!({
                    "action": "validate",
                    "path": "configs/agent_guard.toml",
                    "format": "toml"
                }),
            },
            AgentAction::Respond {
                content: json!({"stage": "pre_tool_use", "evidence_refs": ["step_1", "step_2"]})
                    .to_string(),
            },
        ],
    );

    find_planned_call(&normalized, "config_basic", "read_fields");
    find_planned_call(&normalized, "config_basic", "validate");
}

#[test]
fn clawcli_resume_surface_uses_planner_supplied_resume_help_call() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::respond_trace();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.route_reason = "surface=clawcli subcommand=resume".to_string();
    let loop_state = LoopState::new(1);

    let normalized = normalize_runtime_surface_actions(
        &state,
        &route,
        &loop_state,
        "inspect clawcli resume",
        vec![
            AgentAction::CallTool {
                tool: "run_cmd".to_string(),
                args: json!({
                    "command": "target/release/clawcli resume --help 2>&1 || true"
                }),
            },
            AgentAction::Respond {
                content: json!({
                    "surface": "clawcli",
                    "subcommand": "resume",
                    "field_tokens": ["text", "resume_task_id", "resume_trigger"],
                    "evidence_ref": "step_1"
                })
                .to_string(),
            },
        ],
    );

    let cmd_args = find_tool_call(&normalized, "run_cmd");
    assert_eq!(
        cmd_args.get("action").and_then(Value::as_str),
        Some("inspect_cli_help")
    );
    let command = cmd_args
        .get("command")
        .and_then(Value::as_str)
        .expect("command");
    assert!(command.contains("clawcli resume --help"), "{command}");
    assert_eq!(
        cmd_args
            .get("timeout_seconds")
            .and_then(serde_json::Value::as_i64),
        Some(10)
    );
}

#[test]
fn clawcli_resume_surface_ignores_user_text_tokens_without_machine_tokens() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    let loop_state = LoopState::new(1);

    let normalized = normalize_runtime_surface_actions(
        &state,
        &route,
        &loop_state,
        "inspect clawcli resume",
        vec![],
    );

    assert!(normalized.is_empty());
}

#[test]
fn clawcli_resume_required_machine_field_replaces_broad_probe_with_help_call() {
    let state = test_state_with_enabled_skills(&["run_cmd"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    let loop_state = LoopState::new(1);

    let normalized = normalize_runtime_surface_actions(
        &state,
        &route,
        &loop_state,
        "clawcli resume resume_task_id",
        vec![
            AgentAction::CallTool {
                tool: "run_cmd".to_string(),
                args: json!({
                    "command": "which clawcli 2>/dev/null; ls -la /home/guagua/rustclaw 2>/dev/null | head -50; find /home/guagua/rustclaw -maxdepth 3 -name 'clawcli*' -o -name 'CLAWCLI*' 2>/dev/null | head -20",
                    "timeout_seconds": 15,
                    "max_output_bytes": 8192
                }),
            },
            AgentAction::CallTool {
                tool: "run_cmd".to_string(),
                args: json!({
                    "command": "ls /home/guagua/rustclaw/crates 2>/dev/null; ls /home/guagua/rustclaw/src 2>/dev/null; ls /home/guagua/rustclaw/bin 2>/dev/null; find /home/guagua/rustclaw -maxdepth 4 -type d -name 'cli' 2>/dev/null | head -10",
                    "timeout_seconds": 15,
                    "max_output_bytes": 8192
                }),
            },
        ],
    );

    assert_eq!(normalized.len(), 3, "{normalized:?}");
    let cmd_args = find_tool_call(&normalized, "run_cmd");
    assert_eq!(
        cmd_args.get("action").and_then(Value::as_str),
        Some("inspect_cli_help")
    );
    let command = cmd_args
        .get("command")
        .and_then(Value::as_str)
        .expect("command");
    assert_eq!(command, "scripts/clawcli.sh resume --help");
    assert!(!command.contains("/src"), "{command}");
    assert!(matches!(
        normalized.get(1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ));
    assert!(matches!(
        normalized.get(2),
        Some(AgentAction::Respond { content }) if content == "{{last_output}}"
    ));
}

#[test]
fn subagent_review_boundary_surface_uses_readonly_machine_envelope() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let plan_dir = state.skill_rt.workspace_root.join("plan");
    fs::create_dir_all(&plan_dir).expect("create plan dir");
    fs::write(plan_dir.join("current_runtime_plan.md"), "# Current Plan\n")
        .expect("write plan file");
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::respond_trace();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "AGENTS.md".to_string();
    route.route_reason = "surface=subagent role=review context_ref=AGENTS.md".to_string();
    let loop_state = LoopState::new(1);

    let normalized = normalize_runtime_surface_actions(
        &state,
        &route,
        &loop_state,
        "review runtime boundary",
        subagent_review_surface_actions("plan/current_runtime_plan.md"),
    );

    let subagent_args = find_tool_call(&normalized, "subagent");
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
    let agents_args = normalized
        .iter()
        .filter(|action| planned_call_is(action, "fs_basic", "read_text_range"))
        .map(|action| expect_planned_call(action, "fs_basic", "read_text_range"))
        .find(|args| args.get("path").and_then(Value::as_str) == Some("AGENTS.md"))
        .expect("AGENTS.md read");
    assert_eq!(
        agents_args.get("path").and_then(Value::as_str),
        Some("AGENTS.md")
    );
    let plan_args = normalized
        .iter()
        .filter(|action| planned_call_is(action, "fs_basic", "read_text_range"))
        .map(|action| expect_planned_call(action, "fs_basic", "read_text_range"))
        .find(|args| {
            args.get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| path.starts_with("plan/") && path.ends_with(".md"))
        })
        .expect("plan read");
    assert!(
        plan_args
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| path.starts_with("plan/") && path.ends_with(".md")),
        "{plan_args}"
    );
}

#[test]
fn subagent_bounded_batch_surface_uses_children_contract() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let plan_dir = state.skill_rt.workspace_root.join("plan");
    fs::create_dir_all(&plan_dir).expect("create plan dir");
    fs::write(plan_dir.join("current_runtime_plan.md"), "# Current Plan\n")
        .expect("write plan file");
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    route.output_contract.locator_hint = "AGENTS.md".to_string();
    route.route_reason =
        "subagent context_ref=AGENTS.md explorer verifier execution_mode finding_refs current_workspace_scope"
            .to_string();
    let loop_state = LoopState::new(1);

    let normalized = normalize_runtime_surface_actions(
        &state,
        &route,
        &loop_state,
        "subagent batch surface",
        subagent_bounded_batch_surface_actions("plan/current_runtime_plan.md"),
    );

    let subagent_args = find_tool_call(&normalized, "subagent");
    let children = subagent_args
        .get("children")
        .and_then(Value::as_array)
        .expect("children");
    assert_eq!(children.len(), 2);
    assert_eq!(
        children[0].get("role").and_then(Value::as_str),
        Some("explorer")
    );
    assert_eq!(
        children[1].get("role").and_then(Value::as_str),
        Some("verifier")
    );
}

#[test]
fn subagent_review_boundary_surface_resolves_current_plan_when_route_requested_clarify() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let plan_dir = state.skill_rt.workspace_root.join("plan");
    fs::create_dir_all(&plan_dir).expect("create plan dir");
    fs::write(plan_dir.join("current_runtime_plan.md"), "# Current Plan\n")
        .expect("write plan file");
    let mut route = base_route_result();
    route.ask_mode = AskMode::clarify_trace();
    route.needs_clarify = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint.clear();
    route.route_reason =
        "reason_code=missing_locator source=current_plan_boundary_surface context_ref=AGENTS.md role=review"
            .to_string();
    let loop_state = LoopState::new(1);

    let normalized = normalize_runtime_surface_actions(
        &state,
        &route,
        &loop_state,
        "review runtime boundary",
        subagent_review_surface_actions("plan/current_runtime_plan.md"),
    );

    let plan_args = normalized
        .iter()
        .filter(|action| planned_call_is(action, "fs_basic", "read_text_range"))
        .map(|action| expect_planned_call(action, "fs_basic", "read_text_range"))
        .find(|args| {
            args.get("path").and_then(Value::as_str) == Some("plan/current_runtime_plan.md")
        })
        .expect("current plan read");
    assert_eq!(
        plan_args.get("path").and_then(Value::as_str),
        Some("plan/current_runtime_plan.md")
    );
}

#[test]
fn subagent_review_boundary_surface_uses_current_plan_without_plan_text_token() {
    let state = test_state_with_enabled_skills(&["fs_basic"]);
    let plan_dir = state.skill_rt.workspace_root.join("plan");
    fs::create_dir_all(&plan_dir).expect("create plan dir");
    fs::write(plan_dir.join("current_runtime_plan.md"), "# Current Plan\n")
        .expect("write plan file");
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_hint = "AGENTS.md".to_string();
    route.agent_display_name_hint = "review".to_string();
    route.route_reason =
        "subagent_roles=review; current_workspace_scope_from_current_request".to_string();
    let loop_state = LoopState::new(1);

    let normalized = normalize_runtime_surface_actions(
        &state,
        &route,
        &loop_state,
        "review runtime boundary",
        subagent_review_surface_actions("plan/current_runtime_plan.md"),
    );

    let plan_args = normalized
        .iter()
        .filter(|action| planned_call_is(action, "fs_basic", "read_text_range"))
        .map(|action| expect_planned_call(action, "fs_basic", "read_text_range"))
        .find(|args| {
            args.get("path").and_then(Value::as_str) == Some("plan/current_runtime_plan.md")
        })
        .expect("current plan read");
    assert_eq!(
        plan_args.get("path").and_then(Value::as_str),
        Some("plan/current_runtime_plan.md")
    );
}
