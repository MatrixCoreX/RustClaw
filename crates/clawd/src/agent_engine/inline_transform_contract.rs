use super::*;

pub(super) fn parse_answer_candidate_value(candidate: &str) -> Option<Value> {
    let trimmed = candidate.trim();
    serde_json::from_str::<Value>(trimmed)
        .ok()
        .or_else(|| {
            crate::extract_first_json_value_any(trimmed)
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        })
        .or_else(|| {
            trimmed
                .parse::<i64>()
                .ok()
                .map(|value| Value::Number(value.into()))
        })
        .or_else(|| {
            trimmed
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
        })
}

pub(super) fn replace_scalar_path_respond_only_with_auto_locator_observation(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output || is_plain_respond_only_plan(&actions).is_none() {
        return actions;
    }
    let auto_locator_path = auto_locator_path.or_else(|| {
        loop_state
            .output_vars
            .get("auto_locator_path")
            .map(String::as_str)
    });
    if let Some(observation) =
        scalar_path_auto_locator_observation_plan(route_result, auto_locator_path)
    {
        info!("plan_replace_scalar_path_respond_only_with_auto_locator_observation");
        observation
    } else {
        actions
    }
}

pub(super) fn file_delivery_respond_only_observation_plan(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if !route.wants_file_delivery
        && !route.output_contract.delivery_required
        && route.output_contract.response_shape != crate::OutputResponseShape::FileToken
    {
        return None;
    }
    let content = is_plain_respond_only_plan(actions)?;
    let parsed_file_token = crate::finalize::parse_delivery_file_token(content);
    let path = parsed_file_token
        .as_ref()
        .map(|(_kind, raw_path)| raw_path.trim())
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| route.output_contract.locator_hint.trim());
    file_delivery_path_observation_plan(state, path)
}

fn file_delivery_path_observation_plan(state: &AppState, path: &str) -> Option<Vec<AgentAction>> {
    let path = path.trim();
    if path.is_empty() || path.contains('\n') {
        return None;
    }
    let candidate = Path::new(path);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(candidate)
    };
    let stat_action = AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": [resolved.display().to_string()],
            "include_missing": true,
        }),
    };
    if resolved.is_file() {
        let token = format!("FILE:{}", resolved.display());
        return Some(vec![stat_action, AgentAction::Respond { content: token }]);
    }
    Some(vec![stat_action])
}

pub(super) fn replace_file_delivery_respond_only_with_path_observation(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output || is_plain_respond_only_plan(&actions).is_none() {
        return actions;
    }
    if let Some(observation) =
        file_delivery_respond_only_observation_plan(state, route_result, &actions)
    {
        info!("plan_replace_file_delivery_respond_only_with_path_observation");
        observation
    } else {
        actions
    }
}

fn file_delivery_empty_write_path<'a>(
    state: &AppState,
    action: &'a AgentAction,
) -> Option<&'a str> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return None,
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    if canonical != "fs_basic" {
        return None;
    }
    let action_name = args.get("action").and_then(Value::as_str)?.trim();
    if action_name != "write_text" {
        return None;
    }
    let content = args.get("content").and_then(Value::as_str).unwrap_or("");
    if !content.is_empty() {
        return None;
    }
    args.get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

fn file_delivery_empty_write_targets_locator(
    state: &AppState,
    route: &RouteResult,
    action: &AgentAction,
) -> bool {
    let Some(path) = file_delivery_empty_write_path(state, action) else {
        return false;
    };
    let locator = route.output_contract.locator_hint.trim();
    if locator.is_empty() {
        return false;
    }
    let locator_path = resolve_delivery_token_path(state, locator);
    let action_path = resolve_delivery_token_path(state, path);
    same_existing_or_display_path(&locator_path, &action_path)
}

fn file_delivery_empty_write_placeholder_observation_plan(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if !file_delivery_contract_requires_file_token(route)
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
    {
        return None;
    }
    let mut has_empty_locator_write = false;
    for action in actions {
        if file_delivery_empty_write_targets_locator(state, route, action) {
            has_empty_locator_write = true;
            continue;
        }
        if action_is_likely_mutating(state, action) {
            return None;
        }
    }
    if !has_empty_locator_write {
        return None;
    }
    file_delivery_path_observation_plan(state, route.output_contract.locator_hint.trim())
}

pub(super) fn replace_file_delivery_empty_write_with_path_observation(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    if let Some(observation) =
        file_delivery_empty_write_placeholder_observation_plan(state, route_result, &actions)
    {
        info!("plan_replace_file_delivery_empty_write_with_path_observation");
        observation
    } else {
        actions
    }
}

