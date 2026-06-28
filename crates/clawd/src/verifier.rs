use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};

use claw_core::skill_registry::{PlannerCapabilityEffect, PrimaryFallbackRole, SkillRiskLevel};
use serde_json::{json, Value};

use crate::{contract_matrix::FailureAttribution, AppState, ClaimedTask, PlanResult, PlanStep};

#[path = "verifier_risk_policy.rs"]
mod risk_policy;
#[path = "verifier_structured_fields.rs"]
mod structured_fields;
#[path = "verifier_templates.rs"]
mod templates;

use risk_policy::high_risk_side_effect_requires_confirmation;
use templates::{
    step_can_produce_output_for_template_scope, value_contains_unresolved_template,
    TemplatePlaceholderScope,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifyMode {
    ObserveOnly,
    Enforce,
}

impl Default for VerifyMode {
    fn default() -> Self {
        Self::ObserveOnly
    }
}

impl VerifyMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ObserveOnly => "ObserveOnly",
            Self::Enforce => "Enforce",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifyIssueKind {
    SkillNotVisible,
    CapabilityUnavailable,
    MissingRequiredArg,
    DefaultCreationTargetApplied,
    UnresolvedTemplateArg,
    InvalidDependsOn,
    ConfirmationRequired,
    RiskBudgetExceeded,
    PrimaryFallbackConflict,
    RouteClarifyRequired,
    RecipeInspectBeforeMutateRequired,
    RecipeValidationAfterMutateRequired,
    RecipeTargetScopeRequired,
    ContractActionRejected,
    ContractMissing,
    ContractPolicyViolation,
    ContractPreferredActionAvailable,
}

impl Default for VerifyIssueKind {
    fn default() -> Self {
        Self::SkillNotVisible
    }
}

impl VerifyIssueKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SkillNotVisible => "SkillNotVisible",
            Self::CapabilityUnavailable => "CapabilityUnavailable",
            Self::MissingRequiredArg => "MissingRequiredArg",
            Self::DefaultCreationTargetApplied => "DefaultCreationTargetApplied",
            Self::UnresolvedTemplateArg => "UnresolvedTemplateArg",
            Self::InvalidDependsOn => "InvalidDependsOn",
            Self::ConfirmationRequired => "ConfirmationRequired",
            Self::RiskBudgetExceeded => "RiskBudgetExceeded",
            Self::PrimaryFallbackConflict => "PrimaryFallbackConflict",
            Self::RouteClarifyRequired => "RouteClarifyRequired",
            Self::RecipeInspectBeforeMutateRequired => "RecipeInspectBeforeMutateRequired",
            Self::RecipeValidationAfterMutateRequired => "RecipeValidationAfterMutateRequired",
            Self::RecipeTargetScopeRequired => "RecipeTargetScopeRequired",
            Self::ContractActionRejected => "ContractActionRejected",
            Self::ContractMissing => "ContractMissing",
            Self::ContractPolicyViolation => "ContractPolicyViolation",
            Self::ContractPreferredActionAvailable => "ContractPreferredActionAvailable",
        }
    }

    pub(crate) fn failure_attribution(self) -> FailureAttribution {
        match self {
            Self::SkillNotVisible | Self::CapabilityUnavailable => FailureAttribution::ToolGap,
            Self::MissingRequiredArg
            | Self::UnresolvedTemplateArg
            | Self::InvalidDependsOn
            | Self::PrimaryFallbackConflict
            | Self::RouteClarifyRequired => FailureAttribution::ModelError,
            Self::DefaultCreationTargetApplied
            | Self::RecipeInspectBeforeMutateRequired
            | Self::RecipeValidationAfterMutateRequired
            | Self::RecipeTargetScopeRequired => FailureAttribution::CodeGap,
            Self::ConfirmationRequired | Self::RiskBudgetExceeded => {
                FailureAttribution::PermissionDenied
            }
            Self::ContractActionRejected
            | Self::ContractMissing
            | Self::ContractPolicyViolation
            | Self::ContractPreferredActionAvailable => FailureAttribution::ContractGap,
        }
    }

    pub(crate) fn reason_code(self) -> &'static str {
        match self {
            Self::SkillNotVisible => "verify_skill_not_visible",
            Self::CapabilityUnavailable => "verify_capability_unavailable",
            Self::MissingRequiredArg => "verify_missing_required_arg",
            Self::DefaultCreationTargetApplied => "verify_default_creation_target_applied",
            Self::UnresolvedTemplateArg => "verify_unresolved_template_arg",
            Self::InvalidDependsOn => "verify_invalid_depends_on",
            Self::ConfirmationRequired => "verify_confirmation_required",
            Self::RiskBudgetExceeded => "verify_risk_budget_exceeded",
            Self::PrimaryFallbackConflict => "verify_primary_fallback_conflict",
            Self::RouteClarifyRequired => "verify_route_clarify_required",
            Self::RecipeInspectBeforeMutateRequired => {
                "verify_recipe_inspect_before_mutate_required"
            }
            Self::RecipeValidationAfterMutateRequired => {
                "verify_recipe_validation_after_mutate_required"
            }
            Self::RecipeTargetScopeRequired => "verify_recipe_target_scope_required",
            Self::ContractActionRejected => "verify_contract_action_rejected",
            Self::ContractMissing => "verify_contract_missing",
            Self::ContractPolicyViolation => "verify_contract_policy_violation",
            Self::ContractPreferredActionAvailable => "verify_contract_preferred_action_available",
        }
    }

    pub(crate) fn status_code(self) -> &'static str {
        match self {
            Self::SkillNotVisible => "skill_not_visible",
            Self::CapabilityUnavailable => "capability_unavailable",
            Self::MissingRequiredArg => "missing_required_arg",
            Self::DefaultCreationTargetApplied => "default_creation_target_applied",
            Self::UnresolvedTemplateArg => "unresolved_template_arg",
            Self::InvalidDependsOn => "invalid_depends_on",
            Self::ConfirmationRequired => "confirmation_required",
            Self::RiskBudgetExceeded => "risk_budget_exceeded",
            Self::PrimaryFallbackConflict => "primary_fallback_conflict",
            Self::RouteClarifyRequired => "route_clarify_required",
            Self::RecipeInspectBeforeMutateRequired => "recipe_inspect_before_mutate_required",
            Self::RecipeValidationAfterMutateRequired => "recipe_validation_after_mutate_required",
            Self::RecipeTargetScopeRequired => "recipe_target_scope_required",
            Self::ContractActionRejected => "contract_action_rejected",
            Self::ContractMissing => "contract_missing",
            Self::ContractPolicyViolation => "contract_policy_violation",
            Self::ContractPreferredActionAvailable => "contract_preferred_action_available",
        }
    }

    pub(crate) fn message_key(self) -> &'static str {
        match self {
            Self::SkillNotVisible => "clawd.verify.skill_not_visible",
            Self::CapabilityUnavailable => "clawd.verify.capability_unavailable",
            Self::MissingRequiredArg => "clawd.verify.missing_required_arg",
            Self::DefaultCreationTargetApplied => "clawd.verify.default_creation_target_applied",
            Self::UnresolvedTemplateArg => "clawd.verify.unresolved_template_arg",
            Self::InvalidDependsOn => "clawd.verify.invalid_depends_on",
            Self::ConfirmationRequired => "clawd.verify.confirmation_required",
            Self::RiskBudgetExceeded => "clawd.verify.risk_budget_exceeded",
            Self::PrimaryFallbackConflict => "clawd.verify.primary_fallback_conflict",
            Self::RouteClarifyRequired => "clawd.verify.route_clarify_required",
            Self::RecipeInspectBeforeMutateRequired => {
                "clawd.verify.recipe_inspect_before_mutate_required"
            }
            Self::RecipeValidationAfterMutateRequired => {
                "clawd.verify.recipe_validation_after_mutate_required"
            }
            Self::RecipeTargetScopeRequired => "clawd.verify.recipe_target_scope_required",
            Self::ContractActionRejected => "clawd.verify.contract_action_rejected",
            Self::ContractMissing => "clawd.verify.contract_missing",
            Self::ContractPolicyViolation => "clawd.verify.contract_policy_violation",
            Self::ContractPreferredActionAvailable => {
                "clawd.verify.contract_preferred_action_available"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VerifyIssue {
    pub(crate) step_id: String,
    pub(crate) kind: VerifyIssueKind,
    pub(crate) detail: String,
    pub(crate) missing_fields: Vec<String>,
}

pub(crate) struct VerifyInput<'a> {
    pub(crate) route_result: Option<&'a crate::RouteResult>,
    pub(crate) request_text: Option<&'a str>,
    pub(crate) context_bundle_summary: Option<&'a str>,
    pub(crate) plan_result: &'a PlanResult,
    pub(crate) execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
}

#[derive(Debug, Clone)]
pub(crate) struct VerifyResult {
    pub(crate) mode: VerifyMode,
    pub(crate) approved: bool,
    pub(crate) blocked_reason: Option<String>,
    pub(crate) shadow_blocked_reason: Option<String>,
    pub(crate) permission_decision: Value,
    pub(crate) approved_steps: Vec<PlanStep>,
    pub(crate) needs_confirmation: bool,
    pub(crate) rewritten_steps: Vec<PlanStep>,
    pub(crate) issues: Vec<VerifyIssue>,
}

fn required_args_for_skill(skill: &str) -> &'static [&'static str] {
    match skill {
        "run_cmd" => &["command"],
        "read_file" => &["path"],
        "write_file" => &["path", "content"],
        "remove_file" => &["path"],
        "make_dir" => &["path"],
        _ => &[],
    }
}

