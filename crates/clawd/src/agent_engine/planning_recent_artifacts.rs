use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tracing::info;

use super::LoopState;
use crate::{AgentAction, OutputSemanticKind, RouteResult};

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
        || route.output_contract.semantic_kind != OutputSemanticKind::RecentArtifactsJudgment
        || !route.output_contract.requires_content_evidence
        || !should_rewrite
        || has_bounded_file_read
    {
        return actions;
    }

    let mut include_planned_listing = false;
    let mut paths = latest_selected_file_paths(loop_state, 4);
    if paths.is_empty() {
        paths = planned_recent_artifacts_file_paths(workspace_root, &actions, 4);
        include_planned_listing = !paths.is_empty();
    }
    if paths.is_empty() {
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

fn planned_recent_artifacts_file_paths(
    workspace_root: &Path,
    actions: &[AgentAction],
    fallback_limit: usize,
) -> Vec<String> {
    actions
        .iter()
        .find_map(|action| planned_listing_request(action, fallback_limit))
        .map(|request| newest_files_for_listing_request(workspace_root, &request))
        .unwrap_or_default()
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
