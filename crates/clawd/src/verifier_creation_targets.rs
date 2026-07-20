use std::path::{Component, Path};

use serde_json::Value;

use crate::{AppState, ClaimedTask, PlanResult, PlanStep};

use super::{VerifyIssue, VerifyIssueKind};

fn stable_task_suffix(task: &ClaimedTask) -> String {
    let suffix: String = task
        .task_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(8)
        .collect();
    if suffix.is_empty() {
        "task".to_string()
    } else {
        suffix.to_ascii_lowercase()
    }
}

fn value_as_non_empty_str<'a>(
    obj: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Option<&'a str> {
    obj.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn default_make_dir_path(state: &AppState, task: &ClaimedTask) -> String {
    state
        .skill_rt
        .workspace_root
        .join(format!("rustclaw-created-dir-{}", stable_task_suffix(task)))
        .to_string_lossy()
        .to_string()
}

fn default_write_file_path(state: &AppState) -> String {
    crate::ensure_default_file_path(&state.skill_rt.workspace_root, "")
}

fn path_has_parent_dir(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn anchor_creation_path_to_workspace(state: &AppState, path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Some(trimmed.to_string());
    }
    if path_has_parent_dir(candidate) {
        return Some(trimmed.to_string());
    }
    Some(
        state
            .skill_rt
            .workspace_root
            .join(candidate)
            .to_string_lossy()
            .to_string(),
    )
}

fn path_is_safe_workspace_creation_target(
    state: &AppState,
    path: &str,
    allow_existing_dir: bool,
) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return false;
    }
    let candidate = Path::new(trimmed);
    if path_has_parent_dir(candidate) {
        return false;
    }
    let resolved = if candidate.is_absolute() {
        if !candidate.starts_with(&state.skill_rt.workspace_root) {
            return false;
        }
        candidate.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(candidate)
    };
    if allow_existing_dir && resolved.is_dir() {
        return true;
    }
    !resolved.exists()
}

fn set_or_anchor_creation_path(
    state: &AppState,
    step_id: &str,
    normalized_skill: &str,
    obj: &mut serde_json::Map<String, Value>,
    default_path: String,
    issues: &mut Vec<VerifyIssue>,
) -> bool {
    let Some(path) = value_as_non_empty_str(obj, "path") else {
        obj.insert("path".to_string(), Value::String(default_path.clone()));
        issues.push(VerifyIssue {
            step_id: step_id.to_string(),
            kind: VerifyIssueKind::DefaultCreationTargetApplied,
            detail: format!(
                "skill `{normalized_skill}` missing creation target; defaulted path to `{default_path}`"
            ),
            missing_fields: Vec::new(),
        });
        return true;
    };
    let Some(anchored) = anchor_creation_path_to_workspace(state, path) else {
        return false;
    };
    if anchored == path {
        return false;
    }
    obj.insert("path".to_string(), Value::String(anchored.clone()));
    issues.push(VerifyIssue {
        step_id: step_id.to_string(),
        kind: VerifyIssueKind::DefaultCreationTargetApplied,
        detail: format!(
            "skill `{normalized_skill}` relative creation target anchored to `{anchored}`"
        ),
        missing_fields: Vec::new(),
    });
    true
}

fn apply_default_creation_target_to_step(
    state: &AppState,
    task: &ClaimedTask,
    step: &mut PlanStep,
    normalized_skill: &str,
    issues: &mut Vec<VerifyIssue>,
) -> bool {
    if !matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
        return false;
    }
    let Some(obj) = step.args.as_object_mut() else {
        return false;
    };
    match normalized_skill {
        "make_dir" => set_or_anchor_creation_path(
            state,
            &step.step_id,
            normalized_skill,
            obj,
            default_make_dir_path(state, task),
            issues,
        ),
        "write_file" => set_or_anchor_creation_path(
            state,
            &step.step_id,
            normalized_skill,
            obj,
            default_write_file_path(state),
            issues,
        ),
        "fs_basic" => match obj
            .get("action")
            .and_then(|value| value.as_str())
            .map(str::trim)
        {
            Some("make_dir") => set_or_anchor_creation_path(
                state,
                &step.step_id,
                normalized_skill,
                obj,
                default_make_dir_path(state, task),
                issues,
            ),
            Some("write_text" | "append_text") => set_or_anchor_creation_path(
                state,
                &step.step_id,
                normalized_skill,
                obj,
                default_write_file_path(state),
                issues,
            ),
            _ => false,
        },
        _ => false,
    }
}

pub(super) fn apply_default_creation_targets(
    state: &AppState,
    task: &ClaimedTask,
    plan_result: &mut PlanResult,
    issues: &mut Vec<VerifyIssue>,
) {
    for step in &mut plan_result.steps {
        let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
        apply_default_creation_target_to_step(state, task, step, &normalized_skill, issues);
    }
}

pub(super) fn safe_autonomous_creation_step(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    let Some(obj) = args.as_object() else {
        return false;
    };
    match normalized_skill {
        "make_dir" => value_as_non_empty_str(obj, "path")
            .is_some_and(|path| path_is_safe_workspace_creation_target(state, path, true)),
        "write_file" => value_as_non_empty_str(obj, "path")
            .is_some_and(|path| path_is_safe_workspace_creation_target(state, path, false)),
        "fs_basic" => match obj
            .get("action")
            .and_then(|value| value.as_str())
            .map(str::trim)
        {
            Some("make_dir") => value_as_non_empty_str(obj, "path")
                .is_some_and(|path| path_is_safe_workspace_creation_target(state, path, true)),
            Some("write_text" | "append_text") => value_as_non_empty_str(obj, "path")
                .is_some_and(|path| path_is_safe_workspace_creation_target(state, path, false)),
            _ => false,
        },
        _ => false,
    }
}
