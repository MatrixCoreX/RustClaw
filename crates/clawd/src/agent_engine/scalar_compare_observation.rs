use super::*;

pub(super) fn executed_step_scalar_compare_observation_units(
    step: &crate::executor::StepExecutionResult,
) -> usize {
    if step.is_ok() && step.skill.eq_ignore_ascii_case("git_basic") {
        return 1;
    }
    let is_scalar_compare_step_skill = step.skill.eq_ignore_ascii_case("system_basic")
        || step.skill.eq_ignore_ascii_case("fs_basic")
        || step.skill.eq_ignore_ascii_case("config_basic");
    if !step.is_ok() || !is_scalar_compare_step_skill {
        return 0;
    }
    let Some(value) = step
        .output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
        .map(step_output_machine_payload)
    else {
        return 0;
    };
    match value
        .get("action")
        .and_then(Value::as_str)
        .map(|action| action.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("extract_field") => 1,
        Some("extract_fields") => value
            .get("field_paths")
            .and_then(Value::as_array)
            .map(|field_paths| field_paths.len())
            .or_else(|| value.get("fields").and_then(Value::as_array).map(Vec::len))
            .unwrap_or(1),
        Some("count_inventory") | Some("inventory_dir") | Some("count_entries") => value
            .get("path")
            .or_else(|| value.get("resolved_path"))
            .and_then(Value::as_str)
            .is_some_and(|path| !path.trim().is_empty())
            as usize,
        Some("compare_paths") => value
            .get("paths")
            .and_then(Value::as_array)
            .map(|paths| paths.len().min(2))
            .unwrap_or(2),
        Some("path_batch_facts") => value
            .get("facts")
            .and_then(Value::as_array)
            .map(|facts| facts.len().min(2))
            .unwrap_or(0),
        Some("read_field") if step.skill.eq_ignore_ascii_case("config_basic") => 1,
        Some("read_fields") if step.skill.eq_ignore_ascii_case("config_basic") => value
            .get("field_paths")
            .and_then(Value::as_array)
            .map(|field_paths| field_paths.len())
            .or_else(|| value.get("fields").and_then(Value::as_array).map(Vec::len))
            .unwrap_or(1),
        _ => 0,
    }
}

pub(super) fn executed_structured_scalar_observation_units(loop_state: &LoopState) -> usize {
    loop_state
        .executed_step_results
        .iter()
        .map(executed_step_scalar_compare_observation_units)
        .sum()
}

pub(super) fn structured_scalar_plus_text_evidence(actions: &[AgentAction]) -> bool {
    structured_scalar_observation_units(actions) >= 1
        && actions.iter().any(action_reads_workspace_text_content)
}

pub(super) fn structured_scalar_compare_missing_required_extracts_for_round(
    route: &RouteResult,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if !route_requests_structured_scalar_compare(route)
        || !has_executable_observation_or_action(actions)
    {
        return false;
    }
    let scalar_units = structured_scalar_observation_units(actions)
        + executed_structured_scalar_observation_units(loop_state);
    let has_text_evidence = has_workspace_text_content_evidence(loop_state, actions);
    if scalar_units >= 1 && has_text_evidence {
        return false;
    }
    if scalar_units == 1 && actions_satisfy_single_scalar_count(route, actions) {
        return false;
    }
    if scalar_units == 1 && actions_satisfy_single_path_metadata_facts(route, actions) {
        return false;
    }
    if scalar_units == 1
        && actions_satisfy_current_workspace_scalar_field_observation(route, actions)
    {
        return false;
    }
    scalar_units < 2
}

pub(super) fn append_synthesize_answer_for_structured_scalar_compare(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_requests_structured_scalar_compare(route)
        || actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. }
            )
        })
    {
        return actions;
    }
    let scalar_units = structured_scalar_observation_units(&actions);
    let has_scalar_text_evidence = structured_scalar_plus_text_evidence(&actions);
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::RecentScalarEqualityCheck
        && scalar_units >= 2
        && !has_scalar_text_evidence
    {
        return actions;
    }
    if scalar_units < 2 && !has_scalar_text_evidence {
        return actions;
    }
    let evidence_refs = actions
        .iter()
        .enumerate()
        .filter_map(|(idx, action)| {
            (action_scalar_compare_observation_units(action) > 0
                || (has_scalar_text_evidence && action_reads_workspace_text_content(action)))
            .then(|| format!("step_{}", idx + 1))
        })
        .collect::<Vec<_>>();
    let mut rewritten = actions;
    rewritten.push(AgentAction::SynthesizeAnswer {
        evidence_refs: evidence_refs.clone(),
    });
    info!(
        "plan_append_synthesize_answer_for_structured_scalar_compare refs={}",
        evidence_refs.join(",")
    );
    rewritten
}

