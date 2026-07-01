use super::*;

pub(super) fn preferred_structured_action_for_contract_hint(
    state: &AppState,
    route: &RouteResult,
    preferred: &crate::contract_matrix::ActionRef,
    auto_locator_path: Option<&str>,
    original_user_text: &str,
) -> Option<AgentAction> {
    match preferred.skill.as_str() {
        "run_cmd" if run_cmd_available_for_plan(state) => {
            preferred_run_cmd_for_contract_hint(state, route, auto_locator_path)
        }
        "package_manager" if package_manager_available_for_plan(state) => {
            Some(AgentAction::CallSkill {
                skill: "package_manager".to_string(),
                args: serde_json::json!({"action": preferred.action.as_deref().unwrap_or("detect")}),
            })
        }
        "fs_basic" => preferred_fs_basic_for_contract_hint(
            state,
            route,
            preferred.action.as_deref().unwrap_or("stat_paths"),
            auto_locator_path,
            original_user_text,
        ),
        "doc_parse" if doc_parse_is_enabled(state) => {
            let path = first_route_locator_target(route, auto_locator_path)?;
            if !doc_parse_supported_path(&path) {
                return None;
            }
            Some(AgentAction::CallSkill {
                skill: "doc_parse".to_string(),
                args: serde_json::json!({
                    "action": preferred.action.as_deref().unwrap_or("parse_doc"),
                    "path": path,
                    "max_chars": 12000,
                    "include_metadata": true,
                }),
            })
        }
        "config_basic" => preferred_config_basic_for_contract_hint(
            route,
            preferred.action.as_deref(),
            auto_locator_path,
        ),
        "config_edit" => preferred_config_edit_for_contract_hint(
            route,
            preferred.action.as_deref(),
            auto_locator_path,
        ),
        "config_guard" => {
            preferred_config_edit_for_contract_hint(route, Some("guard_config"), auto_locator_path)
        }
        "archive_basic" => preferred_archive_basic_for_contract_hint(
            state,
            route,
            preferred.action.as_deref(),
            auto_locator_path,
            original_user_text,
        ),
        "health_check" if health_check_available_for_plan(state) => Some(AgentAction::CallSkill {
            skill: "health_check".to_string(),
            args: serde_json::json!({}),
        }),
        "process_basic" if process_basic_available_for_plan(state) => {
            Some(AgentAction::CallSkill {
                skill: "process_basic".to_string(),
                args: serde_json::json!({
                    "action": preferred.action.as_deref().unwrap_or("ps"),
                    "limit": 200,
                    "filter": process_status_contract_filter_token(route)
                        .unwrap_or_else(|| "clawd".to_string()),
                }),
            })
        }
        "service_control" => Some(AgentAction::CallSkill {
            skill: "service_control".to_string(),
            args: serde_json::json!({
                "action": preferred.action.as_deref().unwrap_or("status"),
                "target": process_status_contract_filter_token(route)
                    .unwrap_or_else(|| "clawd".to_string()),
                "manager_type": "rustclaw",
            }),
        }),
        "git_basic" if git_basic_available_for_plan(state) => {
            if route.output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary) {
                preferred_fs_basic_for_contract_hint(
                    state,
                    route,
                    "read_text_range",
                    auto_locator_path,
                    original_user_text,
                )
            } else {
                let action = if route
                    .output_contract_marker_is(crate::OutputSemanticKind::GitCommitSubject)
                {
                    "log"
                } else if route
                    .output_contract_marker_is(crate::OutputSemanticKind::GitRepositoryState)
                {
                    "status"
                } else if route
                    .output_contract_marker_is(crate::OutputSemanticKind::RecentScalarEqualityCheck)
                {
                    "current_branch"
                } else {
                    preferred.action.as_deref().unwrap_or("status")
                };
                Some(AgentAction::CallSkill {
                    skill: "git_basic".to_string(),
                    args: serde_json::json!({
                        "action": action,
                    }),
                })
            }
        }
        "db_basic" => {
            if !route.output_contract_marker_is_any(&[
                crate::OutputSemanticKind::SqliteSchemaVersion,
                crate::OutputSemanticKind::SqliteTableListing,
                crate::OutputSemanticKind::SqliteTableNamesOnly,
                crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
            ]) {
                return None;
            }
            let db_path = first_route_locator_target(route, auto_locator_path)?;
            let action = if route
                .output_contract_marker_is(crate::OutputSemanticKind::SqliteSchemaVersion)
            {
                "schema_version"
            } else if route.output_contract_marker_is_any(&[
                crate::OutputSemanticKind::SqliteTableListing,
                crate::OutputSemanticKind::SqliteTableNamesOnly,
                crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
            ]) {
                "list_tables"
            } else {
                preferred.action.as_deref().unwrap_or("list_tables")
            };
            Some(AgentAction::CallSkill {
                skill: "db_basic".to_string(),
                args: serde_json::json!({
                    "action": action,
                    "db_path": db_path,
                }),
            })
        }
        "docker_basic" if docker_basic_available_for_plan(state) => Some(AgentAction::CallSkill {
            skill: "docker_basic".to_string(),
            args: serde_json::json!({
                "action": preferred_docker_basic_action(route, preferred),
            }),
        }),
        _ => None,
    }
}