pub(super) fn action_make_dir_path(state: &AppState, action: &AgentAction) -> Option<String> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return None,
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    let obj = args.as_object()?;
    match canonical.as_str() {
        "make_dir" => obj.get("path").and_then(Value::as_str),
        "fs_basic" => {
            let action = obj.get("action").and_then(Value::as_str)?.trim();
            if action == "make_dir" {
                obj.get("path").and_then(Value::as_str)
            } else {
                None
            }
        }
        _ => None,
    }
    .map(str::trim)
    .filter(|path| !path.is_empty())
    .map(ToString::to_string)
}

pub(super) fn file_delivery_contract_requires_file_token(route: &RouteResult) -> bool {
    route.wants_file_delivery
        || route.output_contract.delivery_required
        || route.output_contract.response_shape == crate::OutputResponseShape::FileToken
}

pub(super) fn file_delivery_contract_is_token_only(route: &RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::FileToken
        && (route.output_contract_is_unclassified()
            || route.output_contract_marker_is(crate::OutputSemanticKind::GeneratedFileDelivery))
}

pub(super) fn generated_file_write_action_path(
    state: &AppState,
    action: &AgentAction,
) -> Option<String> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return None,
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    let obj = args.as_object()?;
    match canonical.as_str() {
        "write_file" => obj.get("path").and_then(Value::as_str),
        "fs_basic" => {
            let action = obj.get("action").and_then(Value::as_str)?.trim();
            if matches!(action, "write_text" | "append_text") {
                obj.get("path").and_then(Value::as_str)
            } else {
                None
            }
        }
        _ => None,
    }
    .map(str::trim)
    .filter(|path| !path.is_empty())
    .map(ToString::to_string)
}

pub(super) fn resolve_delivery_token_path(state: &AppState, path: &str) -> PathBuf {
    let candidate = Path::new(path.trim());
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(candidate)
    }
}

pub(super) fn delivery_write_parent_matches_make_dir(
    state: &AppState,
    write_path: &str,
    make_dir_path: &str,
) -> bool {
    let write_path = resolve_delivery_token_path(state, write_path);
    let make_dir_path = resolve_delivery_token_path(state, make_dir_path);
    write_path
        .parent()
        .is_some_and(|parent| same_existing_or_display_path(parent, &make_dir_path))
}

pub(super) fn strip_redundant_make_dir_before_file_delivery_write(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !file_delivery_contract_requires_file_token(route) {
        return actions;
    }
    let write_paths = actions
        .iter()
        .filter_map(|action| generated_file_write_action_path(state, action))
        .collect::<Vec<_>>();
    if write_paths.is_empty() {
        return actions;
    }
    let original_len = actions.len();
    let stripped = actions
        .into_iter()
        .filter(|action| {
            let Some(make_dir_path) = action_make_dir_path(state, action) else {
                return true;
            };
            !write_paths.iter().any(|write_path| {
                delivery_write_parent_matches_make_dir(state, write_path, &make_dir_path)
            })
        })
        .collect::<Vec<_>>();
    if stripped.len() != original_len {
        info!(
            "plan_strip_redundant_make_dir_before_file_delivery_write removed={}",
            original_len.saturating_sub(stripped.len())
        );
    }
    stripped
}

pub(super) fn append_file_token_after_generated_file_write_delivery(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !file_delivery_contract_requires_file_token(route) {
        return actions;
    }
    if actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::Respond { content }
                if crate::finalize::parse_delivery_file_token(content.trim()).is_some()
        )
    }) {
        return actions;
    }
    let Some(path) = actions
        .iter()
        .rev()
        .find_map(|action| generated_file_write_action_path(state, action))
    else {
        return actions;
    };
    let resolved = resolve_delivery_token_path(state, &path);
    let token = format!("FILE:{}", resolved.display());
    let mut rewritten = actions;
    match rewritten.last_mut() {
        Some(AgentAction::Respond { content }) if file_delivery_contract_is_token_only(route) => {
            *content = token;
        }
        _ => rewritten.push(AgentAction::Respond { content: token }),
    }
    info!("plan_append_file_token_after_generated_file_write_delivery");
    rewritten
}

