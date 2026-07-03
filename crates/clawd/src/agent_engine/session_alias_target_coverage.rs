use super::*;

pub(super) fn session_alias_targets_missing_from_plan(
    state: &AppState,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if loop_state.has_tool_or_skill_output {
        return false;
    }
    let targets = required_session_alias_targets_from_loop_state(loop_state);
    if targets.len() < 2 {
        return false;
    }
    targets.iter().any(|target| {
        !actions
            .iter()
            .any(|action| action_covers_session_alias_target(state, action, target))
    })
}

pub(super) fn session_alias_target_observation_action(
    state: &AppState,
    target: &str,
) -> Option<AgentAction> {
    if !fs_basic_read_available_for_plan(state) {
        return None;
    }
    let path = resolve_existing_metadata_locator_path(&state.skill_rt.workspace_root, target)
        .unwrap_or_else(|| target.trim().to_string());
    let target_path = Path::new(&path);
    if target_path.is_dir() {
        return Some(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "list_dir",
                "path": path,
                "names_only": false,
                "sort_by": "mtime_desc",
                "max_entries": 1000,
            }),
        });
    }
    if target_path.is_file() {
        return Some(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": path,
                "mode": "head",
                "n": 120,
            }),
        });
    }
    Some(AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": [path],
        }),
    })
}

pub(super) fn drop_terminal_discussion_actions_for_alias_target_completion(
    mut actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    while matches!(
        actions.last(),
        Some(AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. })
    ) {
        actions.pop();
    }
    actions
}

pub(super) fn complete_missing_session_alias_target_observations(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    plan_context: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    if route_result.is_some_and(|route| {
        route.output_contract.delivery_required
            || route.output_contract.response_shape == crate::OutputResponseShape::FileToken
    }) {
        return actions;
    }
    let targets = required_session_alias_targets_for_plan_context(
        loop_state,
        user_text,
        original_user_text,
        plan_context,
    );
    if targets.len() < 2 {
        return actions;
    }
    let missing = targets
        .iter()
        .filter(|target| {
            !actions
                .iter()
                .any(|action| action_covers_session_alias_target(state, action, target))
        })
        .filter_map(|target| session_alias_target_observation_action(state, target))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return actions;
    }

    let mut rewritten = drop_terminal_discussion_actions_for_alias_target_completion(actions);
    rewritten.extend(missing);
    let evidence_refs = observation_action_evidence_refs(&rewritten);
    rewritten.push(AgentAction::SynthesizeAnswer {
        evidence_refs: if evidence_refs.is_empty() {
            vec!["last_output".to_string()]
        } else {
            evidence_refs
        },
    });
    rewritten.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    info!("plan_complete_missing_session_alias_target_observations");
    rewritten
}

pub(super) fn scalar_count_plan_uses_listing_instead_of_structured_count(
    state: &AppState,
    route_result: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if loop_state.has_tool_or_skill_output
        || route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarCount
    {
        return false;
    }
    let saw_listing = actions
        .iter()
        .any(|action| action_is_directory_listing_plan_action(state, action));
    let saw_structured_count = actions
        .iter()
        .any(|action| action_is_structured_count_plan_action(state, action));
    saw_listing && !saw_structured_count
}

pub(super) fn action_is_directory_listing_plan_action(
    state: &AppState,
    action: &AgentAction,
) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (state.resolve_canonical_skill_name(skill), args)
        }
        _ => return false,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    skill == "list_dir"
        || (skill == "fs_basic" && action_name.eq_ignore_ascii_case("list_dir"))
        || (skill == "system_basic" && action_name.eq_ignore_ascii_case("inventory_dir"))
}

pub(super) fn action_is_structured_count_plan_action(
    state: &AppState,
    action: &AgentAction,
) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (state.resolve_canonical_skill_name(skill), args)
        }
        AgentAction::CallCapability { capability, .. } => {
            return capability.eq_ignore_ascii_case("filesystem.count_entries");
        }
        _ => return false,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    (skill == "fs_basic" && action_name.eq_ignore_ascii_case("count_entries"))
        || (skill == "system_basic" && action_name.eq_ignore_ascii_case("count_inventory"))
}

