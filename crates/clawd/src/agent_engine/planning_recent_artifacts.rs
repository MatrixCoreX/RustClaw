use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tracing::info;

use super::planning_actions::build_plan_result;
use super::LoopState;
use crate::{
    AgentAction, OutputScalarCountTargetKind, OutputSemanticKind, PlanKind, PlanResult, RouteResult,
};

pub(super) fn recent_artifacts_judgment_deterministic_plan_result(
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1 || loop_state.has_tool_or_skill_output {
        return None;
    }
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract_marker_is(OutputSemanticKind::RecentArtifactsJudgment)
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
    {
        return None;
    }
    let path = recent_artifacts_directory_locator_path(route, auto_locator_path)?;
    if !Path::new(&path).is_dir() {
        return None;
    }

    let limit = recent_artifact_selector_limit(route)
        .unwrap_or(4)
        .clamp(1, 1000);
    let sort_by = recent_artifact_selector_sort_by(route).unwrap_or_else(|| "mtime_desc".into());
    let include_hidden = route
        .output_contract
        .self_extension
        .list_selector
        .include_hidden
        .or_else(|| {
            selector_bool_machine_token(route.resolved_intent.as_str(), "selector_include_hidden")
        })
        .or_else(|| {
            selector_bool_machine_token(route.route_reason.as_str(), "selector_include_hidden")
        })
        .unwrap_or(false);
    let mut args = json!({
        "action": "list_dir",
        "path": path,
        "sort_by": sort_by,
        "max_entries": limit,
        "names_only": false,
        "include_hidden": include_hidden,
    });
    if let Some(obj) = args.as_object_mut() {
        match recent_artifact_selector_target_kind(route) {
            OutputScalarCountTargetKind::File => {
                obj.insert("files_only".to_string(), Value::Bool(true));
                obj.insert("dirs_only".to_string(), Value::Bool(false));
            }
            OutputScalarCountTargetKind::Dir => {
                obj.insert("dirs_only".to_string(), Value::Bool(true));
                obj.insert("files_only".to_string(), Value::Bool(false));
            }
            OutputScalarCountTargetKind::Any => {
                obj.insert("files_only".to_string(), Value::Bool(false));
                obj.insert("dirs_only".to_string(), Value::Bool(false));
            }
        }
    }
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args,
    }];
    let raw_plan_text = serde_json::to_string(&json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn recent_artifacts_directory_locator_path(
    route: &RouteResult,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let hint = route.output_contract.locator_hint.trim();
    let auto_dir = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter(|path| Path::new(path).is_dir());
    if !hint.is_empty() {
        if let Some(path) = auto_dir.filter(|path| locator_path_matches_hint(path, hint)) {
            return Some(path.to_string());
        }
        if Path::new(hint).is_dir()
            || matches!(
                route.output_contract.locator_kind,
                crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
            )
            || hint.contains(['/', '\\'])
            || hint.starts_with('.')
        {
            return Some(hint.to_string());
        }
    }
    auto_dir
        .or_else(|| {
            (route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
                || hint.is_empty())
            .then_some(".")
        })
        .map(ToString::to_string)
}

fn locator_path_matches_hint(path: &str, hint: &str) -> bool {
    let path = path.trim().trim_end_matches(['/', '\\']);
    let hint = hint.trim().trim_end_matches(['/', '\\']);
    if path.is_empty() || hint.is_empty() {
        return false;
    }
    path.eq_ignore_ascii_case(hint)
        || path.ends_with(&format!("/{hint}"))
        || path.ends_with(&format!("\\{hint}"))
}

pub(super) fn normalize_recent_artifacts_listing_selectors(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route.output_contract_marker_is(OutputSemanticKind::RecentArtifactsJudgment) {
        return actions;
    }
    actions
        .into_iter()
        .map(|action| normalize_recent_artifacts_listing_action(route, action))
        .collect()
}

fn normalize_recent_artifacts_listing_action(
    route: &RouteResult,
    action: AgentAction,
) -> AgentAction {
    match action {
        AgentAction::CallTool { tool, mut args } => {
            apply_recent_artifact_listing_selector(route, tool.as_str(), &mut args);
            AgentAction::CallTool { tool, args }
        }
        AgentAction::CallSkill { skill, mut args } => {
            apply_recent_artifact_listing_selector(route, skill.as_str(), &mut args);
            AgentAction::CallSkill { skill, args }
        }
        AgentAction::CallCapability {
            capability,
            mut args,
        } => {
            apply_recent_artifact_listing_selector(route, capability.as_str(), &mut args);
            AgentAction::CallCapability { capability, args }
        }
        other => other,
    }
}

fn apply_recent_artifact_listing_selector(route: &RouteResult, skill: &str, args: &mut Value) {
    if !action_is_recent_artifacts_listing_parts(skill, args) {
        return;
    }
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    match recent_artifact_selector_target_kind(route) {
        OutputScalarCountTargetKind::File => {
            obj.insert("files_only".to_string(), Value::Bool(true));
            obj.insert("dirs_only".to_string(), Value::Bool(false));
        }
        OutputScalarCountTargetKind::Dir => {
            obj.insert("dirs_only".to_string(), Value::Bool(true));
            obj.insert("files_only".to_string(), Value::Bool(false));
        }
        OutputScalarCountTargetKind::Any => {
            obj.insert("files_only".to_string(), Value::Bool(false));
            obj.insert("dirs_only".to_string(), Value::Bool(false));
        }
    }
    obj.insert("names_only".to_string(), Value::Bool(false));
    if let Some(limit) = recent_artifact_selector_limit(route) {
        obj.insert(
            "max_entries".to_string(),
            Value::Number(serde_json::Number::from(limit)),
        );
    }
    if let Some(sort_by) = recent_artifact_selector_sort_by(route) {
        obj.insert("sort_by".to_string(), Value::String(sort_by));
    }
    if let Some(include_hidden) = route
        .output_contract
        .self_extension
        .list_selector
        .include_hidden
        .or_else(|| {
            selector_bool_machine_token(route.resolved_intent.as_str(), "selector_include_hidden")
        })
        .or_else(|| {
            selector_bool_machine_token(route.route_reason.as_str(), "selector_include_hidden")
        })
    {
        obj.insert("include_hidden".to_string(), Value::Bool(include_hidden));
    }
}

pub(super) fn rewrite_recent_artifacts_field_extraction_to_selected_file_reads(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    workspace_root: &Path,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    let has_field_extraction = actions.iter().any(action_is_field_extraction);
    let has_bounded_file_read = actions.iter().any(action_is_bounded_file_read);
    let has_listing = actions.iter().any(action_is_recent_artifacts_listing);
    let should_rewrite = has_field_extraction
        || (!has_bounded_file_read
            && (has_listing
                || (loop_state.has_tool_or_skill_output
                    && actions.iter().any(action_is_terminal_discussion))));

    if route.needs_clarify
        || !route.output_contract_marker_is(OutputSemanticKind::RecentArtifactsJudgment)
        || !route.output_contract.requires_content_evidence
        || !should_rewrite
        || has_bounded_file_read
    {
        return actions;
    }

    let selector_target_kind = recent_artifact_selector_target_kind(route);
    let mut include_planned_listing = false;
    let mut paths = latest_selected_file_paths_for_selector(route, loop_state, 4);
    if paths.is_empty() {
        paths =
            planned_recent_artifacts_file_paths_for_selector(route, workspace_root, &actions, 4);
        include_planned_listing = !paths.is_empty();
    }
    if paths.is_empty() || selector_target_kind == OutputScalarCountTargetKind::Dir {
        return actions;
    }

    let mut rewritten = if include_planned_listing {
        actions
            .iter()
            .find(|action| action_is_recent_artifacts_listing(action))
            .cloned()
            .into_iter()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    rewritten.extend(
        paths
            .iter()
            .map(|path| AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({
                    "action": "read_text_range",
                    "path": path,
                    "mode": "head",
                    "n": 40,
                }),
            })
            .collect::<Vec<_>>(),
    );
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
        "plan_rewrite_recent_artifacts_field_extraction_to_file_reads paths={} refs={}",
        paths.join(","),
        evidence_refs.join(",")
    );
    rewritten
}