pub(super) fn fs_basic_stat_paths_action_for_explicit_targets(targets: &[String]) -> AgentAction {
    AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": targets,
            "include_missing": true,
            "fields": ["exists", "kind", "size", "modified"],
        }),
    }
}

pub(super) fn rewrite_split_dir_basename_stat_paths_to_auto_locator_file(
    state: &AppState,
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ExistenceWithPath
                | crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::FilePaths
        )
        || actions.len() != 1
    {
        return actions;
    }
    let Some(auto_locator_path) = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    else {
        return actions;
    };
    let auto_locator = Path::new(auto_locator_path);
    if !auto_locator.is_file() {
        return actions;
    }
    let Some(auto_parent) = auto_locator.parent() else {
        return actions;
    };
    let Some(auto_name) = auto_locator
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return actions;
    };

    let (call_name, args, call_kind) = match &actions[0] {
        AgentAction::CallTool { tool, args } => (tool.as_str(), args, PlannedCallKind::Tool),
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args, PlannedCallKind::Skill),
        _ => return actions,
    };
    if !call_name.eq_ignore_ascii_case("fs_basic") {
        return actions;
    }
    let Some(obj) = args.as_object() else {
        return actions;
    };
    if obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_none_or(|action| !action.eq_ignore_ascii_case("stat_paths"))
    {
        return actions;
    }
    let paths = string_list_from_value(obj.get("paths"))
        .into_iter()
        .chain(string_list_from_value(obj.get("targets")))
        .chain(string_list_from_value(obj.get("path")))
        .collect::<Vec<_>>();
    if paths.len() != 2 {
        return actions;
    }

    let mut has_parent = false;
    let mut has_basename = false;
    for path in &paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        let resolved = resolve_workspace_path(&state.skill_rt.workspace_root, trimmed);
        if same_existing_or_display_path(&resolved, auto_parent) {
            has_parent = true;
            continue;
        }
        if !trimmed.contains('/')
            && !trimmed.contains('\\')
            && trimmed.eq_ignore_ascii_case(auto_name)
        {
            has_basename = true;
            continue;
        }
        if Path::new(trimmed)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(auto_name))
        {
            has_basename = true;
        }
    }
    if !has_parent || !has_basename {
        return actions;
    }

    let mut rewritten = obj.clone();
    rewritten.insert(
        "paths".to_string(),
        Value::Array(vec![Value::String(auto_locator_path.to_string())]),
    );
    rewritten.insert("include_missing".to_string(), Value::Bool(true));
    rewritten.remove("path");
    rewritten.remove("targets");
    let args = Value::Object(rewritten);
    info!(
        "plan_rewrite_split_dir_basename_stat_paths_to_auto_locator_file path={}",
        crate::truncate_for_log(auto_locator_path)
    );
    match call_kind {
        PlannedCallKind::Tool => vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args,
        }],
        PlannedCallKind::Skill => vec![AgentAction::CallSkill {
            skill: "fs_basic".to_string(),
            args,
        }],
    }
}