fn preferred_docker_basic_action(
    route: &RouteResult,
    preferred: &crate::contract_matrix::ActionRef,
) -> &'static str {
    if let Some(action) = preferred.action.as_deref() {
        return match action {
            "images" => "images",
            "logs" => "logs",
            "inspect" => "inspect",
            "start" => "start",
            "stop" => "stop",
            "restart" => "restart",
            "version" => "version",
            _ => "ps",
        };
    }
    if let Some(action) = route_capability_action_for_namespaces(route, &["docker"]) {
        return docker_basic_action_from_capability_action(action);
    }
    "ps"
}

pub(super) fn route_has_contract_hint_context(
    route: &RouteResult,
    original_user_text: &str,
) -> bool {
    crate::intent_router::contract_test_hint_semantic_kind(original_user_text).is_some()
        || crate::intent_router::contract_test_hint_value(
            original_user_text,
            "preferred_action_ref",
        )
        .is_some()
        || route.has_route_reason_machine_marker("contract_hint_fast_path")
}

pub(super) fn contract_hint_existence_summary_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::ExistenceWithPathSummary) {
        return None;
    }
    let target = first_route_locator_target(route, auto_locator_path)?;
    let read_path =
        preferred_read_text_range_path_for_contract_hint(&target, &state.skill_rt.workspace_root)
            .unwrap_or_else(|| target.clone());
    let observations = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "stat_paths",
                "paths": [target],
                "include_missing": true,
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": read_path,
                "mode": "head",
                "n": 80,
            }),
        },
    ];
    if !observations.iter().all(|action| {
        let Some((skill, args)) = planned_execution_action_ref(action) else {
            return false;
        };
        crate::contract_matrix::action_policy_for_route(Some(route), skill, args)
            .is_some_and(|policy| policy.is_allowed())
    }) {
        return None;
    }
    let mut actions = observations;
    actions.extend([
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ]);
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn contract_hint_preferred_action_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    original_user_text: &str,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
    {
        return None;
    }
    if !route_has_contract_hint_context(route, original_user_text) {
        return None;
    }
    if let Some(plan_result) = contract_hint_existence_summary_deterministic_plan_result(
        state,
        goal,
        route,
        auto_locator_path,
    ) {
        return Some(plan_result);
    }
    let preferred_actions =
        if let Some(preferred) = contract_hint_preferred_action_ref(original_user_text) {
            vec![preferred]
        } else {
            crate::contract_matrix::preferred_action_refs_for_route(route)
        };
    for preferred in preferred_actions {
        let Some(action) = preferred_structured_action_for_contract_hint(
            state,
            route,
            &preferred,
            auto_locator_path,
            original_user_text,
        ) else {
            continue;
        };
        let (skill, args) = match &action {
            AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
            AgentAction::CallTool { tool, args } => (tool.as_str(), args),
            _ => continue,
        };
        if !contract_hint_preferred_action_allowed(route, skill, args) {
            continue;
        }
        let actions = vec![action];
        let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
            .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
        return Some(build_plan_result(
            goal,
            &raw_plan_text,
            PlanKind::Single,
            &actions,
        ));
    }
    None
}

fn contract_hint_preferred_action_allowed(route: &RouteResult, skill: &str, args: &Value) -> bool {
    if crate::contract_matrix::action_policy_for_route(Some(route), skill, args)
        .is_some_and(|policy| policy.is_allowed())
    {
        return true;
    }
    if skill != "run_cmd"
        || args
            .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
            .and_then(Value::as_bool)
            != Some(true)
    {
        return false;
    }
    route_has_capability_namespace(route, &["docker", "package", "package_manager"])
}

pub(super) fn planned_execution_action_ref<'a>(
    action: &'a AgentAction,
) -> Option<(&'a str, &'a Value)> {
    match action {
        AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
        AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
        _ => None,
    }
}

pub(super) fn mark_user_named_output_path_action(action: AgentAction) -> AgentAction {
    match action {
        AgentAction::CallSkill { skill, mut args } => {
            if let Some(obj) = args.as_object_mut() {
                obj.insert(
                    super::super::CLAWD_USER_NAMED_OUTPUT_PATH_ARG.to_string(),
                    Value::Bool(true),
                );
            }
            AgentAction::CallSkill { skill, args }
        }
        AgentAction::CallTool { tool, mut args } => {
            if let Some(obj) = args.as_object_mut() {
                obj.insert(
                    super::super::CLAWD_USER_NAMED_OUTPUT_PATH_ARG.to_string(),
                    Value::Bool(true),
                );
            }
            AgentAction::CallTool { tool, args }
        }
        other => other,
    }
}

pub(super) fn readonly_file_read_candidate_for_rejected_run_cmd(
    action: &AgentAction,
) -> Option<AgentAction> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        _ => return None,
    };
    if !skill.trim().eq_ignore_ascii_case("run_cmd")
        || args
            .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
            .and_then(Value::as_bool)
            == Some(true)
    {
        return None;
    }
    let command = run_cmd_command_from_args(args)?;
    let (mode, n, path) = readonly_file_read_from_shell_command(command)?;
    Some(AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "read_text_range",
            "path": absolutize_readonly_file_path_from_run_cmd_args(&path, args),
            "mode": mode,
            "n": n,
        }),
    })
}

pub(super) fn readonly_find_candidate_for_rejected_run_cmd(
    action: &AgentAction,
) -> Option<AgentAction> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        _ => return None,
    };
    if !skill.trim().eq_ignore_ascii_case("run_cmd")
        || args
            .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
            .and_then(Value::as_bool)
            == Some(true)
    {
        return None;
    }
    let command = run_cmd_command_from_args(args)?;
    let find = readonly_find_extension_from_shell_command(command)?;
    Some(AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": find.root,
            "extension": find.extension,
            "files_only": true,
            "recursive": true,
        }),
    })
}