pub(super) fn route_qualifies_for_lightweight_repair_skip(
    route_result: Option<&RouteResult>,
) -> bool {
    route_result.is_some_and(|route| {
        crate::task_context_builder::uses_light_execution_context_budget(
            route,
            &route.resolved_intent,
        )
    })
}

pub(super) async fn repair_plan_actions(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    turn_analysis: &str,
    user_text: &str,
    repair_reason: &str,
    tool_spec: &str,
    skill_playbooks: &str,
    attempt_ledger: &str,
    raw_plan: &str,
    round_no: usize,
) -> Result<String, String> {
    let runtime_os = runtime_os_label();
    let runtime_shell = runtime_shell_label();
    let workspace_root = state.skill_rt.workspace_root.display().to_string();
    let resolved_prompt = crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        PLAN_REPAIR_PROMPT_LOGICAL_PATH,
    )
    .map_err(|e| e.to_string())?;
    let prompt_template = resolved_prompt.template;
    let prompt_source = resolved_prompt.source;
    let prompt_version = resolved_prompt.version;
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request_for_prompt =
        crate::language_policy::task_user_request_for_prompt(task, user_text);
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__GOAL__", goal),
            ("__TURN_ANALYSIS__", turn_analysis),
            ("__USER_REQUEST__", &user_request_for_prompt),
            ("__REPAIR_REASON__", repair_reason),
            ("__TOOL_SPEC__", tool_spec),
            ("__SKILL_PLAYBOOKS__", skill_playbooks),
            ("__ATTEMPT_LEDGER__", attempt_ledger),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
            ("__RUNTIME_OS__", &runtime_os),
            ("__RUNTIME_SHELL__", &runtime_shell),
            ("__WORKSPACE_ROOT__", &workspace_root),
            ("__RAW_PLAN__", raw_plan),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "plan_repair_prompt",
        &prompt_source,
        prompt_version.as_deref(),
        Some(round_no),
    );
    let repaired =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await?;
    info!(
        "plan_llm_repair_response task_id={} round={} raw={}",
        task.task_id,
        round_no,
        crate::truncate_for_log(&repaired)
    );
    Ok(repaired)
}

