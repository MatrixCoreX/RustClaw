use super::*;

pub(super) fn rewrite_single_target_structured_field_read_to_auto_locator(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route_result) = route_result else {
        return actions;
    };
    if route_result.needs_clarify
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
        || !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::Strict
        )
    {
        return actions;
    }
    let Some(auto_locator_path) = auto_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty() && path_has_structured_document_extension(value))
    else {
        return actions;
    };
    let auto_locator = std::path::Path::new(auto_locator_path);
    if !auto_locator.is_file() {
        return actions;
    }
    let Some((idx, kind, current_path)) = single_structured_field_read_action(&actions) else {
        return actions;
    };
    if same_existing_or_display_path(std::path::Path::new(&current_path), auto_locator) {
        return actions;
    }
    if structured_field_read_path_should_not_be_overwritten_by_auto_locator(
        route_result,
        actions.get(idx),
        &current_path,
        auto_locator,
    ) {
        return actions;
    }

    let mut rewritten = actions;
    let Some(action) = rewritten.get_mut(idx) else {
        return rewritten;
    };
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => {
            if let Some(obj) = args.as_object_mut() {
                obj.insert(
                    "path".to_string(),
                    Value::String(auto_locator_path.to_string()),
                );
            }
        }
        _ => return rewritten,
    }
    info!(
        "plan_rewrite_single_target_structured_field_read_to_auto_locator idx={} kind={:?} from={} to={}",
        idx, kind, current_path, auto_locator_path
    );
    rewritten
}

pub(super) fn structured_field_read_path_should_not_be_overwritten_by_auto_locator(
    route: &RouteResult,
    action: Option<&AgentAction>,
    current_path: &str,
    auto_locator: &Path,
) -> bool {
    let Some(action) = action else {
        return false;
    };
    let args = match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args,
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => return false,
    };
    let Some(request) = structured_extract_request(args) else {
        return false;
    };
    let current = Path::new(current_path);
    if !structured_file_has_all_fields(current, &request.fields)
        || structured_file_has_all_fields(auto_locator, &request.fields)
    {
        return false;
    }
    request
        .fields
        .iter()
        .all(|field| field.starts_with("workspace.package."))
        || route_allows_structured_candidate_read_target_repair(route)
}

pub(super) fn rewrite_single_target_file_read_to_auto_locator(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route_result) = route_result else {
        return actions;
    };
    if route_result.needs_clarify || route_result.output_contract.delivery_required {
        return actions;
    }
    let Some(target_path) = auto_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            matches!(
                route_result.output_contract.locator_kind,
                crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
            )
            .then(|| route_result.output_contract.locator_hint.trim())
            .filter(|value| !value.is_empty())
        })
    else {
        return actions;
    };
    let target_locator = std::path::Path::new(target_path);
    if !target_locator.is_file() {
        return actions;
    }
    let Some((idx, kind, current_path)) = single_file_read_action(&actions) else {
        return actions;
    };
    if current_path == target_path {
        return actions;
    }
    if action_path_is_concrete_locator(&current_path) {
        return actions;
    }

    // 当前轮 route/ordinal/auto-locator 已解析成一个具体文件时，这个路径比 LLM
    // 在厚上下文里“顺手抄到的旧文件路径”更权威。这里只在单目标读文件链路上收口，
    // 避免把多文件 read 计划错误折叠成同一个 target。
    let mut rewritten = actions;
    let Some(action) = rewritten.get_mut(idx) else {
        return rewritten;
    };
    match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => {
            if let Some(obj) = args.as_object_mut() {
                obj.insert("path".to_string(), Value::String(target_path.to_string()));
            }
        }
        _ => return rewritten,
    }
    info!(
        "plan_rewrite_single_target_file_read_to_auto_locator idx={} kind={:?} from={} to={}",
        idx, kind, current_path, target_path
    );
    rewritten
}

