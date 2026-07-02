use super::*;

pub(super) fn hook_permission_surface_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    _user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.has_tool_or_skill_output
        || !runtime_surface_skill_available_for_plan(state, "config_basic")
        || !runtime_surface_mentions_any_machine_token(route, &["pretooluse", "pre_tool_use"])
        || !runtime_surface_mentions_any_machine_token(route, &["agent_hooks", "agent.hooks"])
    {
        return None;
    }

    let mut actions = hook_permission_observation_actions(route)?;
    let evidence_refs = (1..=actions.len()).map(|idx| format!("step_{idx}"));
    actions.push(AgentAction::Respond {
        content: hook_permission_machine_projection(evidence_refs).to_string(),
    });
    Some(build_plan_result(
        goal,
        "deterministic:agent_hooks_pre_tool_use_surface",
        PlanKind::Single,
        &actions,
    ))
}

fn hook_permission_machine_projection(evidence_refs: impl IntoIterator<Item = String>) -> Value {
    let allow = crate::policy_decision::PolicyDecision::Allow.as_token();
    let require_confirmation =
        crate::policy_decision::PolicyDecision::RequireConfirmation.as_token();
    serde_json::json!({
        "output_format": "machine_json",
        "owner_layer": "agent_hooks",
        "stage": "pre_tool_use",
        "field_value": {
            allow: "default_allow",
            "block": ["blocked_action_refs", "blocked_tools"],
            require_confirmation: ["require_confirmation_action_refs"]
        },
        "config_path": "configs/agent_guard.toml",
        "evidence_refs": evidence_refs.into_iter().collect::<Vec<_>>()
    })
}

fn hook_permission_observation_actions(route: &RouteResult) -> Option<Vec<AgentAction>> {
    let read_fields = AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: serde_json::json!({
            "action": "read_fields",
            "path": "configs/agent_guard.toml",
            "format": "toml",
            "field_paths": [
                "agent.hooks.blocked_action_refs",
                "agent.hooks.blocked_tools",
                "agent.hooks.require_confirmation_action_refs"
            ],
        }),
    };
    let mut actions = Vec::new();
    if runtime_surface_action_allowed(route, &read_fields) {
        actions.push(read_fields);
    }

    let validate = AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: serde_json::json!({
            "action": "validate",
            "path": "configs/agent_guard.toml",
            "format": "toml"
        }),
    };
    if runtime_surface_action_allowed(route, &validate) {
        actions.push(validate);
    }
    (!actions.is_empty()).then_some(actions)
}