pub(super) fn rewrite_constructed_missing_stat_path_to_exact_find_entries(
    state: &AppState,
    route_result: Option<&RouteResult>,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_allows_constructed_stat_path_search_repair(route) || actions.len() != 1 {
        return actions;
    }
    let Some((path, call_kind)) = single_fs_basic_stat_path_candidate(&actions[0]) else {
        return actions;
    };
    if let Some((root, pattern, ext)) = file_paths_missing_stat_path_selector_search_repair(
        &state.skill_rt.workspace_root,
        route,
        &path,
        user_text,
    ) {
        info!(
            "plan_rewrite_file_paths_missing_stat_path_to_find_entries root={} pattern={}",
            crate::truncate_for_log(&root),
            crate::truncate_for_log(&pattern)
        );
        let mut args = serde_json::json!({
            "action": "find_entries",
            "root": root,
            "pattern": pattern,
            "target_kind": "file",
            "recursive": true,
        });
        if let (Some(ext), Some(obj)) = (ext, args.as_object_mut()) {
            obj.insert("ext".to_string(), Value::String(ext));
        }
        return match call_kind {
            PlannedCallKind::Tool => vec![AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args,
            }],
            PlannedCallKind::Skill => vec![AgentAction::CallSkill {
                skill: "fs_basic".to_string(),
                args,
            }],
        };
    }
    let (root, basename, exact) = if let Some((root, basename)) =
        constructed_missing_stat_path_search_repair(
            &state.skill_rt.workspace_root,
            &path,
            user_text,
        ) {
        (root, basename, true)
    } else if let Some((root, pattern)) = constructed_directory_stat_path_search_repair(
        &state.skill_rt.workspace_root,
        &path,
        user_text,
    ) {
        (root, pattern, false)
    } else {
        return actions;
    };
    info!(
        "plan_rewrite_constructed_missing_stat_path_to_find_entries root={} basename={}",
        crate::truncate_for_log(&root),
        crate::truncate_for_log(&basename)
    );
    let args = serde_json::json!({
        "action": "find_entries",
        "root": root,
        "name_pattern": basename,
        "target_kind": "file",
        "exact": exact,
    });
    match call_kind {
        PlannedCallKind::Tool => vec![AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args,
        }],
        PlannedCallKind::Skill => vec![AgentAction::CallSkill {
            skill: "fs_basic".to_string(),
            args,
        }],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlannedCallKind {
    Tool,
    Skill,
}

pub(super) fn route_allows_constructed_stat_path_search_repair(route: &RouteResult) -> bool {
    !route.needs_clarify
        && !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
}

pub(super) fn file_paths_missing_stat_path_selector_search_repair(
    workspace_root: &Path,
    route: &RouteResult,
    raw_path: &str,
    user_text: &str,
) -> Option<(String, String, Option<String>)> {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::FilePaths {
        return None;
    }
    let raw_path = raw_path.trim();
    if raw_path.is_empty() || raw_path.contains(['*', '?', '[', ']']) {
        return None;
    }
    let candidate = resolve_workspace_path(workspace_root, raw_path);
    if candidate.exists() {
        return None;
    }
    let parent = candidate.parent()?.to_path_buf();
    if !parent.is_dir()
        || !file_paths_missing_stat_parent_is_anchored(
            workspace_root,
            route,
            raw_path,
            &parent,
            user_text,
        )
    {
        return None;
    }
    let ext = Path::new(raw_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_ascii_lowercase());
    let pattern = directory_child_structural_selector_for_file_paths(
        workspace_root,
        raw_path,
        &parent,
        user_text,
        ext.as_deref(),
    )?;
    let root = Path::new(raw_path)
        .parent()
        .and_then(|parent| parent.to_str())
        .map(str::trim)
        .filter(|parent| !parent.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            parent
                .strip_prefix(workspace_root)
                .ok()
                .and_then(|relative| relative.to_str())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| parent.display().to_string());
    Some((root, pattern, ext))
}

pub(super) fn file_paths_missing_stat_parent_is_anchored(
    workspace_root: &Path,
    route: &RouteResult,
    raw_path: &str,
    parent: &Path,
    user_text: &str,
) -> bool {
    let raw_parent = Path::new(raw_path)
        .parent()
        .and_then(|parent| parent.to_str())
        .map(str::trim)
        .filter(|parent| !parent.is_empty());
    if raw_parent.is_some_and(|parent| structural_token_present(user_text, parent))
        || path_text_variants(workspace_root, raw_parent.unwrap_or_default(), parent)
            .iter()
            .any(|variant| structural_token_present(user_text, variant))
    {
        return true;
    }
    let locator_hint = route.output_contract.locator_hint.trim();
    if locator_hint.is_empty() {
        return false;
    }
    let locator = resolve_workspace_path(workspace_root, locator_hint);
    same_existing_or_display_path(&locator, parent)
        || locator
            .parent()
            .is_some_and(|locator_parent| same_existing_or_display_path(locator_parent, parent))
}