pub(super) fn replace_contract_rejected_actions_with_preferred_refs(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    let preferred_actions = crate::contract_matrix::preferred_action_refs_for_route(route);
    if preferred_actions.is_empty() {
        return actions;
    }
    let original_user_text = original_user_text.unwrap_or_default();
    let file_paths_has_allowed_executable = route
        .output_contract_marker_is(crate::OutputSemanticKind::FilePaths)
        && actions.iter().any(|action| {
            matches!(
                file_paths_contract_executable_action_allowed(action),
                Some(true)
            )
        });
    let quantity_compare_has_text_evidence = route
        .output_contract_marker_is(crate::OutputSemanticKind::QuantityComparison)
        && actions.iter().any(action_reads_workspace_text_content);
    let compound_plan_has_content_read =
        actions.len() > 1 && actions.iter().any(action_reads_workspace_text_content);
    let quantity_compare_directory_name_pair = route
        .output_contract_marker_is(crate::OutputSemanticKind::QuantityComparison)
        && actions
            .iter()
            .filter(|action| planned_find_entries_directory_name(action).is_some())
            .take(3)
            .count()
            == 2;
    let prefer_registry_repair_for_ad_hoc_command =
        actions_use_ad_hoc_command_without_route_preferred_skill(state, route, &actions);
    let scratch_filesystem_lifecycle_plan =
        crate::agent_engine::route_can_upgrade_scratch_filesystem_lifecycle(route)
            && crate::agent_engine::scratch_filesystem_lifecycle_plan_actions_match(
                state, &actions,
            );

    actions
        .into_iter()
        .enumerate()
        .map(|(idx, action)| {
            let Some((skill, args)) = planned_execution_action_ref(&action) else {
                return action;
            };
            let Some(policy) = crate::contract_matrix::action_policy_for_route(
        Some(route),
                skill,
                args,
            ) else {
                return action;
            };
            if policy.is_allowed() {
                return action;
            }
            let normalized_skill = state.resolve_canonical_skill_name(skill);
            if scratch_filesystem_lifecycle_plan
                && crate::agent_engine::scratch_filesystem_lifecycle_action_allowed(
                    state,
                    &normalized_skill,
                    args,
                )
            {
                info!(
                    "plan_keep_scratch_filesystem_lifecycle_action idx={} contract={} action={}",
                    idx, policy.contract_match, policy.action_key
                );
                return action;
            }
            if crate::agent_engine::route_can_upgrade_scratch_filesystem_lifecycle(route)
                && crate::agent_engine::scratch_filesystem_cleanup_recovery_action_allowed(
                    state,
                    loop_state,
                    &normalized_skill,
                    args,
                )
            {
                info!(
                    "plan_keep_scratch_filesystem_cleanup_recovery_action idx={} contract={} action={}",
                    idx, policy.contract_match, policy.action_key
                );
                return action;
            }
            if active_ops_recipe_allows_mutation_despite_summary_contract(
                state,
                loop_state,
                &normalized_skill,
                args,
                policy.decision,
            ) {
                info!(
                    "plan_keep_active_ops_recipe_mutation_despite_contract_hint idx={} contract={} action={} phase={}",
                    idx,
                    policy.contract_match,
                    policy.action_key,
                    loop_state.execution_recipe.phase.as_str()
                );
                return action;
            }
            if structured_run_cmd_async_start_allows_planner_authority_despite_contract(
                &normalized_skill,
                args,
                policy.decision,
            ) {
                info!(
                    "plan_keep_structured_run_cmd_async_start idx={} contract={} action={} decision={}",
                    idx,
                    policy.contract_match,
                    policy.action_key,
                    policy.decision.as_str()
                );
                return action;
            }
            if super::super::action_is_user_named_new_workspace_write(
                &state.skill_rt.workspace_root,
                original_user_text,
                &normalized_skill,
                args,
            ) {
                info!(
                    "plan_keep_user_named_output_write_despite_contract_hint idx={} contract={} action={}",
                    idx,
                    policy.contract_match,
                    policy.action_key
                );
                return mark_user_named_output_path_action(action);
            }
            if normalized_skill.eq_ignore_ascii_case("run_cmd")
                && run_cmd_command_from_args(args).is_some_and(|command| {
                    should_preserve_user_supplied_shell_command(
                        command,
                        original_user_text,
                        Some(original_user_text),
                    )
                })
            {
                info!(
                    "plan_keep_user_supplied_run_cmd_despite_contract_hint idx={} contract={} action={}",
                    idx,
                    policy.contract_match,
                    policy.action_key
                );
                return action;
            }
            if let Some(candidate) = readonly_file_read_candidate_for_rejected_run_cmd(&action) {
                if let Some((candidate_skill, candidate_args)) =
                    planned_execution_action_ref(&candidate)
                {
                    if crate::contract_matrix::action_policy_for_route(
        Some(route),
                        candidate_skill,
                        candidate_args,
                    )
                    .is_some_and(|candidate_policy| candidate_policy.is_allowed())
                    {
                        info!(
                            "plan_replace_contract_rejected_readonly_run_cmd idx={} contract={} from={} to=fs_basic.read_text_range",
                            idx,
                            policy.contract_match,
                            policy.action_key
                        );
                        return candidate;
                    }
                }
            }
            if let Some(candidate) = readonly_find_candidate_for_rejected_run_cmd(&action) {
                if let Some((candidate_skill, candidate_args)) =
                    planned_execution_action_ref(&candidate)
                {
                    if crate::contract_matrix::action_policy_for_route(
        Some(route),
                        candidate_skill,
                        candidate_args,
                    )
                    .is_some_and(|candidate_policy| candidate_policy.is_allowed())
                    {
                        info!(
                            "plan_replace_contract_rejected_find_run_cmd idx={} contract={} from={} to=fs_basic.find_entries",
                            idx,
                            policy.contract_match,
                            policy.action_key
                        );
                        return candidate;
                    }
                }
            }
            if route.output_contract_marker_is(crate::OutputSemanticKind::FilePaths)
                && normalized_skill.eq_ignore_ascii_case("fs_basic")
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.trim().eq_ignore_ascii_case("stat_paths"))
            {
                return action;
            }
            if compound_plan_has_content_read
                && fs_basic_stat_paths_has_targets(&normalized_skill, args)
            {
                info!(
                    "plan_keep_compound_stat_paths_supporting_content_read idx={} contract={} action={}",
                    idx, policy.contract_match, policy.action_key
                );
                return action;
            }
            if file_paths_has_allowed_executable {
                return action;
            }
            if prefer_registry_repair_for_ad_hoc_command
                && normalized_skill.eq_ignore_ascii_case("run_cmd")
            {
                info!(
                    "plan_keep_registry_preferred_ad_hoc_command_for_repair idx={} contract={} action={}",
                    idx, policy.contract_match, policy.action_key
                );
                return action;
            }
            if quantity_compare_has_text_evidence && action_is_structured_scalar_field_read(&action)
            {
                if let Some(request) = structured_extract_request(args) {
                    if request.fields.len() == 1 {
                        let (path, field_path) = resolve_structured_scalar_read_target_and_field(
                            state,
                            route,
                            &request.path,
                            &request.fields[0],
                        );
                        let candidate = config_basic_read_field_action(path, field_path);
                        if let Some((candidate_skill, candidate_args)) =
                            planned_execution_action_ref(&candidate)
                        {
                            if crate::contract_matrix::action_policy_for_route(
        Some(route),
                                candidate_skill,
                                candidate_args,
                            )
                            .is_some_and(|candidate_policy| candidate_policy.is_allowed())
                            {
                                info!(
                                    "plan_keep_quantity_compare_structured_scalar_read idx={} contract={} from={} to=config_basic.read_field",
                                    idx, policy.contract_match, policy.action_key
                                );
                                return candidate;
                            }
                        }
                    }
                }
            }
            if quantity_compare_directory_name_pair
                && planned_find_entries_directory_name(&action).is_some()
            {
                info!(
                    "plan_keep_quantity_compare_directory_name_search idx={} contract={} action={}",
                    idx, policy.contract_match, policy.action_key
                );
                return action;
            }
            if registry_declares_non_mutating_planner_action(state, &normalized_skill, args) {
                info!(
                    "plan_keep_registry_non_mutating_action idx={} contract={} action={}",
                    idx, policy.contract_match, policy.action_key
                );
                return action;
            }

            for preferred in &preferred_actions {
                let Some(candidate) = preferred_structured_action_for_contract_hint(
                    state,
                    route,
                    preferred,
                    auto_locator_path,
                    original_user_text,
                ) else {
                    continue;
                };
                let Some((candidate_skill, candidate_args)) =
                    planned_execution_action_ref(&candidate)
                else {
                    continue;
                };
                let Some(candidate_policy) =
                    crate::contract_matrix::action_policy_for_route(
        Some(route),
                        candidate_skill,
                        candidate_args,
                    )
                else {
                    continue;
                };
                if !candidate_policy.is_allowed() {
                    continue;
                }
                info!(
                    "plan_replace_contract_rejected_action idx={} contract={} from={} decision={} to={}",
                    idx,
                    policy.contract_match,
                    policy.action_key,
                    policy.decision.as_str(),
                    candidate_policy.action_key
                );
                return inherit_preferred_action_filters_from_rejected_action(
                    route,
                    candidate,
                    &action,
                );
            }
            action
        })
        .collect()
}

