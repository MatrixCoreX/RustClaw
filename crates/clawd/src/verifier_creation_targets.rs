use std::collections::HashSet;
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

fn output_contract_requires_generated_file_path_write(
    output_contract: Option<&crate::IntentOutputContract>,
) -> bool {
    output_contract.is_some_and(|contract| {
        contract.semantic_kind == crate::OutputSemanticKind::GeneratedFilePathReport
            && contract.response_shape == crate::OutputResponseShape::Scalar
            && !contract.delivery_required
    })
}

fn plan_step_action_key(state: &AppState, step: &PlanStep) -> Option<String> {
    if !matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
        return None;
    }
    let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
    crate::evidence_policy::ActionRef::from_skill_args(&normalized_skill, &step.args)
        .map(|action| action.as_key())
}

fn plan_has_generated_file_path_write_step(state: &AppState, plan_result: &PlanResult) -> bool {
    plan_result.steps.iter().any(|step| {
        plan_step_action_key(state, step).is_some_and(|action_key| {
            crate::evidence_policy::action_matches_policy_tokens(
                &action_key,
                &["fs_basic.write_text".to_string()],
            )
        })
    })
}

fn media_artifact_step_writes_own_output(state: &AppState, step: &PlanStep) -> bool {
    if !matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
        return false;
    }
    let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
    if !matches!(
        normalized_skill.as_str(),
        "audio_synthesize" | "image_generate" | "image_edit" | "video_generate" | "music_generate"
    ) {
        return false;
    }
    step.args
        .get("output_path")
        .or_else(|| step.args.get("path"))
        .and_then(|value| value.as_str())
        .is_some_and(|path| !path.trim().is_empty())
}

fn plan_has_media_artifact_output_step(state: &AppState, plan_result: &PlanResult) -> bool {
    plan_result
        .steps
        .iter()
        .any(|step| media_artifact_step_writes_own_output(state, step))
}

fn generated_file_path_report_target_path(
    state: &AppState,
    output_contract: &crate::IntentOutputContract,
) -> Option<String> {
    let hint = output_contract.locator_hint.trim();
    if hint.is_empty() {
        return None;
    }
    anchor_creation_path_to_workspace(state, hint)
}

fn unique_plan_step_id(plan_result: &PlanResult, base: &str) -> String {
    let existing = plan_result
        .steps
        .iter()
        .map(|step| step.step_id.as_str())
        .collect::<HashSet<_>>();
    if !existing.contains(base) {
        return base.to_string();
    }
    for idx in 2.. {
        let candidate = format!("{base}_{idx}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!("unbounded unique step id search")
}

fn step_can_feed_generated_file_path_write(step: &PlanStep) -> bool {
    matches!(step.action_type.as_str(), "call_skill" | "call_tool")
}

fn generated_file_path_report_insert_index(plan_result: &PlanResult) -> Option<(usize, String)> {
    let insert_idx = plan_result
        .steps
        .iter()
        .position(|step| matches!(step.action_type.as_str(), "respond" | "synthesize_answer"))
        .unwrap_or(plan_result.steps.len());
    let dependency = plan_result
        .steps
        .iter()
        .take(insert_idx)
        .rev()
        .find(|step| step_can_feed_generated_file_path_write(step))
        .map(|step| step.step_id.clone())?;
    Some((insert_idx, dependency))
}

pub(super) fn apply_generated_file_path_report_write_repair(
    state: &AppState,
    output_contract: Option<&crate::IntentOutputContract>,
    plan_result: &mut PlanResult,
) {
    if !output_contract_requires_generated_file_path_write(output_contract)
        || plan_has_generated_file_path_write_step(state, plan_result)
        || plan_has_media_artifact_output_step(state, plan_result)
    {
        return;
    }
    let Some(output_contract) = output_contract else {
        return;
    };
    let Some(target_path) = generated_file_path_report_target_path(state, output_contract) else {
        return;
    };
    if crate::media_artifact_paths::is_media_artifact_path(&target_path) {
        return;
    }
    let Some((insert_idx, dependency)) = generated_file_path_report_insert_index(plan_result)
    else {
        return;
    };
    let step_id = unique_plan_step_id(plan_result, "contract_write_generated_file_path");
    let write_step = PlanStep {
        step_id: step_id.clone(),
        action_type: "call_tool".to_string(),
        skill: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "write_text",
            "path": target_path,
            "content": "{{last_output}}",
        }),
        depends_on: vec![dependency],
        why: "contract generated_file_path_report requires writing the observed content to the requested file before reporting its path".to_string(),
    };
    plan_result.steps.insert(insert_idx, write_step);
    if !plan_result
        .steps
        .iter()
        .skip(insert_idx + 1)
        .any(|step| matches!(step.action_type.as_str(), "respond" | "synthesize_answer"))
    {
        let synthesize_id =
            unique_plan_step_id(plan_result, "contract_synthesize_generated_file_path");
        plan_result.steps.push(PlanStep {
            step_id: synthesize_id,
            action_type: "synthesize_answer".to_string(),
            skill: "synthesize_answer".to_string(),
            args: serde_json::json!({ "evidence_refs": ["last_output"] }),
            depends_on: vec![step_id],
            why: "contract generated_file_path_report requires a final single path answer"
                .to_string(),
        });
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