pub(super) fn rewrite_session_alias_delivery_observations_to_route_locator(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(target) = session_alias_delivery_target(route_result, loop_state) else {
        return actions;
    };
    let target = target.trim().to_string();
    if target.is_empty() {
        return actions;
    }
    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .map(|mut action| {
            if rewrite_session_alias_delivery_observation_action_path(&mut action, &target) {
                changed = true;
            }
            action
        })
        .collect::<Vec<_>>();
    if changed {
        info!(
            "plan_rewrite_session_alias_delivery_observations_to_route_locator target={}",
            target
        );
    }
    rewritten
}

fn session_alias_delivery_target(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.delivery_required
        || route.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
        || route.output_contract.response_shape != crate::OutputResponseShape::FileToken
    {
        return None;
    }

    if matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
    ) && !route.output_contract.locator_hint.trim().is_empty()
        && route_reason_has_marker(route, "session_alias_locator_prebound_from_current_request")
    {
        return Some(route.output_contract.locator_hint.trim().to_string());
    }

    let targets = required_session_alias_targets_from_loop_state(loop_state);
    (targets.len() == 1).then(|| targets[0].clone())
}

pub(super) fn rewrite_active_bound_target_observations_to_matching_locator_hint(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(target) = matching_active_bound_target(route_result, loop_state) else {
        return actions;
    };
    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .map(|mut action| {
            if rewrite_session_alias_delivery_observation_action_path(&mut action, &target) {
                changed = true;
            }
            action
        })
        .collect::<Vec<_>>();
    if changed {
        info!(
            "plan_rewrite_active_bound_target_observations_to_matching_locator_hint target={}",
            target
        );
    }
    rewritten
}

fn matching_active_bound_target(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.wants_file_delivery
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return None;
    }
    let locator_hint = route.output_contract.locator_hint.trim();
    if locator_hint.is_empty()
        || locator_hint.contains('/')
        || locator_hint.contains('\\')
        || std::path::Path::new(locator_hint).is_absolute()
    {
        return None;
    }
    let targets = active_bound_targets_from_loop_state(loop_state);
    let mut matches = targets
        .into_iter()
        .filter(|target| path_basename_matches(target, locator_hint))
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    (matches.len() == 1).then(|| matches.remove(0))
}

fn active_bound_targets_from_loop_state(loop_state: &LoopState) -> Vec<String> {
    let Some(raw) = loop_state.output_vars.get("active_bound_targets") else {
        return Vec::new();
    };
    let Ok(values) = serde_json::from_str::<Vec<String>>(raw) else {
        return Vec::new();
    };
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn path_basename_matches(path: &str, basename: &str) -> bool {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(basename))
}

pub(super) fn rewrite_session_alias_delivery_observation_action_path(
    action: &mut AgentAction,
    target: &str,
) -> bool {
    let args = match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args }
            if tool.eq_ignore_ascii_case("fs_basic") =>
        {
            args
        }
        AgentAction::CallCapability { capability, args }
            if capability.eq_ignore_ascii_case("fs_basic")
                || capability.strip_prefix("fs_basic.").is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "stat_paths" | "read_text_range" | "find_entries" | "list_dir"
                    )
                }) =>
        {
            args
        }
        _ => return false,
    };
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    match action_name {
        "stat_paths" => {
            let Some(paths) = obj.get_mut("paths").and_then(Value::as_array_mut) else {
                return false;
            };
            if paths.len() != 1 || paths.first().and_then(Value::as_str) == Some(target) {
                return false;
            }
            if paths
                .first()
                .and_then(Value::as_str)
                .is_some_and(action_path_is_concrete_locator)
            {
                return false;
            }
            paths[0] = Value::String(target.to_string());
            true
        }
        "read_text_range" | "find_entries" | "list_dir" => {
            let Some(path_value) = obj.get_mut("path") else {
                return false;
            };
            if path_value.as_str() == Some(target) {
                return false;
            }
            if path_value
                .as_str()
                .is_some_and(action_path_is_concrete_locator)
            {
                return false;
            }
            *path_value = Value::String(target.to_string());
            true
        }
        _ => false,
    }
}