fn structured_run_cmd_async_start_allows_planner_authority_despite_contract(
    normalized_skill: &str,
    args: &Value,
    policy_decision: crate::contract_matrix::ActionPolicyDecision,
) -> bool {
    if !normalized_skill.eq_ignore_ascii_case("run_cmd")
        || !matches!(
            policy_decision,
            crate::contract_matrix::ActionPolicyDecision::RejectedForbidden
                | crate::contract_matrix::ActionPolicyDecision::RejectedNotAllowed
        )
        || args.get("async_start").and_then(Value::as_bool) != Some(true)
        || run_cmd_command_from_args(args).is_none()
    {
        return false;
    }

    positive_bounded_i64_arg(args, "poll_after_seconds", 1, 86_400)
        && positive_bounded_i64_arg(args, "expires_in_seconds", 1, 604_800)
}

fn positive_bounded_i64_arg(args: &Value, key: &str, min: i64, max: i64) -> bool {
    args.get(key)
        .and_then(Value::as_i64)
        .is_some_and(|value| value >= min && value <= max)
}

fn registry_declares_non_mutating_planner_action(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    let Some(action) = args
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_registry_action_token)
        .filter(|action| !action.is_empty())
    else {
        return false;
    };
    state
        .skill_manifest(normalized_skill)
        .is_some_and(|manifest| {
            manifest.planner_capabilities.into_iter().any(|mapping| {
                mapping
                    .action
                    .as_deref()
                    .map(normalize_registry_action_token)
                    .is_some_and(|mapped| mapped == action)
                    && matches!(
                        mapping.effect,
                        Some(
                            claw_core::skill_registry::PlannerCapabilityEffect::Observe
                                | claw_core::skill_registry::PlannerCapabilityEffect::Validate
                        )
                    )
            })
        })
}