pub(super) fn existing_file_delivery_observation_path(
    state: &AppState,
    action: &AgentAction,
) -> Option<PathBuf> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return None,
    };
    if state.resolve_canonical_skill_name(skill) != "fs_basic" {
        return None;
    }
    let obj = args.as_object()?;
    let action = obj.get("action").and_then(Value::as_str).map(str::trim);
    if !matches!(action, Some("stat_paths" | "path_batch_facts")) {
        return None;
    }
    let path = obj
        .get("paths")
        .and_then(Value::as_array)
        .and_then(|paths| paths.iter().find_map(Value::as_str))
        .or_else(|| obj.get("path").and_then(Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let resolved = resolve_delivery_token_path(state, path);
    resolved.is_file().then_some(resolved)
}

pub(super) fn append_file_token_after_existing_file_delivery_observation(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !file_delivery_contract_requires_file_token(route) {
        return actions;
    }
    if !file_delivery_contract_is_token_only(route) {
        return actions;
    }
    if actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::Respond { content }
                if crate::finalize::parse_delivery_file_token(content.trim()).is_some()
        )
    }) {
        return actions;
    }
    let Some(path) = actions
        .iter()
        .rev()
        .find_map(|action| existing_file_delivery_observation_path(state, action))
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            if hint.is_empty() {
                return None;
            }
            let resolved = resolve_delivery_token_path(state, hint);
            resolved.is_file().then_some(resolved)
        })
    else {
        return actions;
    };
    let token = format!("FILE:{}", path.display());
    let mut rewritten = actions;
    match rewritten.last_mut() {
        Some(AgentAction::Respond { content }) => *content = token,
        _ => rewritten.push(AgentAction::Respond { content: token }),
    }
    info!("plan_append_file_token_after_existing_file_delivery_observation");
    rewritten
}

pub(super) fn route_is_existing_file_content_delivery(
    state: &AppState,
    route_result: Option<&RouteResult>,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route.output_contract.delivery_required
        || route.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
    {
        return false;
    }
    match route.effective_output_contract_semantic_kind() {
        crate::OutputSemanticKind::GeneratedFileDelivery => {
            if route.output_contract.response_shape != crate::OutputResponseShape::FileToken {
                return false;
            }
        }
        kind if kind.is_content_excerpt_summary() => {}
        _ => return false,
    }
    let hint = route.output_contract.locator_hint.trim();
    if hint.is_empty() {
        return false;
    }
    let resolved = resolve_delivery_token_path(state, hint);
    resolved.is_file()
}

pub(super) fn action_is_existing_file_content_read(state: &AppState, action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return false,
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    if canonical == "doc_parse" || canonical == "read_file" {
        return true;
    }
    if canonical != "fs_basic" {
        return false;
    }
    args.as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|action| matches!(action, "read_text_range" | "read_range"))
}

pub(super) fn respond_content_has_file_token_line_and_prose(content: &str) -> bool {
    let mut has_token = false;
    let mut has_non_token = false;
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if crate::finalize::parse_delivery_file_token(line).is_some() {
            has_token = true;
        } else {
            has_non_token = true;
        }
    }
    has_token && has_non_token
}

pub(super) fn rewrite_mixed_file_token_prose_respond_to_synthesize_answer(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !route_is_existing_file_content_delivery(state, route_result)
        || (!actions
            .iter()
            .any(|action| action_is_existing_file_content_read(state, action))
            && !loop_state
                .executed_step_results
                .iter()
                .any(executed_step_is_successful_text_read))
    {
        return actions;
    }
    let mut rewritten = actions;
    let Some(AgentAction::Respond { content }) = rewritten.last() else {
        return rewritten;
    };
    if !respond_content_has_file_token_line_and_prose(content) {
        return rewritten;
    }
    *rewritten.last_mut().expect("last action exists") = AgentAction::SynthesizeAnswer {
        evidence_refs: vec!["last_output".to_string()],
    };
    info!("plan_rewrite_mixed_file_token_prose_respond_to_synthesize_answer");
    rewritten
}

pub(super) fn scalar_count_locator_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !scalar_count_contract_allows_count_shape(route)
        || !route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount)
        || route_requests_hidden_entries_count(route)
    {
        return None;
    }
    route_directory_locator_path(route, auto_locator_path)
}

fn scalar_count_active_listing_path(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !scalar_count_contract_allows_count_shape(route)
        || !route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount)
        || route_requests_hidden_entries_count(route)
        || route.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route.output_contract.locator_hint.trim().is_empty()
        || ![
            "active_listing_target_required",
            "target_locator_required",
            "missing_target_locator",
        ]
        .iter()
        .any(|marker| route_reason_has_structural_marker(route, marker))
    {
        return None;
    }
    let Some(raw) = loop_state.output_vars.get("active_listing_bound_targets") else {
        return None;
    };
    let Ok(values) = serde_json::from_str::<Vec<String>>(raw) else {
        return None;
    };
    let mut targets = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    (targets.len() == 1).then(|| targets.remove(0))
}

