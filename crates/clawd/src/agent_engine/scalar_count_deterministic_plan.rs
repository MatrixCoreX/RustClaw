use super::*;

pub(super) fn scalar_count_filter_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !scalar_count_contract_allows_count_shape(route)
        || !route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount)
    {
        return None;
    }
    let hint = scalar_count_filter_hint_for_route_or_turn(route, turn_analysis)?;
    let path = route_directory_locator_path(route, auto_locator_path)?;
    let mut args = serde_json::json!({
        "action": "count_entries",
        "path": path,
    });
    if let Some(obj) = args.as_object_mut() {
        apply_scalar_count_filter_hint(obj, &hint);
    }
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args,
    }];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn path_metadata_compare_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.is_execute_gate()
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || !route_requests_path_metadata_compare(route)
    {
        return None;
    }
    let (left, right) = two_route_locator_targets(route)?;
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "compare_paths",
            "left_path": left,
            "right_path": right,
        }),
    }];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn resolve_directory_locator_for_dir_compare(
    workspace_root: &Path,
    raw: &str,
) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let path = Path::new(raw);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    if !path.is_dir() {
        return None;
    }
    Some(path.canonicalize().unwrap_or(path).display().to_string())
}

pub(super) fn scalar_path_auto_locator_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions = scalar_path_auto_locator_observation_plan(route_result, auto_locator_path)?;
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn scalar_path_current_workspace_deterministic_plan_result(
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
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || !route.output_contract_marker_is(crate::OutputSemanticKind::ScalarPathOnly)
        || route.output_contract.locator_kind != crate::OutputLocatorKind::CurrentWorkspace
    {
        return None;
    }
    let path = state.skill_rt.workspace_root.display().to_string();
    if path.trim().is_empty() {
        return None;
    }
    let action = AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": [path],
            "include_missing": true,
        }),
    };
    let AgentAction::CallTool { tool, args } = &action else {
        return None;
    };
    if crate::contract_matrix::action_policy_for_route(Some(route), tool, args)
        .is_some_and(|policy| !policy.is_allowed())
    {
        return None;
    }
    let actions = vec![action];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn scalar_path_directory_locator_search_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || !route.output_contract_marker_is(crate::OutputSemanticKind::ScalarPathOnly)
    {
        return None;
    }
    let root = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&root).is_dir() {
        return None;
    }
    let target =
        single_name_target_for_directory_locator(route, current_user_text).or_else(|| {
            single_existing_name_target_for_directory_locator(&root, route, current_user_text)
        })?;
    Some(vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": root,
            "pattern": target,
            "target_kind": "any",
            "max_results": 50,
        }),
    }])
}

