use super::*;

#[cfg(test)]
pub(super) fn directory_entry_groups_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    user_text: &str,
    original_user_text: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryEntryGroups
    {
        return None;
    }
    let path = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&path).is_dir() {
        return None;
    }
    let requested_limit =
        requested_directory_entry_groups_result_limit(route, user_text, original_user_text);
    let bounded = requested_limit.is_some();
    let sort_by = requested_directory_entry_groups_inventory_sort_by(
        route,
        user_text,
        original_user_text,
        bounded,
    );
    let metadata_required = directory_entry_groups_inventory_requires_metadata(route, &sort_by);
    let include_hidden = route
        .output_contract
        .self_extension
        .list_selector
        .include_hidden
        .unwrap_or(false);
    Some(vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": path,
            "names_only": bounded && !metadata_required,
            "max_entries": requested_limit.unwrap_or(1000),
            "sort_by": sort_by,
            "include_hidden": include_hidden,
        }),
    }])
}

#[cfg(test)]
pub(super) fn directory_entry_groups_auto_locator_deterministic_plan_result(
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
    let actions = directory_entry_groups_auto_locator_observation_plan(
        route_result,
        auto_locator_path,
        user_text,
        original_user_text,
    )?;
    let actions = normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        Some(goal),
        auto_locator_path,
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
pub(super) fn requested_directory_entry_groups_result_limit(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
) -> Option<u64> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryEntryGroups
        || route.output_contract.delivery_required
    {
        return None;
    }
    route
        .output_contract
        .self_extension
        .list_selector
        .limit
        .or_else(|| contract_hint_selector_limit(user_text))
        .or_else(|| original_user_text.and_then(contract_hint_selector_limit))
        .or_else(|| contract_hint_selector_limit(&route.route_reason))
}

#[cfg(test)]
pub(super) fn requested_directory_entry_groups_inventory_sort_by(
    route: &RouteResult,
    user_text: &str,
    original_user_text: Option<&str>,
    bounded: bool,
) -> String {
    route
        .output_contract
        .self_extension
        .list_selector
        .sort_by
        .clone()
        .filter(|sort_by| {
            directory_entry_groups_sort_by_has_structural_support(
                route,
                sort_by,
                user_text,
                original_user_text,
            )
        })
        .or_else(|| contract_hint_selector_sort_by(user_text))
        .or_else(|| original_user_text.and_then(contract_hint_selector_sort_by))
        .or_else(|| contract_hint_selector_sort_by(&route.route_reason))
        .unwrap_or_else(|| {
            if bounded {
                "name".to_string()
            } else {
                "mtime_desc".to_string()
            }
        })
}

#[cfg(test)]
pub(super) fn directory_entry_groups_sort_by_has_structural_support(
    route: &RouteResult,
    sort_by: &str,
    user_text: &str,
    original_user_text: Option<&str>,
) -> bool {
    let sort_by = sort_by.trim();
    if !matches!(
        sort_by,
        "size_desc" | "size_asc" | "mtime_desc" | "mtime_asc"
    ) {
        return true;
    }
    if route
        .output_contract
        .self_extension
        .list_selector
        .include_metadata
        .is_some_and(|value| value)
    {
        return true;
    }
    if route
        .output_contract
        .self_extension
        .list_selector
        .sort_by
        .as_deref()
        .is_some_and(|current_turn_sort_by| current_turn_sort_by == sort_by)
    {
        return true;
    }
    [
        Some(user_text),
        original_user_text,
        Some(route.route_reason.as_str()),
    ]
    .into_iter()
    .flatten()
    .filter_map(contract_hint_selector_sort_by)
    .any(|hint| hint == sort_by)
}

#[cfg(test)]
pub(super) fn directory_entry_groups_inventory_requires_metadata(
    route: &RouteResult,
    sort_by: &str,
) -> bool {
    route
        .output_contract
        .self_extension
        .list_selector
        .include_metadata
        .is_some_and(|value| value)
        || matches!(
            sort_by.trim(),
            "size_desc" | "size_asc" | "mtime_desc" | "mtime_asc"
        )
}

