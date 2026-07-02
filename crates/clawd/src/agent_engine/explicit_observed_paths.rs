use super::*;

pub(super) fn action_observed_paths_for_explicit_file_targets(action: &AgentAction) -> Vec<String> {
    let Some(skill) = planned_action_skill_name(action).map(str::trim) else {
        return Vec::new();
    };
    let Some(args) = action_args(action).and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if skill.eq_ignore_ascii_case("read_file") || skill.eq_ignore_ascii_case("doc_parse") {
        if let Some(path) = args.get("path").and_then(Value::as_str) {
            paths.push(path.to_string());
        }
        return paths;
    }
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();

    if skill.eq_ignore_ascii_case("fs_basic") {
        match action_name.as_str() {
            "stat_paths" | "find_entries" | "compare_paths" => {
                paths.extend(string_list_from_value(args.get("paths")));
                paths.extend(string_list_from_value(args.get("targets")));
                paths.extend(string_list_from_value(args.get("path")));
                if let Some(path) = args.get("left_path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
                if let Some(path) = args.get("right_path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
            }
            "read_text_range" => {
                if let Some(path) = args.get("path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
            }
            _ => {}
        }
        return paths;
    }

    if skill.eq_ignore_ascii_case("config_basic") {
        match action_name.as_str() {
            "read_field" | "read_fields" | "list_keys" | "validate" => {
                if let Some(path) = args.get("path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
            }
            _ => {}
        }
        return paths;
    }

    if skill.eq_ignore_ascii_case("system_basic") {
        match action_name.as_str() {
            "path_batch_facts" => {
                paths.extend(string_list_from_value(args.get("paths")));
                paths.extend(string_list_from_value(args.get("targets")));
                paths.extend(string_list_from_value(args.get("path")));
            }
            "compare_paths" => {
                paths.extend(string_list_from_value(args.get("paths")));
                paths.extend(string_list_from_value(args.get("targets")));
                if let Some(path) = args.get("left_path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
                if let Some(path) = args.get("right_path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
            }
            "read_range" | "extract_field" | "extract_fields" => {
                if let Some(path) = args.get("path").and_then(Value::as_str) {
                    paths.push(path.to_string());
                }
            }
            _ => {}
        }
    }
    paths
}

pub(super) fn explicit_file_targets_covered_by_plan(
    actions: &[AgentAction],
    targets: &[String],
) -> Vec<bool> {
    let mut covered = vec![false; targets.len()];
    for action in actions {
        for path in action_observed_paths_for_explicit_file_targets(action) {
            for (idx, target) in targets.iter().enumerate() {
                if !covered[idx] && plan_path_matches_explicit_file_target(&path, target) {
                    covered[idx] = true;
                }
            }
        }
    }
    covered
}

pub(super) fn explicit_multi_file_metadata_plan_covers_targets(
    actions: &[AgentAction],
    targets: &[String],
) -> bool {
    actions.iter().any(|action| {
        let Some(skill) = planned_action_skill_name(action).map(str::trim) else {
            return false;
        };
        let Some(args) = action_args(action).and_then(Value::as_object) else {
            return false;
        };
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        let covers_metadata = matches!(
            (
                skill.to_ascii_lowercase().as_str(),
                action_name.to_ascii_lowercase().as_str()
            ),
            ("fs_basic", "stat_paths")
                | ("fs_basic", "compare_paths")
                | ("system_basic", "path_batch_facts")
                | ("system_basic", "compare_paths")
        );
        if !covers_metadata {
            return false;
        }
        explicit_file_targets_covered_by_plan(std::slice::from_ref(action), targets)
            .into_iter()
            .all(|covered| covered)
    })
}

pub(super) fn ensure_explicit_multi_file_targets_have_path_facts(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || loop_state.has_tool_or_skill_output
    {
        return actions;
    }
    let targets = structured_or_text_multi_file_targets(route, user_text)
        .into_iter()
        .take(4)
        .collect::<Vec<_>>();
    if targets.len() < 2 || explicit_multi_file_metadata_plan_covers_targets(&actions, &targets) {
        return actions;
    }

    if !route_requests_path_metadata_compare(route) {
        return actions;
    }

    if structured_scalar_plus_text_evidence(&actions) {
        return actions;
    }

    if structured_scalar_observation_units(&actions) >= 2 {
        return actions;
    }

    info!(
        "plan_replace_scalar_multi_file_read_with_fs_basic_stat_paths targets={}",
        targets.join(",")
    );
    vec![fs_basic_stat_paths_action_for_explicit_targets(&targets)]
}

pub(super) fn ensure_existence_multi_file_targets_have_path_facts(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || !route.is_execute_gate()
        || route.output_contract.delivery_required
        || loop_state.has_tool_or_skill_output
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ExistenceWithPath
    {
        return actions;
    }
    let targets = structured_or_text_multi_file_targets(route, user_text)
        .into_iter()
        .take(8)
        .collect::<Vec<_>>();
    if targets.len() < 2 || explicit_multi_file_metadata_plan_covers_targets(&actions, &targets) {
        return actions;
    }
    if !actions.iter().any(planned_action_is_path_metadata_facts) {
        return actions;
    }
    info!(
        "plan_replace_incomplete_existence_multi_file_stat_paths targets={}",
        targets.join(",")
    );
    vec![fs_basic_stat_paths_action_for_explicit_targets(&targets)]
}

pub(super) fn rewrite_unresolved_template_arg_multi_file_read_plan(
    route_result: Option<&RouteResult>,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || !route.output_contract.requires_content_evidence
        || !actions.iter().any(action_args_contain_unresolved_template)
    {
        return actions;
    }
    let file_targets = structured_or_text_multi_file_targets(route, user_text)
        .into_iter()
        .filter(|target| filename_candidate_has_document_extension(target))
        .collect::<Vec<_>>();
    if file_targets.len() < 2 {
        return actions;
    }

    let mut rewritten = Vec::new();
    for target in file_targets.iter().take(4) {
        rewritten.push(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": target,
                "mode": "head",
                "n": 40,
            }),
        });
    }
    let evidence_refs = (1..=rewritten.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    rewritten.push(AgentAction::SynthesizeAnswer {
        evidence_refs: evidence_refs.clone(),
    });
    rewritten.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    info!(
        "plan_rewrite_unresolved_template_arg_multi_file_read_plan targets={} refs={}",
        file_targets.join(","),
        evidence_refs.join(",")
    );
    rewritten
}

pub(super) fn resolve_existing_file_target_from_token(
    state: &AppState,
    token: &str,
) -> Option<String> {
    let token = token
        .trim()
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\''
                    | '`'
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | ','
                    | '，'
                    | ';'
                    | '；'
                    | '。'
                    | ':'
                    | '：'
                    | '\\'
            )
        })
        .trim();
    if token.is_empty() {
        return None;
    }
    let path = Path::new(token);
    if !path.is_absolute()
        && !token.starts_with("./")
        && !token.starts_with("../")
        && !(path.components().count() > 1 && path.extension().is_some())
        && !filename_candidate_has_document_extension(token)
    {
        return None;
    }
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    };
    if !resolved.is_file() {
        return None;
    }
    Some(resolved.display().to_string())
}