pub(super) fn scalar_path_directory_locator_search_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    current_user_text: &str,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let actions = scalar_path_directory_locator_search_observation_plan(
        route_result,
        auto_locator_path,
        current_user_text,
    )?;
    let actions = canonicalize_legacy_file_config_capabilities(actions);
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn explicit_command_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    original_user_text: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route_allows_explicit_command_preservation(route_result)
        || !run_cmd_available_for_plan(state)
    {
        return None;
    }
    let request_text = original_user_text.trim();
    if route.output_contract_marker_is(crate::OutputSemanticKind::ExecutionFailedStep) {
        let commands = execution_failed_step_literal_command_segments(
            &state.policy.command_intent,
            request_text,
            turn_analysis,
        );
        if commands.len() >= 2 {
            let continue_on_error = commands.len() == 2
                && conditional_step_update_immediate_command_count(turn_analysis).is_none();
            let mut actions = explicit_run_cmd_observation_actions(
                state,
                request_text,
                commands,
                continue_on_error,
            );
            append_terminal_synthesis_for_step_evidence(&mut actions);
            let raw_plan_text =
                serde_json::to_string(&serde_json::json!({ "steps": actions.clone() }))
                    .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
            return Some(build_plan_result(
                goal,
                &raw_plan_text,
                PlanKind::Single,
                &actions,
            ));
        }
    }
    if explicit_command_plan_needs_terminal_synthesis(route_result) {
        let commands = execution_failed_step_literal_command_segments(
            &state.policy.command_intent,
            request_text,
            turn_analysis,
        );
        if commands.len() >= 2 {
            let continue_on_error = commands.len() == 2
                && conditional_step_update_immediate_command_count(turn_analysis).is_none();
            let mut actions = explicit_run_cmd_observation_actions(
                state,
                request_text,
                commands,
                continue_on_error,
            );
            append_terminal_synthesis_for_step_evidence(&mut actions);
            let raw_plan_text =
                serde_json::to_string(&serde_json::json!({ "steps": actions.clone() }))
                    .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
            return Some(build_plan_result(
                goal,
                &raw_plan_text,
                PlanKind::Single,
                &actions,
            ));
        }
    }
    let command = explicit_command_deterministic_command_segment(
        &state.policy.command_intent,
        request_text,
        route_result,
    )?;
    let mut args = serde_json::json!({
        "command": command,
        "request_text": request_text,
        "cwd": state.skill_rt.workspace_root.display().to_string(),
    });
    args[super::super::CLAWD_LITERAL_COMMAND_ARG] = Value::Bool(true);
    if literal_command_failure_can_replan(route_result) {
        args[super::super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG] = Value::Bool(true);
    }
    let mut actions = vec![AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: args.clone(),
    }];
    if explicit_command_plan_needs_terminal_synthesis(route_result) {
        actions.push(AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        });
        actions.push(AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        });
    }
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions.clone() }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn explicit_command_request_present(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
    route_result: Option<&RouteResult>,
) -> bool {
    if !route_allows_explicit_command_preservation(route_result) {
        return false;
    }
    if explicit_command_segment(runtime, request).is_some() {
        return true;
    }
    route_result.is_some_and(|route| {
        (route.output_contract_marker_is(crate::OutputSemanticKind::ExecutionFailedStep)
            || explicit_command_plan_needs_terminal_synthesis(Some(route)))
            && execution_failed_step_literal_command_segments(runtime, request, None).len() >= 2
    })
}

pub(super) fn append_terminal_synthesis_for_step_evidence(actions: &mut Vec<AgentAction>) {
    let evidence_refs = (1..=actions.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    actions.push(AgentAction::SynthesizeAnswer { evidence_refs });
    actions.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
}

pub(super) fn explicit_run_cmd_observation_actions(
    state: &AppState,
    request_text: &str,
    commands: Vec<String>,
    continue_on_error: bool,
) -> Vec<AgentAction> {
    commands
        .into_iter()
        .map(|command| {
            let mut args = serde_json::json!({
                "command": command,
                "request_text": request_text,
                "cwd": state.skill_rt.workspace_root.display().to_string(),
            });
            args[super::super::CLAWD_LITERAL_COMMAND_ARG] = Value::Bool(true);
            if continue_on_error {
                args[super::super::CLAWD_CONTINUE_ON_ERROR_ARG] = Value::Bool(true);
            }
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args,
            }
        })
        .collect()
}

pub(super) fn explicit_command_deterministic_command_segment(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
    route_result: Option<&RouteResult>,
) -> Option<String> {
    if explicit_command_plan_needs_terminal_synthesis(route_result) {
        configured_distinct_standalone_command_sequence_from_text(runtime, request)
            .or_else(|| explicit_command_single_step_segment(runtime, request))
    } else {
        explicit_command_single_step_segment(runtime, request)
    }
}