#[cfg(test)]
pub(super) fn file_names_auto_locator_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
    original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::FileNames
    {
        return None;
    }
    let path = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&path).is_dir() {
        return None;
    }
    let sort_by = requested_file_names_inventory_sort_by(route, user_text, original_user_text);
    let metadata_required = file_names_inventory_requires_metadata(route, &sort_by);
    let max_entries =
        requested_file_names_result_limit(route, user_text, original_user_text).unwrap_or(1000);
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "list_dir",
            "path": path,
            "names_only": !metadata_required,
            "files_only": true,
            "dirs_only": false,
            "include_hidden": false,
            "max_entries": max_entries,
            "sort_by": sort_by,
        }),
    }];
    let actions = normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        Some(goal),
        auto_locator_path,
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
pub(super) fn directory_tree_auto_locator_observation_plan(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !route_expects_terminal_user_answer(route)
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        )
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::WorkspaceProjectSummary
        )
    {
        return None;
    }
    if crate::evidence_policy::target_locators_for_route(route).len() > 1 {
        return None;
    }
    if directory_purpose_extension_locator(route).is_some() {
        return None;
    }
    let path = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&path).is_dir() {
        return None;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::WorkspaceProjectSummary {
        let dirs_only = directory_has_direct_child_dirs(&path);
        let mut actions = vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "list_dir",
                "path": path.clone(),
                "names_only": false,
                "dirs_only": dirs_only,
                "max_entries": 1000,
                "sort_by": "name",
                "include_hidden": false,
            }),
        }];
        if let Some(readme_path) = workspace_summary_readme_path(&path) {
            actions.push(AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_text_range",
                    "path": readme_path,
                    "mode": "head",
                    "n": 80,
                }),
            });
        }
        let evidence_refs = if actions.len() > 1 {
            vec!["step_1".to_string(), "step_2".to_string()]
        } else {
            vec!["last_output".to_string()]
        };
        actions.extend([
            AgentAction::SynthesizeAnswer { evidence_refs },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ]);
        Some(actions)
    } else {
        let tree_summary = AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({
                "action": "tree_summary",
                "path": path,
                "max_depth": 2,
                "max_children_per_dir": 12,
                "include_hidden": false,
            }),
        };
        Some(vec![tree_summary])
    }
}

#[cfg(test)]
pub(super) fn workspace_summary_readme_path(root: &str) -> Option<String> {
    ["README.md", "README.zh-CN.md", "readme.md"]
        .into_iter()
        .map(|name| Path::new(root).join(name))
        .find(|path| path.is_file())
        .map(|path| path.display().to_string())
}

#[cfg(test)]
pub(super) fn directory_tree_auto_locator_deterministic_plan_result(
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
    if crate::intent::surface_signals::inline_json_transform_request(user_text)
        || original_user_text
            .is_some_and(crate::intent::surface_signals::inline_json_transform_request)
    {
        return None;
    }
    let actions = directory_tree_auto_locator_observation_plan(route_result, auto_locator_path)?;
    let actions = normalize_planned_actions_with_original_and_context(
        state,
        route_result,
        loop_state,
        user_text,
        original_user_text,
        Some(goal),
        auto_locator_path,
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
pub(super) const DIRECTORY_PURPOSE_MAX_TEXT_READS: usize = 24;
#[cfg(test)]
pub(super) const DIRECTORY_PURPOSE_TREE_SUMMARY_TEXT_READ_THRESHOLD: usize = 8;
#[cfg(test)]
pub(super) const DIRECTORY_PURPOSE_EXTENSION_TEXT_READ_LIMIT: usize = 3;
#[cfg(test)]
const DIRECTORY_PURPOSE_EXTENSION_SCAN_DIR_LIMIT: usize = 256;
#[cfg(test)]
const DIRECTORY_PURPOSE_EXTENSION_SCAN_ENTRY_LIMIT: usize = 5000;

#[cfg(test)]
pub(super) fn directory_purpose_text_like_path(path: &Path) -> bool {
    let Some(ext) = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
    else {
        return false;
    };
    matches!(
        ext.as_str(),
        "adoc"
            | "csv"
            | "json"
            | "jsonl"
            | "log"
            | "markdown"
            | "md"
            | "rst"
            | "toml"
            | "txt"
            | "yaml"
            | "yml"
    )
}

#[cfg(test)]
pub(super) fn directory_purpose_direct_text_read_paths(root: &str) -> Vec<String> {
    let root_path = Path::new(root);
    let canonical_root = root_path
        .canonicalize()
        .unwrap_or_else(|_| root_path.to_path_buf());
    let mut candidates = fs::read_dir(root_path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && directory_purpose_text_like_path(path))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        let left_name = left
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let right_name = right
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        left_name.cmp(&right_name)
    });

    let mut selected = Vec::new();
    for candidate in candidates {
        if selected.len() >= DIRECTORY_PURPOSE_MAX_TEXT_READS {
            break;
        }
        let canonical_candidate = candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.clone());
        if !canonical_candidate.starts_with(&canonical_root) || !canonical_candidate.is_file() {
            continue;
        }
        let read_path = canonical_candidate.display().to_string();
        if !selected.iter().any(|existing| existing == &read_path) {
            selected.push(read_path);
        }
    }
    selected
}