pub(super) fn single_fs_basic_stat_path_candidate(
    action: &AgentAction,
) -> Option<(String, PlannedCallKind)> {
    let (name, args, call_kind) = match action {
        AgentAction::CallTool { tool, args } => (tool.as_str(), args, PlannedCallKind::Tool),
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args, PlannedCallKind::Skill),
        _ => return None,
    };
    if !name.eq_ignore_ascii_case("fs_basic") {
        return None;
    }
    let obj = args.as_object()?;
    if obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_none_or(|action| !action.eq_ignore_ascii_case("stat_paths"))
    {
        return None;
    }
    let paths = string_list_from_value(obj.get("paths"))
        .into_iter()
        .chain(string_list_from_value(obj.get("targets")))
        .chain(string_list_from_value(obj.get("path")))
        .collect::<Vec<_>>();
    (paths.len() == 1).then(|| (paths[0].clone(), call_kind))
}

pub(super) fn constructed_missing_stat_path_search_repair(
    workspace_root: &Path,
    raw_path: &str,
    user_text: &str,
) -> Option<(String, String)> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() || raw_path.contains(['*', '?', '[', ']']) {
        return None;
    }
    let candidate = resolve_workspace_path(workspace_root, raw_path);
    if candidate.exists() {
        return None;
    }
    if path_text_variants(workspace_root, raw_path, &candidate)
        .iter()
        .any(|variant| structural_token_present(user_text, variant))
    {
        return None;
    }
    let basename = candidate
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| concrete_file_basename_selector(name))?;
    if !structural_token_present(user_text, basename) {
        return None;
    }
    let parent = candidate.parent()?.to_path_buf();
    if !parent.is_dir() {
        return None;
    }
    let raw_parent = Path::new(raw_path)
        .parent()
        .and_then(|parent| parent.to_str());
    let parent_anchored = raw_parent
        .filter(|parent| !parent.trim().is_empty())
        .is_some_and(|parent| structural_token_present(user_text, parent))
        || path_text_variants(workspace_root, raw_parent.unwrap_or_default(), &parent)
            .iter()
            .any(|variant| structural_token_present(user_text, variant));
    if !parent_anchored {
        return None;
    }
    let root = raw_parent
        .map(str::trim)
        .filter(|parent| !parent.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            parent
                .strip_prefix(workspace_root)
                .ok()
                .and_then(|relative| relative.to_str())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| parent.display().to_string());
    Some((root, basename.to_string()))
}

pub(super) fn constructed_directory_stat_path_search_repair(
    workspace_root: &Path,
    raw_path: &str,
    user_text: &str,
) -> Option<(String, String)> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() || raw_path.contains(['*', '?', '[', ']']) {
        return None;
    }
    let candidate = resolve_workspace_path(workspace_root, raw_path);
    if !candidate.is_dir() {
        return None;
    }
    let directory_anchored = structural_token_present(user_text, raw_path)
        || path_text_variants(workspace_root, raw_path, &candidate)
            .iter()
            .any(|variant| structural_token_present(user_text, variant));
    if !directory_anchored {
        return None;
    }
    let pattern =
        directory_child_name_pattern_selector(workspace_root, raw_path, &candidate, user_text)?;
    Some((raw_path.to_string(), pattern))
}

pub(super) fn directory_child_name_pattern_selector(
    workspace_root: &Path,
    raw_path: &str,
    directory: &Path,
    user_text: &str,
) -> Option<String> {
    let mut path_tokens = structural_selector_tokens(raw_path);
    for component in path_text_variants(workspace_root, raw_path, directory) {
        path_tokens.extend(structural_selector_tokens(&component));
    }
    let mut scores: HashMap<String, usize> = HashMap::new();
    let entries = fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name
            .to_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        for token in structural_selector_tokens(name) {
            if token.len() < 2 || path_tokens.contains(&token) {
                continue;
            }
            if structural_token_present(user_text, &token) {
                *scores.entry(token).or_insert(0) += 1;
            }
        }
    }
    scores
        .into_iter()
        .max_by(|(left_token, left_count), (right_token, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| left_token.len().cmp(&right_token.len()))
                .then_with(|| right_token.cmp(left_token))
        })
        .map(|(token, _)| token)
}