pub(super) fn execution_failed_step_literal_command_segments(
    runtime: &crate::CommandIntentRuntime,
    request: &str,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Vec<String> {
    let quoted = shellish_literal_command_segments(request, true);
    if quoted.len() >= 2 {
        return apply_conditional_step_update_execution_window(quoted, turn_analysis);
    }
    let prefixed = prefixed_shellish_command_segments(runtime, request, true);
    if prefixed.len() >= 2 {
        return apply_conditional_step_update_execution_window(prefixed, turn_analysis);
    }
    apply_conditional_step_update_execution_window(quoted, turn_analysis)
}

pub(super) fn apply_conditional_step_update_execution_window(
    commands: Vec<String>,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Vec<String> {
    let Some(limit) = conditional_step_update_immediate_command_count(turn_analysis) else {
        return commands;
    };
    commands.into_iter().take(limit).collect()
}

pub(super) fn conditional_step_update_immediate_command_count(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<usize> {
    let update = turn_analysis?
        .state_patch
        .as_ref()?
        .get("conditional_step_update")?
        .as_object()?;
    let has_replacement = update
        .get("replacement_command")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let has_original = update
        .get("original_command")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if !has_replacement || !has_original {
        return None;
    }
    let step_to_modify = update.get("step_to_modify")?.as_u64()? as usize;
    step_to_modify.checked_sub(1).filter(|limit| *limit > 0)
}

pub(super) fn explicit_command_plan_needs_terminal_synthesis(
    route_result: Option<&RouteResult>,
) -> bool {
    route_result.is_some_and(|route| {
        route.output_contract.requires_content_evidence
            && !route.output_contract.delivery_required
            && !matches!(
                route.output_contract.response_shape,
                crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
            )
            && route.output_contract_marker_is(crate::OutputSemanticKind::CommandOutputSummary)
    })
}

pub(super) fn contract_hint_preferred_action_ref(
    original_user_text: &str,
) -> Option<crate::contract_matrix::ActionRef> {
    crate::intent_router::contract_test_hint_value(original_user_text, "preferred_action_ref")
        .and_then(|value| crate::contract_matrix::ActionRef::parse(&value))
}

pub(super) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(super) fn route_locator_targets(route: &RouteResult) -> Vec<String> {
    crate::task_contract::target_locators_for_route(route)
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect()
}

pub(super) fn first_route_locator_target(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    route_locator_targets(route).into_iter().next().or_else(|| {
        auto_locator_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_string)
    })
}

pub(super) fn two_route_locator_targets(route: &RouteResult) -> Option<(String, String)> {
    let targets = route_locator_targets(route);
    (targets.len() >= 2).then(|| (targets[0].clone(), targets[1].clone()))
}

pub(super) fn recent_child_paths_for_directory(path: &str, limit: usize) -> Option<Vec<String>> {
    let mut entries = fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| {
                let modified = metadata.modified().ok();
                (entry.path(), modified)
            })
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    entries.sort_by(|(left_path, left_modified), (right_path, right_modified)| {
        right_modified
            .cmp(left_modified)
            .then_with(|| left_path.cmp(right_path))
    });
    let out = entries
        .into_iter()
        .take(limit)
        .map(|(path, _)| path.display().to_string())
        .collect::<Vec<_>>();
    (!out.is_empty()).then_some(out)
}

pub(super) fn preferred_read_text_range_path_for_contract_hint(
    path: &str,
    workspace_root: &Path,
) -> Option<String> {
    let raw_target = Path::new(path);
    let target_storage;
    let target = if raw_target.is_absolute() || raw_target.exists() {
        raw_target
    } else {
        target_storage = workspace_root.join(raw_target);
        target_storage.as_path()
    };
    if target.is_file() {
        return Some(target.display().to_string());
    }
    if !target.is_dir() {
        return Some(path.to_string());
    }

    for name in [
        "README.md",
        "README.zh-CN.md",
        "README_cn.md",
        "package.json",
        "Cargo.toml",
    ] {
        let candidate = target.join(name);
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
    }

    let mut candidates = fs::read_dir(target)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            matches!(
                ext.as_str(),
                "md" | "txt" | "toml" | "json" | "yaml" | "yml"
            )
            .then_some(path)
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .map(|path| path.display().to_string())
}

pub(super) fn scalar_path_find_entries_args(path: &str) -> Value {
    let path_obj = Path::new(path);
    let root = path_obj
        .parent()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(".");
    let pattern = path_obj
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path);
    serde_json::json!({
        "action": "find_entries",
        "root": root,
        "pattern": pattern,
        "target_kind": "any",
        "max_results": 50,
    })
}

pub(super) fn route_prefers_text_excerpt_action_for_contract_hint(route: &RouteResult) -> bool {
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && route.output_contract.locator_kind == crate::OutputLocatorKind::Path
        && (route.output_contract_is_unclassified()
            || route.output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary))
}