fn planned_recent_artifacts_file_paths_for_selector(
    route: &RouteResult,
    workspace_root: &Path,
    actions: &[AgentAction],
    fallback_limit: usize,
) -> Vec<String> {
    if recent_artifact_selector_target_kind(route) == OutputScalarCountTargetKind::Dir {
        return Vec::new();
    }
    let Some(request) = actions
        .iter()
        .find_map(|action| planned_listing_request(action, fallback_limit))
    else {
        return Vec::new();
    };
    if recent_artifact_selector_target_kind(route) == OutputScalarCountTargetKind::Any
        && planned_listing_request_may_select_dirs(workspace_root, &request)
    {
        return Vec::new();
    }
    newest_files_for_listing_request(workspace_root, &request)
}

#[derive(Debug)]
struct PlannedListingRequest {
    path: String,
    limit: usize,
    sort_by: String,
}

fn planned_listing_request(
    action: &AgentAction,
    fallback_limit: usize,
) -> Option<PlannedListingRequest> {
    if !action_is_recent_artifacts_listing(action) {
        return None;
    }
    let args = match action {
        AgentAction::CallSkill { args, .. }
        | AgentAction::CallTool { args, .. }
        | AgentAction::CallCapability { args, .. } => args,
        _ => return None,
    };
    let path = args
        .get("path")
        .or_else(|| args.get("root"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())?
        .to_string();
    let limit = args
        .get("max_entries")
        .or_else(|| args.get("limit"))
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .filter(|value| *value > 0)
        .unwrap_or(fallback_limit);
    let sort_by = args
        .get("sort_by")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("mtime_desc")
        .to_string();
    Some(PlannedListingRequest {
        path,
        limit,
        sort_by,
    })
}

fn newest_files_for_listing_request(
    workspace_root: &Path,
    request: &PlannedListingRequest,
) -> Vec<String> {
    let dir = resolve_planned_dir(workspace_root, &request.path);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then_some((entry.path(), metadata))
        })
        .collect::<Vec<_>>();
    match request.sort_by.as_str() {
        "mtime_asc" => files.sort_by_key(|(_, metadata)| metadata.modified().ok()),
        "name" | "name_asc" => {
            files.sort_by_key(|(path, _)| path.file_name().map(|name| name.to_os_string()))
        }
        _ => files.sort_by_key(|(_, metadata)| std::cmp::Reverse(metadata.modified().ok())),
    }
    files
        .into_iter()
        .take(request.limit)
        .filter_map(|(path, _)| display_planned_child_path(workspace_root, &request.path, &path))
        .collect()
}