#[cfg(test)]
pub(super) fn directory_purpose_extension_text_read_paths(root: &str, ext: &str) -> Vec<String> {
    let normalized_ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if normalized_ext.is_empty() {
        return Vec::new();
    }
    let root_path = Path::new(root);
    let canonical_root = root_path
        .canonicalize()
        .unwrap_or_else(|_| root_path.to_path_buf());
    let mut candidates = Vec::new();
    let mut stack = vec![root_path.to_path_buf()];
    let mut visited_dirs = 0usize;
    let mut seen_entries = 0usize;
    while let Some(dir) = stack.pop() {
        if visited_dirs >= DIRECTORY_PURPOSE_EXTENSION_SCAN_DIR_LIMIT
            || seen_entries >= DIRECTORY_PURPOSE_EXTENSION_SCAN_ENTRY_LIMIT
        {
            break;
        }
        visited_dirs += 1;
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if seen_entries >= DIRECTORY_PURPOSE_EXTENSION_SCAN_ENTRY_LIMIT {
                break;
            }
            seen_entries += 1;
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if file_type.is_file()
                && directory_purpose_text_like_path(&path)
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|value| value.trim().trim_start_matches('.').to_ascii_lowercase())
                    .is_some_and(|value| value == normalized_ext)
            {
                candidates.push(path);
            }
        }
    }
    candidates.sort_by(|left, right| {
        let left_size = fs::metadata(left).map(|meta| meta.len()).unwrap_or(0);
        let right_size = fs::metadata(right).map(|meta| meta.len()).unwrap_or(0);
        let left_name = left
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let right_name = right
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        right_size
            .cmp(&left_size)
            .then_with(|| left_name.cmp(&right_name))
    });

    let mut selected = Vec::new();
    for candidate in candidates {
        if selected.len() >= DIRECTORY_PURPOSE_EXTENSION_TEXT_READ_LIMIT {
            break;
        }
        let canonical_candidate = candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.clone());
        if !canonical_candidate.starts_with(&canonical_root) || !canonical_candidate.is_file() {
            continue;
        }
        let read_path = canonical_candidate.display().to_string();
        if !selected.iter().any(|existing| existing == &read_path) {
            selected.push(read_path);
        }
    }
    selected
}

#[cfg(test)]
pub(super) fn directory_has_direct_child_dirs(root: &str) -> bool {
    fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_type()
                .ok()
                .is_some_and(|file_type| file_type.is_dir())
        })
}

#[cfg(test)]
pub(super) fn selector_target_kind_from_machine_token(
    token: &str,
) -> Option<crate::OutputScalarCountTargetKind> {
    match token.trim() {
        "file" => Some(crate::OutputScalarCountTargetKind::File),
        "dir" => Some(crate::OutputScalarCountTargetKind::Dir),
        "any" => Some(crate::OutputScalarCountTargetKind::Any),
        _ => None,
    }
}

#[cfg(test)]
pub(super) fn directory_purpose_selector_target_kind(
    route: &RouteResult,
) -> crate::OutputScalarCountTargetKind {
    let selector = route
        .output_contract
        .self_extension
        .list_selector
        .target_kind;
    if selector != crate::OutputScalarCountTargetKind::Any {
        return selector;
    }
    [route.route_reason.as_str()]
        .into_iter()
        .filter_map(contract_hint_selector_target_kind)
        .find_map(|token| selector_target_kind_from_machine_token(&token))
        .unwrap_or(crate::OutputScalarCountTargetKind::Any)
}

