use super::*;

pub(super) fn action_supports_direct_observed_finalize(
    state: &AppState,
    route_result: Option<&RouteResult>,
    action: &AgentAction,
) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let canonical = state.resolve_canonical_skill_name(skill);
            if action_supports_read_range_direct_observed_finalize(route_result, &canonical, args) {
                return true;
            }
            if action_supports_structured_direct_observed_finalize(route_result, &canonical, args) {
                return true;
            }
            if canonical == "process_basic" {
                let action_name = args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .map(str::to_ascii_lowercase)
                    .unwrap_or_default();
                return route_result.is_some_and(|route| {
                    route.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
                        && matches!(action_name.as_str(), "ps" | "port_list")
                        && !route_allows_model_language_terminal_respond(Some(route))
                });
            }
            if route_result.is_some_and(|route| {
                route.output_contract.requires_content_evidence
                    && route_expects_terminal_user_answer(route)
            }) {
                return false;
            }
            if canonical == "run_cmd" && route_explicitly_requests_raw_command_output(route_result)
            {
                return true;
            }
            match canonical.as_str() {
                "health_check" | "service_control" => true,
                "system_basic" => args
                    .get("action")
                    .and_then(|value| value.as_str())
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "info" | "diagnose_runtime"
                        )
                    })
                    .unwrap_or(false),
                _ if !state.is_builtin_skill(&canonical) => true,
                _ => false,
            }
        }
        AgentAction::SynthesizeAnswer { .. } => false,
        AgentAction::CallCapability { .. } => false,
        AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
    }
}

pub(super) fn action_supports_read_range_direct_observed_finalize(
    route_result: Option<&RouteResult>,
    canonical: &str,
    args: &Value,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(|action| action.trim().to_ascii_lowercase());
    let action = action.as_deref();
    let is_read_range = (canonical == "fs_basic" && action == Some("read_text_range"))
        || (canonical == "system_basic" && action == Some("read_range"));
    if !is_read_range
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Scalar
        )
    {
        return false;
    }
    route.ask_mode.is_plain_act()
}

pub(super) fn action_supports_structured_direct_observed_finalize(
    route_result: Option<&RouteResult>,
    canonical: &str,
    args: &Value,
) -> bool {
    if route_result.is_some_and(|route| {
        !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::StructuredKeys
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::QuantityComparison
                | crate::OutputSemanticKind::ContentPresenceCheck
                | crate::OutputSemanticKind::ConfigValidation
                | crate::OutputSemanticKind::ConfigRiskAssessment
        )
    }) {
        return false;
    }
    let action = args
        .get("action")
        .and_then(|value| value.as_str())
        .map(|action| action.trim().to_ascii_lowercase());
    let action = action.as_deref();
    let response_shape = route_result.map(|route| route.output_contract.response_shape);
    let one_sentence = matches!(
        response_shape,
        Some(crate::OutputResponseShape::OneSentence)
    );
    match canonical {
        "config_basic" => match action {
            Some("read_field" | "read_fields") => true,
            Some("list_keys") => {
                !one_sentence
                    && route_result.is_none_or(|route| {
                        !matches!(
                            route.output_contract.semantic_kind,
                            crate::OutputSemanticKind::FileNames
                        ) || args
                            .get("path")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|path| !path.is_empty())
                            .or_else(|| {
                                let hint = route.output_contract.locator_hint.trim();
                                (!hint.is_empty()).then_some(hint)
                            })
                            .is_some_and(path_has_structured_document_extension)
                    })
            }
            Some("validate") => !one_sentence,
            _ => false,
        },
        "config_edit" => match action {
            Some("guard_config") => route_result.is_some_and(|route| {
                route.output_contract.semantic_kind
                    == crate::OutputSemanticKind::ConfigRiskAssessment
            }),
            _ => false,
        },
        "system_basic" => match action {
            Some("extract_field" | "extract_fields") => true,
            Some("structured_keys") => {
                !one_sentence
                    && route_result.is_none_or(|route| {
                        !matches!(
                            route.output_contract.semantic_kind,
                            crate::OutputSemanticKind::FileNames
                        ) || args
                            .get("path")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|path| !path.is_empty())
                            .or_else(|| {
                                let hint = route.output_contract.locator_hint.trim();
                                (!hint.is_empty()).then_some(hint)
                            })
                            .is_some_and(path_has_structured_document_extension)
                    })
            }
            Some("tree_summary") => route_result.is_some_and(|route| {
                matches!(
                    route.output_contract.semantic_kind,
                    crate::OutputSemanticKind::None
                )
            }),
            Some("dir_compare") => route_result.is_some_and(|route| {
                matches!(
                    route.output_contract.semantic_kind,
                    crate::OutputSemanticKind::None | crate::OutputSemanticKind::QuantityComparison
                )
            }),
            _ => false,
        },
        "fs_basic" => match action {
            Some("grep_text") => route_result.is_some_and(|route| {
                route.output_contract.semantic_kind
                    == crate::OutputSemanticKind::ContentPresenceCheck
            }),
            Some("find_entries") => route_result.is_some_and(|route| {
                matches!(
                    route.output_contract.semantic_kind,
                    crate::OutputSemanticKind::FileNames
                        | crate::OutputSemanticKind::DirectoryNames
                        | crate::OutputSemanticKind::FilePaths
                )
            }),
            Some("list_dir") => route_result.is_some_and(|route| {
                matches!(
                    route.output_contract.semantic_kind,
                    crate::OutputSemanticKind::FileNames
                        | crate::OutputSemanticKind::DirectoryNames
                        | crate::OutputSemanticKind::DirectoryEntryGroups
                )
            }),
            _ => false,
        },
        _ => false,
    }
}