pub(super) fn plan_repair_reason(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    initial_actions: Option<&[AgentAction]>,
) -> &'static str {
    let Some(actions) = initial_actions else {
        return "plan_parse_failed";
    };
    if actions_violate_recipe_target_scope(state, loop_state, actions) {
        return match loop_state.execution_recipe.target_scope {
            crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo => {
                "current_repo_scope_rejects_external_target"
            }
            crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace => {
                "external_workspace_requires_explicit_target"
            }
            crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield => {
                "greenfield_requires_artifact_creation"
            }
            crate::execution_recipe::ExecutionRecipeTargetScope::Unknown
            | crate::execution_recipe::ExecutionRecipeTargetScope::System => {
                "ops_closed_loop_requires_scope_alignment"
            }
        };
    }
    if loop_state.execution_recipe.is_active()
        && matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Apply
        )
        && !actions
            .iter()
            .any(|action| action_is_likely_mutating(state, action))
    {
        return "ops_closed_loop_apply_requires_mutation";
    }
    if actions_missing_recipe_profile_validation(state, loop_state, actions) {
        return match loop_state.execution_recipe.profile {
            crate::execution_recipe::ExecutionRecipeProfile::ConfigChange => {
                "config_change_requires_post_change_validation"
            }
            crate::execution_recipe::ExecutionRecipeProfile::CodeChange => {
                "code_change_requires_verification"
            }
            crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring => {
                "skill_authoring_requires_integration_validation"
            }
            crate::execution_recipe::ExecutionRecipeProfile::PackageChange => {
                "package_change_requires_validation"
            }
            crate::execution_recipe::ExecutionRecipeProfile::DatabaseChange => {
                "database_change_requires_validation"
            }
            _ => "ops_closed_loop_requires_validation",
        };
    }
    if contains_unavailable_skill_action(state, actions) {
        return "unavailable_skill_requires_replan";
    }
    if session_alias_targets_missing_from_plan(state, loop_state, actions) {
        return "current_request_mentions_multiple_session_alias_targets_but_plan_omits_target";
    }
    let Some(route_result) = route_result else {
        return "non_actionable_plan_for_current_route";
    };
    if structured_scalar_compare_missing_required_extracts_for_round(
        route_result,
        loop_state,
        actions,
    ) {
        return "structured_scalar_compare_requires_extract_fields";
    }
    if actions_use_ad_hoc_command_without_route_preferred_skill(state, route_result, actions) {
        return "preferred_skill_required_for_semantic_route";
    }
    if no_content_evidence_execute_route_read_only_file_plan_requires_repair(
        state,
        Some(route_result),
        loop_state,
        actions,
    ) {
        return "execute_route_requires_non_readonly_file_plan";
    }
    if plain_act_filesystem_text_read_only_plan_requires_repair(
        state,
        Some(route_result),
        loop_state,
        actions,
    ) {
        return "plain_act_file_action_requires_non_readonly_plan";
    }
    if content_evidence_plan_only_has_locator_observation(route_result, loop_state, actions) {
        return "content_evidence_requires_content_observation";
    }
    if scalar_count_plan_uses_listing_instead_of_structured_count(
        state,
        route_result,
        loop_state,
        actions,
    ) {
        return "scalar_count_requires_structured_count_action";
    }
    if observation_only_plan_missing_user_answer(state, route_result, loop_state, actions) {
        return "plan_missing_terminal_user_answer";
    }
    "non_actionable_plan_for_current_route"
}

pub(super) fn can_fallback_to_initial_plan_after_repair_failure(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(route_result) = route_result else {
        return false;
    };
    let has_terminal_answer = has_discussion_followup_action(actions);
    let fallback_shape_is_safe = if has_terminal_answer {
        !observation_only_plan_missing_user_answer(state, route_result, loop_state, actions)
            && !content_evidence_plan_only_has_locator_observation(
                route_result,
                loop_state,
                actions,
            )
    } else {
        true
    };
    !route_result.needs_clarify
        && !loop_state.has_tool_or_skill_output
        && !contains_unavailable_skill_action(state, actions)
        && !session_alias_targets_missing_from_plan(state, loop_state, actions)
        && !structured_scalar_compare_missing_required_extracts_for_round(
            route_result,
            loop_state,
            actions,
        )
        && !no_content_evidence_execute_route_read_only_file_plan_requires_repair(
            state,
            Some(route_result),
            loop_state,
            actions,
        )
        && !plain_act_filesystem_text_read_only_plan_requires_repair(
            state,
            Some(route_result),
            loop_state,
            actions,
        )
        && !scalar_count_plan_uses_listing_instead_of_structured_count(
            state,
            route_result,
            loop_state,
            actions,
        )
        && (!actions_use_ad_hoc_command_without_route_preferred_skill(state, route_result, actions)
            || safe_observation_run_cmd_plan_can_fallback(state, Some(route_result), actions))
        && has_executable_observation_or_action(actions)
        && fallback_shape_is_safe
}

pub(super) fn safe_observation_run_cmd_plan_can_fallback(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return false;
    }

    let mut saw_run_cmd = false;
    for action in actions {
        match action {
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. } => {
                if !is_non_mutating_run_cmd_action(state, action) {
                    return false;
                }
                saw_run_cmd = true;
            }
            AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. } => {}
            AgentAction::CallCapability { .. } | AgentAction::Think { .. } => return false,
        }
    }
    saw_run_cmd
}