#[cfg(test)]
pub(super) fn directory_purpose_selector_limit(route: &RouteResult) -> Option<u64> {
    route
        .output_contract
        .self_extension
        .list_selector
        .limit
        .or_else(|| contract_hint_selector_limit(&route.route_reason))
}

#[cfg(test)]
pub(super) fn directory_purpose_selector_sort_by(route: &RouteResult) -> Option<String> {
    route
        .output_contract
        .self_extension
        .list_selector
        .sort_by
        .clone()
        .or_else(|| contract_hint_selector_sort_by(&route.route_reason))
}

#[cfg(test)]
pub(super) fn apply_directory_purpose_selector_inventory_args(
    route: &RouteResult,
    obj: &mut serde_json::Map<String, Value>,
) {
    match directory_purpose_selector_target_kind(route) {
        crate::OutputScalarCountTargetKind::File => {
            obj.insert("files_only".to_string(), Value::Bool(true));
            obj.insert("dirs_only".to_string(), Value::Bool(false));
        }
        crate::OutputScalarCountTargetKind::Dir => {
            obj.insert("dirs_only".to_string(), Value::Bool(true));
            obj.insert("files_only".to_string(), Value::Bool(false));
        }
        crate::OutputScalarCountTargetKind::Any => {}
    }
    if let Some(limit) = directory_purpose_selector_limit(route) {
        obj.insert(
            "max_entries".to_string(),
            Value::Number(serde_json::Number::from(limit)),
        );
    }
    if let Some(sort_by) = directory_purpose_selector_sort_by(route) {
        obj.insert("sort_by".to_string(), Value::String(sort_by));
    }
}

#[cfg(test)]
pub(super) fn directory_purpose_auto_locator_deterministic_plan_result(
    _state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    _user_text: &str,
    _original_user_text: Option<&str>,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryPurposeSummary
        || !route_expects_terminal_user_answer(route)
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        )
        || directory_purpose_extension_locator(route).is_some()
    {
        return None;
    }
    if crate::evidence_policy::target_locators_for_route(route).len() > 1 {
        return None;
    }
    let path = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&path).is_dir() {
        return None;
    }
    let read_paths = directory_purpose_direct_text_read_paths(&path);
    if read_paths.len() > DIRECTORY_PURPOSE_TREE_SUMMARY_TEXT_READ_THRESHOLD {
        let dirs_only = directory_has_direct_child_dirs(&path);
        let mut list_args = serde_json::json!({
            "action": "list_dir",
            "path": path,
            "names_only": false,
            "dirs_only": dirs_only,
            "max_entries": 1000,
            "sort_by": "name",
            "include_hidden": false,
        });
        if let Some(obj) = list_args.as_object_mut() {
            apply_directory_purpose_selector_inventory_args(route, obj);
        }
        let actions = vec![
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: list_args,
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
        return Some(build_plan_result(
            goal,
            &raw_plan_text,
            PlanKind::Single,
            &actions,
        ));
    }

    let mut list_args = serde_json::json!({
        "action": "list_dir",
        "path": path,
        "names_only": false,
        "max_entries": 1000,
        "sort_by": "name",
    });
    if let Some(obj) = list_args.as_object_mut() {
        apply_directory_purpose_selector_inventory_args(route, obj);
    }
    let mut actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: list_args,
    }];
    actions.extend(read_paths.into_iter().map(|path| AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "read_text_range",
            "path": path,
            "mode": "head",
            "n": 40,
        }),
    }));
    let evidence_refs = (1..=actions.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    actions.push(AgentAction::SynthesizeAnswer { evidence_refs });
    actions.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
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
pub(super) fn directory_purpose_extension_locator(route: &RouteResult) -> Option<String> {
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::DirectoryPurposeSummary
        || route_requests_extension_assess_gap(route)
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace | crate::OutputLocatorKind::Path
        )
    {
        return None;
    }
    extension_from_globish_pattern(route.output_contract.locator_hint.trim())
        .or_else(|| structural_extension_filter_from_text(&route.resolved_intent))
}

#[cfg(test)]
fn route_requests_extension_assess_gap(route: &RouteResult) -> bool {
    route_has_machine_token(route, "extension.assess_gap")
        || (route_has_machine_token(route, "extension_manager")
            && route_has_machine_token(route, "assess_gap"))
}