pub(super) fn observation_only_plan_can_finalize_from_direct_output(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> bool {
    if route_result.is_some_and(|route| {
        route.output_contract.requires_content_evidence
            && route_expects_terminal_user_answer(route)
            && structured_scalar_observation_units(actions) > 1
            && !last_executable_action(actions).is_some_and(action_is_structured_field_bundle_read)
    }) {
        return false;
    }
    last_executable_action(actions)
        .is_some_and(|action| action_supports_direct_observed_finalize(state, route_result, action))
}

pub(super) fn action_is_structured_field_bundle_read(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        _ => return false,
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(|action| action.trim().to_ascii_lowercase());
    let action = action.as_deref();
    let fields = args.get("field_paths").or_else(|| args.get("fields"));
    let field_count = string_list_from_value(fields).len();
    ((skill.eq_ignore_ascii_case("config_basic") && action == Some("read_fields"))
        || (skill.eq_ignore_ascii_case("system_basic") && action == Some("extract_fields")))
        && field_count > 0
}

pub(super) fn route_uses_runtime_owned_observed_finalizer(route_result: &RouteResult) -> bool {
    if route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return true;
    }
    matches!(
        route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
            | crate::OutputSemanticKind::HiddenEntriesCheck
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::ContentPresenceCheck
            | crate::OutputSemanticKind::ServiceStatus
            | crate::OutputSemanticKind::ScalarPathOnly
            | crate::OutputSemanticKind::ExistenceWithPath
            | crate::OutputSemanticKind::RecentScalarEqualityCheck
            | crate::OutputSemanticKind::GitCommitSubject
            | crate::OutputSemanticKind::GitRepositoryState
            | crate::OutputSemanticKind::StructuredKeys
            | crate::OutputSemanticKind::ArchiveList
            | crate::OutputSemanticKind::ArchiveRead
            | crate::OutputSemanticKind::ArchivePack
            | crate::OutputSemanticKind::ArchiveUnpack
            | crate::OutputSemanticKind::DockerPs
            | crate::OutputSemanticKind::DockerImages
            | crate::OutputSemanticKind::DockerLogs
            | crate::OutputSemanticKind::DockerContainerLifecycle
    )
}

pub(super) fn observation_action_evidence_refs(actions: &[AgentAction]) -> Vec<String> {
    let refs = actions
        .iter()
        .enumerate()
        .filter_map(|(idx, action)| {
            matches!(
                action,
                AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
            )
            .then(|| format!("step_{}", idx + 1))
        })
        .collect::<Vec<_>>();
    match refs.as_slice() {
        [] => Vec::new(),
        [_] => vec!["last_output".to_string()],
        _ => refs,
    }
}