pub(super) fn action_is_filesystem_text_read_observation(
    state: &AppState,
    action: &AgentAction,
) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill, args)
        }
        AgentAction::SynthesizeAnswer { .. }
        | AgentAction::CallCapability { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => {
            return false;
        }
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    match canonical.as_str() {
        "read_file" => true,
        "fs_basic" => args
            .get("action")
            .and_then(Value::as_str)
            .map(|action| action.trim().eq_ignore_ascii_case("read_text_range"))
            .unwrap_or(false),
        "system_basic" => args
            .get("action")
            .and_then(Value::as_str)
            .map(|action| action.trim().eq_ignore_ascii_case("read_range"))
            .unwrap_or(false),
        "doc_parse" => args
            .get("path")
            .and_then(Value::as_str)
            .map(|path| !path.trim().is_empty())
            .unwrap_or(false),
        _ => false,
    }
}

pub(super) fn no_content_evidence_execute_route_read_only_file_plan_requires_repair(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.needs_clarify
        || loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !(route_locator_hint_is_path_like(route) || actions.iter().any(action_has_path_like_arg))
        || actions
            .iter()
            .any(|action| action_is_likely_mutating(state, action))
    {
        return false;
    }
    if active_anchor_detached_read_only_plan_can_execute(state, route, actions) {
        return false;
    }
    if existing_observed_synthesis_read_only_plan_can_execute(state, route, actions) {
        return false;
    }
    let executable_actions = actions.iter().filter(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    });
    let mut saw_read = false;
    for action in executable_actions {
        if !action_is_filesystem_text_read_observation(state, action) {
            return false;
        }
        saw_read = true;
    }
    saw_read
}

fn existing_observed_synthesis_read_only_plan_can_execute(
    state: &AppState,
    route: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if !route_reason_has_marker(route, "existing_observed_context_synthesis") {
        return false;
    }
    let has_synthesis = actions
        .iter()
        .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }));
    let has_respond = actions
        .iter()
        .any(|action| matches!(action, AgentAction::Respond { .. }));
    if !has_synthesis || !has_respond {
        return false;
    }

    let mut saw_read = false;
    for action in actions {
        match action {
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. } => {
                if !action_is_filesystem_text_read_observation(state, action) {
                    return false;
                }
                saw_read = true;
            }
            AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. } => {}
            AgentAction::CallCapability { .. } | AgentAction::Think { .. } => return false,
        }
    }
    saw_read
}

fn active_anchor_detached_read_only_plan_can_execute(
    state: &AppState,
    route: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if !route_reason_has_marker(
        route,
        "active_task_scope_refinement_detached_from_structured_anchor",
    ) {
        return false;
    }
    let mut saw_read = false;
    for action in actions {
        match action {
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. } => {
                if !action_is_filesystem_text_read_observation(state, action) {
                    return false;
                }
                saw_read = true;
            }
            AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. } => {}
            AgentAction::CallCapability { .. } | AgentAction::Think { .. } => return false,
        }
    }
    saw_read
}

pub(super) fn plain_act_filesystem_text_read_only_plan_requires_repair(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.needs_clarify
        || loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || !route.ask_mode.is_plain_act()
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || !(route_locator_hint_is_path_like(route) || actions.iter().any(action_has_path_like_arg))
        || actions
            .iter()
            .any(|action| action_is_likely_mutating(state, action))
    {
        return false;
    }
    if crate::task_context_builder::uses_light_execution_context_budget(
        route,
        &route.resolved_intent,
    ) && observation_only_plan_can_finalize_from_direct_output(state, Some(route), actions)
    {
        return false;
    }
    let executable_actions = actions.iter().filter(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    });
    let mut saw_read = false;
    for action in executable_actions {
        if !action_is_filesystem_text_read_observation(state, action) {
            return false;
        }
        saw_read = true;
    }
    saw_read
}