pub(super) fn collect_existing_file_targets_from_text(state: &AppState, text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for token in text.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '"' | '\''
                    | '`'
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | ','
                    | '，'
                    | ';'
                    | '；'
                    | '。'
                    | ':'
                    | '：'
                    | '\\'
            )
    }) {
        let Some(path) = resolve_existing_file_target_from_token(state, token) else {
            continue;
        };
        if !targets.iter().any(|existing: &String| existing == &path) {
            targets.push(path);
        }
    }
    targets
}

pub(super) fn collect_file_targets_from_route_scope(
    state: &AppState,
    route: &RouteResult,
    user_text: &str,
) -> Vec<String> {
    let mut targets = Vec::new();
    for target in structured_or_text_multi_file_targets(route, user_text) {
        if let Some(path) = resolve_existing_file_target_from_token(state, &target) {
            if !targets.iter().any(|existing: &String| existing == &path) {
                targets.push(path);
            }
        }
    }
    for path in collect_existing_file_targets_from_text(state, user_text) {
        if !targets.iter().any(|existing: &String| existing == &path) {
            targets.push(path);
        }
    }
    for source in [
        route.resolved_intent.as_str(),
        route.route_reason.as_str(),
        route.output_contract.locator_hint.as_str(),
    ] {
        for path in collect_existing_file_targets_from_text(state, source) {
            if !targets.iter().any(|existing: &String| existing == &path) {
                targets.push(path);
            }
        }
    }
    targets
}