fn normalize_registry_action_token(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if matches!(ch, '-' | ' ' | '.') {
                '_'
            } else {
                ch
            }
        })
        .collect()
}

fn active_ops_recipe_allows_mutation_despite_summary_contract(
    state: &AppState,
    loop_state: &LoopState,
    normalized_skill: &str,
    args: &Value,
    policy_decision: crate::contract_matrix::ActionPolicyDecision,
) -> bool {
    if policy_decision != crate::contract_matrix::ActionPolicyDecision::RejectedNotAllowed {
        return false;
    }
    let recipe = loop_state.execution_recipe;
    if !matches!(
        recipe.kind,
        crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop
    ) || !matches!(
        recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Apply
            | crate::execution_recipe::ExecutionRecipePhase::Repair
    ) {
        return false;
    }
    let effect =
        crate::execution_recipe::classify_skill_action_effect(state, normalized_skill, args);
    effect.mutates
        && !crate::execution_recipe::action_conflicts_with_recipe_target_scope(
            recipe,
            state,
            normalized_skill,
            args,
        )
}

pub(super) fn inherit_preferred_action_filters_from_rejected_action(
    route: &RouteResult,
    mut candidate: AgentAction,
    rejected: &AgentAction,
) -> AgentAction {
    let Some((_, rejected_args)) = planned_execution_action_ref(rejected) else {
        return candidate;
    };
    let Some(rejected_obj) = rejected_args.as_object() else {
        return candidate;
    };
    let (AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args }) =
        &mut candidate
    else {
        return candidate;
    };
    if !tool.eq_ignore_ascii_case("fs_basic") {
        return candidate;
    }
    let Some(candidate_obj) = args.as_object_mut() else {
        return candidate;
    };
    let action_name = candidate_obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !action_name.eq_ignore_ascii_case("count_entries") {
        return candidate;
    }
    inherit_count_entries_filters_from_rejected_action(route, candidate_obj, rejected_obj);
    candidate
}

pub(super) fn inherit_count_entries_filters_from_rejected_action(
    route: &RouteResult,
    out: &mut serde_json::Map<String, Value>,
    rejected: &serde_json::Map<String, Value>,
) {
    if let Some(hint) = scalar_count_filter_hint_from_route(route) {
        apply_scalar_count_filter_hint(out, &hint);
    }
    let dirs_only = structured_directory_filter_requested(rejected);
    let files_only = structured_file_filter_requested(rejected);
    let ext_filter = structured_ext_filter_arg(rejected);
    if dirs_only {
        out.insert("kind_filter".to_string(), Value::String("dir".to_string()));
        out.insert("count_files".to_string(), Value::Bool(false));
        out.insert("count_dirs".to_string(), Value::Bool(true));
        out.insert("dirs_only".to_string(), Value::Bool(true));
        out.insert("files_only".to_string(), Value::Bool(false));
    } else if files_only {
        out.insert("kind_filter".to_string(), Value::String("file".to_string()));
        out.insert("count_files".to_string(), Value::Bool(true));
        out.insert("count_dirs".to_string(), Value::Bool(false));
        out.insert("files_only".to_string(), Value::Bool(true));
        out.insert("dirs_only".to_string(), Value::Bool(false));
    }
    if let Some(ext_filter) = ext_filter {
        out.insert("ext_filter".to_string(), ext_filter);
        if !dirs_only {
            out.insert("files_only".to_string(), Value::Bool(true));
            out.insert("dirs_only".to_string(), Value::Bool(false));
            out.insert("kind_filter".to_string(), Value::String("file".to_string()));
        }
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::HiddenEntriesCheck) {
        out.insert("include_hidden".to_string(), Value::Bool(true));
    } else if dirs_only || files_only || out.get("ext_filter").is_some() {
        out.insert("include_hidden".to_string(), Value::Bool(false));
    } else if let Some(include_hidden) = rejected.get("include_hidden").and_then(Value::as_bool) {
        out.insert("include_hidden".to_string(), Value::Bool(include_hidden));
    }
    if let Some(recursive) = rejected.get("recursive").and_then(Value::as_bool) {
        out.insert("recursive".to_string(), Value::Bool(recursive));
    }
    if let Some(hint) = scalar_count_filter_hint_from_route(route) {
        apply_scalar_count_filter_hint(out, &hint);
    }
}

