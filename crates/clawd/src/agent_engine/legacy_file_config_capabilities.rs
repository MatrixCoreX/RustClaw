use super::*;

pub(super) fn canonicalize_legacy_file_config_capabilities(
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .enumerate()
        .map(|(idx, action)| match action {
            AgentAction::CallTool { tool, args } => {
                let Some(canonical) =
                    crate::virtual_tools::canonicalize_legacy_tool_call(&tool, args.clone())
                else {
                    return AgentAction::CallTool { tool, args };
                };
                info!(
                    "plan_canonicalize_legacy_tool idx={} from={} to={} args={}",
                    idx,
                    tool,
                    canonical.tool,
                    crate::truncate_for_log(&canonical.args.to_string())
                );
                AgentAction::CallTool {
                    tool: canonical.tool,
                    args: canonical.args,
                }
            }
            AgentAction::CallSkill { skill, args } => {
                let Some(canonical) =
                    crate::virtual_tools::canonicalize_legacy_tool_call(&skill, args.clone())
                else {
                    return AgentAction::CallSkill { skill, args };
                };
                info!(
                    "plan_canonicalize_legacy_tool idx={} from={} to={} args={}",
                    idx,
                    skill,
                    canonical.tool,
                    crate::truncate_for_log(&canonical.args.to_string())
                );
                AgentAction::CallTool {
                    tool: canonical.tool,
                    args: canonical.args,
                }
            }
            other => other,
        })
        .collect()
}

