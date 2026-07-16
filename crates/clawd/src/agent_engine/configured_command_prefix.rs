use super::*;

#[path = "configured_command_prefix_run_cmd_contract.rs"]
mod configured_command_prefix_run_cmd_contract;
use configured_command_prefix_run_cmd_contract::{
    annotate_readonly_cli_surface_run_cmds,
    ensure_clawcli_resume_surface_help_for_required_machine_field,
    ensure_run_cmd_async_start_for_runtime_async_job_contract,
    rewrite_backend_identity_metadata_respond_to_runtime_identity,
};

pub(super) fn trim_leading_command_delimiters(mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        let Some(ch) = text.chars().next() else {
            return text;
        };
        if matches!(
            ch,
            ':' | '：' | '-' | '—' | '–' | '`' | '"' | '\'' | '“' | '”' | ' '
        ) {
            text = &text[ch.len_utf8()..];
            continue;
        }
        return text;
    }
}

pub(super) fn looks_like_concrete_command_tail(tail: &str) -> bool {
    let tail = trim_leading_command_delimiters(tail);
    let first_token = tail
        .split_whitespace()
        .next()
        .unwrap_or(tail)
        .trim_matches(|ch: char| {
            ch.is_ascii_punctuation()
                || matches!(ch, '，' | '。' | '；' | '：' | '、' | '！' | '？')
        });
    first_token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .count()
        >= 2
}

pub(super) fn contains_angle_placeholder_token(text: &str) -> bool {
    let mut chars = text.char_indices().peekable();
    while let Some((start_idx, ch)) = chars.next() {
        if ch != '<' {
            continue;
        }
        let Some((end_idx, _)) = chars.clone().find(|(_, candidate)| *candidate == '>') else {
            continue;
        };
        let inner = text[start_idx + ch.len_utf8()..end_idx].trim();
        if inner.is_empty() {
            continue;
        }
        let has_identifier_char = inner.chars().any(|candidate| candidate.is_alphanumeric());
        let placeholder_shaped = inner.chars().all(|candidate| {
            candidate.is_alphanumeric() || matches!(candidate, '_' | '-' | '.' | ' ' | '\t')
        });
        if has_identifier_char && placeholder_shaped {
            return true;
        }
    }
    false
}

pub(super) fn literal_command_segment_has_unresolved_template(segment: &str) -> bool {
    contains_angle_placeholder_token(segment) || literal_segment_looks_like_output_template(segment)
}

pub(super) fn literal_segment_looks_like_output_template(segment: &str) -> bool {
    let segment = segment.trim();
    if segment.is_empty()
        || segment.contains('\n')
        || segment
            .chars()
            .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<'))
    {
        return false;
    }
    let mut words = segment.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    let Some(rest) = words.next() else {
        return false;
    };
    if words.next().is_some() || !first.ends_with(':') {
        return false;
    }
    let label = first.trim_end_matches(':');
    let label_ok = !label.is_empty()
        && label
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    let placeholder_ok = rest
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '<' | '>'));
    label_ok && placeholder_ok
}

pub(super) fn shellish_literal_command_segments(
    request: &str,
    allow_bare_token: bool,
) -> Vec<String> {
    let mut parts = request.split('`');
    parts.next();
    parts
        .step_by(2)
        .map(|segment| crate::bootstrap::config_loaders::trim_command_text(segment.to_string()))
        .filter(|segment| {
            !literal_command_segment_has_unresolved_template(segment)
                && looks_like_concrete_command_tail(segment)
                && (allow_bare_token
                    || segment
                        .chars()
                        .any(|ch| matches!(ch, '|' | ';' | '&' | '>' | '<') || ch.is_whitespace()))
        })
        .collect()
}

pub(super) fn shellish_literal_command_segment(request: &str) -> Option<String> {
    shellish_literal_command_segments(request, true)
        .into_iter()
        .next()
}

pub(super) fn simple_bare_command_token(token: &str) -> bool {
    !token.is_empty()
        && !token.starts_with('-')
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && token.chars().any(|ch| ch.is_ascii_alphabetic())
        && token
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .count()
            >= 2
}

pub(super) fn command_token_resolves_in_path(
    token: &str,
    path_env: Option<&std::ffi::OsStr>,
) -> bool {
    let Some(path_env) = path_env else {
        return false;
    };
    std::env::split_paths(path_env).any(|dir| dir.join(token).is_file())
}