pub(super) fn scoped_plan_context_file_targets(
    state: &AppState,
    plan_context: Option<&str>,
) -> Vec<String> {
    let Some(plan_context) = plan_context else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    if let Some((_, tail)) = plan_context.split_once("### RECENT_EXECUTION_EVENTS") {
        let mut event_request_targets = Vec::new();
        for line in tail.lines() {
            let Some((_, request_tail)) = line.split_once(" request=") else {
                continue;
            };
            let request = request_tail
                .split(" result=")
                .next()
                .unwrap_or(request_tail)
                .trim();
            for path in collect_existing_file_targets_from_text(state, request) {
                if !event_request_targets
                    .iter()
                    .any(|existing: &String| existing == &path)
                {
                    event_request_targets.push(path);
                }
            }
        }
        event_request_targets.reverse();
        for path in event_request_targets {
            if !targets.iter().any(|existing: &String| existing == &path) {
                targets.push(path);
            }
        }
    }
    for section in direct_answer_gate_context_resolved_intents(plan_context) {
        for path in collect_existing_file_targets_from_text(state, &section) {
            if !targets.iter().any(|existing: &String| existing == &path) {
                targets.push(path);
            }
        }
    }
    for marker in ["Resolved semantic request:", "Turn analysis:"] {
        let Some((_, tail)) = plan_context.split_once(marker) else {
            continue;
        };
        let section = tail.split("\n\n").next().unwrap_or(tail).trim();
        for path in collect_existing_file_targets_from_text(state, section) {
            if !targets.iter().any(|existing: &String| existing == &path) {
                targets.push(path);
            }
        }
    }
    targets
}

fn direct_answer_gate_context_resolved_intents(plan_context: &str) -> Vec<String> {
    plan_context
        .split("\n\n")
        .filter_map(|section| {
            let value: Value = serde_json::from_str(section.trim()).ok()?;
            value
                .pointer("/direct_answer_gate/resolved_intent")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|resolved_intent| !resolved_intent.is_empty())
                .map(ToOwned::to_owned)
        })
        .collect()
}

pub(super) fn authoritative_current_file_target_path(
    state: &AppState,
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    if let Some(raw_auto_path) = auto_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let auto_path = resolve_workspace_path(&state.skill_rt.workspace_root, raw_auto_path);
        if auto_path.is_file() {
            return Some(auto_path.display().to_string());
        }
    }

    if matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
    ) {
        let hint = route.output_contract.locator_hint.trim();
        if !hint.is_empty() {
            let hint_path = resolve_workspace_path(&state.skill_rt.workspace_root, hint);
            if hint_path.is_file() {
                return Some(hint_path.display().to_string());
            }
        }
    }

    None
}

pub(super) fn seed_authoritative_current_file_target(
    state: &AppState,
    route: &RouteResult,
    auto_locator_path: Option<&str>,
    targets: &mut Vec<String>,
) {
    if !targets.is_empty() {
        return;
    }
    if let Some(path) = authoritative_current_file_target_path(state, route, auto_locator_path) {
        targets.push(path);
    }
}

