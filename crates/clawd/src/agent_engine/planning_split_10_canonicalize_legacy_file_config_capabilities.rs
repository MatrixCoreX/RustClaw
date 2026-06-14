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

pub(super) fn rewrite_rustclaw_config_validation_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.iter().any(is_config_guard_action) {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| {
            let Some((path, format, profile)) =
                config_validation_action_target(&action, route_result, auto_locator_path)
                    .map(|(path, format, profile)| (path, format, Some(profile)))
                    .or_else(|| {
                        plain_rustclaw_main_config_validation_action_target(
                            &action,
                            route_result,
                            auto_locator_path,
                        )
                        .map(|(path, format)| (path, format, None))
                    })
            else {
                return action;
            };
            if profile == Some(ConfigValidationProfile::SyntaxOnly) {
                return action;
            }
            let candidate = config_basic_guard_action(path, format);
            if !planned_action_allowed_by_current_contract(route_result, &candidate) {
                return action;
            }
            let candidate_path = match &candidate {
                AgentAction::CallTool { args, .. } => {
                    args.get("path").and_then(Value::as_str).unwrap_or_default()
                }
                _ => "",
            };
            info!(
                "plan_rewrite_rustclaw_config_validation_to_guard path={}",
                crate::truncate_for_log(candidate_path)
            );
            candidate
        })
        .collect()
}

pub(super) fn planned_action_allowed_by_current_contract(
    route_result: Option<&RouteResult>,
    action: &AgentAction,
) -> bool {
    let Some(route) = route_result else {
        return true;
    };
    let Some((skill, args)) = planned_execution_action_ref(action) else {
        return true;
    };
    crate::contract_matrix::action_policy_for_output_contract(
        Some(&route.output_contract),
        skill,
        args,
    )
    .is_some_and(|policy| policy.is_allowed())
}

pub(super) fn repair_guard_config_default_path_for_invalid_locator(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallTool { tool, args } => {
                let args = repair_guard_config_args_for_invalid_locator(
                    &tool,
                    args,
                    route_result,
                    auto_locator_path,
                );
                AgentAction::CallTool { tool, args }
            }
            AgentAction::CallSkill { skill, args } => {
                let args = repair_guard_config_args_for_invalid_locator(
                    &skill,
                    args,
                    route_result,
                    auto_locator_path,
                );
                AgentAction::CallSkill { skill, args }
            }
            other => other,
        })
        .collect()
}

pub(super) fn repair_guard_config_args_for_invalid_locator(
    skill: &str,
    args: Value,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Value {
    let Some(action_name) = args.get("action").and_then(Value::as_str).map(str::trim) else {
        return args;
    };
    let is_guard_action = (skill.eq_ignore_ascii_case("config_edit")
        && action_name.eq_ignore_ascii_case("guard_config"))
        || (skill.eq_ignore_ascii_case("config_basic")
            && action_name.eq_ignore_ascii_case("guard_rustclaw_config"))
        || skill.eq_ignore_ascii_case("config_guard");
    if !is_guard_action {
        return args;
    }
    let should_repair = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .is_none_or(|path| {
            !is_rustclaw_config_guard_path(path) && !path_has_structured_text_extension(path)
        });
    if !should_repair {
        return args;
    }
    let path = route_result
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|path| is_rustclaw_config_guard_path(path))
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| is_rustclaw_config_guard_path(path))
        })
        .unwrap_or("configs/config.toml");
    let mut obj = args.as_object().cloned().unwrap_or_default();
    obj.insert("path".to_string(), Value::String(path.to_string()));
    obj.entry("format".to_string())
        .or_insert_with(|| Value::String("toml".to_string()));
    Value::Object(obj)
}

pub(super) fn rewrite_rustclaw_config_risk_assessment_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if route_result.is_none_or(|route| {
        route.output_contract.semantic_kind != crate::OutputSemanticKind::ConfigRiskAssessment
    }) || actions.iter().any(is_config_basic_guard_action)
    {
        return actions;
    }
    let Some(path) =
        rustclaw_config_risk_assessment_target(route_result, auto_locator_path, &actions)
    else {
        return actions;
    };
    info!(
        "plan_rewrite_rustclaw_config_risk_assessment_to_guard path={}",
        crate::truncate_for_log(&path)
    );
    vec![config_basic_guard_action(path, Some("toml".to_string()))]
}

pub(super) fn rustclaw_config_risk_assessment_target(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<String> {
    actions
        .iter()
        .filter_map(planned_config_risk_observation_path)
        .find(|path| is_rustclaw_config_guard_path(path))
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| is_rustclaw_config_guard_path(path))
        })
        .or_else(|| {
            route_result
                .map(|route| route.output_contract.locator_hint.trim())
                .filter(|path| is_rustclaw_config_guard_path(path))
        })
        .map(ToString::to_string)
}