fn action_path_is_concrete_locator(path: &str) -> bool {
    let path = path.trim();
    if path.is_empty() {
        return false;
    }
    let path = std::path::Path::new(path);
    path.is_absolute() || path.components().count() > 1
}

pub(super) fn collapse_route_target_file_content_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route_result) = route_result else {
        return actions;
    };
    if !route_allows_route_target_file_content_plan(route_result) {
        return actions;
    }
    let Some(target_path) = route_target_file_content_path(route_result, auto_locator_path) else {
        return actions;
    };
    if !actions
        .iter()
        .all(route_target_file_content_plan_action_is_collapsible)
    {
        return actions;
    }
    let Some(args) = route_target_file_content_read_args(&actions, target_path) else {
        return actions;
    };
    if actions.len() == 1 {
        if let Some((_, _, current_path)) = single_file_read_action(&actions) {
            if current_path == target_path {
                return actions;
            }
        }
    }
    info!(
        "plan_collapse_route_target_file_content_plan target={} steps={}",
        target_path,
        actions.len()
    );
    vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args,
    }]
}

pub(super) fn route_allows_route_target_file_content_plan(route: &RouteResult) -> bool {
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Strict | crate::OutputResponseShape::Scalar
        )
        && route.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput)
        && matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
}

pub(super) fn route_target_file_content_path<'a>(
    route: &'a RouteResult,
    auto_locator_path: Option<&'a str>,
) -> Option<&'a str> {
    if let Some(path) = auto_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty() && Path::new(value).is_file())
    {
        return Some(path);
    }
    let hint = route.output_contract.locator_hint.trim();
    (!hint.is_empty() && Path::new(hint).is_file()).then_some(hint)
}

pub(super) fn route_target_file_content_plan_action_is_collapsible(action: &AgentAction) -> bool {
    match action {
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => true,
        AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("read_file") => args
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| !path.trim().is_empty()),
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if skill.eq_ignore_ascii_case("fs_basic") {
                matches!(
                    action_name,
                    "read_text_range" | "list_dir" | "inventory_dir"
                )
            } else if skill.eq_ignore_ascii_case("system_basic") {
                matches!(action_name, "read_range" | "list_dir" | "inventory_dir")
            } else {
                false
            }
        }
        AgentAction::CallCapability { .. } => false,
    }
}

pub(super) fn route_target_file_content_read_args(
    actions: &[AgentAction],
    target_path: &str,
) -> Option<Value> {
    for action in actions.iter().rev() {
        match action {
            AgentAction::CallSkill { skill, args } if skill.eq_ignore_ascii_case("read_file") => {
                let mut obj = serde_json::Map::new();
                obj.insert(
                    "action".to_string(),
                    Value::String("read_text_range".to_string()),
                );
                obj.insert("path".to_string(), Value::String(target_path.to_string()));
                if let Some(raw) = args.get("raw").cloned() {
                    obj.insert("raw".to_string(), raw);
                }
                return Some(Value::Object(obj));
            }
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args } => {
                let action_name = args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default();
                let is_range_read = (skill.eq_ignore_ascii_case("fs_basic")
                    && action_name.eq_ignore_ascii_case("read_text_range"))
                    || (skill.eq_ignore_ascii_case("system_basic")
                        && action_name.eq_ignore_ascii_case("read_range"));
                if !is_range_read {
                    continue;
                }
                let mut args = args.clone();
                if let Some(obj) = args.as_object_mut() {
                    obj.insert(
                        "action".to_string(),
                        Value::String("read_text_range".to_string()),
                    );
                    obj.insert("path".to_string(), Value::String(target_path.to_string()));
                }
                return Some(args);
            }
            AgentAction::Think { .. }
            | AgentAction::Respond { .. }
            | AgentAction::SynthesizeAnswer { .. }
            | AgentAction::CallCapability { .. } => {}
        }
    }
    None
}