pub(super) fn single_authoritative_current_file_target(
    state: &AppState,
    route: &RouteResult,
    auto_locator_path: Option<&str>,
    targets: &[String],
) -> bool {
    if targets.len() != 1 {
        return false;
    }
    let Some(current_path) =
        authoritative_current_file_target_path(state, route, auto_locator_path)
    else {
        return false;
    };
    let current_path = Path::new(&current_path);
    let target_path = resolve_workspace_path(&state.skill_rt.workspace_root, &targets[0]);
    same_existing_or_display_path(current_path, &target_path)
}

pub(super) fn replace_content_evidence_synthesize_only_with_file_reads(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || !route.is_execute_gate()
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || loop_state.has_tool_or_skill_output
        || actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
            )
        })
        || !actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
            )
        })
    {
        return actions;
    }

    let mut targets = collect_file_targets_from_route_scope(state, route, user_text);
    seed_authoritative_current_file_target(state, route, auto_locator_path, &mut targets);
    if targets.len() < 2
        && !single_authoritative_current_file_target(state, route, auto_locator_path, &targets)
    {
        for path in scoped_plan_context_file_targets(state, plan_context) {
            if !targets.iter().any(|existing| existing == &path) {
                targets.push(path);
            }
        }
    }
    if targets.len() < 2 {
        return actions;
    }
    let targets = targets.into_iter().take(4).collect::<Vec<_>>();
    let mut rewritten = Vec::new();
    for path in &targets {
        rewritten.push(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": path,
                "mode": "head",
                "n": 60,
            }),
        });
    }
    let evidence_refs = (1..=rewritten.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    rewritten.push(AgentAction::SynthesizeAnswer {
        evidence_refs: evidence_refs.clone(),
    });
    rewritten.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    info!(
        "plan_replace_synthesize_only_content_evidence_with_file_reads targets={} refs={}",
        targets.join(","),
        evidence_refs.join(",")
    );
    rewritten
}

pub(super) fn ensure_explicit_multi_file_targets_have_content_reads(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    plan_context: Option<&str>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route_has_unresolved_clarify_or_locator_marker(route)
        || !route.is_execute_gate()
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || loop_state.has_tool_or_skill_output
        || !route_expects_terminal_user_answer(route)
        || route_requests_path_metadata_compare(route)
        || route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
    {
        return actions;
    }

    let mut targets = collect_file_targets_from_route_scope(state, route, user_text);
    seed_authoritative_current_file_target(state, route, auto_locator_path, &mut targets);
    if targets.len() < 2
        && !single_authoritative_current_file_target(state, route, auto_locator_path, &targets)
    {
        for path in scoped_plan_context_file_targets(state, plan_context) {
            if !targets.iter().any(|existing| existing == &path) {
                targets.push(path);
            }
        }
    }
    let targets = targets.into_iter().take(4).collect::<Vec<_>>();
    if targets.len() < 2 {
        return actions;
    }

    let covered = explicit_file_targets_covered_by_plan(&actions, &targets);
    if covered.iter().all(|covered| *covered) {
        return actions;
    }

    let mut observations = actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                AgentAction::CallSkill { .. }
                    | AgentAction::CallTool { .. }
                    | AgentAction::CallCapability { .. }
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    if observations.is_empty() {
        return actions;
    }

    let mut appended = Vec::new();
    for (target, covered) in targets.iter().zip(covered.iter()) {
        if *covered {
            continue;
        }
        appended.push(target.clone());
        observations.push(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": target,
                "mode": "head",
                "n": 60,
            }),
        });
    }
    if appended.is_empty() {
        return actions;
    }

    let evidence_refs = (1..=observations.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    observations.push(AgentAction::SynthesizeAnswer {
        evidence_refs: evidence_refs.clone(),
    });
    observations.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    info!(
        "plan_append_missing_explicit_content_reads targets={} appended={} refs={}",
        targets.join(","),
        appended.join(","),
        evidence_refs.join(",")
    );
    observations
}