pub(super) fn directory_child_structural_selector_for_file_paths(
    workspace_root: &Path,
    raw_path: &str,
    directory: &Path,
    user_text: &str,
    ext_filter: Option<&str>,
) -> Option<String> {
    let mut path_tokens = structural_selector_tokens(raw_path);
    for component in path_text_variants(workspace_root, raw_path, directory) {
        path_tokens.extend(structural_selector_tokens(&component));
    }
    if let Some(ext) = ext_filter {
        path_tokens.insert(ext.to_ascii_lowercase());
    }

    let mut scores: HashMap<String, usize> = HashMap::new();
    let entries = fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(ext_filter) = ext_filter {
            let entry_ext = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(str::trim)
                .unwrap_or_default();
            if !entry_ext.eq_ignore_ascii_case(ext_filter) {
                continue;
            }
        }
        let Some(name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        for token in structural_selector_candidates(name) {
            if token.len() < 2 || path_tokens.contains(&token) {
                continue;
            }
            if structural_token_present(user_text, &token) {
                *scores.entry(token).or_insert(0) += 1;
            }
        }
    }
    scores
        .into_iter()
        .max_by(|(left_token, left_count), (right_token, right_count)| {
            left_token
                .len()
                .cmp(&right_token.len())
                .then_with(|| left_count.cmp(right_count))
                .then_with(|| right_token.cmp(left_token))
        })
        .map(|(token, _)| token)
}

pub(super) fn structural_selector_tokens(text: &str) -> HashSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 2)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

pub(super) fn structural_selector_candidates(text: &str) -> HashSet<String> {
    let mut candidates = structural_selector_tokens(text);
    let words = text
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 2)
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    for window in 2..=words.len().min(4) {
        for slice in words.windows(window) {
            candidates.insert(slice.join("_"));
            candidates.insert(slice.join("-"));
        }
    }
    candidates
}

pub(super) fn path_text_variants(
    workspace_root: &Path,
    raw_path: &str,
    resolved: &Path,
) -> Vec<String> {
    let mut variants = Vec::new();
    push_unique_path_text_variant(&mut variants, raw_path);
    push_unique_path_text_variant(&mut variants, &raw_path.replace('\\', "/"));
    push_unique_path_text_variant(&mut variants, &resolved.display().to_string());
    if let Ok(relative) = resolved.strip_prefix(workspace_root) {
        if let Some(relative) = relative.to_str() {
            push_unique_path_text_variant(&mut variants, relative);
            push_unique_path_text_variant(&mut variants, &relative.replace('\\', "/"));
        }
    }
    variants
}

pub(super) fn push_unique_path_text_variant(out: &mut Vec<String>, value: &str) {
    let trimmed = value.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || out.iter().any(|existing| existing == trimmed) {
        return;
    }
    out.push(trimmed.to_string());
}

pub(super) fn concrete_file_basename_selector(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && !trimmed.contains('/')
        && !trimmed.contains('\\')
        && !trimmed.contains(['*', '?', '[', ']', '(', ')', '{', '}', '|'])
        && Path::new(trimmed)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::trim)
            .is_some_and(|ext| !ext.is_empty())
}