pub(super) fn rewrite_extract_field_alias_args(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    let mut rewritten = actions;
    for (idx, action) in rewritten.iter_mut().enumerate() {
        let AgentAction::CallSkill { skill, args } = action else {
            continue;
        };
        if !skill.eq_ignore_ascii_case("system_basic") {
            continue;
        }
        let Some(obj) = args.as_object_mut() else {
            continue;
        };
        let is_extract_field = obj
            .get("action")
            .and_then(|value| value.as_str())
            .is_some_and(|action| action.eq_ignore_ascii_case("extract_field"));
        if !is_extract_field || obj.contains_key("field_path") {
        } else if let Some(field_value) = obj.remove("field") {
            let Value::String(field_path) = field_value else {
                obj.insert("field".to_string(), field_value);
                continue;
            };
            let trimmed = field_path.trim();
            if trimmed.is_empty() {
                obj.insert("field".to_string(), Value::String(field_path));
                continue;
            }
            obj.insert("field_path".to_string(), Value::String(trimmed.to_string()));
            info!(
                "plan_rewrite_extract_field_alias idx={} alias=field canonical=field_path",
                idx
            );
        }
        if !obj.contains_key("path") {
            if let Some(path_value) = obj.remove("file_path") {
                match path_value {
                    Value::String(path) if !path.trim().is_empty() => {
                        obj.insert("path".to_string(), Value::String(path.trim().to_string()));
                        info!(
                            "plan_rewrite_extract_field_alias idx={} alias=file_path canonical=path",
                            idx
                        );
                    }
                    other => {
                        obj.insert("file_path".to_string(), other);
                    }
                }
            }
        }
        if !obj.contains_key("path") {
            if let Some(path_value) = obj.remove("target") {
                match path_value {
                    Value::String(path) if !path.trim().is_empty() => {
                        obj.insert("path".to_string(), Value::String(path.trim().to_string()));
                        info!(
                            "plan_rewrite_extract_field_alias idx={} alias=target canonical=path",
                            idx
                        );
                    }
                    other => {
                        obj.insert("target".to_string(), other);
                    }
                }
            }
        }
    }
    rewritten
}

#[derive(Debug, Clone)]
pub(super) struct StructuredExtractRequest {
    pub(super) path: String,
    pub(super) fields: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct StructuredFieldCandidate {
    path: PathBuf,
    depth: usize,
    package_name: Option<String>,
}

pub(super) fn rewrite_extract_field_paths_to_structured_candidates(
    state: &AppState,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify || route.output_contract.delivery_required {
        return actions;
    }

    let mut rewritten = actions;
    for (idx, action) in rewritten.iter_mut().enumerate() {
        let (skill, args) = match action {
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args } => (skill, args),
            _ => {
                continue;
            }
        };
        if !skill.eq_ignore_ascii_case("system_basic")
            && !skill.eq_ignore_ascii_case("config_basic")
        {
            continue;
        }
        let Some(request) = structured_extract_request(args) else {
            continue;
        };
        let current = resolve_workspace_path(&state.skill_rt.workspace_root, &request.path);
        if rewrite_cargo_workspace_package_fields_to_workspace_package(
            route,
            args,
            &state.skill_rt.workspace_root,
            &current,
            &request,
        ) {
            continue;
        }
        if structured_file_has_all_fields(&current, &request.fields) {
            continue;
        }
        if !should_repair_structured_extract_path(
            route,
            &state.skill_rt.workspace_root,
            &request.path,
            &current,
            auto_locator_path,
        ) {
            continue;
        }

        let Some(replacement) = find_structured_field_candidate(
            &state.skill_rt.workspace_root,
            &current,
            &request.fields,
            state.skill_rt.locator_scan_max_files,
        ) else {
            continue;
        };
        let replacement_text = replacement.display().to_string();
        let Some(obj) = args.as_object_mut() else {
            continue;
        };
        obj.insert("path".to_string(), Value::String(replacement_text.clone()));
        info!(
            "plan_rewrite_extract_field_path idx={} from={} to={} fields={:?}",
            idx, request.path, replacement_text, request.fields
        );
    }
    rewritten
}