pub(super) fn content_excerpt_explicit_file_targets_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if route_has_unresolved_clarify_or_locator_marker(route)
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || !route_expects_terminal_user_answer(route)
        || route_requests_path_metadata_compare(route)
        || route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        || route.output_contract.semantic_kind == crate::OutputSemanticKind::CommandOutputSummary
        || route.output_contract.semantic_kind
            == crate::OutputSemanticKind::FilesystemMutationResult
    {
        return None;
    }

    let mut targets = collect_file_targets_from_route_scope(state, route, user_text);
    if let Some(original_user_text) = original_user_text {
        for path in collect_file_targets_from_route_scope(state, route, original_user_text) {
            if !targets.iter().any(|existing| existing == &path) {
                targets.push(path);
            }
        }
    }
    seed_authoritative_current_file_target(state, route, auto_locator_path, &mut targets);
    let targets = targets
        .into_iter()
        .filter(|path| explicit_content_read_supported_target(path))
        .take(4)
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return None;
    }

    let slice_spec = content_slice_spec_from_sources([
        route.resolved_intent.as_str(),
        route.route_reason.as_str(),
        goal,
        user_text,
        original_user_text.unwrap_or_default(),
    ]);
    if slice_spec.is_none()
        && targets.len() == 1
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
        )
        && doc_parse_is_enabled(state)
        && doc_parse_supported_path(&targets[0])
        && !repo_text_artifact_prefers_bounded_fs_read(&targets[0])
        && route_allows_single_document_parse_synthesis(route)
        && route_contract_allows_doc_parse(route)
    {
        let actions = vec![
            AgentAction::CallSkill {
                skill: "doc_parse".to_string(),
                args: serde_json::json!({
                    "action": "parse_doc",
                    "path": targets[0],
                    "max_chars": 12000,
                    "include_metadata": true
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
        info!(
            "plan_deterministic_content_excerpt_explicit_doc_parse_target target={}",
            targets[0]
        );
        return Some(build_plan_result(
            goal,
            &raw_plan_text,
            PlanKind::Single,
            &actions,
        ));
    }
    if slice_spec.is_none()
        && targets.len() == 1
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ContentExcerptSummary
        && log_analyze_is_enabled(state)
        && log_analyze_supported_path(&targets[0])
        && contract_allows_log_analyze_for_path(route, &targets[0])
    {
        return None;
    }
    let mut actions = Vec::new();
    for path in &targets {
        let mut args = serde_json::Map::new();
        args.insert(
            "action".to_string(),
            Value::String("read_text_range".to_string()),
        );
        args.insert("path".to_string(), Value::String(path.clone()));
        apply_content_slice_spec_to_read_args(&mut args, slice_spec.clone(), "head", 120);
        actions.push(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: Value::Object(args),
        });
    }
    let mut actions = rewrite_rustclaw_main_config_excerpt_read_to_guard(
        route_result,
        auto_locator_path,
        actions,
    );
    let evidence_refs = (1..=actions.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    actions.push(AgentAction::SynthesizeAnswer {
        evidence_refs: evidence_refs.clone(),
    });
    actions.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    info!(
        "plan_deterministic_content_excerpt_explicit_file_targets targets={} refs={}",
        targets.join(","),
        evidence_refs.join(",")
    );
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn explicit_content_read_supported_target(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.trim().to_ascii_lowercase().as_str(),
                "md" | "txt" | "log" | "json" | "toml" | "yaml" | "yml" | "rs" | "csv"
            )
        })
        .unwrap_or(false)
}

pub(super) fn strip_unresolved_template_reads_after_inventory_dir(
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let mut saw_locator_listing = false;
    let mut old_to_new: Vec<Option<usize>> = vec![None; actions.len() + 1];
    let mut stripped = Vec::new();
    let mut stripped_indices = Vec::new();

    for (idx, action) in actions.into_iter().enumerate() {
        let old_idx = idx + 1;
        if saw_locator_listing
            && is_unresolved_template_read_action(&action)
            && !is_indexed_last_output_read_action(&action)
        {
            stripped_indices.push(old_idx);
            continue;
        }
        if is_locator_listing_action(&action) {
            saw_locator_listing = true;
        }
        old_to_new[old_idx] = Some(stripped.len() + 1);
        stripped.push(action);
    }

    if stripped_indices.is_empty() {
        return stripped;
    }

    for action in &mut stripped {
        if let AgentAction::SynthesizeAnswer { evidence_refs } = action {
            *evidence_refs = rewrite_evidence_refs_after_step_strip(evidence_refs, &old_to_new);
        }
    }

    info!(
        "plan_strip_unresolved_template_reads_after_inventory_dir stripped_steps={}",
        stripped_indices
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    stripped
}

pub(super) fn is_locator_listing_action(action: &AgentAction) -> bool {
    is_fs_basic_listing_action(action)
        || is_system_basic_inventory_dir_action(action)
        || is_fs_search_observation_action(action)
}

pub(super) fn is_fs_basic_listing_action(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }
            if skill.eq_ignore_ascii_case("fs_basic")
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.eq_ignore_ascii_case("list_dir"))
    )
}