pub(super) fn planned_find_entries_directory_name(
    action: &AgentAction,
) -> Option<(String, String)> {
    let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
        action
    else {
        return None;
    };
    if !skill.eq_ignore_ascii_case("fs_basic") {
        return None;
    }
    let obj = args.as_object()?;
    if obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| action.eq_ignore_ascii_case("find_entries"))?
        .is_empty()
    {
        return None;
    }
    if obj
        .get("ext")
        .or_else(|| obj.get("extension"))
        .is_some_and(has_non_empty_json_value)
    {
        return None;
    }
    let pattern = obj
        .get("pattern")
        .or_else(|| obj.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if pattern.contains(['*', '?', '[', ']']) || pattern.contains('/') || pattern.contains('\\') {
        return None;
    }
    let root = obj
        .get("root")
        .or_else(|| obj.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(".");
    Some((root.to_string(), pattern.to_string()))
}

pub(super) fn rewrite_dir_compare_paths_to_unique_workspace_directories(
    state: &AppState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let max_visits = directory_name_locator_scan_limit(state);
    actions
        .into_iter()
        .map(|action| {
            let action = rewrite_dir_compare_action_paths(
                &state.skill_rt.workspace_root,
                max_visits,
                action,
            );
            rewrite_compare_paths_action_to_system_dir_compare(action)
        })
        .collect()
}

pub(super) fn rewrite_dir_compare_action_paths(
    workspace_root: &Path,
    max_visits: usize,
    action: AgentAction,
) -> AgentAction {
    match action {
        AgentAction::CallSkill { skill, args } => {
            if let Some(args) =
                rewrite_dir_compare_args(workspace_root, max_visits, &skill, args.clone())
            {
                AgentAction::CallSkill { skill, args }
            } else {
                AgentAction::CallSkill { skill, args }
            }
        }
        AgentAction::CallTool { tool, args } => {
            if let Some(args) =
                rewrite_dir_compare_args(workspace_root, max_visits, &tool, args.clone())
            {
                AgentAction::CallTool { tool, args }
            } else {
                AgentAction::CallTool { tool, args }
            }
        }
        other => other,
    }
}

pub(super) fn rewrite_dir_compare_args(
    workspace_root: &Path,
    max_visits: usize,
    skill: &str,
    args: Value,
) -> Option<Value> {
    if !skill.eq_ignore_ascii_case("system_basic") && !skill.eq_ignore_ascii_case("fs_basic") {
        return None;
    }
    let obj = args.as_object()?;
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| {
            action.eq_ignore_ascii_case("dir_compare")
                || action.eq_ignore_ascii_case("compare_paths")
        })?;
    let (left_raw, right_raw) = if action.eq_ignore_ascii_case("compare_paths") {
        let paths = obj.get("paths").and_then(Value::as_array)?;
        if paths.len() != 2 {
            return None;
        }
        (paths[0].as_str()?, paths[1].as_str()?)
    } else {
        (
            obj.get("left_path")
                .or_else(|| obj.get("left"))
                .and_then(Value::as_str)?,
            obj.get("right_path")
                .or_else(|| obj.get("right"))
                .and_then(Value::as_str)?,
        )
    };
    let left = resolve_dir_compare_path_or_unique_name(workspace_root, left_raw, max_visits)?;
    let right = resolve_dir_compare_path_or_unique_name(workspace_root, right_raw, max_visits)?;
    if left.eq_ignore_ascii_case(left_raw.trim()) && right.eq_ignore_ascii_case(right_raw.trim()) {
        return None;
    }
    let mut rewritten = obj.clone();
    rewritten.insert(
        "action".to_string(),
        Value::String("dir_compare".to_string()),
    );
    rewritten.insert("left_path".to_string(), Value::String(left.clone()));
    rewritten.insert("right_path".to_string(), Value::String(right.clone()));
    rewritten.remove("paths");
    info!(
        "plan_rewrite_dir_compare_paths left={} right={}",
        crate::truncate_for_log(&left),
        crate::truncate_for_log(&right)
    );
    Some(Value::Object(rewritten))
}

pub(super) fn resolve_dir_compare_path_or_unique_name(
    workspace_root: &Path,
    raw: &str,
    max_visits: usize,
) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let candidate = Path::new(raw);
    let absolute_candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_root.join(candidate)
    };
    if absolute_candidate.is_dir() {
        return Some(
            absolute_candidate
                .canonicalize()
                .unwrap_or(absolute_candidate)
                .display()
                .to_string(),
        );
    }
    if raw.contains('/') || raw.contains('\\') {
        return None;
    }
    resolve_directory_name_under(workspace_root, ".", raw, max_visits)
        .map(|relative| workspace_root.join(relative))
        .filter(|path| path.is_dir())
        .map(|path| path.canonicalize().unwrap_or(path).display().to_string())
}