pub(super) fn action_has_path_like_arg(action: &AgentAction) -> bool {
    let args = match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args,
        AgentAction::SynthesizeAnswer { .. }
        | AgentAction::CallCapability { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => {
            return false;
        }
    };
    action_path_arg(args).is_some_and(|path| {
        let value = path.trim();
        !value.is_empty()
            && (Path::new(value).is_absolute()
                || value.contains('/')
                || value.contains('\\')
                || value.starts_with('.')
                || Path::new(value).extension().is_some())
    })
}

pub(super) fn scalar_path_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ScalarPathOnly | crate::OutputSemanticKind::FileBasename
        )
    {
        return None;
    }
    let hint = route.output_contract.locator_hint.trim().to_string();
    let path = (!hint.is_empty() && Path::new(&hint).exists())
        .then_some(hint)
        .or_else(|| {
            auto_locator_path
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
        })?;
    let path = path.as_str();
    let resolved_path = Path::new(path);
    if !resolved_path.exists() {
        return None;
    }
    let current_workspace_directory = route.output_contract.locator_kind
        == crate::OutputLocatorKind::CurrentWorkspace
        && route.output_contract.locator_hint.trim().is_empty();
    if resolved_path.is_dir() && !current_workspace_directory {
        return None;
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

#[cfg(test)]
pub(super) fn route_requires_scalar_content_observation(route: &RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        || route
            .route_reason
            .contains("execution_required_read_file_extract_scalar")
        || route
            .route_reason
            .contains("request_requires_fresh_file_observation_to_extract_title")
}

#[cfg(test)]
pub(super) fn scalar_content_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || !route_requires_scalar_content_observation(route)
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::ExistenceWithPath
                | crate::OutputSemanticKind::ExistenceWithPathSummary
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
                | crate::OutputSemanticKind::GeneratedFilePathReport
                | crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
        )
    {
        return None;
    }
    let path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty()).then_some(hint)
        })?;
    if !Path::new(path).is_file() {
        return None;
    }
    if is_supported_archive_path(path) {
        return None;
    }
    if route_requests_config_validation(route) {
        return Some(vec![config_basic_validate_action(path.to_string())]);
    }
    Some(vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": path,
                "mode": "head",
                "n": 120
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ])
}

#[cfg(test)]
fn route_requests_config_validation(route: &RouteResult) -> bool {
    route.output_contract_marker_is(crate::OutputSemanticKind::ConfigValidation)
        || crate::evidence_policy::final_answer_shape_for_route(route)
            == Some(crate::evidence_policy::FinalAnswerShape::ValidationVerdict)
}

#[cfg(test)]
pub(super) fn file_facts_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::QuantityComparison
        )
        || (!route_expects_terminal_user_answer(route)
            && !(route.output_contract.semantic_kind
                == crate::OutputSemanticKind::QuantityComparison
                && route.output_contract.response_shape == crate::OutputResponseShape::Scalar))
        || route.output_contract.response_shape == crate::OutputResponseShape::FileToken
        || (matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        ) && route.output_contract.semantic_kind
            != crate::OutputSemanticKind::QuantityComparison)
    {
        return None;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && crate::evidence_policy::target_locators_for_route(route).len() > 1
    {
        return None;
    }
    let path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty()).then_some(hint)
        })?;
    let path_ref = Path::new(path);
    let quantity_metadata_target = route.output_contract.semantic_kind
        == crate::OutputSemanticKind::QuantityComparison
        && path_ref.exists();
    if !(path_ref.is_file() || quantity_metadata_target) || is_supported_archive_path(path) {
        return None;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::Strict
        )
        && path_ref.is_dir()
    {
        let selector = &route.output_contract.self_extension.list_selector;
        let max_entries = selector
            .limit
            .or_else(|| contract_hint_selector_limit(&route.route_reason));
        let has_selector_metadata = max_entries.is_some()
            || selector.sort_by.is_some()
            || contract_hint_selector_extension(&route.route_reason).is_some()
            || contract_hint_selector_sort_by(&route.route_reason).is_some();
        if route.output_contract.response_shape == crate::OutputResponseShape::Strict
            && max_entries.is_none()
            && !has_selector_metadata
        {
            // A strict single-target quantity contract without selector metadata is a
            // path metadata request, not a ranked directory inventory request.
        } else {
            let max_entries = max_entries.unwrap_or(50);
            return Some(vec![
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "list_dir",
                        "path": path,
                        "files_only": true,
                        "names_only": false,
                        "sort_by": "size_desc",
                        "max_entries": max_entries,
                    }),
                },
                AgentAction::SynthesizeAnswer {
                    evidence_refs: vec!["last_output".to_string()],
                },
                AgentAction::Respond {
                    content: "{{last_output}}".to_string(),
                },
            ]);
        }
    }
    let targets = vec![path.to_string()];
    let mut actions = vec![fs_basic_stat_paths_action_for_explicit_targets(&targets)];
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison
        && path_ref.is_dir()
    {
        let recursive_count = !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
        );
        actions.push(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "count_entries",
                "path": path,
                "recursive": recursive_count,
                "count_files": true,
                "count_dirs": true,
            }),
        });
    }
    let evidence_refs = (1..=actions.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    actions.push(AgentAction::SynthesizeAnswer { evidence_refs });
    actions.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    Some(actions)
}

