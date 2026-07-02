use super::*;

pub(super) fn action_targets_route_locator_artifact(
    state: &AppState,
    route: &RouteResult,
    action: &AgentAction,
) -> bool {
    if !route_locator_hint_is_path_like(route) {
        return false;
    }
    let locator = resolve_workspace_path(
        &state.skill_rt.workspace_root,
        route.output_contract.locator_hint.trim(),
    );
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let Some(raw_path) = action_path_arg(args) else {
                return false;
            };
            let action_path = resolve_workspace_path(&state.skill_rt.workspace_root, raw_path);
            match state.resolve_canonical_skill_name(skill).as_str() {
                "write_file" | "remove_file" => {
                    same_existing_or_display_path(&locator, &action_path)
                }
                "make_dir" => {
                    same_existing_or_display_path(&locator, &action_path)
                        || locator.parent().is_some_and(|parent| {
                            same_existing_or_display_path(parent, &action_path)
                        })
                }
                "fs_basic" => match args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| action.trim().to_ascii_lowercase())
                    .as_deref()
                {
                    Some("write_text" | "append_text" | "remove_path") => {
                        same_existing_or_display_path(&locator, &action_path)
                    }
                    Some("make_dir") => {
                        same_existing_or_display_path(&locator, &action_path)
                            || locator.parent().is_some_and(|parent| {
                                same_existing_or_display_path(parent, &action_path)
                            })
                    }
                    _ => false,
                },
                _ => false,
            }
        }
        AgentAction::SynthesizeAnswer { .. } => false,
        AgentAction::CallCapability { .. } => false,
        AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
    }
}

pub(super) fn strip_unrequested_workspace_artifact_mutations(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_disallows_unrequested_workspace_artifact_mutation(route, loop_state)
        || !actions
            .iter()
            .any(|action| action_is_likely_mutating(state, action))
    {
        return actions;
    }

    let original_len = actions.len();
    let mut removed_mutations = 0usize;
    let mut removed_discussion = 0usize;
    let stripped = actions
        .into_iter()
        .filter(|action| {
            if action_is_likely_mutating(state, action) {
                if action_targets_route_locator_artifact(state, route, action) {
                    return true;
                }
                removed_mutations += 1;
                return false;
            }
            if is_discussion_followup_action(action) {
                removed_discussion += 1;
                return false;
            }
            true
        })
        .collect::<Vec<_>>();
    if removed_mutations > 0 {
        info!(
            "plan_strip_unrequested_workspace_artifact_mutations removed_mutations={} removed_discussion={} kept={}",
            removed_mutations,
            removed_discussion,
            original_len.saturating_sub(removed_mutations + removed_discussion)
        );
    }
    stripped
}

pub(super) fn action_reads_workspace_text_content(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
            if skill.eq_ignore_ascii_case("read_file")
                || skill.eq_ignore_ascii_case("doc_parse") =>
        {
            true
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic") =>
        {
            args.get("action")
                .and_then(|value| value.as_str())
                .is_some_and(|action| {
                    matches!(
                        action.trim().to_ascii_lowercase().as_str(),
                        "read_text_range" | "grep_text"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_search") =>
        {
            args.get("action")
                .and_then(|value| value.as_str())
                .is_some_and(|action| action.trim().eq_ignore_ascii_case("grep_text"))
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("system_basic") =>
        {
            args.get("action")
                .and_then(|value| value.as_str())
                .is_some_and(|action| action.trim().eq_ignore_ascii_case("read_range"))
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::CallSkill { .. }
        | AgentAction::CallTool { .. } => false,
    }
}

pub(super) fn action_observes_content_presence_search(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic")
                || skill.eq_ignore_ascii_case("fs_search") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| action.eq_ignore_ascii_case("grep_text"))
        }
        _ => false,
    }
}

pub(super) fn step_output_machine_payload(value: Value) -> Value {
    value
        .get("extra")
        .filter(|extra| extra.is_object())
        .cloned()
        .unwrap_or(value)
}

pub(super) fn executed_step_reads_workspace_text_content(
    step: &crate::executor::StepExecutionResult,
) -> bool {
    if !step.is_ok() {
        return false;
    }
    if step.skill.eq_ignore_ascii_case("read_file") || step.skill.eq_ignore_ascii_case("doc_parse")
    {
        return step
            .output
            .as_deref()
            .map(str::trim)
            .is_some_and(|output| !output.is_empty());
    }
    if step.skill.eq_ignore_ascii_case("fs_basic") {
        return step
            .output
            .as_deref()
            .and_then(|output| serde_json::from_str::<Value>(output).ok())
            .map(step_output_machine_payload)
            .and_then(|value| {
                value.get("action").and_then(Value::as_str).map(|action| {
                    matches!(
                        action.trim().to_ascii_lowercase().as_str(),
                        "read_text_range" | "grep_text"
                    )
                })
            })
            .unwrap_or(false);
    }
    if !step.skill.eq_ignore_ascii_case("system_basic") {
        if !step.skill.eq_ignore_ascii_case("fs_search") {
            return false;
        }
        return step
            .output
            .as_deref()
            .and_then(|output| serde_json::from_str::<Value>(output).ok())
            .map(step_output_machine_payload)
            .and_then(|value| {
                value
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| action.trim().eq_ignore_ascii_case("grep_text"))
            })
            .unwrap_or(false);
    }
    step.output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
        .map(step_output_machine_payload)
        .and_then(|value| {
            value
                .get("action")
                .and_then(Value::as_str)
                .map(|action| action.trim().eq_ignore_ascii_case("read_range"))
        })
        .unwrap_or(false)
}

pub(super) fn action_observes_locator_only(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "find_entries" | "stat_paths"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_search") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "find_name" | "find_ext"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("system_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "find_path" | "path_batch_facts"
                    )
                })
        }
        _ => false,
    }
}