pub(super) fn contract_hint_selector_value(original_user_text: &str, key: &str) -> Option<String> {
    crate::intent_router::contract_test_hint_value(original_user_text, key)
        .or_else(|| inline_selector_machine_token_value(original_user_text, key))
        .or_else(|| json_selector_machine_token_value(original_user_text, key))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn inline_selector_machine_token_value(text: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    text.split(|ch: char| {
        ch.is_whitespace() || matches!(ch, ',' | ';' | '[' | ']' | '{' | '}' | '(' | ')')
    })
    .find_map(|token| token.strip_prefix(&needle))
    .map(|value| {
        value
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ':' | '.'))
            .to_string()
    })
    .filter(|value| !value.is_empty())
}

pub(super) fn json_selector_machine_token_value(text: &str, key: &str) -> Option<String> {
    let quoted_key = format!("\"{key}\"");
    let mut remaining = text;
    while let Some((_, after_key)) = remaining.split_once(&quoted_key) {
        let after_colon = after_key.trim_start();
        let Some(after_colon) = after_colon.strip_prefix(':') else {
            remaining = after_key;
            continue;
        };
        let value = after_colon.trim_start();
        if let Some(after_quote) = value.strip_prefix('"') {
            let Some((raw, _)) = after_quote.split_once('"') else {
                return None;
            };
            return (!raw.trim().is_empty()).then(|| raw.trim().to_string());
        }
        let raw = value
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
            .collect::<String>();
        return (!raw.trim().is_empty()).then(|| raw.trim().to_string());
    }
    None
}

pub(super) fn contract_hint_selector_bool(original_user_text: &str, key: &str) -> Option<bool> {
    contract_hint_selector_value(original_user_text, key).and_then(|value| {
        match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    })
}

pub(super) fn contract_hint_selector_query(original_user_text: &str) -> Option<String> {
    contract_hint_selector_value(original_user_text, "selector_query")
        .map(|value| value.replace(['\r', '\n'], " "))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value.len() <= 160)
}

pub(super) fn contract_hint_selector_case_insensitive(original_user_text: &str) -> Option<bool> {
    ["selector_case_insensitive", "selector_ignore_case"]
        .iter()
        .find_map(|key| contract_hint_selector_bool(original_user_text, key))
}

pub(super) fn contract_hint_selector_include_metadata(original_user_text: &str) -> Option<bool> {
    ["selector_include_metadata", "include_metadata"]
        .iter()
        .find_map(|key| contract_hint_selector_bool(original_user_text, key))
}

pub(super) fn contract_hint_selector_extension(original_user_text: &str) -> Option<String> {
    ["selector_extension", "file_extension"]
        .iter()
        .find_map(|key| contract_hint_selector_value(original_user_text, key))
        .map(|value| value.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|value| {
            (1..=16).contains(&value.len()) && value.chars().all(|ch| ch.is_ascii_alphanumeric())
        })
}

pub(super) fn contract_hint_selector_limit(original_user_text: &str) -> Option<u64> {
    contract_hint_selector_value(original_user_text, "selector_limit")
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.clamp(1, 1000))
}

pub(super) fn contract_hint_selector_sort_by(original_user_text: &str) -> Option<String> {
    contract_hint_selector_value(original_user_text, "selector_sort_by")
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| {
            matches!(
                value.as_str(),
                "name" | "name_desc" | "mtime_desc" | "mtime_asc" | "size_desc" | "size_asc"
            )
        })
}

pub(super) fn contract_hint_selector_target_kind(original_user_text: &str) -> Option<String> {
    contract_hint_selector_value(original_user_text, "selector_target_kind")
        .map(|value| value.to_ascii_lowercase())
        .and_then(|value| match value.as_str() {
            "file" | "files" => Some("file".to_string()),
            "dir" | "dirs" | "directory" | "directories" => Some("dir".to_string()),
            "any" => Some("any".to_string()),
            _ => None,
        })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ContentSliceSpec {
    mode: Option<String>,
    n: Option<u64>,
    start_line: Option<u64>,
    end_line: Option<u64>,
}

impl ContentSliceSpec {
    fn merge_from(&mut self, other: ContentSliceSpec) {
        if self.mode.is_none() {
            self.mode = other.mode;
        }
        if self.n.is_none() {
            self.n = other.n;
        }
        if self.start_line.is_none() {
            self.start_line = other.start_line;
        }
        if self.end_line.is_none() {
            self.end_line = other.end_line;
        }
    }

    fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.n.is_none()
            && self.start_line.is_none()
            && self.end_line.is_none()
    }
}