pub(super) fn leading_shellish_command_sequence_segment_with_path_env(
    request: &str,
    path_env: Option<&std::ffi::OsStr>,
) -> Option<String> {
    let request = request.trim_start();
    if request.is_empty() {
        return None;
    }
    let ascii_end = request
        .char_indices()
        .find_map(|(idx, ch)| (!ch.is_ascii()).then_some(idx))
        .unwrap_or(request.len());
    let ascii_prefix = request[..ascii_end].trim();
    if ascii_prefix.is_empty() {
        return None;
    }
    let mut commands = Vec::new();
    for raw_token in ascii_prefix.split_whitespace() {
        let token = raw_token
            .trim_matches(|ch: char| ch.is_ascii_punctuation() && !matches!(ch, '_' | '-' | '.'));
        if !simple_bare_command_token(token) || !command_token_resolves_in_path(token, path_env) {
            break;
        }
        commands.push(token.to_string());
    }
    (commands.len() >= 3).then(|| commands.join("; "))
}

pub(super) fn leading_shellish_command_sequence_segment(request: &str) -> Option<String> {
    let path_env = std::env::var_os("PATH");
    leading_shellish_command_sequence_segment_with_path_env(request, path_env.as_deref())
}

pub(crate) fn explicit_machine_syntax_command_segment(request: &str) -> Option<String> {
    leading_shellish_command_sequence_segment(request)
        .or_else(|| shellish_literal_command_segment(request))
}

pub(super) fn route_allows_explicit_command_preservation(
    route_result: Option<&RouteResult>,
) -> bool {
    route_result.is_some_and(|route| {
        route.output_contract.requires_content_evidence
            || route.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput)
    })
}

fn route_allows_machine_syntax_command(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        !route.needs_clarify
            && !route.output_contract.delivery_required
            && !route.wants_file_delivery
            && route.output_contract.response_shape != crate::OutputResponseShape::FileToken
    })
}

pub(super) fn run_cmd_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("run_cmd")
}

pub(super) fn process_basic_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("process_basic")
}

pub(super) fn system_basic_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("system_basic")
}

pub(super) fn health_check_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("health_check")
}

pub(super) fn action_is_run_cmd(state: &AppState, action: &AgentAction) -> bool {
    planned_action_skill_name(action)
        .map(|skill| state.resolve_canonical_skill_name(skill) == "run_cmd")
        .unwrap_or(false)
}

pub(super) fn literal_command_failure_can_replan(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        !route.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput)
            && !route.output_contract_marker_is(crate::OutputSemanticKind::ExecutionFailedStep)
    })
}

pub(super) fn route_contract_defers_literal_command_to_planner(
    route_result: Option<&RouteResult>,
) -> bool {
    route_result.is_some_and(|route| {
        route.output_contract.requires_content_evidence
            && !route.output_contract.delivery_required
            && ([
                crate::OutputSemanticKind::StructuredKeys,
                crate::OutputSemanticKind::DirectoryPurposeSummary,
                crate::OutputSemanticKind::DirectoryEntryGroups,
                crate::OutputSemanticKind::FileNames,
                crate::OutputSemanticKind::DirectoryNames,
                crate::OutputSemanticKind::FilePaths,
                crate::OutputSemanticKind::ContentExcerptSummary,
                crate::OutputSemanticKind::ContentExcerptWithSummary,
                crate::OutputSemanticKind::ExistenceWithPath,
                crate::OutputSemanticKind::ExistenceWithPathSummary,
                crate::OutputSemanticKind::RecentScalarEqualityCheck,
                crate::OutputSemanticKind::RecentArtifactsJudgment,
                crate::OutputSemanticKind::SqliteTableListing,
                crate::OutputSemanticKind::SqliteTableNamesOnly,
                crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
                crate::OutputSemanticKind::SqliteSchemaVersion,
            ]
            .iter()
            .any(|kind| route.output_contract_marker_is(*kind))
                || (route.output_contract_marker_is(crate::OutputSemanticKind::ScalarPathOnly)
                    && scalar_path_contract_has_structural_locator(route)))
    })
}

fn scalar_path_contract_has_structural_locator(route: &RouteResult) -> bool {
    match route.output_contract.locator_kind {
        crate::OutputLocatorKind::CurrentWorkspace => true,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename => {
            !route.output_contract.locator_hint.trim().is_empty()
        }
        _ => false,
    }
}