pub(super) fn apply_scalar_count_contract_filter_to_count_entries_actions(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount) {
        return actions;
    }
    let Some(hint) = scalar_count_filter_hint_from_route(route) else {
        return actions;
    };
    actions
        .into_iter()
        .map(|mut action| {
            let (AgentAction::CallTool { tool, args }
            | AgentAction::CallSkill { skill: tool, args }) = &mut action
            else {
                return action;
            };
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            let is_count_entries = (tool.eq_ignore_ascii_case("fs_basic")
                && action_name.eq_ignore_ascii_case("count_entries"))
                || (tool.eq_ignore_ascii_case("system_basic")
                    && action_name.eq_ignore_ascii_case("count_inventory"));
            if !is_count_entries {
                return action;
            }
            if let Some(obj) = args.as_object_mut() {
                apply_scalar_count_filter_hint(obj, &hint);
            }
            action
        })
        .collect()
}

pub(super) fn package_manager_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("package_manager")
}

pub(super) fn git_basic_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("git_basic")
}

pub(super) fn normalizer_answer_candidate_from_resolved_prompt(
    resolved_prompt: &str,
) -> Option<String> {
    let _ = resolved_prompt;
    None
}

pub(super) fn package_manager_detect_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || !route_has_package_detect_machine_signal(route)
        || !package_manager_available_for_plan(state)
    {
        return None;
    }
    let mut args = serde_json::json!({"action": "detect"});
    if let Some(path) = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        args["path"] = Value::String(path.to_string());
    }
    let actions = vec![
        AgentAction::CallSkill {
            skill: "package_manager".to_string(),
            args,
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn package_docker_readonly_probe_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !package_manager_available_for_plan(state)
        || !docker_basic_available_for_plan(state)
        || !route_has_package_docker_probe_tokens(route)
    {
        return None;
    }
    let actions = vec![
        AgentAction::CallSkill {
            skill: "package_manager".to_string(),
            args: serde_json::json!({"action": "detect"}),
        },
        AgentAction::CallSkill {
            skill: "docker_basic".to_string(),
            args: serde_json::json!({"action": "version"}),
        },
        AgentAction::CallSkill {
            skill: "docker_basic".to_string(),
            args: serde_json::json!({"action": "ps"}),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string(),
            ],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    if !actions
        .iter()
        .all(|action| readonly_probe_action_allowed(route, action))
    {
        return None;
    }
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn route_has_package_docker_probe_tokens(route: &RouteResult) -> bool {
    let has_package = route_has_capability_namespace(route, &["package", "package_manager"]);
    let has_docker = route_has_capability_namespace(route, &["docker"]);
    has_package && has_docker
}

fn route_has_package_detect_machine_signal(route: &RouteResult) -> bool {
    route_capability_action_for_namespaces(route, &["package", "package_manager"])
        .is_some_and(|action| action_has_any_segment(action, &["detect"]))
}

fn route_has_capability_namespace(route: &RouteResult, namespaces: &[&str]) -> bool {
    route_capability_action_for_namespaces(route, namespaces).is_some()
}

fn route_capability_action_for_namespaces<'a>(
    route: &'a RouteResult,
    namespaces: &[&str],
) -> Option<&'a str> {
    [&route.route_reason, &route.resolved_intent]
        .iter()
        .filter_map(|text| machine_context_capability_action_for_namespaces(text, namespaces))
        .next()
}

fn machine_context_capability_action_for_namespaces<'a>(
    machine_context: &'a str,
    namespaces: &[&str],
) -> Option<&'a str> {
    machine_context
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | '(' | ')' | '[' | ']'))
        .filter_map(|part| capability_action_for_namespace_token(part.trim(), namespaces))
        .next()
}

fn capability_action_for_namespace_token<'a>(
    token: &'a str,
    namespaces: &[&str],
) -> Option<&'a str> {
    let capability = token.strip_prefix("capability_ref=")?;
    let (namespace, action) = capability.split_once('.')?;
    if namespace.is_empty()
        || action.is_empty()
        || !namespaces.iter().any(|candidate| namespace == *candidate)
        || !capability.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
    {
        return None;
    }
    Some(action)
}

fn docker_basic_action_from_capability_action(action: &str) -> &'static str {
    if action_has_any_segment(action, &["image", "images"]) {
        "images"
    } else if action_has_any_segment(action, &["log", "logs"]) {
        "logs"
    } else if action_has_any_segment(action, &["inspect"]) {
        "inspect"
    } else if action_has_any_segment(action, &["start"]) {
        "start"
    } else if action_has_any_segment(action, &["stop"]) {
        "stop"
    } else if action_has_any_segment(action, &["restart"]) {
        "restart"
    } else if action_has_any_segment(action, &["version"]) {
        "version"
    } else {
        "ps"
    }
}

fn action_has_any_segment(action: &str, needles: &[&str]) -> bool {
    action
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')))
        .any(|segment| {
            let segment = segment.trim();
            !segment.is_empty()
                && needles.iter().any(|needle| {
                    segment == *needle
                        || segment.starts_with(&format!("{needle}_"))
                        || segment.ends_with(&format!("_{needle}"))
                        || segment.contains(&format!("_{needle}_"))
                })
        })
}

fn readonly_probe_action_allowed(route: &RouteResult, action: &AgentAction) -> bool {
    let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
        action
    else {
        return true;
    };
    crate::contract_matrix::action_policy_for_route(Some(route), skill, args)
        .is_some_and(|policy| policy.is_allowed())
}