pub(super) fn canonicalize_quantity_compare_structured_field_reads(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route.output_contract_marker_is(crate::OutputSemanticKind::QuantityComparison)
        || !actions.iter().any(action_reads_workspace_text_content)
    {
        return actions;
    }

    actions
        .into_iter()
        .map(|action| {
            let (skill, args) = match &action {
                AgentAction::CallSkill { skill, args }
                | AgentAction::CallTool { tool: skill, args } => (skill.as_str(), args),
                _ => return action,
            };
            if !skill.eq_ignore_ascii_case("system_basic")
                && !skill.eq_ignore_ascii_case("config_basic")
            {
                return action;
            }
            let Some(request) = structured_extract_request(args) else {
                return action;
            };
            if request.fields.len() != 1 {
                return action;
            }
            let (path, field_path) = resolve_structured_scalar_read_target_and_field(
                state,
                route,
                &request.path,
                &request.fields[0],
            );
            config_basic_read_field_action(path, field_path)
        })
        .collect()
}

pub(super) fn rewrite_cargo_workspace_package_fields_to_workspace_package(
    route: &RouteResult,
    args: &mut Value,
    workspace_root: &Path,
    current: &Path,
    request: &StructuredExtractRequest,
) -> bool {
    if current.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
        return false;
    }
    let Some((target_path, rewritten_fields)) =
        resolve_cargo_workspace_package_fields(workspace_root, current, &request.fields)
    else {
        return false;
    };
    if structured_file_has_all_fields(current, &request.fields)
        && route_locator_targets_current_path(route, workspace_root, current)
    {
        return false;
    }
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let action_name = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if matches!(
        action_name.to_ascii_lowercase().as_str(),
        "extract_field" | "read_field"
    ) && rewritten_fields.len() == 1
    {
        obj.insert(
            "field_path".to_string(),
            Value::String(rewritten_fields[0].clone()),
        );
    } else if matches!(
        action_name.to_ascii_lowercase().as_str(),
        "extract_fields" | "read_fields"
    ) {
        obj.remove("fields");
        obj.insert(
            "field_paths".to_string(),
            Value::Array(
                rewritten_fields
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    } else {
        return false;
    }
    obj.insert(
        "path".to_string(),
        Value::String(target_path.display().to_string()),
    );
    info!(
        "plan_rewrite_cargo_workspace_package_fields from={} to={} fields={:?}",
        current.display(),
        target_path.display(),
        rewritten_fields
    );
    true
}

pub(super) fn resolve_cargo_workspace_package_fields(
    workspace_root: &Path,
    current: &Path,
    fields: &[String],
) -> Option<(PathBuf, Vec<String>)> {
    let current_value = parse_structured_file_value(current)?;
    let rewritten_fields = cargo_workspace_package_field_paths(fields)?;
    if lookup_structured_field_value(&current_value, "workspace").is_some()
        && lookup_structured_field_value(&current_value, "package").is_none()
        && rewritten_fields
            .iter()
            .all(|field| lookup_structured_field_value(&current_value, field).is_some())
    {
        return Some((current.to_path_buf(), rewritten_fields));
    }
    if !fields.iter().all(|field| {
        lookup_structured_field_value(&current_value, field)
            .is_some_and(is_cargo_workspace_inherited_marker)
    }) {
        return None;
    }
    let target =
        find_cargo_workspace_manifest_with_fields(workspace_root, current, &rewritten_fields)?;
    Some((target, rewritten_fields))
}

pub(super) fn cargo_workspace_package_field_paths(fields: &[String]) -> Option<Vec<String>> {
    let mut rewritten = Vec::with_capacity(fields.len());
    for field in fields {
        let suffix = field.strip_prefix("package.")?;
        if suffix.trim().is_empty() {
            return None;
        }
        rewritten.push(format!("workspace.package.{suffix}"));
    }
    Some(rewritten)
}

pub(super) fn is_cargo_workspace_inherited_marker(value: &Value) -> bool {
    value.as_object().is_some_and(|obj| {
        obj.len() == 1
            && obj
                .get("workspace")
                .and_then(Value::as_bool)
                .unwrap_or(false)
    })
}

pub(super) fn find_cargo_workspace_manifest_with_fields(
    workspace_root: &Path,
    current: &Path,
    fields: &[String],
) -> Option<PathBuf> {
    let workspace_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let mut dir = current.parent()?.to_path_buf();
    loop {
        let manifest = dir.join("Cargo.toml");
        if !same_existing_or_display_path(&manifest, current)
            && manifest.is_file()
            && parse_structured_file_value(&manifest).is_some_and(|value| {
                lookup_structured_field_value(&value, "workspace").is_some()
                    && fields
                        .iter()
                        .all(|field| lookup_structured_field_value(&value, field).is_some())
            })
        {
            return Some(manifest);
        }
        if same_existing_or_display_path(&dir, &workspace_root) || !dir.pop() {
            break;
        }
    }
    None
}

pub(super) fn structured_extract_request(args: &Value) -> Option<StructuredExtractRequest> {
    let obj = args.as_object()?;
    let action = obj.get("action").and_then(Value::as_str)?;
    let path = obj
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let mut fields = match action {
        action
            if matches!(
                action.to_ascii_lowercase().as_str(),
                "extract_field" | "read_field"
            ) =>
        {
            obj.get("field_path")
                .or_else(|| obj.get("field"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| vec![value.to_string()])
                .unwrap_or_default()
        }
        action
            if matches!(
                action.to_ascii_lowercase().as_str(),
                "extract_fields" | "read_fields"
            ) =>
        {
            string_list_from_value(obj.get("field_paths").or_else(|| obj.get("fields")))
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect()
        }
        _ => Vec::new(),
    };
    fields.sort();
    fields.dedup();
    if fields.is_empty() {
        return None;
    }
    Some(StructuredExtractRequest { path, fields })
}

pub(super) fn should_repair_structured_extract_path(
    route: &RouteResult,
    workspace_root: &Path,
    raw_path: &str,
    current: &Path,
    auto_locator_path: Option<&str>,
) -> bool {
    let raw = Path::new(raw_path);
    let raw_is_bare_filename = raw.components().count() == 1;
    if raw_is_bare_filename {
        if auto_locator_is_workspace_root_scope(workspace_root, auto_locator_path)
            && is_workspace_root_direct_child(workspace_root, current)
        {
            return false;
        }
        return true;
    }
    if !matches!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Filename
    ) {
        return false;
    }
    if !is_workspace_root_direct_child(workspace_root, current) {
        return false;
    }
    auto_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|auto_path| same_existing_or_display_path(Path::new(auto_path), current))
        .unwrap_or(true)
}