pub(super) fn resolve_existing_metadata_locator_path(
    workspace_root: &Path,
    raw: &str,
) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let raw_path = Path::new(raw);
    let candidate = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        workspace_root.join(raw_path)
    };
    if !candidate.exists() {
        return None;
    }
    Some(
        candidate
            .canonicalize()
            .unwrap_or(candidate)
            .display()
            .to_string(),
    )
}

#[cfg(test)]
pub(super) fn file_facts_auto_locator_target_path(
    workspace_root: &Path,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    if let Some(path) = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        return Some(path.to_string());
    }
    let route = route_result?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::QuantityComparison {
        return None;
    }
    let targets = route_locator_targets(route);
    if targets.len() > 1 {
        return None;
    }
    if let Some(target) = targets.first() {
        if let Some(path) = resolve_existing_metadata_locator_path(workspace_root, target) {
            return Some(path);
        }
    }
    let hint = route.output_contract.locator_hint.trim();
    if !hint.is_empty() {
        if let Some(path) = resolve_existing_metadata_locator_path(workspace_root, hint) {
            return Some(path);
        }
    }
    if route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace {
        return Some(
            workspace_root
                .canonicalize()
                .unwrap_or_else(|_| workspace_root.to_path_buf())
                .display()
                .to_string(),
        );
    }
    None
}

#[cfg(test)]
pub(super) fn file_facts_auto_locator_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let target = file_facts_auto_locator_target_path(
        &state.skill_rt.workspace_root,
        route_result,
        auto_locator_path,
    )?;
    let actions = file_facts_auto_locator_observation_plan(route_result, Some(&target))?;
    let actions = normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        Some(goal),
        Some(&target),
        actions,
    );
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

#[cfg(test)]
pub(super) fn generic_directory_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !route_expects_terminal_user_answer(route)
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryPurposeSummary
                | crate::OutputSemanticKind::WorkspaceProjectSummary
        )
    {
        return None;
    }

    let path = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&path).is_dir() {
        return None;
    }

    let mut args = serde_json::json!({
        "action": "list_dir",
        "path": path,
        "names_only": false,
        "max_entries": 1000,
        "sort_by": if route.output_contract.semantic_kind == crate::OutputSemanticKind::DirectoryEntryGroups {
            "mtime_desc"
        } else {
            "size_desc"
        },
    });
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::FileNames
        || route.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths
    {
        args["files_only"] = Value::Bool(true);
    } else if route.output_contract.semantic_kind == crate::OutputSemanticKind::DirectoryNames {
        args["dirs_only"] = Value::Bool(true);
    }

    Some(vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args,
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ])
}