pub(super) fn contract_hint_slice_mode(text: &str) -> Option<String> {
    contract_hint_selector_value(text, "slice_mode")
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| matches!(value.as_str(), "head" | "tail" | "range"))
}

pub(super) fn contract_hint_slice_number(text: &str, key: &str) -> Option<u64> {
    contract_hint_selector_value(text, key)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.clamp(1, 1_000_000))
}

pub(super) fn contract_hint_slice_n(text: &str) -> Option<u64> {
    contract_hint_slice_number(text, "slice_n").map(|value| value.clamp(1, 500))
}

pub(super) fn content_slice_spec_from_text(text: &str) -> Option<ContentSliceSpec> {
    let spec = ContentSliceSpec {
        mode: contract_hint_slice_mode(text),
        n: contract_hint_slice_n(text),
        start_line: contract_hint_slice_number(text, "slice_start"),
        end_line: contract_hint_slice_number(text, "slice_end"),
    };
    (!spec.is_empty()).then_some(spec)
}

pub(super) fn content_slice_spec_from_sources<'a>(
    sources: impl IntoIterator<Item = &'a str>,
) -> Option<ContentSliceSpec> {
    let mut merged = ContentSliceSpec::default();
    for source in sources {
        if let Some(spec) = content_slice_spec_from_text(source) {
            merged.merge_from(spec);
        }
    }
    (!merged.is_empty()).then_some(merged)
}

pub(super) fn route_content_slice_spec(route: &RouteResult) -> Option<ContentSliceSpec> {
    content_slice_spec_from_sources([route.resolved_intent.as_str(), route.route_reason.as_str()])
}

pub(super) fn apply_content_slice_spec_to_read_args(
    obj: &mut serde_json::Map<String, Value>,
    spec: Option<ContentSliceSpec>,
    default_mode: &str,
    default_n: u64,
) {
    let spec = spec.unwrap_or_default();
    let uses_explicit_range = spec.start_line.is_some() || spec.end_line.is_some();
    let mode = if uses_explicit_range {
        "range".to_string()
    } else {
        spec.mode.unwrap_or_else(|| default_mode.to_string())
    };
    obj.insert("mode".to_string(), Value::String(mode.clone()));
    if mode == "range" {
        if let Some(start_line) = spec.start_line {
            obj.insert("start_line".to_string(), Value::Number(start_line.into()));
        }
        if let Some(end_line) = spec.end_line {
            obj.insert("end_line".to_string(), Value::Number(end_line.into()));
        }
    }
    let n = spec.n.unwrap_or(default_n).clamp(1, 500);
    obj.insert("n".to_string(), Value::Number(n.into()));
}