pub(super) fn content_evidence_plan_only_has_locator_observation(
    route_result: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if route_result.output_contract_marker_is(crate::OutputSemanticKind::ContentPresenceCheck) {
        let executable_actions = actions
            .iter()
            .filter(|action| {
                matches!(
                    action,
                    AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
                )
            })
            .collect::<Vec<_>>();
        if !executable_actions.is_empty()
            && !executable_actions.iter().any(|action| {
                action_observes_content_presence_search(action)
                    || action_reads_workspace_text_content(action)
            })
        {
            return true;
        }
    }
    if route_uses_runtime_owned_observed_finalizer(route_result)
        && has_tool_or_skill_observation(actions)
    {
        return false;
    }
    if path_metadata_facts_plan_satisfies_route(route_result, actions) {
        return false;
    }
    if name_listing_observation_plan_satisfies_route(route_result, actions) {
        return false;
    }
    if structured_listing_terminal_plan_satisfies_observation(actions) {
        return false;
    }
    if loop_state.has_tool_or_skill_output
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || !route_expects_terminal_user_answer(route_result)
        || has_workspace_text_content_evidence(loop_state, actions)
    {
        return false;
    }
    let executable_actions = actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
            )
        })
        .collect::<Vec<_>>();
    !executable_actions.is_empty()
        && executable_actions
            .iter()
            .all(|action| action_observes_locator_only(action))
}

pub(super) fn name_listing_observation_plan_satisfies_route(
    route_result: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if !route_result.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::FileNames,
        crate::OutputSemanticKind::DirectoryNames,
        crate::OutputSemanticKind::FilePaths,
    ]) {
        return false;
    }
    actions.iter().any(action_provides_name_listing_evidence)
}

pub(super) fn structured_listing_terminal_plan_satisfies_observation(
    actions: &[AgentAction],
) -> bool {
    if !has_discussion_followup_action(actions) {
        return false;
    }
    actions.iter().any(action_provides_name_listing_evidence)
        && actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::SynthesizeAnswer { evidence_refs }
                    if !evidence_refs.is_empty()
            )
        })
}

pub(super) fn expand_compound_listing_and_content_synthesis_refs(
    route_result: Option<&RouteResult>,
    _loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route.output_contract.requires_content_evidence || route.output_contract.delivery_required {
        return actions;
    }
    if !actions.iter().any(action_provides_name_listing_evidence)
        || !actions.iter().any(action_reads_workspace_text_content)
    {
        return actions;
    }
    let mut rewritten = actions;
    for idx in 0..rewritten.len() {
        let AgentAction::SynthesizeAnswer { evidence_refs } = &rewritten[idx] else {
            continue;
        };
        if evidence_refs.len() != 1 {
            continue;
        }
        let prior_refs = observation_action_evidence_refs(&rewritten[..idx]);
        if prior_refs.len() < 2 {
            continue;
        }
        if let AgentAction::SynthesizeAnswer { evidence_refs } = &mut rewritten[idx] {
            *evidence_refs = prior_refs.clone();
            info!(
                "plan_expand_compound_listing_content_synthesis_refs refs={}",
                prior_refs.join(",")
            );
        }
    }
    rewritten
}