pub(super) fn package_manager_dry_run_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    original_user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !package_manager_available_for_plan(state)
    {
        return None;
    }
    let packages = normalizer_answer_candidate_from_resolved_prompt(&route.resolved_intent)
        .and_then(|candidate| {
            crate::package_commands::package_install_packages_from_commandish_text(&candidate)
        })
        .or_else(|| {
            crate::package_commands::package_install_packages_from_preview_text(original_user_text)
        })?;
    let actions = vec![
        AgentAction::CallSkill {
            skill: "package_manager".to_string(),
            args: serde_json::json!({
                "action": "smart_install",
                "packages": packages,
                "dry_run": true,
                "use_sudo": true
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn existence_with_path_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract_marker_is(crate::OutputSemanticKind::ExistenceWithPath)
    {
        return None;
    }
    let single_file_delivery_probe = route_requests_single_file_delivery_probe(route);
    if route.output_contract.delivery_required && !single_file_delivery_probe {
        return None;
    }

    let hint = route.output_contract.locator_hint.trim();
    let current_surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_text);
    let current_has_structural_locator =
        current_surface.has_explicit_path_or_url() || current_surface.has_filename_candidates();
    if route.output_contract.locator_kind == crate::OutputLocatorKind::Path {
        let path = auto_locator_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .or_else(|| (!hint.is_empty()).then_some(hint));
        if let Some(path) = path {
            if is_supported_archive_path(path)
                && archive_entry_target_for_route_or_text(route, current_user_text, path).is_some()
            {
                return None;
            }
        }
    }
    let explicit_targets = explicit_existence_file_targets(current_user_text)
        .into_iter()
        .take(8)
        .collect::<Vec<_>>();
    if single_file_delivery_probe {
        return single_file_delivery_probe_action(route, auto_locator_path, &explicit_targets)
            .map(|action| vec![action]);
    }
    if explicit_targets.len() == 1
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
    {
        return Some(vec![fs_basic_stat_paths_action_for_explicit_targets(
            &explicit_targets,
        )]);
    }
    if explicit_targets.len() >= 2 {
        return Some(vec![fs_basic_stat_paths_action_for_explicit_targets(
            &explicit_targets,
        )]);
    }

    match route.output_contract.locator_kind {
        crate::OutputLocatorKind::Filename | crate::OutputLocatorKind::CurrentWorkspace
            if !hint.is_empty() =>
        {
            Some(vec![AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "find_entries",
                    "root": ".",
                    "pattern": hint,
                    "target_kind": "any",
                    "max_results": 50,
                }),
            }])
        }
        crate::OutputLocatorKind::Path => {
            let path = auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .or_else(|| (!hint.is_empty()).then_some(hint))?;
            if is_supported_archive_path(path)
                && archive_entry_target_for_route_or_text(route, current_user_text, path).is_some()
            {
                return None;
            }
            if Path::new(path).is_dir() {
                if let Some(file_name) =
                    single_filename_target_for_directory_locator(route, current_user_text)
                {
                    return Some(vec![AgentAction::CallTool {
                        tool: "fs_basic".to_string(),
                        args: serde_json::json!({
                            "action": "find_entries",
                            "root": path,
                            "pattern": file_name,
                            "target_kind": "file",
                            "max_results": 50,
                        }),
                    }]);
                }
                if let Some(pattern) =
                    single_name_target_for_directory_locator(route, current_user_text)
                        .or_else(|| {
                            single_existing_name_target_for_directory_locator(
                                path,
                                route,
                                current_user_text,
                            )
                        })
                        .or_else(|| {
                            directory_child_name_pattern_selector_from_texts(
                                path,
                                Path::new(path),
                                &[current_user_text, route.resolved_intent.as_str()],
                            )
                        })
                {
                    return Some(vec![AgentAction::CallTool {
                        tool: "fs_basic".to_string(),
                        args: serde_json::json!({
                            "action": "find_entries",
                            "root": path,
                            "pattern": pattern,
                            "target_kind": "any",
                            "max_results": 50,
                        }),
                    }]);
                }
                if !current_has_structural_locator {
                    return None;
                }
            }
            Some(vec![AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "stat_paths",
                    "paths": [path],
                    "include_missing": true,
                }),
            }])
        }
        _ => None,
    }
}

pub(super) fn existing_file_delivery_probe_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract_marker_is(crate::OutputSemanticKind::GeneratedFileDelivery)
        || route.output_contract.locator_kind != crate::OutputLocatorKind::Filename
        || !route_requests_single_file_delivery_probe(route)
        || turn_analysis
            .and_then(|analysis| analysis.state_patch.as_ref())
            .is_some()
    {
        return None;
    }
    let explicit_targets = explicit_existence_file_targets(current_user_text)
        .into_iter()
        .take(1)
        .collect::<Vec<_>>();
    let action = single_file_delivery_probe_action(route, auto_locator_path, &explicit_targets)?;
    let actions = canonicalize_legacy_file_config_capabilities(vec![action]);
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn route_requests_single_file_delivery_probe(route: &RouteResult) -> bool {
    route.output_contract.delivery_required
        && route.output_contract.response_shape == crate::OutputResponseShape::FileToken
        && route.output_contract.delivery_intent == crate::OutputDeliveryIntent::FileSingle
        && route.wants_file_delivery
}

fn single_file_delivery_probe_action(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
    explicit_targets: &[String],
) -> Option<AgentAction> {
    let hint = route.output_contract.locator_hint.trim();
    let target = explicit_targets
        .first()
        .map(String::as_str)
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
        })
        .or_else(|| (!hint.is_empty()).then_some(hint))?;
    let path_like = target.contains(['/', '\\'])
        || Path::new(target).is_absolute()
        || matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
        );
    if path_like {
        return Some(AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "path_batch_facts",
                "paths": [target],
                "include_missing": true,
            }),
        });
    }
    Some(AgentAction::CallSkill {
        skill: "fs_search".to_string(),
        args: serde_json::json!({
            "action": "find_name",
            "pattern": target,
            "root": ".",
            "max_results": 50,
        }),
    })
}