pub(super) fn preferred_run_cmd_for_contract_hint(
    state: &AppState,
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<AgentAction> {
    let cwd = state.skill_rt.workspace_root.display().to_string();
    let command = if crate::machine_capability_ref::route_capability_action_for_namespaces(
        route,
        &["package", "package_manager"],
    )
    .is_some_and(|action| action_has_any_segment(action, &["detect"]))
    {
        r#"for m in apt-get apt dnf yum brew pacman zypper apk; do if command -v "$m" >/dev/null 2>&1; then printf 'manager=%s\nbasis=command_path:%s\n' "$m" "$m"; exit 0; fi; done; printf 'manager=unknown\nbasis=path_scan_none\n'"#.to_string()
    } else if let Some(action) =
        crate::machine_capability_ref::route_capability_action_for_namespaces(route, &["docker"])
    {
        docker_readonly_probe_command_from_capability_action(action).to_string()
    } else {
        match route.effective_output_contract_semantic_kind() {
            crate::OutputSemanticKind::QuantityComparison => {
                let (left, right) = two_route_locator_targets(route)?;
                format!(
                    "stat -c 'size_bytes=%s path=%n' {} {} 2>/dev/null || wc -c {} {}",
                    shell_single_quote(&left),
                    shell_single_quote(&right),
                    shell_single_quote(&left),
                    shell_single_quote(&right)
                )
            }
            crate::OutputSemanticKind::ScalarCount => {
                let path = first_route_locator_target(route, auto_locator_path)?;
                format!(
                    "find {} -mindepth 1 -maxdepth 1 2>/dev/null | wc -l | tr -d ' '",
                    shell_single_quote(&path)
                )
            }
            crate::OutputSemanticKind::RecentScalarEqualityCheck => {
                "git branch --show-current | awk '{print \"field_value=\" $0}'".to_string()
            }
            crate::OutputSemanticKind::ServiceStatus => {
                let filter = process_status_contract_filter_token(route)
                    .unwrap_or_else(|| "clawd".to_string());
                format!(
                    "ps -eo pid,comm,args | grep -F {} | grep -v grep || true",
                    shell_single_quote(&filter)
                )
            }
            crate::OutputSemanticKind::SqliteTableListing
            | crate::OutputSemanticKind::SqliteTableNamesOnly
            | crate::OutputSemanticKind::SqliteDatabaseKindJudgment => {
                let db_path = first_route_locator_target(route, auto_locator_path)?;
                format!(
                    "sqlite3 {} '.tables' | tr -s ' ' '\\n' | sed '/^$/d'",
                    shell_single_quote(&db_path)
                )
            }
            crate::OutputSemanticKind::SqliteSchemaVersion => {
                let db_path = first_route_locator_target(route, auto_locator_path)?;
                format!(
                    "sqlite3 {} 'PRAGMA schema_version;' | awk '{{print \"schema_version=\" $0}}'",
                    shell_single_quote(&db_path)
                )
            }
            _ => return None,
        }
    };
    let mut args = serde_json::json!({
        "command": command,
        "cwd": cwd,
    });
    args[super::super::CLAWD_LITERAL_COMMAND_ARG] = Value::Bool(true);
    Some(AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args,
    })
}