pub(super) fn auto_locator_is_workspace_root_scope(
    workspace_root: &Path,
    auto_locator_path: Option<&str>,
) -> bool {
    let Some(raw_auto_path) = auto_locator_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let auto_path = resolve_workspace_path(workspace_root, raw_auto_path);
    auto_path.is_dir() && same_existing_or_display_path(&auto_path, workspace_root)
}

pub(super) fn resolve_workspace_path(workspace_root: &Path, raw_path: &str) -> PathBuf {
    let candidate = Path::new(raw_path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    }
}

pub(super) fn same_existing_or_display_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

pub(super) fn is_workspace_root_direct_child(workspace_root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(workspace_root) else {
        return false;
    };
    relative.components().count() == 1
}

pub(super) fn find_structured_field_candidate(
    workspace_root: &Path,
    current: &Path,
    fields: &[String],
    scan_max_files: usize,
) -> Option<PathBuf> {
    let file_name = current.file_name()?.to_os_string();
    let prefer_current_package = workspace_manifest_lacks_package_fields(current, fields);
    let max_files = scan_max_files.max(500);
    let mut candidates = Vec::new();
    let mut seen_files = 0usize;
    collect_structured_field_candidates(
        workspace_root,
        workspace_root,
        &file_name,
        fields,
        max_files,
        &mut seen_files,
        &mut candidates,
    );
    if prefer_current_package {
        let current_package_name = env!("CARGO_PKG_NAME");
        let preferred: Vec<_> = candidates
            .iter()
            .filter(|candidate| candidate.package_name.as_deref() == Some(current_package_name))
            .collect();
        if preferred.len() == 1 {
            return Some(preferred[0].path.clone());
        }
    }

    candidates.sort_by(|left, right| {
        left.depth
            .cmp(&right.depth)
            .then_with(|| left.path.cmp(&right.path))
    });
    let best = candidates.first()?;
    if candidates
        .get(1)
        .is_some_and(|next| next.depth == best.depth)
    {
        return None;
    }
    Some(best.path.clone())
}