pub(super) fn planned_config_risk_observation_path(action: &AgentAction) -> Option<&str> {
    planned_structured_config_observation_path(action)
        .or_else(|| planned_bounded_file_read_path(action))
}

pub(super) fn rewrite_rustclaw_main_config_excerpt_read_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.iter().any(is_config_guard_action) {
        return actions;
    }
    let Some(path) =
        rustclaw_main_config_excerpt_guard_target(route_result, auto_locator_path, &actions)
    else {
        return actions;
    };
    info!(
        "plan_rewrite_rustclaw_main_config_excerpt_read_to_guard path={}",
        crate::truncate_for_log(&path)
    );
    vec![config_basic_guard_action(path, Some("toml".to_string()))]
}

pub(super) fn rustclaw_main_config_excerpt_guard_target(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ContentExcerptSummary
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
    {
        return None;
    }
    actions
        .iter()
        .filter_map(planned_broad_rustclaw_main_config_read_path)
        .find(|path| is_rustclaw_main_config_path(path))
        .or_else(|| {
            let has_broad_main_config_read = actions
                .iter()
                .any(|action| planned_broad_config_read_without_path(action));
            has_broad_main_config_read.then(|| {
                auto_locator_path
                    .map(str::trim)
                    .filter(|path| is_rustclaw_main_config_path(path))
            })?
        })
        .or_else(|| {
            let has_broad_main_config_read = actions
                .iter()
                .any(|action| planned_broad_config_read_without_path(action));
            has_broad_main_config_read.then(|| {
                let hint = route.output_contract.locator_hint.trim();
                is_rustclaw_main_config_path(hint).then_some(hint)
            })?
        })
        .map(ToString::to_string)
}

pub(super) fn planned_broad_rustclaw_main_config_read_path(action: &AgentAction) -> Option<&str> {
    planned_bounded_file_read_path(action)
        .filter(|path| is_rustclaw_main_config_path(path))
        .filter(|_| planned_broad_config_excerpt_read(action))
}

pub(super) fn planned_broad_config_read_without_path(action: &AgentAction) -> bool {
    planned_bounded_file_read_path(action).is_none() && planned_broad_config_excerpt_read(action)
}

pub(super) fn planned_broad_config_excerpt_read(action: &AgentAction) -> bool {
    let args = match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args,
        AgentAction::CallCapability { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Think { .. } => return false,
    };
    if !action_observes_bounded_file_content(action) {
        return false;
    }
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if mode.eq_ignore_ascii_case("tail") || mode.eq_ignore_ascii_case("last") {
        return false;
    }
    let n = args
        .get("n")
        .or_else(|| args.get("line_count"))
        .or_else(|| args.get("count"))
        .or_else(|| args.get("limit"))
        .and_then(parse_positive_usize);
    if n.is_some_and(|value| value < 80) {
        return false;
    }
    let start_line = args
        .get("start_line")
        .or_else(|| args.get("line_start"))
        .and_then(parse_i64_value);
    if start_line.is_some_and(|line| line > 1) {
        return false;
    }
    let end_line = args
        .get("end_line")
        .or_else(|| args.get("line_end"))
        .and_then(parse_i64_value);
    if let (Some(start), Some(end)) = (start_line, end_line) {
        if end >= start && (end - start + 1) < 80 {
            return false;
        }
    } else if let Some(end) = end_line {
        if end < 80 {
            return false;
        }
    }
    let max_bytes = args.get("max_bytes").and_then(parse_positive_usize);
    if max_bytes.is_some_and(|value| value < 4096) {
        return false;
    }
    true
}

pub(super) fn rewrite_invalid_rustclaw_config_section_field_reads_to_guard(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.iter().any(is_config_guard_action) {
        return actions;
    }
    let Some((path, format)) = invalid_rustclaw_config_section_field_read_target(
        route_result,
        auto_locator_path,
        &actions,
    ) else {
        return actions;
    };
    info!(
        "plan_rewrite_invalid_rustclaw_config_section_field_read_to_guard path={}",
        crate::truncate_for_log(&path)
    );
    vec![config_basic_guard_action(path, format)]
}

pub(super) fn prefer_config_basic_guard_for_rustclaw_config_actions(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| {
            let Some((path, format)) =
                config_guard_action_target_path(&action, route_result, auto_locator_path)
            else {
                return action;
            };
            let candidate = config_basic_guard_action(path, format);
            if planned_action_allowed_by_current_contract(route_result, &candidate) {
                candidate
            } else {
                action
            }
        })
        .collect()
}