fn resolve_planned_dir(workspace_root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

fn planned_listing_request_may_select_dirs(
    workspace_root: &Path,
    request: &PlannedListingRequest,
) -> bool {
    let dir = resolve_planned_dir(workspace_root, &request.path);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        entry
            .file_type()
            .ok()
            .is_some_and(|file_type| file_type.is_dir())
    })
}

fn display_planned_child_path(
    workspace_root: &Path,
    requested_dir: &str,
    child: &Path,
) -> Option<String> {
    if Path::new(requested_dir).is_absolute() {
        return Some(child.display().to_string());
    }
    child
        .strip_prefix(workspace_root)
        .ok()
        .map(|path| path.display().to_string())
        .or_else(|| Some(child.display().to_string()))
}

fn action_is_terminal_discussion(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
    )
}

fn action_is_recent_artifacts_listing(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        AgentAction::CallCapability { capability, args } => (capability.as_str(), args),
        _ => return false,
    };
    action_is_recent_artifacts_listing_parts(skill, args)
}

fn action_is_recent_artifacts_listing_parts(skill: &str, args: &Value) -> bool {
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let skill = skill.to_ascii_lowercase();
    matches!(
        (skill.as_str(), action_name),
        ("fs_basic", "list_dir")
            | ("fs_basic", "inventory_dir")
            | ("fs_basic.list_dir", _)
            | ("fs_basic.inventory_dir", _)
            | ("system_basic", "inventory_dir")
            | ("system_basic.inventory_dir", _)
    )
}

fn action_is_field_extraction(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        AgentAction::CallCapability { capability, args } => (capability.as_str(), args),
        _ => return false,
    };
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let skill = skill.to_ascii_lowercase();
    matches!(
        (skill.as_str(), action_name),
        ("system_basic", "extract_field")
            | ("system_basic", "extract_fields")
            | ("system_basic.extract_field", _)
            | ("system_basic.extract_fields", _)
            | ("config_basic", "read_field")
            | ("config_basic", "read_fields")
            | ("config_basic", "extract_field")
            | ("config_basic", "extract_fields")
            | ("config_basic.read_field", _)
            | ("config_basic.read_fields", _)
            | ("config_basic.extract_field", _)
            | ("config_basic.extract_fields", _)
    )
}

fn action_is_bounded_file_read(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        AgentAction::CallCapability { capability, args } => (capability.as_str(), args),
        _ => return false,
    };
    (skill.eq_ignore_ascii_case("fs_basic")
        || skill.eq_ignore_ascii_case("fs_basic.read_text_range"))
        && args
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .map(|action| action.is_empty() || action == "read_text_range")
            .unwrap_or_else(|| skill.eq_ignore_ascii_case("fs_basic.read_text_range"))
}

fn latest_selected_file_paths(loop_state: &LoopState, limit: usize) -> Vec<String> {
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() {
            continue;
        }
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        let paths = selected_file_paths_from_output(output, limit);
        if !paths.is_empty() {
            return paths;
        }
    }
    loop_state
        .last_output
        .as_deref()
        .map(|output| selected_file_paths_from_output(output, limit))
        .unwrap_or_default()
}