pub(super) fn collect_structured_field_candidates(
    workspace_root: &Path,
    dir: &Path,
    file_name: &std::ffi::OsStr,
    fields: &[String],
    max_files: usize,
    seen_files: &mut usize,
    out: &mut Vec<StructuredFieldCandidate>,
) {
    if *seen_files >= max_files {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if should_prune_structured_candidate_dir(&entry.file_name()) {
                continue;
            }
            collect_structured_field_candidates(
                workspace_root,
                &path,
                file_name,
                fields,
                max_files,
                seen_files,
                out,
            );
            if *seen_files >= max_files {
                return;
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        *seen_files += 1;
        if entry.file_name() != file_name {
            continue;
        }
        let Some(root_value) = parse_structured_file_value(&path) else {
            continue;
        };
        if !fields
            .iter()
            .all(|field| lookup_structured_field_value(&root_value, field).is_some())
        {
            continue;
        }
        out.push(StructuredFieldCandidate {
            depth: path
                .strip_prefix(workspace_root)
                .ok()
                .map(|relative| relative.components().count())
                .unwrap_or(usize::MAX),
            package_name: lookup_structured_field_value(&root_value, "package.name")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            path,
        });
    }
}

pub(super) fn should_prune_structured_candidate_dir(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_string_lossy().as_ref(),
        ".git"
            | "target"
            | "node_modules"
            | ".venv"
            | "venv"
            | "dist"
            | "build"
            | ".next"
            | ".cache"
    )
}

pub(super) fn structured_file_has_all_fields(path: &Path, fields: &[String]) -> bool {
    let Some(root_value) = parse_structured_file_value(path) else {
        return false;
    };
    fields
        .iter()
        .all(|field| lookup_structured_field_value(&root_value, field).is_some())
}

pub(super) fn workspace_manifest_lacks_package_fields(path: &Path, fields: &[String]) -> bool {
    if path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
        return false;
    }
    if !fields
        .iter()
        .all(|field| field == "package" || field == "package.name" || field.starts_with("package."))
    {
        return false;
    }
    let Some(root_value) = parse_structured_file_value(path) else {
        return false;
    };
    lookup_structured_field_value(&root_value, "workspace").is_some()
        && lookup_structured_field_value(&root_value, "package").is_none()
}

pub(super) fn parse_structured_file_value(path: &Path) -> Option<Value> {
    let contents = fs::read_to_string(path).ok()?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" => serde_json::from_str(&contents).ok(),
        "toml" => toml::from_str::<toml::Value>(&contents)
            .ok()
            .and_then(|value| serde_json::to_value(value).ok()),
        _ => serde_json::from_str(&contents).ok().or_else(|| {
            toml::from_str::<toml::Value>(&contents)
                .ok()
                .and_then(|value| serde_json::to_value(value).ok())
        }),
    }
}