#[cfg(test)]
fn route_has_machine_token(route: &RouteResult, token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    [route.resolved_intent.as_str(), route.route_reason.as_str()]
        .into_iter()
        .any(|text| machine_token_present(text, token))
}

#[cfg(test)]
fn machine_token_present(text: &str, token: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
        .any(|part| part == token || part.starts_with(&format!("{token}.")))
}

#[cfg(test)]
pub(super) fn directory_purpose_extension_inventory_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let route = route_result?;
    let ext = directory_purpose_extension_locator(route)?;
    let root = route_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&root).is_dir() {
        return None;
    }
    let mut actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "find_entries",
            "root": root,
            "ext": ext.clone(),
            "target_kind": "file",
            "max_results": 1000,
            "recursive": true,
            "sort_by": "size_desc",
        }),
    }];
    actions.extend(
        directory_purpose_extension_text_read_paths(&root, &ext)
            .into_iter()
            .map(|path| AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_text_range",
                    "path": path,
                    "mode": "head",
                    "n": 80,
                }),
            }),
    );
    let evidence_refs = (1..=actions.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    actions.push(AgentAction::SynthesizeAnswer { evidence_refs });
    actions.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn step_output_action(value: &Value) -> Option<String> {
    let payload = value
        .get("extra")
        .filter(|extra| extra.is_object())
        .unwrap_or(value);
    payload
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| !action.is_empty())
        .map(|action| action.to_ascii_lowercase())
}

pub(super) fn executed_step_is_successful_text_read(
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
    if !(step.skill.eq_ignore_ascii_case("fs_basic")
        || step.skill.eq_ignore_ascii_case("system_basic"))
    {
        return false;
    }
    step.output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
        .and_then(|value| step_output_action(&value))
        .is_some_and(|action| action == "read_text_range" || action == "read_range")
}

#[cfg(test)]
pub(super) fn executed_find_entries_candidate_paths(
    step: &crate::executor::StepExecutionResult,
) -> Vec<String> {
    if !step.is_ok()
        || !(step.skill.eq_ignore_ascii_case("fs_basic")
            || step.skill.eq_ignore_ascii_case("fs_search"))
    {
        return Vec::new();
    }
    let Some(value) = step
        .output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
    else {
        return Vec::new();
    };
    let Some(action) = step_output_action(&value) else {
        return Vec::new();
    };
    if !matches!(action.as_str(), "find_entries" | "find_ext" | "find_name") {
        return Vec::new();
    }
    let payload = value
        .get("extra")
        .filter(|extra| extra.is_object())
        .unwrap_or(&value);
    payload
        .get("results")
        .or_else(|| payload.get("candidates"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
pub(super) fn safe_representative_find_result_paths(
    root: &str,
    candidates: Vec<String>,
) -> Vec<String> {
    let root_path = Path::new(root);
    let canonical_root = root_path
        .canonicalize()
        .unwrap_or_else(|_| root_path.to_path_buf());
    let mut selected = Vec::new();
    for candidate in candidates {
        if selected.len() >= 3 {
            break;
        }
        if candidate.contains('\0') {
            continue;
        }
        let raw = Path::new(&candidate);
        if raw.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        }) {
            continue;
        }
        let full_path = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            root_path.join(raw)
        };
        let canonical_candidate = full_path
            .canonicalize()
            .unwrap_or_else(|_| full_path.clone());
        if !canonical_candidate.starts_with(&canonical_root) || !canonical_candidate.is_file() {
            continue;
        }
        let read_path = canonical_candidate.display().to_string();
        if !selected.iter().any(|existing| existing == &read_path) {
            selected.push(read_path);
        }
    }
    selected
}