pub(super) fn append_synthesize_for_observation_only_terminal_answer(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route_result) = route_result else {
        return actions;
    };
    if route_uses_runtime_owned_observed_finalizer(route_result)
        || !route_expects_terminal_user_answer(route_result)
        || has_authoritative_delivery(loop_state)
        || has_discussion_followup_action(&actions)
        || workspace_synthesis_needs_more_text_evidence(Some(route_result), loop_state, &actions)
        || recent_artifacts_judgment_needs_selected_content_reads(
            route_result,
            loop_state,
            &actions,
        )
        || observation_only_plan_can_finalize_from_direct_output(
            state,
            Some(route_result),
            &actions,
        )
    {
        return actions;
    }
    let evidence_refs = observation_action_evidence_refs(&actions);
    if evidence_refs.is_empty() {
        return actions;
    }
    let refs_log = evidence_refs.join(",");
    let mut rewritten = actions;
    rewritten.push(AgentAction::SynthesizeAnswer {
        evidence_refs: evidence_refs.clone(),
    });
    rewritten.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    info!("plan_append_synthesize_for_observation_only_terminal_answer refs={refs_log}");
    rewritten
}

pub(super) fn recent_artifacts_judgment_needs_selected_content_reads(
    route: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    route.output_contract.semantic_kind == crate::OutputSemanticKind::RecentArtifactsJudgment
        && route.output_contract.requires_content_evidence
        && !has_workspace_text_content_evidence(loop_state, actions)
        && actions.iter().any(action_provides_name_listing_evidence)
}

pub(super) fn append_respond_for_terminal_synthesize_answer(
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !matches!(actions.last(), Some(AgentAction::SynthesizeAnswer { .. })) {
        return actions;
    }
    let mut rewritten = actions;
    rewritten.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    info!("plan_append_respond_for_terminal_synthesize_answer");
    rewritten
}

pub(super) fn replace_workspace_synthesis_respond_only_plan(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if loop_state.has_tool_or_skill_output
        || !route_needs_workspace_respond_only_default_evidence(route)
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        )
        || is_plain_respond_only_plan(&actions).is_none()
    {
        return actions;
    }

    info!("plan_replace_workspace_synthesis_respond_only_with_default_evidence");
    workspace_summary_default_evidence_actions()
}

pub(super) fn workspace_summary_default_evidence_actions() -> Vec<AgentAction> {
    vec![
        AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "workspace_glance",
                "path": ".",
                "max_entries": 30,
            }),
        },
        AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: serde_json::json!({
                "action": "read_fields",
                "path": "Cargo.toml",
                "format": "toml",
                "field_paths": [
                    "workspace.package.version",
                    "package.version",
                    "workspace.package.name",
                    "package.name",
                    "workspace.package.description",
                    "package.description",
                ],
            }),
        },
        AgentAction::CallSkill {
            skill: "git_basic".to_string(),
            args: serde_json::json!({
                "action": "log",
                "n": 8,
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": "README.md",
                "mode": "head",
                "n": 40,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string(),
                "step_4".to_string(),
            ],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ]
}