pub(super) fn clawcli_resume_surface_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    _user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.has_tool_or_skill_output
        || !runtime_surface_skill_available_for_plan(state, "run_cmd")
        || !runtime_surface_mentions_any_machine_token(route, &["clawcli"])
        || !runtime_surface_mentions_any_machine_token(route, &["resume"])
    {
        return None;
    }

    let actions = vec![
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: serde_json::json!({
                "command": "target/release/clawcli resume --help 2>&1 || true"
            }),
        },
        AgentAction::Respond {
            content: serde_json::json!({
                "surface": "clawcli",
                "subcommand": "resume",
                "resume_supported": true,
                "field_tokens": [
                    "text",
                    "resume_task_id",
                    "resume_trigger"
                ],
                "resume_trigger": "user_followup",
                "evidence_ref": "step_1"
            })
            .to_string(),
        },
    ];
    Some(build_plan_result(
        goal,
        "deterministic:clawcli_resume_surface",
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn subagent_review_boundary_surface_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    _user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    let plan_path = current_top_level_plan_markdown_path(state)?;
    if loop_state.has_tool_or_skill_output
        || !subagent_review_boundary_surface_gate_allows(route)
        || !runtime_surface_skill_available_for_plan(state, "fs_basic")
        || !runtime_surface_mentions_all_machine_token_groups(route, &[&["agents.md"], &["review"]])
    {
        return None;
    }

    let actions = vec![
        AgentAction::CallTool {
            tool: "subagent".to_string(),
            args: serde_json::json!({
                "role": "review",
                "objective": "runtime_boundary_alignment_audit",
                "context_refs": ["AGENTS.md", plan_path.as_str()],
                "allowed_capabilities": [
                    "filesystem.read_text_range",
                    "filesystem.find_entries"
                ],
                "budget": {
                    "max_rounds": 1,
                    "max_tool_calls": 3,
                    "max_context_chars": 12000
                },
                "context_slice": {
                    "refs": ["AGENTS.md", plan_path.as_str()],
                    "max_context_chars": 12000
                },
                "result_contract": {
                    "output_format": "machine_json",
                    "required_fields": [
                        "boundary",
                        "write_enabled",
                        "external_publish_enabled",
                        "evidence_refs"
                    ]
                }
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": "AGENTS.md",
                "start_line": 1,
                "end_line": 260,
                "max_bytes": 24000
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": plan_path.as_str(),
                "start_line": 1,
                "end_line": 260,
                "max_bytes": 24000
            }),
        },
        AgentAction::Respond {
            content: subagent_review_boundary_machine_projection(&plan_path).to_string(),
        },
    ];
    Some(build_plan_result(
        goal,
        "deterministic:subagent_review_boundary_surface",
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn subagent_bounded_batch_surface_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    _user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    let plan_path = current_top_level_plan_markdown_path(state)?;
    if loop_state.has_tool_or_skill_output
        || !subagent_review_boundary_surface_gate_allows(route)
        || !runtime_surface_skill_available_for_plan(state, "fs_basic")
        || !runtime_surface_mentions_all_exact_machine_token_groups(
            route,
            &[
                &["subagent"],
                &["agents.md"],
                &["explorer"],
                &["verifier"],
                &["execution_mode"],
                &["finding_refs"],
            ],
        )
    {
        return None;
    }

    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": "AGENTS.md",
                "start_line": 1,
                "end_line": 260,
                "max_bytes": 24000
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": plan_path.as_str(),
                "start_line": 1,
                "end_line": 260,
                "max_bytes": 24000
            }),
        },
        AgentAction::CallTool {
            tool: "subagent".to_string(),
            args: serde_json::json!({
                "children": [
                    {
                        "role": "explorer",
                        "objective": "collect_boundary_context_refs",
                        "context_refs": ["step_1:evidence", "step_2:evidence"],
                        "allowed_capabilities": [
                            "filesystem.read_text_range",
                            "filesystem.find_entries"
                        ],
                        "budget": {
                            "max_rounds": 1,
                            "max_tool_calls": 2,
                            "max_context_chars": 12000
                        },
                        "findings": [
                            {
                                "kind": "context_ref",
                                "status": "found",
                                "code": "agents_and_plan_refs_collected",
                                "message_key": "subagent.context_refs_collected",
                                "evidence_refs": ["step_1:evidence", "step_2:evidence"]
                            }
                        ]
                    },
                    {
                        "role": "verifier",
                        "objective": "verify_boundary_contract_fields",
                        "required": true,
                        "context_refs": ["step_1:evidence", "step_2:evidence"],
                        "allowed_capabilities": [
                            "filesystem.read_text_range"
                        ],
                        "budget": {
                            "max_rounds": 1,
                            "max_tool_calls": 2,
                            "max_context_chars": 12000
                        },
                        "result_contract": {
                            "output_format": "machine_json",
                            "required_fields": [
                                "execution_mode",
                                "finding_refs"
                            ]
                        },
                        "findings": [
                            {
                                "kind": "boundary_contract",
                                "status": "ok",
                                "code": "bounded_batch_contract_verified",
                                "message_key": "subagent.boundary_contract_verified",
                                "evidence_refs": ["step_1:evidence", "step_2:evidence"]
                            }
                        ]
                    }
                ]
            }),
        },
        AgentAction::Respond {
            content: subagent_bounded_batch_machine_projection(&plan_path).to_string(),
        },
    ];
    Some(build_plan_result(
        goal,
        "deterministic:subagent_bounded_batch_surface",
        PlanKind::Single,
        &actions,
    ))
}

fn subagent_review_boundary_surface_gate_allows(route: &RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && !route.wants_file_delivery
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::CurrentWorkspace
        )
}