pub(super) fn missing_target_failure_can_replan(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        route.output_contract.requires_content_evidence
            && [
                crate::OutputSemanticKind::FilePaths,
                crate::OutputSemanticKind::FileNames,
                crate::OutputSemanticKind::DirectoryNames,
                crate::OutputSemanticKind::DirectoryPurposeSummary,
                crate::OutputSemanticKind::ContentExcerptSummary,
                crate::OutputSemanticKind::ContentExcerptWithSummary,
                crate::OutputSemanticKind::ExistenceWithPathSummary,
            ]
            .iter()
            .any(|kind| route.output_contract_marker_is(*kind))
    })
}

pub(super) fn mark_missing_target_repairable_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !missing_target_failure_can_replan(route_result) {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                let canonical = state.resolve_canonical_skill_name(&skill);
                if matches!(
                    canonical.as_str(),
                    "read_file" | "list_dir" | "system_basic"
                ) {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG.to_string(),
                            Value::Bool(true),
                        );
                    }
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                let canonical = state.resolve_canonical_skill_name(&tool);
                if matches!(
                    canonical.as_str(),
                    "read_file" | "list_dir" | "system_basic"
                ) {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG.to_string(),
                            Value::Bool(true),
                        );
                    }
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect()
}

pub(super) fn mark_explicit_literal_run_cmd_actions(
    actions: Vec<AgentAction>,
    failure_repairable: bool,
) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                if skill.trim().eq_ignore_ascii_case("run_cmd") {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::super::CLAWD_LITERAL_COMMAND_ARG.to_string(),
                            Value::Bool(true),
                        );
                        if failure_repairable {
                            obj.insert(
                                super::super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG.to_string(),
                                Value::Bool(true),
                            );
                        }
                    }
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                if tool.trim().eq_ignore_ascii_case("run_cmd") {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            super::super::CLAWD_LITERAL_COMMAND_ARG.to_string(),
                            Value::Bool(true),
                        );
                        if failure_repairable {
                            obj.insert(
                                super::super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG.to_string(),
                                Value::Bool(true),
                            );
                        }
                    }
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect()
}

pub(super) fn normalize_single_explicit_literal_run_cmd_command(
    actions: Vec<AgentAction>,
    exact_command: Option<&str>,
) -> Vec<AgentAction> {
    let Some(exact_command) = exact_command
        .map(str::trim)
        .filter(|command| !command.is_empty())
    else {
        return actions;
    };
    let run_cmd_count = actions
        .iter()
        .filter(|action| action_skill_is_run_cmd(action))
        .count();
    if run_cmd_count != 1 {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                if skill.trim().eq_ignore_ascii_case("run_cmd") {
                    if let Some(obj) = args.as_object_mut() {
                        let current = obj.get("command").and_then(Value::as_str).map(str::trim);
                        if current != Some(exact_command) {
                            obj.insert(
                                "command".to_string(),
                                Value::String(exact_command.to_string()),
                            );
                            info!("plan_rewrite_explicit_literal_run_cmd_command");
                        }
                    }
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                if tool.trim().eq_ignore_ascii_case("run_cmd") {
                    if let Some(obj) = args.as_object_mut() {
                        let current = obj.get("command").and_then(Value::as_str).map(str::trim);
                        if current != Some(exact_command) {
                            obj.insert(
                                "command".to_string(),
                                Value::String(exact_command.to_string()),
                            );
                            info!("plan_rewrite_explicit_literal_run_cmd_command");
                        }
                    }
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect()
}

pub(super) fn planned_run_cmds_are_verbatim_user_commands(
    actions: &[AgentAction],
    original_user_text: &str,
) -> bool {
    let mut count = 0usize;
    for action in actions {
        if !action_skill_is_run_cmd(action) {
            continue;
        }
        let Some(command) = run_cmd_command_arg(action) else {
            return false;
        };
        if !request_text_contains_command_verbatim(original_user_text, command) {
            return false;
        }
        count += 1;
    }
    count > 0
}

pub(super) fn replace_explicit_command_substitute_plan_with_run_cmd(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output || !run_cmd_available_for_plan(state) {
        return actions;
    }
    let Some(original_user_text) = original_user_text
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return actions;
    };
    if planner_has_allowed_capability_ref_action(route_result, &actions) {
        return actions;
    }
    let exact_command = explicit_machine_syntax_command_segment(original_user_text);
    if !route_allows_explicit_command_preservation(route_result)
        && !(exact_command.is_some() && route_allows_machine_syntax_command(route_result))
    {
        return actions;
    }
    let has_literal_command_sequence = exact_command.is_some()
        || execution_failed_step_literal_command_segments(original_user_text, None).len() >= 2;
    let planned_verbatim_run_cmds =
        planned_run_cmds_are_verbatim_user_commands(&actions, original_user_text);
    if !has_literal_command_sequence {
        if !planned_verbatim_run_cmds {
            return actions;
        }
    }
    if actions
        .iter()
        .any(|action| action_is_run_cmd(state, action))
    {
        let actions =
            normalize_single_explicit_literal_run_cmd_command(actions, exact_command.as_deref());
        return mark_explicit_literal_run_cmd_actions(
            actions,
            literal_command_failure_can_replan(route_result),
        );
    }
    if route_contract_defers_literal_command_to_planner(route_result) {
        return actions;
    }
    let Some(first_observation_idx) = actions.iter().position(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    }) else {
        return actions;
    };
    let Some(exact_command) = exact_command else {
        return actions;
    };
    let mut rewritten = actions;
    let mut args = serde_json::json!({
        "request_text": original_user_text,
        "cwd": state.skill_rt.workspace_root.display().to_string(),
    });
    args["command"] = serde_json::Value::String(exact_command);
    args[super::super::CLAWD_LITERAL_COMMAND_ARG] = Value::Bool(true);
    if literal_command_failure_can_replan(route_result) {
        args[super::super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG] = Value::Bool(true);
    }
    rewritten[first_observation_idx] = AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args,
    };
    info!("plan_rewrite_explicit_command_substitute_to_run_cmd");
    rewritten
}