pub(super) fn should_prefer_observed_finalize(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> bool {
    let Some(route_result) = route_result else {
        return false;
    };
    !route_result.needs_clarify
        && route_result.output_contract.requires_content_evidence
        && route_expects_terminal_user_answer(route_result)
        && !has_authoritative_delivery(loop_state)
}

pub(super) fn strip_terminal_discussion_for_observed_finalize(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if should_prefer_observed_finalize(route_result, loop_state)
        && has_executable_observation_or_action(&actions)
        && has_discussion_followup_action(&actions)
        && route_result.is_some_and(|route| {
            route.output_contract.semantic_kind
                == crate::OutputSemanticKind::RecentScalarEqualityCheck
        })
    {
        let mut stripped = actions.clone();
        while stripped.last().is_some_and(is_discussion_followup_action) {
            stripped.pop();
        }
        if structured_scalar_observation_units(&stripped) >= 2 {
            return stripped;
        }
    }
    if should_prefer_observed_finalize(route_result, loop_state)
        && has_executable_observation_or_action(&actions)
        && has_discussion_followup_action(&actions)
    {
        return actions;
    }
    if !should_prefer_observed_finalize(route_result, loop_state)
        || loop_state.has_tool_or_skill_output
        || !has_executable_observation_or_action(&actions)
        || !has_discussion_followup_action(&actions)
    {
        return actions;
    }
    let mut stripped = actions.clone();
    while stripped.last().is_some_and(is_discussion_followup_action) {
        if stripped
            .last()
            .is_some_and(should_preserve_terminal_followup_for_observed_finalize)
        {
            break;
        }
        stripped.pop();
    }
    let trailing_preserved_synthesize = stripped
        .last()
        .is_some_and(should_preserve_terminal_followup_for_observed_finalize);
    let prefix_without_terminal = if trailing_preserved_synthesize {
        &stripped[..stripped.len().saturating_sub(1)]
    } else {
        &stripped[..]
    };
    if has_executable_observation_or_action(&stripped)
        && (!has_discussion_followup_action(&stripped)
            || (trailing_preserved_synthesize
                && !has_discussion_followup_action(prefix_without_terminal)))
    {
        stripped
    } else {
        actions
    }
}

pub(super) fn strip_terminal_discussion_for_scalar_path_observation(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || loop_state.has_tool_or_skill_output
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarPathOnly
        || !has_tool_or_skill_observation(&actions)
        || !has_discussion_followup_action(&actions)
    {
        return actions;
    }

    let mut stripped = actions.clone();
    while stripped.last().is_some_and(is_discussion_followup_action) {
        stripped.pop();
    }
    if has_tool_or_skill_observation(&stripped) && !has_discussion_followup_action(&stripped) {
        stripped
    } else {
        actions
    }
}

pub(super) fn strip_terminal_discussion_for_direct_skill_passthrough(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route_result) = route_result else {
        return actions;
    };
    if route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !has_executable_observation_or_action(&actions)
        || !has_discussion_followup_action(&actions)
    {
        return actions;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::FilePaths {
        return actions;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && route_result.output_contract.requires_content_evidence
        && actions
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        return actions;
    }
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
        && route_allows_model_language_terminal_respond(Some(route_result))
        && actions
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
        && actions
            .iter()
            .any(is_process_basic_status_observation_action)
    {
        return actions;
    }
    if has_mixed_last_output_terminal_respond(&actions) {
        return actions;
    }
    let mut stripped = actions.clone();
    while stripped.last().is_some_and(is_discussion_followup_action) {
        stripped.pop();
    }
    if !has_executable_observation_or_action(&stripped) || has_discussion_followup_action(&stripped)
    {
        return actions;
    }
    if observation_only_plan_can_finalize_from_direct_output(state, Some(route_result), &stripped) {
        stripped
    } else {
        actions
    }
}

pub(super) fn is_process_basic_status_observation_action(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        _ => return false,
    };
    if !skill.eq_ignore_ascii_case("process_basic") {
        return false;
    }
    args.get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .is_some_and(|action_name| matches!(action_name.as_str(), "ps" | "port_list"))
}

pub(super) fn delivery_success_terminal_reply(state: &AppState, actions: &[AgentAction]) -> bool {
    let Some(content) = is_plain_respond_only_plan(actions) else {
        return false;
    };
    let Some((_kind, raw_path)) = crate::finalize::parse_delivery_file_token(content) else {
        return false;
    };
    let path = raw_path.trim();
    if path.is_empty() || path.contains('\n') {
        return false;
    }
    let candidate = Path::new(path);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(candidate)
    };
    resolved.is_file()
}

pub(super) fn observation_only_plan_missing_user_answer(
    state: &AppState,
    route_result: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if should_prefer_observed_finalize(Some(route_result), loop_state)
        || (route_uses_runtime_owned_observed_finalizer(route_result)
            && has_executable_observation_or_action(actions))
        || observation_only_plan_can_finalize_from_direct_output(state, Some(route_result), actions)
    {
        return false;
    }
    has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && route_expects_terminal_user_answer(route_result)
        && !has_authoritative_delivery(loop_state)
}

pub(super) fn run_cmd_likely_mutates(command: &str) -> bool {
    let lower = format!(" {} ", command.to_ascii_lowercase());
    // Plain stdout commands are observations; redirection/tee below covers output writes.
    command.contains('>')
        || lower.contains(" tee ")
        || lower.contains(" sed -i")
        || lower.contains(" perl -0pi")
        || lower.contains(" perl -pi")
        || lower.contains(" cat <<")
        || lower.contains(" cp ")
        || lower.contains(" mv ")
        || lower.contains(" systemctl start")
        || lower.contains(" systemctl stop")
        || lower.contains(" systemctl restart")
        || lower.contains(" systemctl reload")
        || lower.contains(" systemctl enable")
        || lower.contains(" systemctl disable")
}