fn is_confirmation_like_skill(skill: &str) -> bool {
    matches!(
        skill,
        "run_cmd" | "write_file" | "remove_file" | "make_dir" | "schedule" | "config_edit"
    )
}

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
    obj: &'a serde_json::Map<String, serde_json::Value>,
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
    obj: &mut serde_json::Map<String, serde_json::Value>,
    default_path: String,
    issues: &mut Vec<VerifyIssue>,
) -> bool {
    let Some(path) = value_as_non_empty_str(obj, "path") else {
        obj.insert(
            "path".to_string(),
            serde_json::Value::String(default_path.clone()),
        );
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
    obj.insert(
        "path".to_string(),
        serde_json::Value::String(anchored.clone()),
    );
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

fn apply_default_creation_targets(
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

fn route_requires_generated_file_path_write(route: Option<&crate::RouteResult>) -> bool {
    route.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::GeneratedFilePathReport
            && route.output_contract.response_shape == crate::OutputResponseShape::Scalar
            && !route.output_contract.delivery_required
    })
}

fn plan_step_action_key(state: &AppState, step: &PlanStep) -> Option<String> {
    if !matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
        return None;
    }
    let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
    crate::contract_matrix::ActionRef::from_skill_args(&normalized_skill, &step.args)
        .map(|action| action.as_key())
}

