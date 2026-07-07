use super::*;

pub(super) fn preferred_structured_action_for_contract_hint(
    state: &AppState,
    route: &RouteResult,
    preferred: &crate::evidence_policy::ActionRef,
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
        "docker_basic" if docker_basic_available_for_plan(state) => {
            preferred_docker_basic_for_contract_hint(route, preferred, auto_locator_path)
        }
        _ => None,
    }
}

fn preferred_docker_basic_for_contract_hint(
    route: &RouteResult,
    preferred: &crate::evidence_policy::ActionRef,
    auto_locator_path: Option<&str>,
) -> Option<AgentAction> {
    let action = preferred_docker_basic_action(route, preferred);
    let mut args = serde_json::json!({ "action": action });
    if docker_basic_action_requires_container(action) {
        args["container"] = Value::String(first_route_locator_target(route, auto_locator_path)?);
    }
    Some(AgentAction::CallSkill {
        skill: "docker_basic".to_string(),
        args,
    })
}

fn docker_basic_action_requires_container(action: &str) -> bool {
    matches!(action, "logs" | "inspect" | "start" | "stop" | "restart")
}

fn preferred_docker_basic_action(
    route: &RouteResult,
    preferred: &crate::evidence_policy::ActionRef,
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
    if let Some(action) =
        crate::machine_capability_ref::route_capability_action_for_namespaces(route, &["docker"])
    {
        return docker_basic_action_from_capability_action(action);
    }
    "ps"
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
    let mut preferred_actions =
        crate::evidence_policy::capability_ref_action_refs_for_route(route, true);
    if preferred_actions.is_empty() {
        preferred_actions =
            crate::evidence_policy::capability_ref_action_refs_for_route(route, false);
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
            let normalized_skill = state.resolve_canonical_skill_name(skill);
            if super::super::action_is_user_named_new_workspace_write(
                &state.skill_rt.workspace_root,
                original_user_text,
                &normalized_skill,
                args,
            ) {
                info!(
                    "plan_mark_user_named_output_write_path idx={} action={}",
                    idx, normalized_skill
                );
                return mark_user_named_output_path_action(action);
            }
            let Some(policy) = crate::evidence_policy::capability_ref_replacement_action_policy_for_route(
                Some(route),
                skill,
                args,
            ) else {
                return action;
            };
            if policy.is_allowed() {
                return action;
            }
            if action_matches_contract_test_preferred_ref(
                original_user_text,
                &normalized_skill,
                args,
            ) {
                info!(
                    "plan_keep_contract_test_preferred_action_ref idx={} contract={} action={}",
                    idx, policy.contract_match, policy.action_key
                );
                return action;
            }
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
                    if crate::evidence_policy::capability_ref_action_policy_for_route(
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
                    if crate::evidence_policy::capability_ref_action_policy_for_route(
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
                            if crate::evidence_policy::capability_ref_action_policy_for_route(
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
            if policy.contract_match != "capability_ref"
                && registry_declares_non_mutating_planner_action(state, &normalized_skill, args)
            {
                info!(
                    "plan_keep_registry_non_mutating_action idx={} contract={} action={}",
                    idx, policy.contract_match, policy.action_key
                );
                return action;
            }

            for preferred in &preferred_actions {
                if !preferred_action_may_replace_contract_rejected_action(route, preferred) {
                    continue;
                }
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
                    crate::evidence_policy::capability_ref_action_policy_for_route(
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

fn action_matches_contract_test_preferred_ref(
    original_user_text: &str,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    let Some(raw_preferred) =
        crate::intent_router::contract_test_hint_value(original_user_text, "preferred_action_ref")
    else {
        return false;
    };
    let Some(preferred) = crate::evidence_policy::ActionRef::parse(&raw_preferred) else {
        return false;
    };
    let Some(action) = crate::evidence_policy::ActionRef::from_skill_args(normalized_skill, args)
    else {
        return false;
    };
    if action.skill != preferred.skill {
        return false;
    }
    preferred
        .action
        .as_deref()
        .is_none_or(|preferred_action| action.action.as_deref() == Some(preferred_action))
}

fn preferred_action_may_replace_contract_rejected_action(
    route: &RouteResult,
    preferred: &crate::evidence_policy::ActionRef,
) -> bool {
    !preferred.skill.eq_ignore_ascii_case("docker_basic")
        || crate::machine_capability_ref::route_has_capability_namespace(route, &["docker"])
}

fn structured_run_cmd_async_start_allows_planner_authority_despite_contract(
    normalized_skill: &str,
    args: &Value,
    policy_decision: crate::evidence_policy::ActionPolicyDecision,
) -> bool {
    if !normalized_skill.eq_ignore_ascii_case("run_cmd")
        || !matches!(
            policy_decision,
            crate::evidence_policy::ActionPolicyDecision::RejectedForbidden
                | crate::evidence_policy::ActionPolicyDecision::RejectedNotAllowed
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
    policy_decision: crate::evidence_policy::ActionPolicyDecision,
) -> bool {
    if policy_decision != crate::evidence_policy::ActionPolicyDecision::RejectedNotAllowed {
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
    let Some(candidate_obj) = args.as_object_mut() else {
        return candidate;
    };
    let action_name = candidate_obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if tool.eq_ignore_ascii_case("doc_parse") && action_name.eq_ignore_ascii_case("parse_doc") {
        if let Some(path) = rejected_obj
            .get("path")
            .or_else(|| rejected_obj.get("file"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            candidate_obj.insert("path".to_string(), Value::String(path.to_string()));
        }
        return candidate;
    }
    if !tool.eq_ignore_ascii_case("fs_basic") {
        return candidate;
    }
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
pub(super) fn has_multiple_quoted_search_name_targets(text: &str) -> bool {
    quoted_search_name_targets(text).len() > 1
}

#[cfg(test)]
pub(super) fn single_quoted_search_name_target(text: &str) -> Option<String> {
    let mut candidates = quoted_search_name_targets(text);
    (candidates.len() == 1).then(|| candidates.remove(0))
}

#[cfg(test)]
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

#[cfg(test)]
pub(super) fn single_identifier_search_name_target_outside_locators(text: &str) -> Option<String> {
    let mut candidates = search_name_targets_outside_locators(text);
    (candidates.len() == 1).then(|| candidates.remove(0))
}

#[cfg(test)]
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