pub(super) fn is_system_basic_inventory_dir_action(action: &AgentAction) -> bool {
    matches!(
        system_basic_action_path(action),
        Some((action_name, _)) if action_name == "inventory_dir"
    )
}

pub(super) fn is_fs_search_observation_action(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
            if skill.eq_ignore_ascii_case("fs_search")
    )
}

pub(super) fn is_unresolved_template_read_action(action: &AgentAction) -> bool {
    let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
        action
    else {
        return false;
    };
    if !value_contains_unresolved_template(args) {
        return false;
    }
    if skill == "read_file" {
        return true;
    }
    if skill == "fs_basic" {
        return args
            .as_object()
            .and_then(|obj| obj.get("action"))
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|action_name| action_name.eq_ignore_ascii_case("read_text_range"));
    }
    if skill != "system_basic" {
        return false;
    }
    args.as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|action_name| action_name.eq_ignore_ascii_case("read_range"))
}

pub(super) fn is_indexed_last_output_read_action(action: &AgentAction) -> bool {
    let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
        action
    else {
        return false;
    };
    if skill != "system_basic" && skill != "fs_basic" {
        return false;
    }
    let Some(obj) = args.as_object() else {
        return false;
    };
    let is_read_range = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|action_name| {
            action_name.eq_ignore_ascii_case("read_range")
                || action_name.eq_ignore_ascii_case("read_text_range")
        });
    if !is_read_range {
        return false;
    }
    let Some(path) = obj.get("path").and_then(Value::as_str) else {
        return false;
    };
    static LAST_OUTPUT_INDEX_RE: OnceLock<Regex> = OnceLock::new();
    let re = LAST_OUTPUT_INDEX_RE.get_or_init(|| {
        Regex::new(r"\{\{\s*last_output(?:\.\d+|\[\s*\d+\s*\])\s*\}\}")
            .expect("last_output indexed placeholder regex")
    });
    re.is_match(path)
}

pub(super) fn action_args_contain_unresolved_template(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { args, .. }
        | AgentAction::CallTool { args, .. }
        | AgentAction::CallCapability { args, .. } => value_contains_unresolved_template(args),
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

pub(super) fn value_contains_unresolved_template(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            let text = text.trim();
            text.contains("{{") && text.contains("}}")
        }
        Value::Array(items) => items.iter().any(value_contains_unresolved_template),
        Value::Object(map) => map.values().any(value_contains_unresolved_template),
        _ => false,
    }
}

pub(super) fn explicit_document_file_targets(user_text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for candidate in explicit_document_path_targets(user_text) {
        if document_target_already_covered(&targets, &candidate) {
            continue;
        }
        targets.push(candidate);
    }
    for candidate in crate::delivery_utils::extract_filename_candidates(user_text) {
        if !filename_candidate_has_document_extension(&candidate)
            || document_target_already_covered(&targets, &candidate)
        {
            continue;
        }
        targets.push(candidate);
    }
    targets
}