fn docker_readonly_probe_command_from_capability_action(action: &str) -> &'static str {
    if action_has_any_segment(action, &["image", "images"]) {
        "docker images 2>&1 || true"
    } else if action_has_any_segment(action, &["inspect", "restart", "start", "stop", "version"]) {
        "docker version 2>&1 || true"
    } else {
        "docker ps 2>&1 || true"
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

pub(super) fn preferred_fs_basic_for_contract_hint(
    state: &AppState,
    route: &RouteResult,
    action_name: &str,
    auto_locator_path: Option<&str>,
    original_user_text: &str,
) -> Option<AgentAction> {
    let action_name = if action_name != "read_text_range"
        && (route.output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary)
            || route_prefers_text_excerpt_action_for_contract_hint(route))
    {
        "read_text_range"
    } else {
        action_name
    };
    let mut args = match action_name {
        "stat_paths" => {
            if route.output_contract_marker_is(crate::OutputSemanticKind::RecentArtifactsJudgment) {
                let root = first_route_locator_target(route, auto_locator_path)?;
                let paths =
                    recent_child_paths_for_directory(&root, 2).unwrap_or_else(|| vec![root]);
                serde_json::json!({"action": "stat_paths", "paths": paths})
            } else if let Some((left, right)) = two_route_locator_targets(route) {
                serde_json::json!({"action": "stat_paths", "paths": [left, right]})
            } else {
                let path = first_route_locator_target(route, auto_locator_path)?;
                serde_json::json!({"action": "stat_paths", "paths": [path]})
            }
        }
        "find_entries" => {
            let path = first_route_locator_target(route, auto_locator_path)?;
            if route.output_contract_marker_is(crate::OutputSemanticKind::ScalarPathOnly) {
                scalar_path_find_entries_args(&path)
            } else {
                let default_target_kind = if route
                    .output_contract_marker_is(crate::OutputSemanticKind::DirectoryEntryGroups)
                {
                    "any"
                } else {
                    "file"
                };
                let target_kind = contract_hint_selector_target_kind(original_user_text)
                    .unwrap_or_else(|| default_target_kind.to_string());
                serde_json::json!({
                    "action": "find_entries",
                    "root": path,
                    "target_kind": target_kind,
                    "max_results": 50,
                    "include_hidden": route.output_contract_marker_is(crate::OutputSemanticKind::HiddenEntriesCheck),
                })
            }
        }
        "count_entries" => {
            let path = first_route_locator_target(route, auto_locator_path)?;
            let mut args = serde_json::json!({"action": "count_entries", "path": path});
            if let Some(hint) = scalar_count_filter_hint_from_route(route) {
                if let Some(obj) = args.as_object_mut() {
                    apply_scalar_count_filter_hint(obj, &hint);
                }
            }
            args
        }
        "compare_paths" => {
            let (left, right) = two_route_locator_targets(route)?;
            serde_json::json!({"action": "compare_paths", "left_path": left, "right_path": right})
        }
        "list_dir" => {
            let path = first_route_locator_target(route, auto_locator_path)?;
            let mut args = serde_json::json!({
                "action": "list_dir",
                "path": path,
                "names_only": false,
                "max_entries": 1000,
                "sort_by": if route.output_contract_marker_is(crate::OutputSemanticKind::RecentArtifactsJudgment) {
                    "mtime_desc"
                } else {
                    "name"
                },
            });
            if let Some(kind) = contract_hint_selector_target_kind(original_user_text) {
                if kind == "file" {
                    args["files_only"] = Value::Bool(true);
                } else if kind == "dir" {
                    args["dirs_only"] = Value::Bool(true);
                }
            }
            if route.output_contract_marker_is(crate::OutputSemanticKind::HiddenEntriesCheck) {
                args["include_hidden"] = Value::Bool(true);
            }
            args
        }
        "read_text_range" => {
            let target = first_route_locator_target(route, auto_locator_path)?;
            let path = preferred_read_text_range_path_for_contract_hint(
                &target,
                &state.skill_rt.workspace_root,
            )
            .unwrap_or(target);
            let mut args = serde_json::json!({
                "action": "read_text_range",
                "path": path
            });
            if let Some(obj) = args.as_object_mut() {
                let spec = content_slice_spec_from_sources([
                    original_user_text,
                    route.resolved_intent.as_str(),
                    route.route_reason.as_str(),
                ]);
                apply_content_slice_spec_to_read_args(obj, spec, "head", 80);
            }
            args
        }
        "grep_text" => {
            let path = first_route_locator_target(route, auto_locator_path)?;
            let query = contract_hint_selector_query(original_user_text)?;
            let mut args = serde_json::json!({
                "action": "grep_text",
                "root": path,
                "query": query,
                "max_results": 50,
            });
            if contract_hint_selector_case_insensitive(original_user_text).unwrap_or(
                route.output_contract_marker_is(crate::OutputSemanticKind::ContentPresenceCheck),
            ) {
                args["case_insensitive"] = Value::Bool(true);
            }
            args
        }
        _ => return None,
    };
    if let Some(obj) = args.as_object_mut() {
        if let Some(limit) = contract_hint_selector_limit(original_user_text) {
            let key = if action_name == "list_dir" {
                "max_entries"
            } else {
                "max_results"
            };
            obj.insert(key.to_string(), Value::Number(limit.into()));
        }
        if let Some(sort_by) = contract_hint_selector_sort_by(original_user_text) {
            obj.insert("sort_by".to_string(), Value::String(sort_by));
        }
        if let Some(extension) = contract_hint_selector_extension(original_user_text) {
            let key = if action_name == "list_dir" {
                "ext_filter"
            } else {
                "extension"
            };
            obj.insert(key.to_string(), Value::String(extension));
        }
    }
    Some(AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args,
    })
}