pub(super) fn single_filename_target_for_directory_locator(
    route: &RouteResult,
    current_user_text: &str,
) -> Option<String> {
    let filenames = crate::delivery_utils::extract_filename_candidates(current_user_text);
    if filenames.len() == 1 {
        return Some(filenames[0].clone());
    }
    if !filenames.is_empty() {
        return None;
    }
    let current_surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_text);
    if !current_surface.has_explicit_path_or_url() && !current_surface.has_filename_candidates() {
        return None;
    }
    let filenames = crate::delivery_utils::extract_filename_candidates(&route.resolved_intent);
    (filenames.len() == 1).then(|| filenames[0].clone())
}

pub(super) fn explicit_existence_file_targets(user_text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(
            user_text,
        )
    {
        if locator.locator_kind != crate::OutputLocatorKind::Path
            || document_target_already_covered(&targets, &locator.locator_hint)
        {
            continue;
        }
        targets.push(locator.locator_hint);
    }
    for candidate in crate::delivery_utils::extract_filename_candidates(user_text) {
        if document_target_already_covered(&targets, &candidate) {
            continue;
        }
        targets.push(candidate);
    }
    targets
}

pub(super) fn search_name_target_token_is_safe(candidate: &str) -> bool {
    let trimmed = candidate.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    });
    if trimmed.is_empty()
        || trimmed.len() > 128
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || crate::worker::has_explicit_path_or_url_locator_hint(trimmed)
        || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(trimmed)
    {
        return false;
    }
    let mut has_ascii_alnum = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            has_ascii_alnum = true;
            continue;
        }
        if !matches!(ch, '_' | '-' | '.') {
            return false;
        }
    }
    has_ascii_alnum
}

pub(super) fn push_unique_search_name_candidate(values: &mut Vec<String>, candidate: &str) {
    let trimmed = candidate.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '`'
                | ','
                | '，'
                | '。'
                | ':'
                | '：'
                | ';'
                | '；'
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '《'
                | '》'
        )
    });
    if search_name_target_token_is_safe(trimmed)
        && !values
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(trimmed))
    {
        values.push(trimmed.to_string());
    }
}

fn quoted_search_name_targets(text: &str) -> Vec<String> {
    static QUOTED_RE: OnceLock<Regex> = OnceLock::new();
    let re = QUOTED_RE.get_or_init(|| {
        Regex::new(r#""([^"\n]+)"|'([^'\n]+)'|`([^`\n]+)`"#).expect("quoted search name regex")
    });
    let mut candidates = Vec::new();
    for caps in re.captures_iter(text) {
        let candidate = caps
            .get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .map(|m| m.as_str())
            .unwrap_or_default();
        push_unique_search_name_candidate(&mut candidates, candidate);
    }
    candidates
}

fn fs_basic_stat_paths_has_targets(skill: &str, args: &Value) -> bool {
    if !skill.eq_ignore_ascii_case("fs_basic") {
        return false;
    }
    if args
        .get("action")
        .and_then(Value::as_str)
        .is_none_or(|action| !action.trim().eq_ignore_ascii_case("stat_paths"))
    {
        return false;
    }
    args.get("path")
        .and_then(Value::as_str)
        .is_some_and(|path| !path.trim().is_empty())
        || args
            .get("paths")
            .and_then(Value::as_array)
            .is_some_and(|paths| {
                paths
                    .iter()
                    .any(|path| path.as_str().is_some_and(|path| !path.trim().is_empty()))
            })
}

pub(super) fn has_multiple_quoted_search_name_targets(text: &str) -> bool {
    quoted_search_name_targets(text).len() > 1
}

pub(super) fn single_quoted_search_name_target(text: &str) -> Option<String> {
    let mut candidates = quoted_search_name_targets(text);
    (candidates.len() == 1).then(|| candidates.remove(0))
}

pub(super) fn search_name_targets_outside_locators(text: &str) -> Vec<String> {
    let mut remaining = text.to_string();
    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
    {
        remaining = remaining.replace(&locator.locator_hint, " ");
    }
    for filename in crate::delivery_utils::extract_filename_candidates(text) {
        remaining = remaining.replace(&filename, " ");
    }
    let mut candidates = Vec::new();
    for token in
        remaining.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
    {
        push_unique_search_name_candidate(&mut candidates, token);
    }
    candidates
}

pub(super) fn single_identifier_search_name_target_outside_locators(text: &str) -> Option<String> {
    let mut candidates = search_name_targets_outside_locators(text);
    (candidates.len() == 1).then(|| candidates.remove(0))
}

pub(super) fn single_existing_name_target_for_directory_locator(
    root: &str,
    route: &RouteResult,
    current_user_text: &str,
) -> Option<String> {
    let mut matching_tokens = Vec::new();
    for text in [current_user_text, route.resolved_intent.as_str()] {
        for token in search_name_targets_outside_locators(text) {
            if directory_has_unique_entry_for_search_name(root, &token)
                && !matching_tokens
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&token))
            {
                matching_tokens.push(token);
            }
        }
    }
    (matching_tokens.len() == 1).then(|| matching_tokens.remove(0))
}