fn planner_has_allowed_capability_ref_action(
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if crate::machine_capability_ref::route_capability_ref_tokens(route).is_empty() {
        return false;
    }
    actions.iter().any(|action| {
        let Some((skill, args)) = planned_execution_action_ref(action) else {
            return false;
        };
        !skill.eq_ignore_ascii_case("run_cmd")
            && crate::evidence_policy::capability_ref_action_policy_for_route(
                Some(route),
                skill,
                args,
            )
            .is_some_and(|policy| policy.is_allowed())
    })
}

#[cfg(test)]
pub(super) fn normalize_planned_actions(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    normalize_planned_actions_with_original(
        state,
        route_result,
        loop_state,
        user_text,
        None,
        auto_locator_path,
        actions,
    )
}

#[cfg(test)]
pub(super) fn normalize_planned_actions_with_original(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        None,
        auto_locator_path,
        actions,
    )
}

fn normalize_action_arg_aliases(state: &AppState, actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|mut action| {
            match &mut action {
                AgentAction::CallTool { tool, args } => {
                    let normalized = state.resolve_canonical_skill_name(tool);
                    super::super::arg_resolver::normalize_skill_arg_aliases(&normalized, args);
                }
                AgentAction::CallSkill { skill, args } => {
                    let normalized = state.resolve_canonical_skill_name(skill);
                    super::super::arg_resolver::normalize_skill_arg_aliases(&normalized, args);
                }
                AgentAction::CallCapability { .. }
                | AgentAction::SynthesizeAnswer { .. }
                | AgentAction::Respond { .. }
                | AgentAction::Think { .. } => {}
            }
            action
        })
        .collect()
}