fn scalar_count_current_workspace_scope_path(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !scalar_count_contract_allows_count_shape(route)
        || !route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount)
        || route_requests_hidden_entries_count(route)
        || !route.output_contract.locator_hint.trim().is_empty()
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace | crate::OutputLocatorKind::None
        )
        || !(route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
            || route_reason_has_structural_marker(
                route,
                "current_workspace_scope_from_current_request",
            ))
    {
        return None;
    }
    let Some(raw) = loop_state
        .output_vars
        .get("current_workspace_scalar_count_targets")
    else {
        return None;
    };
    let Ok(values) = serde_json::from_str::<Vec<String>>(raw) else {
        return None;
    };
    let mut targets = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();
    (targets.len() == 1).then(|| targets.remove(0))
}

pub(super) fn scalar_count_contract_allows_count_shape(route: &RouteResult) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::OneSentence
    ) || (route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route.output_contract.exact_sentence_count == Some(1))
}

pub(super) fn replace_scalar_count_plan_with_count_inventory(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    if actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
                if skill.eq_ignore_ascii_case("run_cmd")
        )
    }) {
        return actions;
    }
    let Some(path) = scalar_count_explicit_count_path_from_actions(&actions)
        .or_else(|| scalar_count_active_listing_path(route_result, loop_state))
        .or_else(|| scalar_count_current_workspace_scope_path(route_result, loop_state))
        .or_else(|| scalar_count_locator_path(route_result, auto_locator_path))
    else {
        return actions;
    };
    if !Path::new(&path).is_dir() {
        info!("plan_replace_scalar_count_missing_locator_with_path_facts path={path}");
        let answer = if crate::language_policy::request_language_hint(user_text)
            .to_ascii_lowercase()
            .starts_with("en")
        {
            format!("{path} does not exist, so the matching item count cannot be computed.")
        } else {
            format!("{path} 不存在，无法统计匹配项数量。")
        };
        return vec![
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "stat_paths",
                    "paths": [path],
                    "include_missing": true,
                }),
            },
            AgentAction::Respond { content: answer },
        ];
    }
    info!("plan_replace_scalar_count_plan_with_count_inventory");
    if scalar_count_actions_include_listing(&actions) {
        info!("plan_scalar_count_listing_requires_structured_count_repair");
        return actions;
    }
    let inventory_kind = scalar_count_inventory_kind_from_actions(&actions);
    let mut args = serde_json::json!({
        "action": "count_entries",
        "path": path,
    });
    if let Some(obj) = args.as_object_mut() {
        apply_scalar_count_inventory_filters_from_actions(obj, &actions);
        match inventory_kind {
            ScalarCountInventoryKind::Any => {}
            ScalarCountInventoryKind::Files => {
                obj.insert("kind_filter".to_string(), Value::String("file".to_string()));
                obj.insert("count_files".to_string(), Value::Bool(true));
                obj.insert("count_dirs".to_string(), Value::Bool(false));
                obj.insert("files_only".to_string(), Value::Bool(true));
                obj.insert("dirs_only".to_string(), Value::Bool(false));
            }
            ScalarCountInventoryKind::Dirs => {
                obj.insert("kind_filter".to_string(), Value::String("dir".to_string()));
                obj.insert("count_files".to_string(), Value::Bool(false));
                obj.insert("count_dirs".to_string(), Value::Bool(true));
                obj.insert("dirs_only".to_string(), Value::Bool(true));
                obj.insert("files_only".to_string(), Value::Bool(false));
            }
        }
        if let Some(hint) = route_result.and_then(scalar_count_filter_hint_from_route) {
            apply_scalar_count_filter_hint(obj, &hint);
        }
    }
    vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args,
    }]
}

pub(super) fn scalar_count_actions_include_listing(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        let (skill, args) = match action {
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args } => (skill.as_str(), args),
            _ => return false,
        };
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase());
        skill.eq_ignore_ascii_case("list_dir")
            || (skill.eq_ignore_ascii_case("fs_basic")
                && matches!(action_name.as_deref(), Some("list_dir")))
            || (skill.eq_ignore_ascii_case("system_basic")
                && matches!(action_name.as_deref(), Some("inventory_dir")))
    })
}