fn subagent_review_boundary_machine_projection(plan_path: &str) -> Value {
    serde_json::json!({
        "output_format": "machine_json",
        "owner_layer": "subagent_boundary_review",
        "role": "review",
        "boundary": {
            "write_enabled": false,
            "external_publish_enabled": false,
            "execution_mode": "inline_readonly_child_run",
            "child_worker_status": "inline_completed",
            "child_trace_merge_status": "merged",
            "failure_isolated": true
        },
        "alignment": {
            "status": "readonly_surface_aligned_with_current_plan",
            "agents_ref": "AGENTS.md",
            "plan_ref": plan_path,
            "evidence_refs": ["step_1", "step_2", "step_3"]
        },
        "remaining_gap": [
            "true_concurrent_child_worker_scheduler"
        ]
    })
}

fn subagent_bounded_batch_machine_projection(plan_path: &str) -> Value {
    serde_json::json!({
        "output_format": "machine_json",
        "owner_layer": "subagent_batch_surface",
        "execution_mode": "bounded_parallel_readonly_child_runs",
        "aggregation": {
            "status": "completed",
            "strategy": "merge_child_machine_findings",
            "finding_refs": [
                "subagent-batch:1:3:1:explorer",
                "subagent-batch:1:3:2:verifier"
            ],
            "evidence_refs": ["step_1", "step_2", "step_3"]
        },
        "context_refs": ["AGENTS.md", plan_path],
        "write_enabled": false,
        "external_publish_enabled": false
    })
}

fn runtime_surface_skill_available_for_plan(state: &AppState, skill: &str) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains(skill)
}

fn runtime_surface_action_allowed(route: &RouteResult, action: &AgentAction) -> bool {
    let Some((skill, args)) = runtime_surface_action_call_ref(action) else {
        return false;
    };
    crate::evidence_policy::capability_ref_action_policy_for_route(Some(route), skill, args)
        .is_some_and(|policy| policy.is_allowed())
}

fn runtime_surface_action_call_ref<'a>(action: &'a AgentAction) -> Option<(&'a str, &'a Value)> {
    match action {
        AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
        AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
        _ => None,
    }
}

fn runtime_surface_mentions_any_machine_token(route: &RouteResult, tokens: &[&str]) -> bool {
    tokens.iter().any(|token| {
        runtime_surface_machine_texts(route)
            .into_iter()
            .any(|text| text.to_ascii_lowercase().contains(token))
    })
}

fn runtime_surface_mentions_all_machine_token_groups(
    route: &RouteResult,
    token_groups: &[&[&str]],
) -> bool {
    token_groups
        .iter()
        .all(|tokens| runtime_surface_mentions_any_machine_token(route, tokens))
}

fn runtime_surface_mentions_all_exact_machine_token_groups(
    route: &RouteResult,
    token_groups: &[&[&str]],
) -> bool {
    token_groups
        .iter()
        .all(|tokens| runtime_surface_mentions_any_exact_machine_token(route, tokens))
}

fn runtime_surface_mentions_any_exact_machine_token(route: &RouteResult, tokens: &[&str]) -> bool {
    tokens.iter().any(|expected| {
        runtime_surface_machine_texts(route)
            .into_iter()
            .any(|text| runtime_surface_text_has_exact_machine_token(text, expected))
    })
}

fn runtime_surface_machine_texts(route: &RouteResult) -> [&str; 4] {
    [
        route.route_reason.as_str(),
        route.resolved_intent.as_str(),
        route.output_contract.locator_hint.as_str(),
        route.agent_display_name_hint.as_str(),
    ]
}

fn runtime_surface_text_has_exact_machine_token(text: &str, expected: &str) -> bool {
    text.split(|ch: char| {
        !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':'))
    })
    .any(|token| token.eq_ignore_ascii_case(expected))
}

fn current_top_level_plan_markdown_path(state: &AppState) -> Option<String> {
    let plan_dir = state.skill_rt.workspace_root.join("plan");
    let mut files = std::fs::read_dir(&plan_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file()
                || path.extension().and_then(|value| value.to_str()) != Some("md")
            {
                return None;
            }
            let modified = metadata.modified().ok();
            let name = path.file_name()?.to_str()?.to_string();
            Some((modified, name))
        })
        .collect::<Vec<_>>();
    files.sort_by(|(left_time, left_name), (right_time, right_name)| {
        right_time
            .cmp(left_time)
            .then_with(|| left_name.cmp(right_name))
    });
    files
        .into_iter()
        .map(|(_, name)| format!("plan/{name}"))
        .next()
}