pub(super) fn config_basic_guard_action(path: String, format: Option<String>) -> AgentAction {
    let mut args = serde_json::Map::new();
    args.insert(
        "action".to_string(),
        Value::String("guard_rustclaw_config".to_string()),
    );
    args.insert("path".to_string(), Value::String(path));
    if let Some(format) = format {
        args.insert("format".to_string(), Value::String(format));
    }
    AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args: Value::Object(args),
    }
}

pub(super) fn prefer_route_locator_for_rustclaw_config_action_paths(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(locator) = route_result
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|path| !path.is_empty() && is_rustclaw_config_guard_path(path))
    else {
        return actions;
    };
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallTool { tool, args } => AgentAction::CallTool {
                args: prefer_route_locator_for_config_args(&tool, args, locator),
                tool,
            },
            AgentAction::CallSkill { skill, args } => AgentAction::CallSkill {
                args: prefer_route_locator_for_config_args(&skill, args, locator),
                skill,
            },
            other => other,
        })
        .collect()
}

pub(super) fn prefer_route_locator_for_config_args(
    skill: &str,
    args: Value,
    locator: &str,
) -> Value {
    if !matches!(
        skill.trim().to_ascii_lowercase().as_str(),
        "config_basic" | "config_edit" | "config_guard"
    ) {
        return args;
    }
    let Some(path) = args.get("path").and_then(Value::as_str).map(str::trim) else {
        return args;
    };
    if !is_rustclaw_config_guard_path(path) {
        return args;
    }
    let mut obj = args.as_object().cloned().unwrap_or_default();
    obj.insert("path".to_string(), Value::String(locator.to_string()));
    if skill.trim().eq_ignore_ascii_case("config_basic")
        && obj
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|action| action.eq_ignore_ascii_case("guard_rustclaw_config"))
    {
        obj.entry("format".to_string())
            .or_insert_with(|| Value::String("toml".to_string()));
    }
    Value::Object(obj)
}

pub(super) fn config_guard_action_target_path(
    action: &AgentAction,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<(String, Option<String>)> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let is_runtime_guard = (skill.eq_ignore_ascii_case("config_edit")
        && action_name.eq_ignore_ascii_case("guard_config"))
        || skill.eq_ignore_ascii_case("config_guard");
    if !is_runtime_guard {
        return None;
    }
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| is_rustclaw_config_guard_path(path))
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| is_rustclaw_config_guard_path(path))
        })
        .or_else(|| {
            route_result
                .map(|route| route.output_contract.locator_hint.trim())
                .filter(|path| is_rustclaw_config_guard_path(path))
        })?;
    let format = args
        .get("format")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|format| !format.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some("toml".to_string()));
    Some((path.to_string(), format))
}

pub(super) fn invalid_rustclaw_config_section_field_read_target(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: &[AgentAction],
) -> Option<(String, Option<String>)> {
    actions.iter().find_map(|action| {
        let (skill, args) = match action {
            AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
            AgentAction::CallTool { tool, args } => (tool.as_str(), args),
            _ => return None,
        };
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let is_field_read = (skill.eq_ignore_ascii_case("config_basic")
            && matches!(action_name, "read_field" | "read_fields"))
            || (skill.eq_ignore_ascii_case("system_basic")
                && matches!(action_name, "extract_field" | "extract_fields"));
        if !is_field_read || !config_field_args_are_section_headers(args) {
            return None;
        }
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
            })?;
        if !is_rustclaw_main_config_path(path) {
            return None;
        }
        let format = args
            .get("format")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .or_else(|| Some("toml".to_string()));
        Some((path.to_string(), format))
    })
}