pub(super) fn action_is_likely_mutating(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            match skill.trim().to_ascii_lowercase().as_str() {
                "write_file" | "remove_file" | "make_dir" => true,
                "fs_basic" => args
                    .get("action")
                    .and_then(|value| value.as_str())
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "write_text" | "append_text" | "make_dir" | "remove_path"
                        )
                    })
                    .unwrap_or(false),
                "service_control" => args
                    .get("action")
                    .and_then(|value| value.as_str())
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "start" | "stop" | "restart" | "reload" | "enable" | "disable"
                        )
                    })
                    .unwrap_or(false),
                "run_cmd" => args
                    .get("command")
                    .and_then(|value| value.as_str())
                    .map(run_cmd_likely_mutates)
                    .unwrap_or(false),
                _ => false,
            }
        }
        AgentAction::SynthesizeAnswer { .. } => false,
        AgentAction::CallCapability { .. } => false,
        AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
    }
}

pub(super) fn is_non_mutating_run_cmd_action(state: &AppState, action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill, args)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => {
            return false;
        }
    };
    if state.resolve_canonical_skill_name(skill) != "run_cmd" {
        return false;
    }
    if args
        .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
        .and_then(Value::as_bool)
        == Some(true)
    {
        return false;
    }
    let Some(command) = run_cmd_command_from_args(args) else {
        return false;
    };
    if command.is_empty() {
        return false;
    }
    !crate::execution_recipe::classify_skill_action_effect(state, "run_cmd", args).mutates
}

pub(super) fn mark_run_cmd_action_continue_on_error(action: &mut AgentAction) -> bool {
    let args = match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args,
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => {
            return false;
        }
    };
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    if obj.get(super::super::CLAWD_CONTINUE_ON_ERROR_ARG) == Some(&Value::Bool(true)) {
        return false;
    }
    obj.insert(
        super::super::CLAWD_CONTINUE_ON_ERROR_ARG.to_string(),
        Value::Bool(true),
    );
    true
}

pub(super) fn route_or_actions_need_run_cmd_step_status_evidence(
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> bool {
    route_requires_terminal_observation_synthesis(route_result)
        || actions
            .iter()
            .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
}

pub(super) fn mark_non_mutating_run_cmd_sequences_continue_on_error(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !route_or_actions_need_run_cmd_step_status_evidence(route_result, &actions) {
        return actions;
    }
    let mut actions = actions;
    let mut changed = false;
    let mut idx = 0usize;
    while idx < actions.len() {
        if !is_non_mutating_run_cmd_action(state, &actions[idx]) {
            idx += 1;
            continue;
        }
        let start = idx;
        while idx < actions.len() && is_non_mutating_run_cmd_action(state, &actions[idx]) {
            idx += 1;
        }
        if idx.saturating_sub(start) < 2 {
            continue;
        }
        for action in &mut actions[start..idx] {
            changed |= mark_run_cmd_action_continue_on_error(action);
        }
    }
    if changed {
        info!("plan_mark_run_cmd_sequence_continue_on_error");
    }
    actions
}

pub(super) fn action_satisfies_recipe_profile_validation(
    state: &AppState,
    loop_state: &LoopState,
    action: &AgentAction,
) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            crate::execution_recipe::validation_satisfies_recipe_profile(
                loop_state.execution_recipe,
                state,
                skill,
                args,
            )
        }
        AgentAction::SynthesizeAnswer { .. } => false,
        AgentAction::CallCapability { .. } => false,
        AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
    }
}

pub(super) fn actions_missing_recipe_profile_validation(
    state: &AppState,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if !loop_state.execution_recipe.validation_required
        || !matches!(
            loop_state.execution_recipe.profile,
            crate::execution_recipe::ExecutionRecipeProfile::ConfigChange
                | crate::execution_recipe::ExecutionRecipeProfile::CodeChange
                | crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring
        )
    {
        return false;
    }
    let mut saw_mutation = loop_state.execution_recipe.saw_mutation;
    let mut saw_profile_validation = loop_state.execution_recipe.saw_validation
        && !matches!(
            loop_state.execution_recipe.profile,
            crate::execution_recipe::ExecutionRecipeProfile::ConfigChange
                | crate::execution_recipe::ExecutionRecipeProfile::CodeChange
                | crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring
        );
    for action in actions {
        if action_is_likely_mutating(action) {
            saw_mutation = true;
            saw_profile_validation = false;
            continue;
        }
        if saw_mutation && action_satisfies_recipe_profile_validation(state, loop_state, action) {
            saw_profile_validation = true;
        }
    }
    saw_mutation && !saw_profile_validation
}