fn latest_selected_file_paths_for_selector(
    route: &RouteResult,
    loop_state: &LoopState,
    limit: usize,
) -> Vec<String> {
    match recent_artifact_selector_target_kind(route) {
        OutputScalarCountTargetKind::Dir => Vec::new(),
        OutputScalarCountTargetKind::File => latest_selected_file_paths(loop_state, limit),
        OutputScalarCountTargetKind::Any => {
            latest_selected_file_paths_if_entries_are_files(loop_state, limit)
        }
    }
}

fn latest_selected_file_paths_if_entries_are_files(
    loop_state: &LoopState,
    limit: usize,
) -> Vec<String> {
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() {
            continue;
        }
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        if selected_output_contains_non_file_entry(output) {
            return Vec::new();
        }
        let paths = selected_file_paths_from_output(output, limit);
        if !paths.is_empty() {
            return paths;
        }
    }
    let Some(output) = loop_state.last_output.as_deref() else {
        return Vec::new();
    };
    if selected_output_contains_non_file_entry(output) {
        return Vec::new();
    }
    selected_file_paths_from_output(output, limit)
}

fn selected_file_paths_from_output(output: &str, limit: usize) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(output.trim()) else {
        return Vec::new();
    };
    let mut paths = selected_file_paths_from_value(&value, limit);
    if paths.is_empty() {
        if let Some(text) = value.get("text").and_then(Value::as_str) {
            paths = selected_file_paths_from_output(text, limit);
        }
    }
    paths
}

fn selected_file_paths_from_value(value: &Value, limit: usize) -> Vec<String> {
    let source = value.get("extra").unwrap_or(value);
    let Some(entries) = source.get("entries").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for entry in entries {
        if paths.len() >= limit {
            break;
        }
        if entry
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| !kind.eq_ignore_ascii_case("file"))
        {
            continue;
        }
        let Some(path) = entry.get("path").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if path.is_empty() || paths.iter().any(|existing| existing == path) {
            continue;
        }
        paths.push(path.to_string());
    }
    paths
}

fn selected_output_contains_non_file_entry(output: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(output.trim()) else {
        return false;
    };
    selected_value_contains_non_file_entry(&value)
        || value
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(selected_output_contains_non_file_entry)
}

fn selected_value_contains_non_file_entry(value: &Value) -> bool {
    let source = value.get("extra").unwrap_or(value);
    let Some(entries) = source.get("entries").and_then(Value::as_array) else {
        return false;
    };
    entries.iter().any(|entry| {
        entry
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| !kind.eq_ignore_ascii_case("file"))
    })
}

fn recent_artifact_selector_target_kind(route: &RouteResult) -> OutputScalarCountTargetKind {
    let selector = &route.output_contract.self_extension.list_selector;
    if selector.target_kind_specified {
        return selector.target_kind;
    }
    selector_target_kind_machine_token(route.resolved_intent.as_str())
        .or_else(|| selector_target_kind_machine_token(route.route_reason.as_str()))
        .unwrap_or_default()
}

fn selector_target_kind_machine_token(text: &str) -> Option<OutputScalarCountTargetKind> {
    selector_value_machine_token(text, "selector_target_kind").and_then(|raw| match raw.as_str() {
        "file" => Some(OutputScalarCountTargetKind::File),
        "dir" => Some(OutputScalarCountTargetKind::Dir),
        "any" => Some(OutputScalarCountTargetKind::Any),
        _ => None,
    })
}

fn recent_artifact_selector_limit(route: &RouteResult) -> Option<u64> {
    route
        .output_contract
        .self_extension
        .list_selector
        .limit
        .or_else(|| selector_u64_machine_token(route.resolved_intent.as_str(), "selector_limit"))
        .or_else(|| selector_u64_machine_token(route.route_reason.as_str(), "selector_limit"))
        .filter(|limit| *limit > 0)
}

fn recent_artifact_selector_sort_by(route: &RouteResult) -> Option<String> {
    route
        .output_contract
        .self_extension
        .list_selector
        .sort_by
        .clone()
        .or_else(|| {
            selector_value_machine_token(route.resolved_intent.as_str(), "selector_sort_by")
        })
        .or_else(|| selector_value_machine_token(route.route_reason.as_str(), "selector_sort_by"))
}

fn selector_u64_machine_token(text: &str, key: &str) -> Option<u64> {
    selector_value_machine_token(text, key).and_then(|raw| raw.parse::<u64>().ok())
}

fn selector_bool_machine_token(text: &str, key: &str) -> Option<bool> {
    selector_value_machine_token(text, key).and_then(|raw| match raw.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}

fn selector_value_machine_token(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    text.split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | ')' | '('))
        .filter_map(|part| part.trim().strip_prefix(&prefix))
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(ToOwned::to_owned)
        .next()
}
