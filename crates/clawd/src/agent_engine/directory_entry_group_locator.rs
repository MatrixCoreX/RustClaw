use super::*;

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
pub(super) const DIRECTORY_PURPOSE_MAX_TEXT_READS: usize = 24;
#[cfg(test)]
pub(super) const DIRECTORY_PURPOSE_TREE_SUMMARY_TEXT_READ_THRESHOLD: usize = 8;

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