pub(super) fn explicit_document_path_targets(user_text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for token in user_text.split_whitespace() {
        let candidate = trim_structural_document_target_token(token);
        if candidate.is_empty()
            || !(candidate.contains('/') || candidate.contains('\\'))
            || candidate.contains("://")
            || crate::intent::locator_extractor::candidate_looks_like_dotted_version_number(
                &candidate,
            )
            || !filename_candidate_has_document_extension(&candidate)
            || document_target_already_covered(&targets, &candidate)
        {
            continue;
        }
        targets.push(candidate);
    }
    targets
}

pub(super) fn trim_structural_document_target_token(token: &str) -> String {
    token
        .trim()
        .trim_matches(|ch: char| {
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
        })
        .to_string()
}

pub(super) fn document_target_already_covered(targets: &[String], candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return true;
    }
    let candidate_basename = Path::new(candidate)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(candidate);
    targets.iter().any(|existing| {
        if existing.eq_ignore_ascii_case(candidate) {
            return true;
        }
        let existing_basename = Path::new(existing)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(existing);
        existing_basename.eq_ignore_ascii_case(candidate_basename)
            && (existing.contains('/') || existing.contains('\\'))
            && !(candidate.contains('/') || candidate.contains('\\'))
    })
}

pub(super) fn structured_or_text_multi_file_targets(
    route: &RouteResult,
    user_text: &str,
) -> Vec<String> {
    let structured_targets = crate::task_contract::target_locators_for_route(route)
        .into_iter()
        .filter(|target| target.trim() != ".")
        .collect::<Vec<_>>();
    if structured_targets.len() >= 2 {
        return structured_targets;
    }
    // Deprecated compatibility fallback: keep this limited to structural filename
    // tokens and document-like extensions. Semantic target selection should come
    // from TaskContract/route output, not language-specific phrases.
    explicit_document_file_targets(user_text)
}

pub(super) fn filename_candidate_has_document_extension(candidate: &str) -> bool {
    let Some((_, ext)) = candidate.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "md" | "txt" | "json" | "toml" | "yaml" | "yml" | "rs" | "log" | "sqlite" | "db" | "csv"
    )
}

pub(super) fn locator_identity_token(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | ','
                    | '.'
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
                    | '【'
                    | '】'
            )
        })
        .to_ascii_lowercase()
}

pub(super) fn locator_hint_names_workspace_root(workspace_root: &Path, locator_hint: &str) -> bool {
    let Some(root_name) = workspace_root.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let normalized_root = locator_identity_token(root_name);
    let normalized_hint = locator_identity_token(locator_hint);
    !normalized_root.is_empty() && normalized_hint == normalized_root
}

pub(super) fn prune_unscoped_workspace_summary_evidence_for_scope(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    let scope_hint = route.output_contract.locator_hint.trim();
    if route_has_unresolved_clarify_or_locator_marker(route)
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::WorkspaceProjectSummary
        || scope_hint.is_empty()
    {
        return actions;
    }
    if locator_hint_names_workspace_root(&state.skill_rt.workspace_root, scope_hint) {
        return actions;
    }
    let has_scoped_evidence = actions.iter().any(|action| {
        action_is_workspace_summary_evidence(action)
            && workspace_summary_evidence_matches_scope(action, scope_hint)
    });
    if !has_scoped_evidence {
        return actions;
    }
    let original_len = actions.len();
    let pruned = actions
        .into_iter()
        .filter(|action| {
            !action_is_workspace_summary_evidence(action)
                || workspace_summary_explicit_path_facts(action)
                || workspace_summary_evidence_matches_scope(action, scope_hint)
        })
        .collect::<Vec<_>>();
    if pruned.is_empty() {
        return pruned;
    }
    if pruned.len() != original_len {
        info!(
            "plan_prune_workspace_summary_unscoped_evidence scope={} removed={}",
            scope_hint,
            original_len.saturating_sub(pruned.len())
        );
    }
    pruned
}

fn workspace_summary_evidence_matches_scope(action: &AgentAction, scope_hint: &str) -> bool {
    action_workspace_summary_path(action)
        .is_some_and(|path| path_matches_workspace_scope_hint(path, scope_hint))
        || action_observed_paths_for_explicit_file_targets(action)
            .iter()
            .any(|path| path_matches_workspace_scope_hint(path, scope_hint))
}