pub(super) fn action_provides_name_listing_evidence(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
            if skill.eq_ignore_ascii_case("list_dir") =>
        {
            true
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "find_entries" | "list_dir"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_search") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "find_name" | "find_ext"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("system_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "inventory_dir" | "find_path" | "structured_keys"
                    )
                })
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("config_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| action.eq_ignore_ascii_case("list_keys"))
        }
        _ => false,
    }
}

pub(super) fn path_metadata_facts_plan_satisfies_route(
    route_result: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if route_result.output_contract_marker_is(crate::OutputSemanticKind::ExistenceWithPath)
        || (route_result.output_contract_is_unclassified()
            && route_result.output_contract.requires_content_evidence
            && matches!(
                route_result.output_contract.locator_kind,
                crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
            )
            && !route_result.output_contract.delivery_required)
    {
        return path_metadata_facts_response_is_sufficient(actions)
            || path_metadata_facts_synthesizes_terminal_answer(actions);
    }
    route_requests_path_metadata_compare(route_result)
        && (structured_scalar_observation_units(actions) >= 2
            || actions_satisfy_single_path_metadata_facts(route_result, actions))
}

pub(super) fn path_metadata_facts_synthesizes_terminal_answer(actions: &[AgentAction]) -> bool {
    actions.iter().any(planned_action_is_path_metadata_facts)
        && actions
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
        && matches!(
            actions.last(),
            Some(AgentAction::Respond { content }) if content.contains("{{")
        )
}

pub(super) fn has_workspace_text_content_evidence(
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .any(executed_step_reads_workspace_text_content)
        || actions.iter().any(action_reads_workspace_text_content)
}

pub(super) fn has_run_cmd_observation_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(action_skill_is_run_cmd)
}

pub(super) fn workspace_synthesis_needs_more_text_evidence(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    route_needs_workspace_synthesis_evidence(route)
        && has_only_workspace_summary_observation_actions(actions)
        && !has_listing_grounded_synthesis_answer_plan(route, actions)
        && !has_workspace_text_content_evidence(loop_state, actions)
        && !has_compact_structured_observation_answer_plan(actions)
        && !has_mixed_last_output_terminal_respond(actions)
        && !has_run_cmd_observation_action(actions)
}

pub(super) fn has_only_workspace_summary_observation_actions(actions: &[AgentAction]) -> bool {
    let mut saw_observation = false;
    for action in actions {
        match action {
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. } => {
                saw_observation = true;
                if !action_is_workspace_summary_evidence(action) {
                    return false;
                }
            }
            _ => {}
        }
    }
    saw_observation
}

pub(super) fn has_listing_grounded_synthesis_answer_plan(
    route: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    route.output_contract_is_unclassified()
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && has_discussion_followup_action(actions)
        && actions.iter().any(action_is_directory_listing_observation)
}

pub(super) fn action_is_directory_listing_observation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
            if skill.eq_ignore_ascii_case("list_dir") =>
        {
            true
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| action.eq_ignore_ascii_case("list_dir"))
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("system_basic") =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|action| action.eq_ignore_ascii_case("inventory_dir"))
        }
        _ => false,
    }
}

pub(super) fn strip_workspace_synthesis_without_text_evidence(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !workspace_synthesis_needs_more_text_evidence(route_result, loop_state, &actions)
        || !has_discussion_followup_action(&actions)
    {
        return actions;
    }

    let stripped = actions
        .iter()
        .filter(|action| !is_discussion_followup_action(action))
        .cloned()
        .collect::<Vec<_>>();
    if stripped.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    }) {
        info!("plan_strip_workspace_synthesis_without_text_evidence");
        stripped
    } else {
        actions
    }
}

pub(super) fn append_synthesize_for_unscoped_workspace_text_evidence(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_needs_unscoped_workspace_text_evidence(route)
        || has_discussion_followup_action(&actions)
        || workspace_synthesis_needs_more_text_evidence(route_result, loop_state, &actions)
        || has_compact_structured_observation_action(&actions)
    {
        return actions;
    }
    let evidence_refs = actions
        .iter()
        .enumerate()
        .filter_map(|(idx, action)| {
            action_reads_workspace_text_content(action).then(|| format!("step_{}", idx + 1))
        })
        .collect::<Vec<_>>();
    if evidence_refs.is_empty() {
        return actions;
    }
    let mut rewritten = actions;
    let refs_log = evidence_refs.join(",");
    rewritten.push(AgentAction::SynthesizeAnswer { evidence_refs });
    info!("plan_append_unscoped_workspace_text_evidence_synthesis refs={refs_log}");
    rewritten
}