fn plan_has_generated_file_path_write_step(state: &AppState, plan_result: &PlanResult) -> bool {
    plan_result.steps.iter().any(|step| {
        plan_step_action_key(state, step).is_some_and(|action_key| {
            crate::contract_matrix::action_matches_policy_tokens(
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
    route: &crate::RouteResult,
) -> Option<String> {
    let hint = route.output_contract.locator_hint.trim();
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

fn apply_generated_file_path_report_write_repair(
    state: &AppState,
    route: Option<&crate::RouteResult>,
    plan_result: &mut PlanResult,
) {
    if !route_requires_generated_file_path_write(route)
        || plan_has_generated_file_path_write_step(state, plan_result)
        || plan_has_media_artifact_output_step(state, plan_result)
    {
        return;
    }
    let Some(route) = route else {
        return;
    };
    let Some(target_path) = generated_file_path_report_target_path(state, route) else {
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

fn safe_autonomous_creation_step(
    state: &AppState,
    normalized_skill: &str,
    args: &serde_json::Value,
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

fn route_has_confirmation_resume(route_result: Option<&crate::RouteResult>) -> bool {
    route_result
        .map(|route| matches!(route.resume_behavior, crate::ResumeBehavior::ResumeExecute))
        .unwrap_or(false)
}

fn manifest_required_args(state: &AppState, normalized_skill: &str) -> Vec<String> {
    state
        .skill_manifest(normalized_skill)
        .and_then(|manifest| manifest.input_schema)
        .and_then(|schema| schema.get("required").cloned())
        .and_then(|required| required.as_array().cloned())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_schema_token(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if matches!(ch, '-' | ' ' | '.') {
                '_'
            } else {
                ch
            }
        })
        .collect()
}

fn action_scoped_required_args(
    state: &AppState,
    normalized_skill: &str,
    args: &serde_json::Value,
) -> Vec<String> {
    let Some(action) = args
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(|value| value.as_str())
        .map(normalize_schema_token)
        .filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };
    state
        .skill_manifest(normalized_skill)
        .and_then(|manifest| {
            manifest
                .planner_capabilities
                .into_iter()
                .find(|mapping| mapping.action.as_deref() == Some(action.as_str()))
                .map(|mapping| mapping.required)
        })
        .unwrap_or_default()
}

fn action_scoped_risk_level(
    state: &AppState,
    normalized_skill: &str,
    args: &serde_json::Value,
) -> Option<SkillRiskLevel> {
    let action = args
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(|value| value.as_str())
        .map(normalize_schema_token)
        .filter(|value| !value.is_empty())?;
    state.skill_manifest(normalized_skill).and_then(|manifest| {
        manifest
            .planner_capabilities
            .into_iter()
            .find(|mapping| mapping.action.as_deref() == Some(action.as_str()))
            .and_then(|mapping| mapping.risk_level)
    })
}

fn registry_declares_non_mutating_planner_action(
    state: &AppState,
    normalized_skill: &str,
    args: &serde_json::Value,
) -> bool {
    let Some(action) = args
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(|value| value.as_str())
        .map(normalize_schema_token)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    state
        .skill_manifest(normalized_skill)
        .is_some_and(|manifest| {
            manifest.planner_capabilities.into_iter().any(|mapping| {
                mapping
                    .action
                    .as_deref()
                    .map(normalize_schema_token)
                    .is_some_and(|mapped| mapped == action)
                    && matches!(
                        mapping.effect,
                        Some(PlannerCapabilityEffect::Observe | PlannerCapabilityEffect::Validate)
                    )
            })
        })
}

fn registry_action_can_extend_summary_contract(
    state: &AppState,
    normalized_skill: &str,
    args: &serde_json::Value,
    contract_match: &str,
) -> bool {
    contract_match == "command_output_summary"
        && registry_declares_non_mutating_planner_action(state, normalized_skill, args)
}

fn effective_step_risk_level(
    state: &AppState,
    normalized_skill: &str,
    args: &serde_json::Value,
) -> SkillRiskLevel {
    if package_manager_dry_run_install_action(normalized_skill, args) {
        return SkillRiskLevel::Low;
    }
    if task_control_lifecycle_dry_run_action(normalized_skill, args) {
        return SkillRiskLevel::Low;
    }
    if let Some(risk) = action_scoped_risk_level(state, normalized_skill, args) {
        return risk;
    }
    if let Some(risk) = state
        .skill_manifest(normalized_skill)
        .and_then(|manifest| manifest.risk_level)
    {
        return risk;
    }
    let effect =
        crate::execution_recipe::classify_skill_action_effect(state, normalized_skill, args);
    if effect.mutates {
        SkillRiskLevel::High
    } else {
        SkillRiskLevel::Low
    }
}

fn package_manager_dry_run_install_action(
    normalized_skill: &str,
    args: &serde_json::Value,
) -> bool {
    if normalized_skill != "package_manager" {
        return false;
    }
    if args.get("dry_run").and_then(serde_json::Value::as_bool) != Some(true) {
        return false;
    }
    let action = args
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(|value| value.as_str())
        .map(normalize_schema_token)
        .unwrap_or_default();
    matches!(action.as_str(), "install" | "uninstall" | "smart_install")
}

fn task_control_lifecycle_dry_run_action(normalized_skill: &str, args: &serde_json::Value) -> bool {
    if normalized_skill != "task_control" {
        return false;
    }
    if args.get("dry_run").and_then(serde_json::Value::as_bool) != Some(true) {
        return false;
    }
    let action = args
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(|value| value.as_str())
        .map(normalize_schema_token)
        .unwrap_or_default();
    matches!(action.as_str(), "resume" | "pause")
}

fn risk_level_token(risk_level: SkillRiskLevel) -> &'static str {
    match risk_level {
        SkillRiskLevel::Unknown => "unknown",
        SkillRiskLevel::Low => "low",
        SkillRiskLevel::Medium => "medium",
        SkillRiskLevel::High => "high",
    }
}

fn issue_is_policy_denial(kind: VerifyIssueKind) -> bool {
    matches!(
        kind,
        VerifyIssueKind::SkillNotVisible
            | VerifyIssueKind::CapabilityUnavailable
            | VerifyIssueKind::ConfirmationRequired
            | VerifyIssueKind::RiskBudgetExceeded
            | VerifyIssueKind::ContractActionRejected
            | VerifyIssueKind::ContractMissing
            | VerifyIssueKind::ContractPolicyViolation
            | VerifyIssueKind::ContractPreferredActionAvailable
    )
}

fn first_blocking_issue(issues: &[VerifyIssue]) -> Option<&VerifyIssue> {
    issues
        .iter()
        .find(|issue| issue_blocks_in_enforce(issue.kind))
        .or_else(|| issues.first())
}

fn step_permission_decision_json(state: &AppState, step: &PlanStep) -> Value {
    if !matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
        return json!({
            "step_id": step.step_id,
            "action_type": step.action_type,
            "executable": false,
            "decision": crate::policy_decision::PolicyDecision::Deny.as_token(),
        });
    }

    let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
    let effect =
        crate::execution_recipe::classify_skill_action_effect(state, &normalized_skill, &step.args);
    let risk_level = effective_step_risk_level(state, &normalized_skill, &step.args);
    let requires_confirmation = state
        .skill_invocation_requires_confirmation_policy(&normalized_skill, Some(&step.args))
        || is_confirmation_like_skill(&normalized_skill)
        || high_risk_side_effect_requires_confirmation(effect, risk_level, &step.args);
    let action = step
        .args
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(|value| value.as_str())
        .map(normalize_schema_token);
    let registry_policy = action.as_deref().and_then(|action| {
        state
            .skill_manifest(&normalized_skill)
            .and_then(|manifest| {
                manifest
                    .planner_capabilities
                    .into_iter()
                    .find(|mapping| mapping.action.as_deref() == Some(action))
                    .map(|mapping| {
                        json!({
                            "capability": mapping.name,
                            "effect": mapping.effect.map(|effect| effect.as_token()),
                            "risk_level": mapping.risk_level.map(risk_level_token),
                            "once_per_task": mapping.once_per_task,
                            "dedup_scope": mapping.dedup_scope.map(|scope| scope.as_token()),
                            "idempotent": mapping.idempotent,
                        })
                    })
            })
    });
    let decision = if requires_confirmation {
        crate::policy_decision::PolicyDecision::RequireConfirmation
    } else {
        crate::policy_decision::PolicyDecision::Allow
    };

    json!({
        "step_id": step.step_id,
        "action_type": step.action_type,
        "executable": true,
        "decision": decision.as_token(),
        "skill": normalized_skill,
        "action": action,
        "action_effect": {
            "observes": effect.observes,
            "mutates": effect.mutates,
            "validates": effect.validates,
        },
        "risk_level": risk_level_token(risk_level),
        "requires_confirmation": requires_confirmation,
        "registry_policy": registry_policy,
    })
}

fn verify_permission_decision_json(
    state: &AppState,
    plan_result: &PlanResult,
    mode: VerifyMode,
    approved: bool,
    needs_confirmation: bool,
    blocked_reason: Option<&str>,
    shadow_blocked_reason: Option<&str>,
    issues: &[VerifyIssue],
) -> Value {
    let first_issue = first_blocking_issue(issues);
    let denied_by_policy = issues
        .iter()
        .any(|issue| issue_is_policy_denial(issue.kind));
    let decision = crate::policy_decision::PolicyDecision::from_permission_flags(
        approved,
        needs_confirmation,
        denied_by_policy,
        false,
    );
    json!({
        "schema_version": 1,
        "owner_layer": "plan_verifier",
        "mode": mode.as_str(),
        "decision": decision.as_token(),
        "allowed": approved && !needs_confirmation,
        "approved": approved,
        "needs_confirmation": needs_confirmation,
        "denied_by_policy": denied_by_policy,
        "dry_run_required": false,
        "external_provider_blocked": false,
        "reason_code": first_issue
            .map(|issue| issue.kind.reason_code())
            .unwrap_or("verify_allowed"),
        "status_code": first_issue
            .map(|issue| issue.kind.status_code())
            .unwrap_or("allowed"),
        "message_key": first_issue
            .map(|issue| issue.kind.message_key())
            .unwrap_or("clawd.verify.allowed"),
        "issue_count": issues.len(),
        "blocked_reason_present": blocked_reason
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        "shadow_blocked_reason_present": shadow_blocked_reason
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        "steps": plan_result
            .steps
            .iter()
            .map(|step| step_permission_decision_json(state, step))
            .collect::<Vec<_>>(),
    })
}

fn audit_permission_decision(state: &AppState, task: &ClaimedTask, permission_decision: &Value) {
    let detail = json!({
        "task_id": task.task_id,
        "permission_decision": permission_decision,
    })
    .to_string();
    if let Err(err) = crate::repo::insert_audit_log(
        state,
        Some(task.user_id),
        "plan_verifier.permission_decision",
        Some(&detail),
        None,
    ) {
        tracing::warn!(error = %err, "plan_verifier_permission_decision_audit_failed");
    }
}

fn risk_exceeds_ceiling(risk: SkillRiskLevel, risk_ceiling: crate::RiskCeiling) -> bool {
    let risk_rank = match risk {
        SkillRiskLevel::Unknown => 0,
        SkillRiskLevel::Low => 1,
        SkillRiskLevel::Medium => 2,
        SkillRiskLevel::High => 3,
    };
    let ceiling_rank = match risk_ceiling {
        crate::RiskCeiling::Unknown | crate::RiskCeiling::High => return false,
        crate::RiskCeiling::Low => 1,
        crate::RiskCeiling::Medium => 2,
    };
    risk_rank > ceiling_rank
}

fn arg_value_is_present(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Array(values) => values.iter().any(arg_value_is_present),
        serde_json::Value::Object(values) => !values.is_empty(),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
    }
}

fn required_arg_satisfied(
    obj: &serde_json::Map<String, serde_json::Value>,
    required: &str,
) -> bool {
    required
        .split('|')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .any(|key| obj.get(key).is_some_and(arg_value_is_present))
}

fn push_group_conflict_issues(
    issues: &mut Vec<VerifyIssue>,
    group: &str,
    entries: &[(String, String, PrimaryFallbackRole)],
    detail: String,
) {
    for (step_id, normalized_skill, _) in entries {
        issues.push(VerifyIssue {
            step_id: step_id.clone(),
            kind: VerifyIssueKind::PrimaryFallbackConflict,
            detail: format!("group `{group}` skill `{normalized_skill}` conflict: {detail}"),
            missing_fields: Vec::new(),
        });
    }
}

fn verify_primary_fallback_conflicts(
    state: &AppState,
    plan_result: &PlanResult,
    issues: &mut Vec<VerifyIssue>,
) {
    let mut grouped: HashMap<String, Vec<(String, String, PrimaryFallbackRole)>> = HashMap::new();

    for step in &plan_result.steps {
        if !matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
            continue;
        }
        let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
        let Some(manifest) = state.skill_manifest(&normalized_skill) else {
            continue;
        };
        let Some(group) = manifest
            .group
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let role = manifest
            .primary_fallback_role
            .unwrap_or(PrimaryFallbackRole::None);
        if matches!(role, PrimaryFallbackRole::None) {
            continue;
        }
        grouped.entry(group.to_string()).or_default().push((
            step.step_id.clone(),
            normalized_skill,
            role,
        ));
    }

    for (group, entries) in grouped {
        let primary_skills = entries
            .iter()
            .filter(|(_, _, role)| matches!(role, PrimaryFallbackRole::Primary))
            .map(|(_, normalized_skill, _)| normalized_skill.as_str())
            .collect::<HashSet<_>>();
        let fallback_skills = entries
            .iter()
            .filter(|(_, _, role)| matches!(role, PrimaryFallbackRole::Fallback))
            .map(|(_, normalized_skill, _)| normalized_skill.as_str())
            .collect::<HashSet<_>>();

        if !primary_skills.is_empty() && !fallback_skills.is_empty() {
            push_group_conflict_issues(
                issues,
                &group,
                &entries,
                "both primary and fallback steps are present in the same plan".to_string(),
            );
            continue;
        }
        if primary_skills.len() > 1 {
            push_group_conflict_issues(
                issues,
                &group,
                &entries,
                "multiple primary steps are present in the same group".to_string(),
            );
            continue;
        }
        if fallback_skills.len() > 1 {
            push_group_conflict_issues(
                issues,
                &group,
                &entries,
                "multiple fallback steps are present in the same group".to_string(),
            );
        }
    }
}

fn verify_step_args(
    state: &AppState,
    step: &PlanStep,
    normalized_skill: &str,
    template_scope: &TemplatePlaceholderScope,
    issues: &mut Vec<VerifyIssue>,
) {
    let has_unresolved_template = value_contains_unresolved_template(&step.args, template_scope);
    let manifest_required = manifest_required_args(state, normalized_skill);
    let fallback_required = required_args_for_skill(normalized_skill);
    let mut required: Vec<String> = if manifest_required.is_empty() {
        fallback_required
            .iter()
            .map(|key| (*key).to_string())
            .collect()
    } else {
        manifest_required
    };
    for key in action_scoped_required_args(state, normalized_skill, &step.args) {
        if !required.iter().any(|existing| existing == &key) {
            required.push(key);
        }
    }
    if has_unresolved_template {
        issues.push(VerifyIssue {
            step_id: step.step_id.clone(),
            kind: VerifyIssueKind::UnresolvedTemplateArg,
            detail: format!(
                "skill `{normalized_skill}` has unresolved template placeholder in args"
            ),
            missing_fields: Vec::new(),
        });
    }
    if required.is_empty() {
        return;
    }
    let Some(obj) = step.args.as_object() else {
        issues.push(VerifyIssue {
            step_id: step.step_id.clone(),
            kind: VerifyIssueKind::MissingRequiredArg,
            detail: format!("skill `{normalized_skill}` args must be an object"),
            missing_fields: required.clone(),
        });
        return;
    };
    for key in &required {
        if !required_arg_satisfied(obj, key) {
            issues.push(VerifyIssue {
                step_id: step.step_id.clone(),
                kind: VerifyIssueKind::MissingRequiredArg,
                detail: format!("skill `{normalized_skill}` missing required arg `{key}`"),
                missing_fields: vec![key.clone()],
            });
        }
    }
}

fn issue_blocks_in_enforce(kind: VerifyIssueKind) -> bool {
    matches!(
        kind,
        VerifyIssueKind::SkillNotVisible
            | VerifyIssueKind::CapabilityUnavailable
            | VerifyIssueKind::MissingRequiredArg
            | VerifyIssueKind::UnresolvedTemplateArg
            | VerifyIssueKind::InvalidDependsOn
            | VerifyIssueKind::PrimaryFallbackConflict
            | VerifyIssueKind::RiskBudgetExceeded
            | VerifyIssueKind::RouteClarifyRequired
            | VerifyIssueKind::RecipeInspectBeforeMutateRequired
            | VerifyIssueKind::RecipeValidationAfterMutateRequired
            | VerifyIssueKind::RecipeTargetScopeRequired
            | VerifyIssueKind::ContractActionRejected
            | VerifyIssueKind::ContractMissing
            | VerifyIssueKind::ContractPolicyViolation
    )
}

fn route_requires_contract(route_result: Option<&crate::RouteResult>) -> bool {
    route_result
        .map(|route| {
            route.output_contract.semantic_kind != crate::OutputSemanticKind::None
                || route.output_contract.requires_content_evidence
                || route.output_contract.delivery_required
        })
        .unwrap_or(false)
}

fn route_requires_clarify_before_tools(
    state: &AppState,
    route_result: Option<&crate::RouteResult>,
    plan_result: &PlanResult,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route.needs_clarify {
        return false;
    }
    let has_executable_step = plan_result.steps.iter().any(|step| {
        matches!(
            step.action_type.as_str(),
            "call_skill" | "call_tool" | "call_capability"
        )
    });
    if !has_executable_step {
        return false;
    }
    !route_clarify_can_defer_to_runtime_status_plan(state, route, plan_result)
        && !route_clarify_can_defer_to_subagent_review_boundary_surface_plan(
            state,
            route,
            plan_result,
        )
}

fn route_clarify_can_defer_to_runtime_status_plan(
    state: &AppState,
    route: &crate::RouteResult,
    plan_result: &PlanResult,
) -> bool {
    if !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.wants_file_delivery
        || route.output_contract.locator_kind != crate::OutputLocatorKind::None
        || !route.output_contract.locator_hint.trim().is_empty()
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
    {
        return false;
    }
    let mut saw_runtime_status_observation = false;
    for step in plan_result.steps.iter().filter(|step| {
        matches!(
            step.action_type.as_str(),
            "call_skill" | "call_tool" | "call_capability"
        )
    }) {
        match step.action_type.as_str() {
            "call_skill" | "call_tool" => {
                let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
                if normalized_skill != "system_basic"
                    || step.args.get("action").and_then(serde_json::Value::as_str)
                        != Some("runtime_status")
                    || step
                        .args
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .is_none_or(str::is_empty)
                {
                    return false;
                }
                saw_runtime_status_observation = true;
            }
            "call_capability" => {
                if step.skill.trim() != "system.runtime_status"
                    || step
                        .args
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .is_none_or(str::is_empty)
                {
                    return false;
                }
                saw_runtime_status_observation = true;
            }
            _ => return false,
        }
    }
    saw_runtime_status_observation
}

fn route_clarify_can_defer_to_subagent_review_boundary_surface_plan(
    state: &AppState,
    route: &crate::RouteResult,
    plan_result: &PlanResult,
) -> bool {
    if !is_subagent_review_boundary_surface_plan(plan_result)
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.wants_file_delivery
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }

    let mut saw_subagent = false;
    let mut saw_read_range = false;
    for step in plan_result.steps.iter().filter(|step| {
        matches!(
            step.action_type.as_str(),
            "call_skill" | "call_tool" | "call_capability"
        )
    }) {
        if !matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
            return false;
        }
        let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
        if !subagent_review_boundary_surface_action_allowed(
            plan_result,
            &normalized_skill,
            &step.args,
        ) {
            return false;
        }
        match normalized_skill.as_str() {
            "subagent" => saw_subagent = true,
            "fs_basic" => saw_read_range = true,
            _ => {}
        }
    }

    saw_subagent && saw_read_range
}

fn verify_execution_recipe(
    state: &AppState,
    plan_result: &PlanResult,
    recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
    issues: &mut Vec<VerifyIssue>,
) {
    if !recipe.is_active() {
        return;
    }

    let mut observed_before_mutate = recipe.saw_inspect || recipe.saw_validation;
    let mut saw_mutation = recipe.saw_mutation;
    let mut saw_validation_after_mutation = recipe.saw_validation;
    let mut saw_profile_validation_after_mutation = recipe.saw_validation
        && !crate::execution_recipe::profile_requires_specific_validation(recipe.profile);
    let mut first_mutation_step_id: Option<String> = None;
    let mut saw_external_target = recipe.saw_external_target;
    let mut saw_greenfield_creation = recipe.saw_greenfield_creation;
    let mut target_scope_conflict_step_id: Option<String> = None;
    let mut first_action_step_id: Option<String> = None;

    for step in &plan_result.steps {
        if !matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
            continue;
        }
        if first_action_step_id.is_none() {
            first_action_step_id = Some(step.step_id.clone());
        }
        let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
        if crate::execution_recipe::action_targets_external_workspace(
            state,
            &normalized_skill,
            &step.args,
        ) {
            saw_external_target = true;
        }
        if crate::execution_recipe::action_satisfies_greenfield_creation(
            state,
            &normalized_skill,
            &step.args,
        ) {
            saw_greenfield_creation = true;
        }
        if target_scope_conflict_step_id.is_none()
            && crate::execution_recipe::action_conflicts_with_recipe_target_scope(
                recipe,
                state,
                &normalized_skill,
                &step.args,
            )
        {
            target_scope_conflict_step_id = Some(step.step_id.clone());
        }
        let effect = crate::execution_recipe::classify_skill_action_effect(
            state,
            &normalized_skill,
            &step.args,
        );
        if effect.observes {
            observed_before_mutate = true;
        }
        if effect.mutates {
            saw_mutation = true;
            if recipe.inspect_first && !observed_before_mutate && first_mutation_step_id.is_none() {
                first_mutation_step_id = Some(step.step_id.clone());
            }
            saw_validation_after_mutation = false;
        }
        if effect.validates && saw_mutation {
            saw_validation_after_mutation = true;
            if crate::execution_recipe::validation_satisfies_recipe_profile(
                recipe,
                state,
                &normalized_skill,
                &step.args,
            ) {
                saw_profile_validation_after_mutation = true;
            }
        } else if saw_mutation
            && crate::execution_recipe::profile_requires_specific_validation(recipe.profile)
            && crate::execution_recipe::validation_satisfies_recipe_profile(
                recipe,
                state,
                &normalized_skill,
                &step.args,
            )
        {
            saw_validation_after_mutation = true;
            saw_profile_validation_after_mutation = true;
        }
    }

    if let Some(step_id) = first_mutation_step_id {
        issues.push(VerifyIssue {
            step_id,
            kind: VerifyIssueKind::RecipeInspectBeforeMutateRequired,
            detail: "ops_closed_loop requires at least one inspect/read-only evidence step before mutating".to_string(),
            missing_fields: Vec::new(),
        });
    }

    let validation_satisfied =
        if crate::execution_recipe::profile_requires_specific_validation(recipe.profile) {
            saw_profile_validation_after_mutation
        } else {
            saw_validation_after_mutation
        };

    if recipe.validation_required && saw_mutation && !validation_satisfied {
        let step_id = plan_result
            .steps
            .iter()
            .rev()
            .find(|step| matches!(step.action_type.as_str(), "call_skill" | "call_tool"))
            .map(|step| step.step_id.clone())
            .unwrap_or_else(|| "recipe".to_string());
        issues.push(VerifyIssue {
            step_id,
            kind: VerifyIssueKind::RecipeValidationAfterMutateRequired,
            detail: crate::execution_recipe::validation_detail_for_recipe(recipe).to_string(),
            missing_fields: Vec::new(),
        });
    }

    match recipe.target_scope {
        crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo => {
            if let Some(step_id) = target_scope_conflict_step_id {
                issues.push(VerifyIssue {
                    step_id,
                    kind: VerifyIssueKind::RecipeTargetScopeRequired,
                    detail: crate::execution_recipe::target_scope_detail_for_recipe(recipe)
                        .to_string(),
                    missing_fields: Vec::new(),
                });
            }
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace => {
            if target_scope_conflict_step_id.is_none() && !saw_external_target {
                issues.push(VerifyIssue {
                    step_id: first_action_step_id.unwrap_or_default(),
                    kind: VerifyIssueKind::RecipeTargetScopeRequired,
                    detail: crate::execution_recipe::target_scope_detail_for_recipe(recipe)
                        .to_string(),
                    missing_fields: Vec::new(),
                });
            } else if let Some(step_id) = target_scope_conflict_step_id {
                issues.push(VerifyIssue {
                    step_id,
                    kind: VerifyIssueKind::RecipeTargetScopeRequired,
                    detail: crate::execution_recipe::target_scope_detail_for_recipe(recipe)
                        .to_string(),
                    missing_fields: Vec::new(),
                });
            }
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield => {
            if !saw_greenfield_creation {
                issues.push(VerifyIssue {
                    step_id: first_action_step_id.unwrap_or_default(),
                    kind: VerifyIssueKind::RecipeTargetScopeRequired,
                    detail: crate::execution_recipe::target_scope_detail_for_recipe(recipe)
                        .to_string(),
                    missing_fields: Vec::new(),
                });
            }
        }
        crate::execution_recipe::ExecutionRecipeTargetScope::Unknown
        | crate::execution_recipe::ExecutionRecipeTargetScope::System => {}
    }
}

fn rewrite_execution_recipe_steps(
    state: &AppState,
    _route_result: Option<&crate::RouteResult>,
    _request_text: Option<&str>,
    plan_result: &PlanResult,
    recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
) -> Vec<PlanStep> {
    if !matches!(
        recipe.kind,
        crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop
    ) {
        return Vec::new();
    }
    let plan_has_mutation = plan_result.steps.iter().any(|step| {
        matches!(step.action_type.as_str(), "call_skill" | "call_tool")
            && crate::execution_recipe::classify_skill_action_effect(state, &step.skill, &step.args)
                .mutates
    });
    let mut changed = false;
    let mut saw_mutation_in_plan = false;
    let mut rewritten = Vec::with_capacity(plan_result.steps.len() + 2);
    for step in &plan_result.steps {
        if matches!(
            recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Apply
        ) && plan_has_mutation
            && !saw_mutation_in_plan
            && matches!(step.action_type.as_str(), "call_skill" | "call_tool")
        {
            let effect = crate::execution_recipe::classify_skill_action_effect(
                state,
                &step.skill,
                &step.args,
            );
            if apply_phase_pre_mutation_step_should_be_skipped(state, step, effect) {
                changed = true;
                continue;
            }
        }
        if matches!(step.action_type.as_str(), "call_skill" | "call_tool")
            && crate::execution_recipe::classify_skill_action_effect(state, &step.skill, &step.args)
                .mutates
        {
            saw_mutation_in_plan = true;
        }
        if matches!(step.action_type.as_str(), "call_skill" | "call_tool")
            && state.resolve_canonical_skill_name(&step.skill) == "run_cmd"
        {
            if let Some(command) = step.args.get("command").and_then(|value| value.as_str()) {
                if let Some((mutate_command, validate_command)) =
                    crate::execution_recipe::split_run_cmd_mutation_and_validation(command)
                {
                    let mut mutate_step = step.clone();
                    if let Some(obj) = mutate_step.args.as_object_mut() {
                        obj.insert(
                            "command".to_string(),
                            serde_json::Value::String(mutate_command),
                        );
                        obj.remove("timeout_seconds");
                    }
                    let mut validate_step = step.clone();
                    validate_step.step_id = format!("{}__validate", step.step_id);
                    validate_step.depends_on = vec![step.step_id.clone()];
                    if let Some(obj) = validate_step.args.as_object_mut() {
                        obj.insert(
                            "command".to_string(),
                            serde_json::Value::String(validate_command),
                        );
                        obj.remove("timeout_seconds");
                    }
                    rewritten.push(mutate_step);
                    rewritten.push(validate_step);
                    changed = true;
                    continue;
                }
            }
        }
        rewritten.push(step.clone());
    }
    if changed {
        rewritten
    } else {
        Vec::new()
    }
}

fn apply_phase_pre_mutation_step_should_be_skipped(
    state: &AppState,
    step: &PlanStep,
    effect: crate::execution_recipe::ActionEffect,
) -> bool {
    if effect.mutates {
        return false;
    }
    if effect.validates {
        return true;
    }
    let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
    matches!(
        normalized_skill.as_str(),
        "http_basic" | "health_check" | "service_control"
    )
}

fn first_shadow_blocked_reason(issues: &[VerifyIssue]) -> Option<String> {
    issues
        .iter()
        .find(|issue| issue_blocks_in_enforce(issue.kind))
        .map(|issue| issue.detail.clone())
}

fn unresolved_template_response_step(_request_text: Option<&str>) -> PlanStep {
    let content = serde_json::json!({
        "message_key": "clawd.msg.verify.unresolved_template_arg",
        "reason_code": "verify_unresolved_template_arg",
        "missing_slots": ["concrete_input"],
        "accepted_input_kinds": ["json_array", "file_path", "previous_result"],
    })
    .to_string();
    PlanStep {
        step_id: "verify_unresolved_template_response".to_string(),
        action_type: "respond".to_string(),
        skill: "respond".to_string(),
        args: serde_json::json!({ "content": content }),
        depends_on: Vec::new(),
        why: "unresolved template placeholder in executable step args".to_string(),
    }
}

fn unresolved_capability_response_step(_request_text: Option<&str>, capability: &str) -> PlanStep {
    let content = serde_json::json!({
        "message_key": "clawd.msg.verify.capability_unavailable",
        "reason_code": "verify_capability_unavailable",
        "capability": capability,
    })
    .to_string();
    PlanStep {
        step_id: "verify_unresolved_capability_response".to_string(),
        action_type: "respond".to_string(),
        skill: "respond".to_string(),
        args: serde_json::json!({ "content": content }),
        depends_on: Vec::new(),
        why: "unresolved runtime capability before execution".to_string(),
    }
}

pub(crate) fn verify_plan(
    state: &AppState,
    task: &ClaimedTask,
    input: VerifyInput<'_>,
    mode: VerifyMode,
) -> VerifyResult {
    let mut effective_plan_result = input.plan_result.clone();
    let mut issues = Vec::new();
    apply_default_creation_targets(state, task, &mut effective_plan_result, &mut issues);
    apply_generated_file_path_report_write_repair(
        state,
        input.route_result,
        &mut effective_plan_result,
    );
    structured_fields::apply_structured_field_selector_repair(
        state,
        input.route_result,
        input.request_text,
        &mut effective_plan_result,
        &mut issues,
    );

    let route_requires_clarify_before_tools =
        route_requires_clarify_before_tools(state, input.route_result, input.plan_result);
    let visible_skills: HashSet<String> = state
        .planner_available_skills_for_task(task)
        .into_iter()
        .collect();
    let all_step_ids: HashSet<String> = effective_plan_result
        .steps
        .iter()
        .map(|step| step.step_id.clone())
        .collect();
    let confirmation_already_granted = route_has_confirmation_resume(input.route_result);
    let scratch_filesystem_lifecycle_plan = input.route_result.is_some_and(|route| {
        crate::agent_engine::route_can_upgrade_scratch_filesystem_lifecycle(route)
            && crate::agent_engine::scratch_filesystem_lifecycle_plan_steps_match(
                state,
                &effective_plan_result.steps,
            )
    });
    let mut needs_confirmation = false;
    if route_requires_clarify_before_tools {
        issues.push(VerifyIssue {
            step_id: "route".to_string(),
            kind: VerifyIssueKind::RouteClarifyRequired,
            detail: format!(
                "route requires clarify before execution; context={}",
                input.context_bundle_summary.unwrap_or("<none>")
            ),
            missing_fields: Vec::new(),
        });
    }
    let route_contract_missing = input
        .route_result
        .filter(|_| route_requires_contract(input.route_result))
        .is_some_and(|route| {
            crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
                .is_none()
        });
    if route_contract_missing {
        let semantic_kind = input
            .route_result
            .map(|route| route.output_contract.semantic_kind.as_str())
            .unwrap_or("unknown");
        issues.push(VerifyIssue {
            step_id: "route".to_string(),
            kind: VerifyIssueKind::ContractMissing,
            detail: format!("no contract matrix entry matched semantic kind `{semantic_kind}`"),
            missing_fields: Vec::new(),
        });
    }

    let mut template_scope = TemplatePlaceholderScope::default();
    for (idx, step) in effective_plan_result.steps.iter().enumerate() {
        if step.action_type == "call_capability" {
            issues.push(VerifyIssue {
                step_id: step.step_id.clone(),
                kind: VerifyIssueKind::CapabilityUnavailable,
                detail: format!(
                    "capability `{}` was not resolved to an executable tool or skill",
                    step.skill
                ),
                missing_fields: Vec::new(),
            });
        } else if matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
            let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
            if !visible_skills.contains(&normalized_skill)
                && !planner_internal_tool_is_visible(&normalized_skill)
            {
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::SkillNotVisible,
                    detail: format!("skill `{normalized_skill}` is not in planner visible skills"),
                    missing_fields: Vec::new(),
                });
            }
            verify_step_args(state, step, &normalized_skill, &template_scope, &mut issues);
            let subagent_review_boundary_surface_action_allowed =
                subagent_review_boundary_surface_action_allowed(
                    &effective_plan_result,
                    &normalized_skill,
                    &step.args,
                );
            if let Some(policy) = crate::contract_matrix::action_policy_for_output_contract(
                input.route_result.map(|route| &route.output_contract),
                &normalized_skill,
                &step.args,
            ) {
                if !policy.is_allowed()
                    && !crate::agent_engine::action_has_user_named_output_path_marker(&step.args)
                    && !(scratch_filesystem_lifecycle_plan
                        && crate::agent_engine::scratch_filesystem_lifecycle_action_allowed(
                            state,
                            &normalized_skill,
                            &step.args,
                        ))
                    && !registry_action_can_extend_summary_contract(
                        state,
                        &normalized_skill,
                        &step.args,
                        &policy.contract_match,
                    )
                    && !subagent_review_boundary_surface_action_allowed
                {
                    issues.push(VerifyIssue {
                        step_id: step.step_id.clone(),
                        kind: VerifyIssueKind::ContractActionRejected,
                        detail: format!(
                            "action `{}` is rejected by contract `{}` ({}) with final answer shape `{}`",
                            policy.action_key,
                            policy.contract_match,
                            policy.decision.as_str(),
                            policy.final_answer_shape
                        ),
                        missing_fields: Vec::new(),
                    });
                } else if !policy.preferred_actions.is_empty()
                    && !policy.action_matches_preferred()
                    && !subagent_review_boundary_surface_action_allowed
                {
                    issues.push(VerifyIssue {
                        step_id: step.step_id.clone(),
                        kind: VerifyIssueKind::ContractPreferredActionAvailable,
                        detail: format!(
                            "action `{}` is allowed by contract `{}` but preferred action(s) are `{}`",
                            policy.action_key,
                            policy.contract_match,
                            policy.preferred_actions.join(",")
                        ),
                        missing_fields: Vec::new(),
                    });
                }
            } else if route_requires_contract(input.route_result)
                && !route_contract_missing
                && crate::contract_matrix::ActionRef::from_skill_args(&normalized_skill, &step.args)
                    .is_none()
                && !subagent_review_boundary_surface_action_allowed
            {
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::ContractPolicyViolation,
                    detail: format!(
                        "planner step skill `{}` could not be converted to a contract action reference",
                        step.skill
                    ),
                    missing_fields: Vec::new(),
                });
            }
            let safe_autonomous_creation =
                safe_autonomous_creation_step(state, &normalized_skill, &step.args);
            let step_risk = effective_step_risk_level(state, &normalized_skill, &step.args);
            let effect = crate::execution_recipe::classify_skill_action_effect(
                state,
                &normalized_skill,
                &step.args,
            );
            if input
                .route_result
                .is_some_and(|route| risk_exceeds_ceiling(step_risk, route.risk_ceiling))
            {
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::RiskBudgetExceeded,
                    detail: format!(
                        "skill `{normalized_skill}` action risk `{:?}` exceeds route risk ceiling",
                        step_risk
                    ),
                    missing_fields: Vec::new(),
                });
            }
            if !confirmation_already_granted
                && !safe_autonomous_creation
                && !registry_declares_non_mutating_planner_action(
                    state,
                    &normalized_skill,
                    &step.args,
                )
                && (state.skill_invocation_requires_confirmation_policy(
                    &normalized_skill,
                    Some(&step.args),
                ) || is_confirmation_like_skill(&normalized_skill)
                    || high_risk_side_effect_requires_confirmation(effect, step_risk, &step.args))
            {
                needs_confirmation = true;
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::ConfirmationRequired,
                    detail: format!("skill `{normalized_skill}` may require explicit confirmation"),
                    missing_fields: Vec::new(),
                });
            }
        }

        for dep in &step.depends_on {
            if !all_step_ids.contains(dep) {
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::InvalidDependsOn,
                    detail: format!("depends_on references missing step `{dep}`"),
                    missing_fields: Vec::new(),
                });
            }
        }
        if step_can_produce_output_for_template_scope(step) {
            template_scope.register_step_output(step, idx + 1);
        }
    }

    verify_primary_fallback_conflicts(state, &effective_plan_result, &mut issues);
    verify_execution_recipe(
        state,
        &effective_plan_result,
        input.execution_recipe,
        &mut issues,
    );

    let shadow_blocked_reason = first_shadow_blocked_reason(&issues);
    let blocked_reason = if matches!(mode, VerifyMode::Enforce) {
        shadow_blocked_reason.clone()
    } else {
        None
    };

    let approved = blocked_reason.is_none();
    let approved_steps = effective_plan_result.steps.clone();
    let permission_decision = verify_permission_decision_json(
        state,
        &effective_plan_result,
        mode,
        approved,
        needs_confirmation,
        blocked_reason.as_deref(),
        shadow_blocked_reason.as_deref(),
        &issues,
    );
    audit_permission_decision(state, task, &permission_decision);
    let rewritten_steps = if issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::UnresolvedTemplateArg))
    {
        vec![unresolved_template_response_step(input.request_text)]
    } else if let Some(issue) = issues
        .iter()
        .find(|issue| matches!(issue.kind, VerifyIssueKind::CapabilityUnavailable))
    {
        let capability = issue
            .detail
            .split('`')
            .nth(1)
            .unwrap_or("unknown capability");
        vec![unresolved_capability_response_step(
            input.request_text,
            capability,
        )]
    } else {
        rewrite_execution_recipe_steps(
            state,
            input.route_result,
            input.request_text,
            &effective_plan_result,
            input.execution_recipe,
        )
    };

    VerifyResult {
        mode,
        approved,
        blocked_reason,
        shadow_blocked_reason,
        permission_decision,
        approved_steps,
        needs_confirmation,
        rewritten_steps,
        issues,
    }
}

fn planner_internal_tool_is_visible(normalized_skill: &str) -> bool {
    matches!(normalized_skill, "subagent")
}

fn is_subagent_review_boundary_surface_plan(plan_result: &PlanResult) -> bool {
    plan_result.raw_plan_text.trim() == "deterministic:subagent_review_boundary_surface"
}

fn subagent_review_boundary_surface_action_allowed(
    plan_result: &PlanResult,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    if !is_subagent_review_boundary_surface_plan(plan_result) {
        return false;
    }
    match normalized_skill {
        "subagent" => {
            args.get("role")
                .and_then(Value::as_str)
                .map(normalize_schema_token)
                .is_some_and(|role| role == "review")
                && args.get("objective").and_then(Value::as_str).map(str::trim)
                    == Some("runtime_boundary_alignment_audit")
        }
        "fs_basic" => args
            .get("action")
            .and_then(Value::as_str)
            .map(normalize_schema_token)
            .is_some_and(|action| action == "read_text_range"),
        _ => false,
    }
}

#[cfg(test)]
#[path = "verifier_tests.rs"]
mod tests;