fn workspace_summary_explicit_path_facts(action: &AgentAction) -> bool {
    let Some(skill) = planned_action_skill_name(action).map(str::trim) else {
        return false;
    };
    if !skill.eq_ignore_ascii_case("fs_basic") {
        return false;
    }
    let Some(args) = action_args(action).and_then(Value::as_object) else {
        return false;
    };
    args.get("action")
        .and_then(Value::as_str)
        .is_some_and(|action| action.trim().eq_ignore_ascii_case("stat_paths"))
        && !action_observed_paths_for_explicit_file_targets(action).is_empty()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SingleFileReadActionKind {
    ReadFile,
    FsBasicReadTextRange,
    SystemBasicReadRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SingleStructuredFieldReadActionKind {
    ConfigBasic,
    SystemBasic,
}

pub(super) fn single_file_read_action(
    actions: &[AgentAction],
) -> Option<(usize, SingleFileReadActionKind, String)> {
    let mut candidate: Option<(usize, SingleFileReadActionKind, String)> = None;
    for (idx, action) in actions.iter().enumerate() {
        match action {
            AgentAction::Think { .. }
            | AgentAction::Respond { .. }
            | AgentAction::SynthesizeAnswer { .. } => {}
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("read_file") => {
                let Some(path) = args.get("path").and_then(|value| value.as_str()) else {
                    return None;
                };
                if candidate.is_some() {
                    return None;
                }
                candidate = Some((
                    idx,
                    SingleFileReadActionKind::ReadFile,
                    path.trim().to_string(),
                ));
            }
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args }
                if skill.eq_ignore_ascii_case("fs_basic")
                    && args
                        .get("action")
                        .and_then(|value| value.as_str())
                        .is_some_and(|action| action.eq_ignore_ascii_case("read_text_range")) =>
            {
                let Some(path) = args.get("path").and_then(|value| value.as_str()) else {
                    return None;
                };
                if candidate.is_some() {
                    return None;
                }
                candidate = Some((
                    idx,
                    SingleFileReadActionKind::FsBasicReadTextRange,
                    path.trim().to_string(),
                ));
            }
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args }
                if skill.eq_ignore_ascii_case("system_basic")
                    && args
                        .get("action")
                        .and_then(|value| value.as_str())
                        .is_some_and(|action| action.eq_ignore_ascii_case("read_range")) =>
            {
                let Some(path) = args.get("path").and_then(|value| value.as_str()) else {
                    return None;
                };
                if candidate.is_some() {
                    return None;
                }
                candidate = Some((
                    idx,
                    SingleFileReadActionKind::SystemBasicReadRange,
                    path.trim().to_string(),
                ));
            }
            _ => return None,
        }
    }
    candidate
}

pub(super) fn single_structured_field_read_action(
    actions: &[AgentAction],
) -> Option<(usize, SingleStructuredFieldReadActionKind, String)> {
    let mut candidate: Option<(usize, SingleStructuredFieldReadActionKind, String)> = None;
    for (idx, action) in actions.iter().enumerate() {
        match action {
            AgentAction::Think { .. }
            | AgentAction::Respond { .. }
            | AgentAction::SynthesizeAnswer { .. } => {}
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args } => {
                let action_name = args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default();
                let kind = if skill.eq_ignore_ascii_case("config_basic")
                    && matches!(action_name, "read_field" | "read_fields")
                {
                    SingleStructuredFieldReadActionKind::ConfigBasic
                } else if skill.eq_ignore_ascii_case("system_basic")
                    && matches!(action_name, "extract_field" | "extract_fields")
                {
                    SingleStructuredFieldReadActionKind::SystemBasic
                } else {
                    return None;
                };
                let Some(path) = args.get("path").and_then(Value::as_str) else {
                    return None;
                };
                if candidate.is_some() {
                    return None;
                }
                candidate = Some((idx, kind, path.trim().to_string()));
            }
            AgentAction::CallCapability { .. } => return None,
        }
    }
    candidate
}