pub(super) fn action_reads_git_history(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("git_basic") =>
        {
            args.get("action")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .is_some_and(|action| {
                    matches!(
                        action.to_ascii_lowercase().as_str(),
                        "log" | "show" | "status" | "diff" | "diff_cached" | "changed_files"
                    )
                })
        }
        _ => false,
    }
}

pub(super) fn workspace_summary_readme_path_like(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|name| matches!(name.as_str(), "readme.md" | "readme.zh-cn.md"))
}

pub(super) fn action_reads_workspace_summary_readme(action: &AgentAction) -> bool {
    action_reads_workspace_text_content(action)
        && action_workspace_summary_path(action).is_some_and(workspace_summary_readme_path_like)
}

pub(super) fn ensure_workspace_synthesis_has_default_text_evidence(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if loop_state.has_tool_or_skill_output
        || !route_needs_workspace_summary_default_evidence(route)
        || !has_executable_observation_or_action(&actions)
        || !has_discussion_followup_action(&actions)
        || has_mixed_last_output_terminal_respond(&actions)
        || has_run_cmd_observation_action(&actions)
    {
        return actions;
    }
    let has_text_evidence = actions.iter().any(action_reads_workspace_summary_readme);
    let has_git_history = actions.iter().any(action_reads_git_history);
    if has_text_evidence && has_git_history {
        return actions;
    }
    let insert_idx = actions
        .iter()
        .position(is_discussion_followup_action)
        .unwrap_or(actions.len());
    let mut rewritten = Vec::with_capacity(actions.len() + 2);
    rewritten.extend(actions[..insert_idx].iter().cloned());
    if !has_git_history {
        rewritten.push(AgentAction::CallSkill {
            skill: "git_basic".to_string(),
            args: serde_json::json!({
                "action": "log",
                "n": 8,
            }),
        });
    }
    if !has_text_evidence {
        rewritten.push(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": "README.md",
                "mode": "head",
                "n": 40,
            }),
        });
    }
    rewritten.extend(actions[insert_idx..].iter().cloned());
    info!(
        "plan_ensure_workspace_synthesis_default_evidence added_git={} added_text={}",
        !has_git_history, !has_text_evidence
    );
    rewritten
}

pub(super) fn has_compact_structured_observation_answer_plan(actions: &[AgentAction]) -> bool {
    actions
        .iter()
        .filter(|action| action_is_compact_structured_observation(action))
        .take(2)
        .count()
        >= 2
        && has_discussion_followup_action(actions)
}

pub(super) fn has_compact_structured_observation_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(action_is_compact_structured_observation)
}

pub(super) fn action_is_compact_structured_observation(action: &AgentAction) -> bool {
    let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
        action
    else {
        return false;
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if skill.eq_ignore_ascii_case("fs_basic") {
        return matches!(
            action_name.as_str(),
            "list_dir" | "compare_paths" | "stat_paths"
        );
    }
    if skill.eq_ignore_ascii_case("config_basic") {
        return matches!(
            action_name.as_str(),
            "read_field" | "read_fields" | "list_keys"
        );
    }
    skill.eq_ignore_ascii_case("system_basic")
        && matches!(
            action_name.as_str(),
            "count_inventory" | "compare_paths" | "path_batch_facts" | "extract_fields"
        )
}

pub(super) fn action_workspace_summary_path(action: &AgentAction) -> Option<&str> {
    match action {
        AgentAction::CallSkill { skill, args } if skill == "list_dir" || skill == "read_file" => {
            args.get("path").and_then(|value| value.as_str())
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic" =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .filter(|action| {
                    matches!(
                        action.trim().to_ascii_lowercase().as_str(),
                        "list_dir" | "read_text_range" | "stat_paths"
                    )
                })
                .and_then(|_| {
                    args.get("path")
                        .or_else(|| args.get("root"))
                        .and_then(|value| value.as_str())
                })
        }
        AgentAction::CallSkill { skill, args } if skill == "system_basic" => args
            .get("action")
            .and_then(|value| value.as_str())
            .filter(|action| {
                matches!(
                    action.trim().to_ascii_lowercase().as_str(),
                    "inventory_dir" | "read_range" | "workspace_glance" | "tree_summary"
                )
            })
            .and_then(|_| {
                args.get("path")
                    .or_else(|| args.get("root"))
                    .and_then(|value| value.as_str())
            }),
        _ => None,
    }
}

