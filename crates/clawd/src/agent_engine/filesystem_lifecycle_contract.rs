use serde_json::Value;
use std::path::{Component, Path};

use crate::AppState;

use super::LoopState;

pub(crate) fn scratch_filesystem_lifecycle_observed_steps_match(
    state: &AppState,
    loop_state: &LoopState,
) -> bool {
    let mut scratch_root: Option<String> = None;
    let mut saw_fs_action = false;
    let mut saw_create_or_write = false;
    let mut saw_read_or_validate = false;
    let mut saw_cleanup = false;

    for step in loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok())
    {
        if step.skill != "fs_basic" {
            continue;
        }
        let Some(extra) = step.output.as_deref().and_then(step_output_extra) else {
            continue;
        };
        let Some(action_name) = fs_action_name(&extra) else {
            continue;
        };
        let Some(root) = scratch_root_for_fs_args(&state.skill_rt.workspace_root, &extra) else {
            continue;
        };
        if let Some(existing) = scratch_root.as_deref() {
            if existing != root {
                return false;
            }
        } else {
            scratch_root = Some(root);
        }
        saw_fs_action = true;
        match action_name {
            "make_dir" | "write_text" | "append_text" => saw_create_or_write = true,
            "read_text_range" | "read_range" | "stat_paths" => saw_read_or_validate = true,
            "remove_path" => saw_cleanup = true,
            "list_dir" | "find_entries" | "grep_text" | "count_entries" | "compare_paths" => {}
            _ => return false,
        }
    }

    saw_fs_action && saw_create_or_write && saw_read_or_validate && saw_cleanup
}

pub(crate) fn enrich_scratch_filesystem_cleanup_runtime_args(
    state: &AppState,
    loop_state: &LoopState,
    requested_skill: &str,
    requested_args: &Value,
    runtime_skill: &str,
    runtime_args: &mut Value,
) -> bool {
    if requested_skill != "fs_basic"
        || fs_action_name(requested_args) != Some("remove_path")
        || runtime_skill != "remove_file"
    {
        return false;
    }
    let Some(root) = scratch_root_for_fs_args(&state.skill_rt.workspace_root, requested_args)
    else {
        return false;
    };
    if !fs_args_target_scratch_root(&state.skill_rt.workspace_root, requested_args, &root) {
        return false;
    }
    let contract_allows = loop_state.output_contract.as_ref().is_some_and(|contract| {
        contract.semantic_kind_is(crate::OutputSemanticKind::FilesystemMutationResult)
    });
    let recovery_allows =
        scratch_lifecycle_progress_has_write_in_root(state, loop_state, root.as_str())
            || scratch_lifecycle_progress_has_archive_pack_in_root(
                state,
                loop_state,
                root.as_str(),
            );
    if !contract_allows && !recovery_allows {
        return false;
    }
    let Some(obj) = runtime_args.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    let missing_target_kind = obj
        .get("target_kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none();
    if missing_target_kind {
        obj.insert(
            "target_kind".to_string(),
            Value::String("directory".to_string()),
        );
        changed = true;
    }
    if obj.get("recursive").and_then(Value::as_bool) != Some(true) {
        obj.insert("recursive".to_string(), Value::Bool(true));
        changed = true;
    }
    changed
}

fn fs_action_name(args: &Value) -> Option<&str> {
    args.get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| !action.is_empty())
}

fn scratch_root_for_fs_args(workspace_root: &Path, args: &Value) -> Option<String> {
    fs_path_value(args)
        .into_iter()
        .find_map(|path| scratch_root_for_path(workspace_root, path))
}

fn fs_args_target_scratch_root(workspace_root: &Path, args: &Value, root: &str) -> bool {
    let Some(path) = fs_path_value(args) else {
        return false;
    };
    relative_workspace_path(workspace_root, path)
        .map(|relative| relative.trim_matches('/').eq(root))
        .unwrap_or(false)
}

fn fs_path_value(args: &Value) -> Option<&str> {
    args.get("path")
        .and_then(Value::as_str)
        .or_else(|| args.get("root").and_then(Value::as_str))
}

fn scratch_root_for_path(workspace_root: &Path, raw_path: &str) -> Option<String> {
    let relative = relative_workspace_path(workspace_root, raw_path)?;
    let mut parts = relative.split('/').filter(|part| !part.is_empty());
    if parts.next()? != "tmp" {
        return None;
    }
    let scratch_name = parts.next()?;
    if scratch_name == "." || scratch_name == ".." {
        return None;
    }
    Some(format!("tmp/{scratch_name}"))
}

fn scratch_lifecycle_progress_has_write_in_root(
    state: &AppState,
    loop_state: &LoopState,
    root: &str,
) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .filter_map(step_output_extra)
        .any(|extra| {
            let action = extra
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if !matches!(action, "write_text" | "append_text") {
                return false;
            }
            let Some(path) = extra.get("path").and_then(Value::as_str) else {
                return false;
            };
            scratch_root_for_path(&state.skill_rt.workspace_root, path).as_deref() == Some(root)
        })
}

fn scratch_lifecycle_progress_has_archive_pack_in_root(
    state: &AppState,
    loop_state: &LoopState,
    root: &str,
) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "archive_basic")
        .filter_map(|step| step.output.as_deref())
        .filter_map(step_output_extra)
        .any(|extra| {
            let action = extra
                .get("action")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            if action != "pack" {
                return false;
            }
            let Some(path) = extra.get("archive").and_then(Value::as_str) else {
                return false;
            };
            scratch_root_for_path(&state.skill_rt.workspace_root, path).as_deref() == Some(root)
        })
}

fn step_output_extra(output: &str) -> Option<Value> {
    let parsed: Value = serde_json::from_str(output).ok()?;
    parsed.get("extra").cloned()
}

fn relative_workspace_path(workspace_root: &Path, raw_path: &str) -> Option<String> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return None;
    }
    let raw = Path::new(raw_path);
    if raw
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return None;
    }
    if raw.is_absolute() {
        let root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());
        let stripped = raw.strip_prefix(root).ok()?;
        return Some(stripped.to_string_lossy().replace('\\', "/"));
    }
    Some(raw_path.replace('\\', "/"))
}