pub(super) fn normalize_planned_actions_with_original_and_context(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions = crate::capability_resolver::resolve_agent_actions_for_state(state, actions);
    let actions = normalize_action_arg_aliases(state, actions);
    let actions = annotate_readonly_cli_surface_run_cmds(state, actions);
    let actions = ensure_clawcli_resume_surface_help_for_required_machine_field(
        state,
        route_result,
        user_text,
        original_user_text,
        plan_context,
        actions,
    );
    let terminal_mixed_last_output_content = terminal_mixed_last_output_respond_content(&actions);
    let actions = replace_scalar_path_respond_only_with_auto_locator_observation(
        route_result,
        loop_state,
        auto_locator_path,
        actions,
    );
    let actions = replace_file_delivery_respond_only_with_path_observation(
        state,
        route_result,
        loop_state,
        actions,
    );
    let actions = replace_file_delivery_empty_write_with_path_observation(
        state,
        route_result,
        loop_state,
        actions,
    );
    let actions = replace_explicit_command_substitute_plan_with_run_cmd(
        state,
        route_result,
        loop_state,
        original_user_text,
        actions,
    );
    let actions =
        super::super::planning_recent_artifacts::normalize_recent_artifacts_listing_selectors(
            route_result,
            actions,
        );
    let actions =
        super::super::planning_recent_artifacts::rewrite_recent_artifacts_field_extraction_to_selected_file_reads(
            route_result,
            loop_state,
            &state.skill_rt.workspace_root,
            actions,
        );
    let actions = replace_contract_rejected_actions_with_preferred_refs(
        state,
        route_result,
        loop_state,
        original_user_text.or(Some(user_text)),
        auto_locator_path,
        actions,
    );
    let actions =
        ensure_run_cmd_async_start_for_runtime_async_job_contract(state, route_result, actions);
    let actions =
        apply_scalar_count_contract_filter_to_count_entries_actions(route_result, actions);
    let explicit_command_request = original_user_text.or(Some(user_text)).is_some_and(|text| {
        explicit_machine_syntax_command_segment(text).is_some()
            && (route_allows_explicit_command_preservation(route_result)
                || route_allows_machine_syntax_command(route_result))
    });
    let defer_legacy_semantic_rewrites = !explicit_command_request
        && route_result.is_some_and(|route| {
            actions_use_ad_hoc_command_without_route_preferred_skill(state, route, &actions)
        });
    if defer_legacy_semantic_rewrites {
        info!("plan_defer_legacy_semantic_rewrite_to_registry_repair");
    }
    let skip_legacy_semantic_rewrites = explicit_command_request || defer_legacy_semantic_rewrites;
    let actions = normalize_legacy_compatibility_actions(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        plan_context,
        auto_locator_path,
        actions,
        skip_legacy_semantic_rewrites,
    );
    let actions =
        rewrite_process_ps_run_cmd_to_process_basic(state, user_text, original_user_text, actions);
    let actions = rewrite_simple_filesystem_run_cmd_to_fs_basic(
        state,
        user_text,
        original_user_text,
        actions,
    );
    let actions = rewrite_append_run_cmd_to_fs_basic(state, user_text, original_user_text, actions);
    let actions = rewrite_readonly_file_read_run_cmd_to_fs_basic(
        state,
        user_text,
        original_user_text,
        actions,
    );
    let actions = rewrite_readonly_find_run_cmd_to_fs_basic(
        state,
        route_result,
        user_text,
        original_user_text,
        actions,
    );
    let actions =
        rewrite_readonly_count_run_cmd_to_fs_basic(state, user_text, original_user_text, actions);
    let actions =
        super::super::planning_recent_artifacts::normalize_recent_artifacts_listing_selectors(
            route_result,
            actions,
        );
    let actions =
        strip_terminal_discussion_for_direct_skill_passthrough(state, route_result, actions);
    let actions = normalize_evidence_contract_actions(
        state,
        route_result,
        loop_state,
        original_user_text.unwrap_or(user_text),
        plan_context,
        auto_locator_path,
        actions,
    );
    let actions = strip_media_artifact_text_overwrites(&state.skill_rt.workspace_root, actions);
    let actions =
        strip_unrequested_config_edit_actions(route_result, user_text, original_user_text, actions);
    let actions = normalize_terminal_delivery_actions(
        state,
        route_result,
        loop_state,
        user_text,
        terminal_mixed_last_output_content,
        actions,
    );
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let actions = rewrite_single_target_structured_field_read_to_auto_locator(
        route_result,
        auto_locator_path,
        actions,
    );
    let actions = rewrite_active_bound_target_observations_to_matching_locator_hint(
        route_result,
        loop_state,
        actions,
    );
    let actions = rewrite_session_alias_delivery_observations_to_route_locator(
        route_result,
        loop_state,
        actions,
    );
    let actions =
        expand_compound_listing_and_content_synthesis_refs(route_result, loop_state, actions);
    let actions =
        append_terminal_synthesize_for_observation_summary_contract(route_result, actions);
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
        mark_non_mutating_run_cmd_sequences_continue_on_error(state, route_result, actions);
    let actions =
        rewrite_backend_identity_metadata_respond_to_runtime_identity(state, route_result, actions);
    apply_scalar_count_contract_filter_to_count_entries_actions(route_result, actions)
}