pub(super) fn path_matches_workspace_scope_hint(path: &str, scope_hint: &str) -> bool {
    let path = path.trim().trim_end_matches(['/', '\\']);
    let scope_hint = scope_hint.trim().trim_end_matches(['/', '\\']);
    if path.is_empty()
        || scope_hint.is_empty()
        || matches!(path, "." | "./" | "/" | "")
        || matches!(scope_hint, "." | "./" | "/" | "")
    {
        return false;
    }
    let path_lower = path.to_ascii_lowercase();
    let hint_lower = scope_hint.to_ascii_lowercase();
    if path_lower == hint_lower {
        return true;
    }
    if path_lower
        .strip_prefix(&hint_lower)
        .is_some_and(|suffix| suffix.starts_with('/'))
    {
        return true;
    }
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(scope_hint))
}

pub(super) fn route_requests_structured_scalar_compare(route: &RouteResult) -> bool {
    let required_evidence_fields =
        crate::evidence_policy::required_evidence_fields_for_route(route);
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && route.output_contract_marker_is_any(&[
            crate::OutputSemanticKind::QuantityComparison,
            crate::OutputSemanticKind::RecentScalarEqualityCheck,
        ])
        && required_evidence_fields
            .iter()
            .any(|field| matches!(field.as_str(), "field_value" | "size_bytes"))
}

pub(super) fn route_requests_path_metadata_compare(route: &RouteResult) -> bool {
    route_requests_structured_scalar_compare(route)
        && (route.output_contract_marker_is(crate::OutputSemanticKind::QuantityComparison)
            || (route
                .output_contract_marker_is(crate::OutputSemanticKind::RecentScalarEqualityCheck)
                && route_has_multiple_locator_targets(route)))
}

fn route_has_multiple_locator_targets(route: &RouteResult) -> bool {
    crate::evidence_policy::target_locators_for_route(route)
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .take(2)
        .count()
        >= 2
}

pub(super) fn action_scalar_compare_observation_units(action: &AgentAction) -> usize {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic" =>
        {
            match args
                .get("action")
                .and_then(|value| value.as_str())
                .map(|action| action.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("compare_paths") => {
                    let has_pair = args
                        .get("left_path")
                        .and_then(|value| value.as_str())
                        .is_some_and(|path| !path.trim().is_empty())
                        && args
                            .get("right_path")
                            .and_then(|value| value.as_str())
                            .is_some_and(|path| !path.trim().is_empty());
                    if has_pair {
                        2
                    } else {
                        string_list_from_value(args.get("paths"))
                            .into_iter()
                            .chain(string_list_from_value(args.get("targets")))
                            .take(2)
                            .count()
                    }
                }
                Some("count_entries") => {
                    args.get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| !path.trim().is_empty()) as usize
                }
                Some("stat_paths") => string_list_from_value(args.get("paths"))
                    .into_iter()
                    .chain(string_list_from_value(args.get("targets")))
                    .chain(string_list_from_value(args.get("path")))
                    .take(2)
                    .count(),
                Some("list_dir") => {
                    args.get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| !path.trim().is_empty()) as usize
                }
                _ => 0,
            }
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "config_basic" =>
        {
            match args
                .get("action")
                .and_then(|value| value.as_str())
                .map(|action| action.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("read_field") => 1,
                Some("read_fields") => args
                    .get("field_paths")
                    .and_then(|value| value.as_array())
                    .map(|field_paths| field_paths.len())
                    .or_else(|| {
                        args.get("fields")
                            .and_then(|value| value.as_array())
                            .map(Vec::len)
                    })
                    .unwrap_or(1),
                Some("list_keys") | Some("validate") => {
                    args.get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| !path.trim().is_empty()) as usize
                }
                _ => 0,
            }
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "system_basic" =>
        {
            match args
                .get("action")
                .and_then(|value| value.as_str())
                .map(|action| action.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("extract_field") => 1,
                Some("extract_fields") => args
                    .get("field_paths")
                    .and_then(|value| value.as_array())
                    .map(|field_paths| field_paths.len())
                    .unwrap_or(1),
                Some("count_inventory") | Some("inventory_dir") => {
                    args.get("path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| !path.trim().is_empty()) as usize
                }
                Some("compare_paths") => {
                    let has_pair = args
                        .get("left_path")
                        .and_then(|value| value.as_str())
                        .is_some_and(|path| !path.trim().is_empty())
                        && args
                            .get("right_path")
                            .and_then(|value| value.as_str())
                            .is_some_and(|path| !path.trim().is_empty());
                    if has_pair {
                        2
                    } else {
                        string_list_from_value(args.get("paths"))
                            .into_iter()
                            .chain(string_list_from_value(args.get("targets")))
                            .take(2)
                            .count()
                    }
                }
                Some("path_batch_facts") => string_list_from_value(args.get("paths"))
                    .into_iter()
                    .chain(string_list_from_value(args.get("targets")))
                    .take(2)
                    .count(),
                _ => 0,
            }
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "git_basic" =>
        {
            match args
                .get("action")
                .and_then(|value| value.as_str())
                .map(|action| action.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("current_branch" | "rev_parse") => 1,
                _ => 0,
            }
        }
        _ => 0,
    }
}