pub(super) fn lookup_structured_field_value<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<&'a Value> {
    let mut current = value;
    for seg in field_path.split('.') {
        if seg.is_empty() {
            return None;
        }
        if let Ok(idx) = seg.parse::<usize>() {
            current = current.as_array()?.get(idx)?;
        } else {
            current = current.get(seg)?;
        }
    }
    Some(current)
}

pub(super) fn route_requests_sqlite_table_listing(route: &RouteResult) -> bool {
    route_has_database_capability_action(route, "list_tables")
}

pub(super) fn route_requests_sqlite_schema_version(route: &RouteResult) -> bool {
    route_has_database_capability_action(route, "schema_version")
        || route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref()
            .is_some_and(sqlite_schema_version_field_token)
}

fn sqlite_schema_version_field_token(token: &str) -> bool {
    matches!(
        token.trim().to_ascii_lowercase().as_str(),
        "schema_version" | "sqlite_schema_version" | "database.schema_version"
    )
}

fn route_has_database_capability_action(route: &RouteResult, action: &str) -> bool {
    crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["database", "db", "sqlite"],
        &[action],
    )
}

pub(super) fn sqlite_locator_path_for_route(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let hint = route.output_contract.locator_hint.trim();
    [
        auto_locator_path.map(str::trim),
        (!hint.is_empty()).then_some(hint),
    ]
    .into_iter()
    .flatten()
    .find(|path| {
        let lower = path.to_ascii_lowercase();
        lower.ends_with(".sqlite") || lower.ends_with(".db")
    })
    .map(ToString::to_string)
}

pub(super) fn is_sqlite_database_path(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    lower.ends_with(".sqlite") || lower.ends_with(".db")
}

pub(super) fn action_is_text_read_of_sqlite_path(action: &AgentAction) -> bool {
    let Some(path) = sqlite_locator_path_from_action(action) else {
        return false;
    };
    if !is_sqlite_database_path(&path) {
        return false;
    }
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim().to_ascii_lowercase();
            if matches!(skill.as_str(), "read_file" | "fs_basic") {
                return args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "read_range"
                                | "read_text_range"
                                | "read"
                                | "read_text"
                                | "read_file"
                                | "head"
                        )
                    })
                    .unwrap_or(skill == "read_file");
            }
            skill == "system_basic"
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "read_range" | "read_text_range" | "read" | "read_file"
                        )
                    })
                    .unwrap_or(false)
        }
        AgentAction::CallCapability { capability, args } => {
            let capability = capability.trim().to_ascii_lowercase();
            matches!(
                capability.as_str(),
                "fs_basic.read_text_range"
                    | "fs_basic.read"
                    | "fs_basic.read_file"
                    | "system_basic.read_range"
                    | "system_basic.read_file"
                    | "read_file"
            ) || (capability == "fs_basic"
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "read_range" | "read_text_range" | "read" | "read_file"
                        )
                    })
                    .unwrap_or(false))
        }
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}

pub(super) fn action_should_be_sqlite_table_query(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let skill = skill.trim().to_ascii_lowercase();
            if skill == "db_basic" {
                return false;
            }
            if action_is_text_read_of_sqlite_path(action) {
                return true;
            }
            if skill == "read_file" || skill == "run_cmd" {
                return true;
            }
            skill == "system_basic"
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .map(|action| {
                        matches!(
                            action.trim().to_ascii_lowercase().as_str(),
                            "read_range"
                                | "read"
                                | "read_file"
                                | "run_cmd"
                                | "extract_field"
                                | "extract_fields"
                                | "sqlite_table_names"
                                | "sqlite_tables"
                                | "list_tables"
                        )
                    })
                    .unwrap_or(false)
        }
        AgentAction::CallCapability { .. } => action_is_text_read_of_sqlite_path(action),
        AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => false,
    }
}