pub(super) fn config_path_for_contract_hint(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> String {
    first_route_locator_target(route, auto_locator_path)
        .unwrap_or_else(|| "configs/config.toml".to_string())
}

pub(super) fn preferred_config_basic_for_contract_hint(
    route: &RouteResult,
    action_name: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<AgentAction> {
    if route.output_contract_marker_is(crate::OutputSemanticKind::RecentScalarEqualityCheck)
        && first_route_locator_target(route, auto_locator_path).is_none()
    {
        return None;
    }
    let capability_action =
        crate::machine_capability_ref::route_capability_action_for_namespaces(route, &["config"])
            .and_then(config_basic_action_from_capability_action);
    let action = capability_action.or(action_name).unwrap_or("validate");
    let path = config_path_for_contract_hint(route, auto_locator_path);
    if action == "validate" {
        return Some(config_basic_validate_action(path));
    }
    let args = match action {
        "guard_rustclaw_config" => serde_json::json!({
            "action": "guard_rustclaw_config",
            "path": path,
        }),
        "list_keys" => serde_json::json!({
            "action": "list_keys",
            "path": path,
            "max_keys": 200,
        }),
        "read_fields" | "read_field" => serde_json::json!({
            "action": action,
            "path": path,
        }),
        _ => return None,
    };
    Some(AgentAction::CallTool {
        tool: "config_basic".to_string(),
        args,
    })
}

pub(super) fn preferred_config_edit_for_contract_hint(
    route: &RouteResult,
    action_name: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<AgentAction> {
    let capability_action =
        crate::machine_capability_ref::route_capability_action_for_namespaces(route, &["config"])
            .and_then(config_edit_action_from_capability_action);
    let action = capability_action.or(action_name).unwrap_or("guard_config");
    let path = config_path_for_contract_hint(route, auto_locator_path);
    let args = match action {
        "guard_config" => serde_json::json!({
            "action": "guard_config",
            "path": path,
        }),
        "validate_config" => serde_json::json!({
            "action": "validate_config",
            "path": path,
        }),
        _ => return None,
    };
    Some(AgentAction::CallTool {
        tool: "config_edit".to_string(),
        args,
    })
}

fn config_basic_action_from_capability_action(action: &str) -> Option<&'static str> {
    match action {
        "guard_rustclaw_config" => Some("guard_rustclaw_config"),
        "list_keys" => Some("list_keys"),
        "read_field" => Some("read_field"),
        "read_fields" => Some("read_fields"),
        "validate" => Some("validate"),
        _ => None,
    }
}

fn config_edit_action_from_capability_action(action: &str) -> Option<&'static str> {
    match action {
        "guard_after_change" | "guard_config" => Some("guard_config"),
        "validate_after_change" | "validate_config" => Some("validate_config"),
        _ => None,
    }
}

pub(super) fn preferred_archive_basic_for_contract_hint(
    state: &AppState,
    route: &RouteResult,
    action_name: Option<&str>,
    auto_locator_path: Option<&str>,
    original_user_text: &str,
) -> Option<AgentAction> {
    if !archive_basic_enabled_for_planning(state) {
        return None;
    }
    let capability_action =
        crate::machine_capability_ref::route_capability_action_for_namespaces(route, &["archive"])
            .filter(|action| matches!(*action, "list" | "read" | "pack" | "unpack"));
    let action = capability_action.or(action_name).unwrap_or("list");
    let args = match action {
        "list" => {
            let archive = archive_list_auto_locator_target_path(Some(route), auto_locator_path)
                .or_else(|| {
                    let hint = route.output_contract.locator_hint.trim();
                    is_supported_archive_path(hint).then(|| hint.to_string())
                })?;
            serde_json::json!({
                "action": "list",
                "archive": archive,
            })
        }
        "read" => {
            let (archive, member) =
                archive_read_locator_parts(Some(route), auto_locator_path, original_user_text)?;
            serde_json::json!({
                "action": "read",
                "archive": archive,
                "member": member,
            })
        }
        "pack" => {
            let (source, archive) = archive_pack_pair_for_route(route)?;
            serde_json::json!({
                "action": "pack",
                "source": source,
                "archive": archive,
            })
        }
        "unpack" => {
            let (archive, dest) = archive_unpack_pair_for_route(route)?;
            serde_json::json!({
                "action": "unpack",
                "archive": archive,
                "dest": dest,
            })
        }
        _ => return None,
    };
    Some(AgentAction::CallSkill {
        skill: "archive_basic".to_string(),
        args,
    })
}