pub(super) fn config_field_args_are_section_headers(args: &Value) -> bool {
    let fields = args
        .get("field_paths")
        .or_else(|| args.get("fields"))
        .map(config_field_selector_list)
        .filter(|fields| !fields.is_empty())
        .or_else(|| {
            args.get("field_path")
                .or_else(|| args.get("field"))
                .or_else(|| args.get("key"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|field| !field.is_empty())
                .map(|field| vec![field.to_string()])
        })
        .unwrap_or_default();
    fields.len() >= 2
        && fields
            .iter()
            .all(|field| config_field_is_section_header(field))
}

pub(super) fn config_field_selector_list(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .map(ToString::to_string)
            .collect(),
        Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn config_field_is_section_header(field: &str) -> bool {
    let field = field.trim();
    field.len() > 2
        && field.starts_with('[')
        && field.ends_with(']')
        && !field[1..field.len() - 1].trim().is_empty()
}

pub(super) fn is_config_guard_action(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return false,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    (skill.eq_ignore_ascii_case("config_edit") && action_name.eq_ignore_ascii_case("guard_config"))
        || (skill.eq_ignore_ascii_case("config_basic")
            && action_name.eq_ignore_ascii_case("guard_rustclaw_config"))
        || skill.eq_ignore_ascii_case("config_guard")
}

pub(super) fn is_config_basic_guard_action(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return false,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    skill.eq_ignore_ascii_case("config_basic")
        && action_name.eq_ignore_ascii_case("guard_rustclaw_config")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigValidationProfile {
    SyntaxOnly,
    RustClawSemanticGuard,
}

pub(super) fn parse_config_validation_profile(value: &str) -> Option<ConfigValidationProfile> {
    match value.trim().to_ascii_lowercase().as_str() {
        "syntax_only" => Some(ConfigValidationProfile::SyntaxOnly),
        "rustclaw_semantic_guard" => Some(ConfigValidationProfile::RustClawSemanticGuard),
        _ => None,
    }
}

pub(super) fn config_validation_profile_from_args(args: &Value) -> Option<ConfigValidationProfile> {
    args.get("validation_profile")
        .and_then(Value::as_str)
        .and_then(parse_config_validation_profile)
        .or_else(|| {
            args.get("_clawd_validation")
                .and_then(Value::as_object)
                .and_then(|obj| obj.get("validation_profile"))
                .and_then(Value::as_str)
                .and_then(parse_config_validation_profile)
        })
}

pub(super) fn config_validation_action_target(
    action: &AgentAction,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<(String, Option<String>, ConfigValidationProfile)> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    let profile = config_validation_profile_from_args(args)?;
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let is_validation = (skill.eq_ignore_ascii_case("config_basic")
        && action_name.eq_ignore_ascii_case("validate"))
        || (skill.eq_ignore_ascii_case("system_basic")
            && action_name.eq_ignore_ascii_case("validate_structured"))
        || (skill.eq_ignore_ascii_case("config_edit")
            && action_name.eq_ignore_ascii_case("validate_config"));
    if !is_validation {
        return None;
    }
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
        })?;
    if !is_rustclaw_main_config_path(path) {
        return None;
    }
    let format = args
        .get("format")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some("toml".to_string()));
    Some((path.to_string(), format, profile))
}

pub(super) fn plain_rustclaw_main_config_validation_action_target(
    action: &AgentAction,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<(String, Option<String>)> {
    if route_result.is_none_or(|route| !route.output_contract.requires_content_evidence) {
        return None;
    }
    if route_result.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ConfigValidation
    }) {
        return None;
    }
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    if config_validation_profile_from_args(args).is_some() {
        return None;
    }
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let is_validation = (skill.eq_ignore_ascii_case("config_basic")
        && action_name.eq_ignore_ascii_case("validate"))
        || (skill.eq_ignore_ascii_case("system_basic")
            && action_name.eq_ignore_ascii_case("validate_structured"))
        || (skill.eq_ignore_ascii_case("config_edit")
            && action_name.eq_ignore_ascii_case("validate_config"));
    if !is_validation {
        return None;
    }
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
        })?;
    if !is_rustclaw_main_config_path(path) {
        return None;
    }
    let format = args
        .get("format")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| Some("toml".to_string()));
    Some((path.to_string(), format))
}

pub(super) fn is_rustclaw_main_config_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").trim().to_ascii_lowercase();
    normalized == "configs/config.toml"
        || normalized.ends_with("/configs/config.toml")
        || normalized == "config.toml"
}

pub(super) fn is_rustclaw_config_guard_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").trim().to_ascii_lowercase();
    if is_rustclaw_main_config_path(&normalized) {
        return true;
    }
    let relative_configs_path = normalized.starts_with("configs/") && normalized.ends_with(".toml");
    let absolute_configs_path = normalized.contains("/configs/") && normalized.ends_with(".toml");
    relative_configs_path || absolute_configs_path
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
        route_result
            .map(|route| route.resolved_intent.as_str())
            .unwrap_or_default(),
        None,
        None,
        actions,
    );
    let actions = rewrite_structured_scalar_field_read_plan_to_read_field(
        state,
        route_result,
        route_result
            .map(|route| route.resolved_intent.as_str())
            .unwrap_or_default(),
        None,
        None,
        actions,
    );
    let actions =
        enforce_output_contract_tool_args(route_result, user_text, original_user_text, actions);
    prune_file_paths_contract_disallowed_actions(route_result, actions)
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