#[cfg(test)]
pub(super) fn directory_purpose_representative_reads_after_find_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if !loop_state.has_tool_or_skill_output
        || directory_purpose_extension_locator(route).is_none()
        || loop_state
            .executed_step_results
            .iter()
            .any(executed_step_is_successful_text_read)
    {
        return None;
    }
    let root = route_directory_locator_path(route, auto_locator_path)?;
    let candidates = loop_state
        .executed_step_results
        .iter()
        .rev()
        .flat_map(executed_find_entries_candidate_paths)
        .collect::<Vec<_>>();
    let selected = safe_representative_find_result_paths(&root, candidates);
    if selected.is_empty() {
        return None;
    }
    let mut actions = selected
        .into_iter()
        .map(|path| AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": path,
                "mode": "head",
                "n": 60,
            }),
        })
        .collect::<Vec<_>>();
    let evidence_refs = (1..=actions.len())
        .map(|idx| format!("step_{idx}"))
        .collect::<Vec<_>>();
    actions.push(AgentAction::SynthesizeAnswer { evidence_refs });
    actions.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
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
pub(super) fn directory_compare_locator_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return None;
    }
    let targets = crate::evidence_policy::target_locators_for_route(route);
    if targets.len() != 2 {
        return None;
    }
    let left =
        resolve_directory_locator_for_dir_compare(&state.skill_rt.workspace_root, &targets[0])?;
    let right =
        resolve_directory_locator_for_dir_compare(&state.skill_rt.workspace_root, &targets[1])?;
    if left.eq_ignore_ascii_case(&right) {
        return None;
    }
    let actions = vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "dir_compare",
            "left_path": left,
            "right_path": right,
            "recursive": true,
            "include_hidden": false,
            "max_diffs": 20,
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

#[cfg(test)]
pub(super) fn quantity_compare_pair_locator_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    original_user_text: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::QuantityComparison
    {
        return None;
    }
    let mut targets = crate::evidence_policy::target_locators_for_route(route);
    if targets.len() != 2 {
        if let Some(text_targets) = original_user_text
            .and_then(|text| explicit_existing_metadata_locator_pair_from_text(state, text))
        {
            targets = text_targets;
        }
    }
    if targets.len() != 2 {
        return None;
    }
    let left = resolve_existing_metadata_locator_path(&state.skill_rt.workspace_root, &targets[0])?;
    let right =
        resolve_existing_metadata_locator_path(&state.skill_rt.workspace_root, &targets[1])?;
    if left.eq_ignore_ascii_case(&right) {
        return None;
    }
    let left_is_dir = Path::new(&left).is_dir();
    let right_is_dir = Path::new(&right).is_dir();
    if left_is_dir && right_is_dir {
        let actions = vec![
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "count_entries",
                    "path": left,
                    "recursive": false,
                    "include_hidden": false,
                }),
            },
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "count_entries",
                    "path": right,
                    "recursive": false,
                    "include_hidden": false,
                }),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];
        let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
            .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
        return Some(build_plan_result(
            goal,
            &raw_plan_text,
            PlanKind::Single,
            &actions,
        ));
    }
    if left_is_dir || right_is_dir {
        return None;
    }
    let action = AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "compare_paths",
            "left_path": left,
            "right_path": right,
        }),
    };
    let (skill, args) = match &action {
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    if !crate::evidence_policy::capability_ref_action_policy_for_route(Some(route), skill, args)
        .is_some_and(|policy| policy.is_allowed())
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

#[cfg(test)]
pub(super) fn explicit_existing_metadata_locator_pair_from_text(
    state: &AppState,
    text: &str,
) -> Option<Vec<String>> {
    let mut raw_targets = Vec::new();
    let surface = crate::intent::surface_signals::analyze_prompt_surface(text);
    if let Some((left, right)) = surface.locator_target_pair.as_ref() {
        push_unique_metadata_locator_candidate(&mut raw_targets, left);
        push_unique_metadata_locator_candidate(&mut raw_targets, right);
    } else {
        for locator in
            crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        {
            if matches!(locator.locator_kind, crate::OutputLocatorKind::Path) {
                push_unique_metadata_locator_candidate(&mut raw_targets, &locator.locator_hint);
            }
        }
    }
    let mut resolved = Vec::new();
    for raw in raw_targets {
        let Some(path) =
            resolve_existing_metadata_locator_path(&state.skill_rt.workspace_root, &raw)
        else {
            continue;
        };
        if !resolved
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&path))
        {
            resolved.push(path);
        }
        if resolved.len() > 2 {
            return None;
        }
    }
    (resolved.len() == 2).then_some(resolved)
}

#[cfg(test)]
pub(super) fn push_unique_metadata_locator_candidate(out: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if !out
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(value))
    {
        out.push(value.to_string());
    }
}