pub(super) fn structured_scalar_observation_units(actions: &[AgentAction]) -> usize {
    actions
        .iter()
        .map(action_scalar_compare_observation_units)
        .sum()
}

pub(super) fn action_is_single_directory_count_observation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "fs_basic" =>
        {
            args.get("action")
                .and_then(Value::as_str)
                .map(|action| action.trim().to_ascii_lowercase())
                .as_deref()
                == Some("count_entries")
                && args
                    .get("path")
                    .and_then(Value::as_str)
                    .is_some_and(|path| !path.trim().is_empty())
        }
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "system_basic" =>
        {
            matches!(
                args.get("action")
                    .and_then(Value::as_str)
                    .map(|action| action.trim().to_ascii_lowercase())
                    .as_deref(),
                Some("count_inventory")
            ) && args
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| !path.trim().is_empty())
        }
        _ => false,
    }
}

pub(super) fn actions_satisfy_single_scalar_count(
    route: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar {
        return false;
    }
    let count_actions = actions
        .iter()
        .filter(|action| action_is_single_directory_count_observation(action))
        .count();
    count_actions == 1
}

pub(super) fn action_is_single_path_metadata_observation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(|action| action.trim().to_ascii_lowercase());
            let paths = string_list_from_value(args.get("paths"))
                .into_iter()
                .chain(string_list_from_value(args.get("targets")))
                .chain(string_list_from_value(args.get("path")))
                .map(|path| path.trim().to_string())
                .filter(|path| !path.is_empty())
                .collect::<Vec<_>>();
            paths.len() == 1
                && ((skill == "fs_basic" && action_name.as_deref() == Some("stat_paths"))
                    || (skill == "system_basic"
                        && action_name.as_deref() == Some("path_batch_facts")))
        }
        AgentAction::Think { .. }
        | AgentAction::CallCapability { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

pub(super) fn actions_satisfy_single_path_metadata_facts(
    route: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::QuantityComparison)
        || route.output_contract.delivery_required
        || crate::evidence_policy::target_locators_for_route(route).len() > 1
    {
        return false;
    }
    let mut metadata_observations = 0usize;
    for action in actions {
        if action_is_single_path_metadata_observation(action) {
            metadata_observations += 1;
            continue;
        }
        if matches!(
            action,
            AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
        ) {
            continue;
        }
        return false;
    }
    metadata_observations == 1
}

pub(super) fn action_is_git_scalar_field_observation(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill == "git_basic" =>
        {
            matches!(
                args.get("action")
                    .and_then(Value::as_str)
                    .map(|action| action.trim().to_ascii_lowercase())
                    .as_deref(),
                Some("current_branch" | "rev_parse")
            )
        }
        _ => false,
    }
}

pub(super) fn actions_satisfy_current_workspace_scalar_field_observation(
    route: &RouteResult,
    actions: &[AgentAction],
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::RecentScalarEqualityCheck)
        || route.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
    {
        return false;
    }
    let mut git_scalar_observations = 0usize;
    for action in actions {
        if action_is_git_scalar_field_observation(action) {
            git_scalar_observations += 1;
            continue;
        }
        if matches!(
            action,
            AgentAction::SynthesizeAnswer { .. }
                | AgentAction::Respond { .. }
                | AgentAction::Think { .. }
        ) {
            continue;
        }
        return false;
    }
    git_scalar_observations == 1
}