pub(super) fn actions_violate_recipe_target_scope(
    state: &AppState,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    match loop_state.execution_recipe.target_scope {
        crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo => {
            actions.iter().any(|action| match action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args } => {
                    crate::execution_recipe::action_conflicts_with_recipe_target_scope(
                        loop_state.execution_recipe,
                        state,
                        skill,
                        args,
                    )
                }
                AgentAction::SynthesizeAnswer { .. } => false,
                AgentAction::CallCapability { .. } => false,
                AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
            })
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace => {
            let mut saw_external_target = loop_state.execution_recipe.saw_external_target;
            let mut saw_scope_conflict = false;
            for action in actions {
                match action {
                    AgentAction::CallSkill { skill, args }
                    | AgentAction::CallTool { tool: skill, args } => {
                        if crate::execution_recipe::action_targets_external_workspace(
                            state, skill, args,
                        ) {
                            saw_external_target = true;
                        }
                        if crate::execution_recipe::action_conflicts_with_recipe_target_scope(
                            loop_state.execution_recipe,
                            state,
                            skill,
                            args,
                        ) {
                            saw_scope_conflict = true;
                        }
                    }
                    AgentAction::SynthesizeAnswer { .. } => {}
                    AgentAction::CallCapability { .. } => {}
                    AgentAction::Respond { .. } | AgentAction::Think { .. } => {}
                }
            }
            saw_scope_conflict || !saw_external_target
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield => {
            !loop_state.execution_recipe.saw_greenfield_creation
                && !actions.iter().any(|action| match action {
                    AgentAction::CallSkill { skill, args }
                    | AgentAction::CallTool { tool: skill, args } => {
                        crate::execution_recipe::action_satisfies_greenfield_creation(
                            state, skill, args,
                        )
                    }
                    AgentAction::SynthesizeAnswer { .. } => false,
                    AgentAction::CallCapability { .. } => false,
                    AgentAction::Respond { .. } | AgentAction::Think { .. } => false,
                })
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::Unknown
        | crate::execution_recipe::ExecutionRecipeTargetScope::System => false,
    }
}

pub(super) fn should_force_actionable_plan_repair(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    let Some(route_result) = route_result else {
        return false;
    };
    if route_result.needs_clarify {
        return false;
    }
    if route_result.output_contract.delivery_required
        && !loop_state.has_tool_or_skill_output
        && is_delivery_failure_terminal_reply(actions)
    {
        return false;
    }
    if route_result.output_contract.delivery_required
        && !loop_state.has_tool_or_skill_output
        && delivery_success_terminal_reply(state, actions)
    {
        return false;
    }
    if loop_state.execution_recipe.is_active()
        && matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Apply
        )
        && !actions.iter().any(action_is_likely_mutating)
    {
        return true;
    }
    if actions_missing_recipe_profile_validation(state, loop_state, actions) {
        return true;
    }
    if actions_violate_recipe_target_scope(state, loop_state, actions) {
        return true;
    }
    if contains_unavailable_skill_action(state, actions) {
        return true;
    }
    if session_alias_targets_missing_from_plan(state, loop_state, actions) {
        return true;
    }
    if !loop_state.execution_recipe.is_active()
        && terminal_reply_mentions_observed_missing_target(loop_state, actions)
    {
        return false;
    }
    if structured_scalar_compare_missing_required_extracts_for_round(
        route_result,
        loop_state,
        actions,
    ) {
        return true;
    }
    if actions_use_ad_hoc_command_without_route_preferred_skill(state, route_result, actions) {
        return true;
    }
    if no_content_evidence_execute_route_read_only_file_plan_requires_repair(
        state,
        Some(route_result),
        loop_state,
        actions,
    ) {
        return true;
    }
    if plain_act_filesystem_text_read_only_plan_requires_repair(
        state,
        Some(route_result),
        loop_state,
        actions,
    ) {
        return true;
    }
    if content_evidence_plan_only_has_locator_observation(route_result, loop_state, actions) {
        return true;
    }
    if scalar_count_plan_uses_listing_instead_of_structured_count(
        state,
        route_result,
        loop_state,
        actions,
    ) {
        return true;
    }
    let lightweight_route_has_executable_plan =
        route_qualifies_for_lightweight_repair_skip(Some(route_result))
            && !loop_state.has_tool_or_skill_output
            && has_executable_observation_or_action(actions);
    if lightweight_route_has_executable_plan
        && !observation_only_plan_missing_user_answer(state, route_result, loop_state, actions)
    {
        return false;
    }
    if observation_only_plan_missing_user_answer(state, route_result, loop_state, actions) {
        return true;
    }
    if has_executable_observation_or_action(actions) {
        return false;
    }
    if has_discussion_followup_action(actions) && loop_state.has_tool_or_skill_output {
        return false;
    }
    let requires_action_before_reply =
        !loop_state.has_tool_or_skill_output && route_result.is_execute_gate();
    route_result.output_contract.requires_content_evidence || requires_action_before_reply
}