pub(super) fn normalize_legacy_compatibility_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
    skip_legacy_semantic_rewrites: bool,
) -> Vec<AgentAction> {
    let actions = rewrite_service_status_plan_to_service_control(
        route_result,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = split_sequential_run_cmd_actions(user_text, original_user_text, actions);
    let actions = replace_hidden_entries_count_plan_with_inventory_dir(
        route_result,
        loop_state,
        auto_locator_path,
        actions,
    );
    let actions = replace_scalar_count_plan_with_count_inventory(
        route_result,
        loop_state,
        auto_locator_path,
        user_text,
        actions,
    );
    let actions =
        replace_structured_keys_read_plan(route_result, loop_state, auto_locator_path, actions);
    let actions = ensure_existence_path_summary_has_bounded_content(
        route_result,
        loop_state,
        auto_locator_path,
        actions,
    );
    let actions = ensure_content_excerpt_summary_has_bounded_content(
        route_result,
        loop_state,
        auto_locator_path,
        actions,
    );
    let actions =
        prefer_log_analyze_for_single_log_synthesis(state, route_result, loop_state, actions);
    let actions =
        prefer_doc_parse_for_single_document_synthesis(state, route_result, loop_state, actions);
    let actions =
        strip_terminal_discussion_for_observed_finalize(route_result, loop_state, actions);
    let actions = complete_missing_session_alias_target_observations(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        plan_context,
        actions,
    );
    let actions =
        strip_terminal_discussion_for_scalar_path_observation(route_result, loop_state, actions);
    let actions =
        strip_terminal_discussion_for_direct_skill_passthrough(state, route_result, actions);
    let actions = normalize_action_schema_aliases(
        state,
        route_result,
        user_text,
        original_user_text,
        actions,
    );
    let actions =
        expand_compound_listing_and_content_synthesis_refs(route_result, loop_state, actions);
    let actions = repair_guard_config_default_path_for_invalid_locator(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions = prefer_config_basic_guard_for_rustclaw_config_actions(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_directory_entry_groups_tree_summary_to_list_dir(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_archive_basic_short_archive_to_active_bound_target(plan_context, actions);
    let actions = rewrite_invalid_rustclaw_config_section_field_reads_to_guard(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions =
        rewrite_rustclaw_config_risk_assessment_to_guard(route_result, auto_locator_path, actions);
    let actions = rewrite_rustclaw_main_config_excerpt_read_to_guard(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions =
        rewrite_rustclaw_config_validation_to_guard(route_result, auto_locator_path, actions);
    let actions = prefer_route_locator_for_rustclaw_config_action_paths(route_result, actions);
    let actions = rewrite_sqlite_table_listing_plan_to_db_basic(
        route_result,
        auto_locator_path,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = rewrite_sqlite_schema_version_plan_to_db_basic(
        route_result,
        auto_locator_path,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = rewrite_sqlite_table_probe_to_requested_schema_value(
        route_result,
        user_text,
        original_user_text,
        actions,
    );
    let actions = rewrite_sqlite_count_query_to_requested_schema_column(
        route_result,
        user_text,
        original_user_text,
        actions,
    );
    let actions = rewrite_docker_readonly_run_cmd_to_docker_basic(
        state,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = rewrite_archive_unpack_run_cmd_to_archive_basic(
        route_result,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = rewrite_archive_pack_plan_to_archive_basic(
        route_result,
        skip_legacy_semantic_rewrites,
        actions,
    );
    let actions = rewrite_single_target_structured_field_read_to_auto_locator(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions =
        rewrite_single_target_file_read_to_auto_locator(route_result, auto_locator_path, actions);
    let actions =
        rewrite_session_alias_delivery_observations_to_route_locator(route_result, actions);
    let actions = collapse_route_target_file_content_plan(route_result, auto_locator_path, actions);
    actions
}

pub(super) fn rewrite_directory_entry_groups_tree_summary_to_list_dir(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if route_result.is_none_or(|route| {
        route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryEntryGroups
    }) {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args }
                if skill.eq_ignore_ascii_case("system_basic")
                    && args
                        .get("action")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .is_some_and(|action| action.eq_ignore_ascii_case("tree_summary")) =>
            {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .or_else(|| {
                        auto_locator_path
                            .map(str::trim)
                            .filter(|path| !path.is_empty())
                    })
                    .or_else(|| {
                        route_result
                            .map(|route| route.output_contract.locator_hint.trim())
                            .filter(|path| !path.is_empty())
                    });
                let mut mapped = serde_json::Map::new();
                mapped.insert("action".to_string(), Value::String("list_dir".to_string()));
                if let Some(path) = path {
                    mapped.insert("path".to_string(), Value::String(path.to_string()));
                }
                mapped.insert("names_only".to_string(), Value::Bool(false));
                mapped.insert("max_entries".to_string(), Value::Number(1000.into()));
                mapped.insert("sort_by".to_string(), Value::String("name".to_string()));
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: Value::Object(mapped),
                }
            }
            other => other,
        })
        .collect()
}

pub(super) fn normalize_action_schema_aliases(
    state: &AppState,
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions = normalize_doc_parse_schema_aliases(actions);
    let actions = normalize_transform_schema_aliases(actions);
    let actions = normalize_fs_basic_schema_aliases(actions);
    let actions = normalize_system_basic_schema_aliases(actions);
    let actions = rewrite_readonly_runtime_status_run_cmd_to_system_basic(state, actions);
    let actions = rewrite_raw_runtime_status_to_run_cmd(state, route_result, actions);
    let actions = normalize_git_basic_schema_aliases(route_result, actions);
    let actions = fill_missing_read_range_path_from_route_locator(route_result, actions);
    let actions = rewrite_filtered_list_dir_to_inventory_dir(state, route_result, actions);
    let actions = inject_structural_extension_filter_for_directory_inventory(
        route_result,
        user_text,
        original_user_text,
        actions,
    );
    let actions = normalize_archive_basic_schema_aliases(route_result, actions);
    let actions = strip_file_lines_count_before_tail_read_range(actions);
    let actions = strip_directory_read_range_after_inventory_dir(actions);
    let actions = broaden_default_read_range_for_structured_text(actions);
    let actions = rewrite_config_validation_read_plan_to_validate(route_result, None, actions);
    let actions =
        rewrite_invalid_rustclaw_config_section_field_reads_to_guard(route_result, None, actions);
    let actions = rewrite_rustclaw_config_risk_assessment_to_guard(route_result, None, actions);
    let actions = rewrite_structured_multi_field_read_plan_to_read_fields(
        route_result,
        "",
        false,
        None,
        None,
        actions,
    );
    let actions = rewrite_structured_scalar_field_read_plan_to_read_field(
        state,
        route_result,
        "",
        false,
        None,
        None,
        actions,
    );
    let actions =
        enforce_output_contract_tool_args(route_result, user_text, original_user_text, actions);
    prune_file_paths_contract_disallowed_actions(route_result, actions)
}

pub(super) fn rewrite_readonly_runtime_status_run_cmd_to_system_basic(
    state: &AppState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !system_basic_available_for_plan(state) {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| {
            let Some((skill, args)) = (match &action {
                AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
                AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
                _ => None,
            }) else {
                return action;
            };
            if state.resolve_canonical_skill_name(skill) != "run_cmd" {
                return action;
            }
            if args
                .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return action;
            }
            let Some(kind) = args
                .get("command")
                .and_then(Value::as_str)
                .and_then(runtime_status_kind_for_single_command)
            else {
                return action;
            };
            info!("plan_rewrite_run_cmd_to_system_basic_runtime_status kind={kind}");
            AgentAction::CallTool {
                tool: "system_basic".to_string(),
                args: serde_json::json!({
                    "action": "runtime_status",
                    "kind": kind,
                }),
            }
        })
        .collect()
}

fn runtime_status_kind_for_single_command(command: &str) -> Option<&'static str> {
    let command = command.trim();
    if command.is_empty()
        || command.contains([';', '|', '&', '>', '<', '\n', '\r'])
        || command.contains('`')
        || command.contains("$(")
    {
        return None;
    }
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    let first = tokens
        .first()
        .and_then(|token| token.rsplit('/').next())
        .unwrap_or_default();
    match first {
        "date" => Some("current_time"),
        "pwd" => Some("current_working_directory"),
        "hostname" => Some("host_name"),
        "whoami" => Some("current_user"),
        "id" if tokens.get(1) == Some(&"-un") => Some("current_user"),
        "uname" if tokens.get(1) == Some(&"-r") => Some("kernel_release"),
        _ => None,
    }
}

pub(super) fn rewrite_raw_runtime_status_to_run_cmd(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        || !run_cmd_available_for_plan(state)
        || system_basic_available_for_plan(state)
    {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| {
            let Some((skill, args)) = (match &action {
                AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
                AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
                _ => None,
            }) else {
                return action;
            };
            if state.resolve_canonical_skill_name(skill) != "system_basic" {
                return action;
            }
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if !action_name.eq_ignore_ascii_case("runtime_status") {
                return action;
            }
            let Some(command) = args
                .get("kind")
                .and_then(Value::as_str)
                .map(str::trim)
                .and_then(runtime_status_query_run_cmd_command)
            else {
                return action;
            };
            let mut run_args = serde_json::json!({
                "command": command,
                "cwd": state.skill_rt.workspace_root.display().to_string(),
            });
            run_args[super::super::CLAWD_LITERAL_COMMAND_ARG] = Value::Bool(true);
            if !crate::contract_matrix::action_policy_for_output_contract(
                Some(&route.output_contract),
                "run_cmd",
                &run_args,
            )
            .is_some_and(|policy| policy.is_allowed())
            {
                return action;
            }
            info!(
                "plan_rewrite_raw_runtime_status_to_run_cmd kind={}",
                args.get("kind")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .unwrap_or_default()
            );
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: run_args,
            }
        })
        .collect()
}

pub(super) fn broaden_default_read_range_for_structured_text(
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|mut action| {
            match &mut action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args }
                    if skill.eq_ignore_ascii_case("fs_basic")
                        || skill.eq_ignore_ascii_case("system_basic") =>
                {
                    let Some(obj) = args.as_object_mut() else {
                        return action;
                    };
                    if !is_read_range_action(skill, obj) {
                        return action;
                    }
                    if read_range_has_explicit_bounds(obj) {
                        return action;
                    }
                    let Some(path) = obj.get("path").and_then(Value::as_str).map(str::to_string)
                    else {
                        return action;
                    };
                    if !path_has_structured_text_extension(&path) {
                        return action;
                    }
                    obj.entry("mode".to_string())
                        .or_insert_with(|| Value::String("head".to_string()));
                    obj.entry("n".to_string())
                        .or_insert(Value::Number(500.into()));
                    info!(
                        "plan_broaden_structured_text_read_range path={}",
                        crate::truncate_for_log(&path)
                    );
                }
                _ => {}
            }
            action
        })
        .collect()
}