pub(super) fn rewrite_compare_paths_action_to_system_dir_compare(
    action: AgentAction,
) -> AgentAction {
    match action {
        AgentAction::CallTool { tool, args }
            if tool.eq_ignore_ascii_case("fs_basic")
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.eq_ignore_ascii_case("dir_compare")) =>
        {
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args,
            }
        }
        AgentAction::CallSkill { skill, args }
            if skill.eq_ignore_ascii_case("fs_basic")
                && args
                    .get("action")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action.eq_ignore_ascii_case("dir_compare")) =>
        {
            AgentAction::CallSkill {
                skill: "system_basic".to_string(),
                args,
            }
        }
        other => other,
    }
}

pub(super) fn resolve_directory_name_under(
    workspace_root: &Path,
    root_hint: &str,
    name: &str,
    max_visits: usize,
) -> Option<String> {
    let root_path = Path::new(root_hint);
    let root = if root_path.is_absolute() {
        root_path.to_path_buf()
    } else {
        workspace_root.join(root_path)
    };
    if !root.is_dir() {
        return None;
    }
    let mut stack = vec![root];
    let mut matches = Vec::new();
    let mut visits = 0usize;
    while let Some(dir) = stack.pop() {
        visits = visits.saturating_add(1);
        if visits > max_visits {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut children = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                file_type.is_dir().then(|| entry.path())
            })
            .collect::<Vec<_>>();
        children.sort();
        for child in children.into_iter().rev() {
            if child
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|file_name| file_name.eq_ignore_ascii_case(name))
            {
                matches.push(child.clone());
                if matches.len() > 1 {
                    return None;
                }
            }
            stack.push(child);
        }
    }
    let resolved = matches.pop()?;
    resolved
        .strip_prefix(workspace_root)
        .ok()
        .and_then(|relative| relative.to_str())
        .map(|relative| relative.trim_start_matches('/').to_string())
        .filter(|relative| !relative.is_empty())
        .or_else(|| resolved.to_str().map(ToString::to_string))
}

pub(super) fn replace_directory_compare_search_plan(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !route.output_contract.requires_content_evidence
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::QuantityComparison
    {
        return actions;
    }
    let executable = actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
            )
        })
        .collect::<Vec<_>>();
    if executable.len() != 2
        || actions.iter().any(|action| {
            matches!(
                action,
                AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
            )
        })
    {
        return actions;
    }
    let Some((left_root, left_name)) = planned_find_entries_directory_name(executable[0]) else {
        return actions;
    };
    let Some((right_root, right_name)) = planned_find_entries_directory_name(executable[1]) else {
        return actions;
    };
    let max_visits = directory_name_locator_scan_limit(state);
    let Some(left_path) = resolve_directory_name_under(
        &state.skill_rt.workspace_root,
        &left_root,
        &left_name,
        max_visits,
    ) else {
        return actions;
    };
    let Some(right_path) = resolve_directory_name_under(
        &state.skill_rt.workspace_root,
        &right_root,
        &right_name,
        max_visits,
    ) else {
        return actions;
    };
    if left_path.eq_ignore_ascii_case(&right_path) {
        return actions;
    }
    info!(
        "plan_replace_directory_compare_search_with_dir_compare left={} right={}",
        crate::truncate_for_log(&left_path),
        crate::truncate_for_log(&right_path)
    );
    vec![AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "dir_compare",
            "left_path": left_path,
            "right_path": right_path,
            "recursive": true,
            "include_hidden": false,
            "max_diffs": 20,
        }),
    }]
}

pub(super) fn directory_name_locator_scan_limit(state: &AppState) -> usize {
    state.skill_rt.locator_scan_max_files.max(50_000)
}

pub(super) fn plan_path_matches_explicit_file_target(path: &str, target: &str) -> bool {
    let Some(path) = normalize_plan_path(path) else {
        return false;
    };
    let Some(target) = normalize_plan_path(target) else {
        return false;
    };
    let path = path.replace('\\', "/");
    let target = target.replace('\\', "/");
    if path.eq_ignore_ascii_case(&target) {
        return true;
    }
    let path_lower = path.to_ascii_lowercase();
    let target_lower = target.to_ascii_lowercase();
    path_lower.ends_with(&format!("/{target_lower}"))
        || target_lower.ends_with(&format!("/{path_lower}"))
        || Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(&target))
}