pub(super) fn required_session_alias_targets_from_loop_state(
    loop_state: &LoopState,
) -> Vec<String> {
    let Some(raw) = loop_state
        .output_vars
        .get("required_session_alias_targets")
        .map(String::as_str)
    else {
        return Vec::new();
    };
    let Ok(values) = serde_json::from_str::<Vec<String>>(raw) else {
        return Vec::new();
    };
    let mut targets = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    targets
}

pub(super) fn required_session_alias_targets_for_plan_context(
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    plan_context: Option<&str>,
) -> Vec<String> {
    let loop_targets = required_session_alias_targets_from_loop_state(loop_state);
    if loop_targets.len() >= 2 {
        return loop_targets;
    }
    let mut bindings = Vec::new();
    if let Some(plan_context) = plan_context {
        bindings.extend(super::super::session_alias_bindings_from_context_summary(
            plan_context,
        ));
    }
    for source in [original_user_text, Some(user_text)].into_iter().flatten() {
        bindings.extend(super::super::session_alias_bindings_from_context_summary(
            source,
        ));
    }
    let mut seen_bindings = std::collections::BTreeSet::new();
    bindings.retain(|binding| {
        let alias = binding.alias.trim();
        let target = binding.target.trim();
        !alias.is_empty()
            && !target.is_empty()
            && seen_bindings.insert((alias.to_string(), target.to_string()))
    });
    if bindings.is_empty() {
        return loop_targets;
    }
    let request_surfaces = [original_user_text, Some(user_text)];
    let mut targets = request_surfaces
        .into_iter()
        .flatten()
        .flat_map(|surface| {
            let surface = surface
                .split("### SESSION_ALIAS_BINDINGS")
                .next()
                .unwrap_or(surface);
            crate::conversation_state::alias_bindings_mentioned_in_prompt(&bindings, surface)
                .into_iter()
                .filter_map(|binding| {
                    let target = binding.target.trim();
                    (!target.is_empty()).then_some(target.to_string())
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    if targets.len() >= 2 {
        targets
    } else {
        loop_targets
    }
}

pub(super) fn session_alias_target_coverage_tokens(state: &AppState, target: &str) -> Vec<String> {
    let target = target.trim();
    if target.is_empty() {
        return Vec::new();
    }
    let mut tokens = vec![target.to_string()];
    if let Ok(relative) = Path::new(target).strip_prefix(&state.skill_rt.workspace_root) {
        let relative = relative.to_string_lossy().trim().to_string();
        if !relative.is_empty() {
            tokens.push(relative);
        }
    }
    tokens.sort();
    tokens.dedup();
    tokens
}

pub(super) fn value_contains_session_alias_target_token(value: &Value, tokens: &[String]) -> bool {
    match value {
        Value::String(text) => tokens
            .iter()
            .any(|token| !token.is_empty() && text.contains(token)),
        Value::Array(items) => items
            .iter()
            .any(|item| value_contains_session_alias_target_token(item, tokens)),
        Value::Object(map) => map
            .values()
            .any(|item| value_contains_session_alias_target_token(item, tokens)),
        _ => false,
    }
}

pub(super) fn action_covers_session_alias_target(
    state: &AppState,
    action: &AgentAction,
    target: &str,
) -> bool {
    let tokens = session_alias_target_coverage_tokens(state, target);
    if tokens.is_empty() {
        return false;
    }
    match action {
        AgentAction::CallSkill { args, .. }
        | AgentAction::CallTool { args, .. }
        | AgentAction::CallCapability { args, .. } => {
            value_contains_session_alias_target_token(args, &tokens)
        }
        AgentAction::Think { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. } => false,
    }
}
