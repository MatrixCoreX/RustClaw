use std::collections::{HashMap, HashSet};

use claw_core::skill_registry::{PlannerCapabilityEffect, PrimaryFallbackRole, SkillRiskLevel};
use serde_json::Value;

use crate::{evidence_policy::FailureAttribution, AppState, ClaimedTask, PlanResult, PlanStep};

#[path = "verifier_creation_targets.rs"]
mod creation_targets;
#[path = "verifier_permission.rs"]
mod permission;
#[path = "verifier_risk_policy.rs"]
mod risk_policy;
#[path = "verifier_templates.rs"]
mod templates;

use creation_targets::{apply_default_creation_targets, safe_autonomous_creation_step};
use permission::{
    audit_permission_decision, context_bundle_has_redacted_workspace_child_locator,
    push_unbound_locator_boundary_clarify_issue, step_reads_path_content_under_unbound_locator,
    step_sandbox_denial_reason, validation_run_cmd_can_run_autonomously,
    verify_permission_decision_json, workspace_filesystem_mutation_can_run_autonomously,
};
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
    InvalidArgumentValue,
    DefaultCreationTargetApplied,
    UnresolvedTemplateArg,
    InvalidDependsOn,
    ConfirmationRequired,
    SandboxPolicyDenied,
    #[cfg_attr(not(test), allow(dead_code))]
    RiskBudgetExceeded,
    PrimaryFallbackConflict,
    BoundaryClarifyRequired,
    RecipeInspectBeforeMutateRequired,
    RecipeValidationAfterMutateRequired,
    RecipeTargetScopeRequired,
    #[cfg_attr(not(test), allow(dead_code))]
    ContractActionRejected,
    ContractMissing,
    ContractPolicyViolation,
    #[cfg_attr(not(test), allow(dead_code))]
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
            Self::InvalidArgumentValue => "InvalidArgumentValue",
            Self::DefaultCreationTargetApplied => "DefaultCreationTargetApplied",
            Self::UnresolvedTemplateArg => "UnresolvedTemplateArg",
            Self::InvalidDependsOn => "InvalidDependsOn",
            Self::ConfirmationRequired => "ConfirmationRequired",
            Self::SandboxPolicyDenied => "SandboxPolicyDenied",
            Self::RiskBudgetExceeded => "RiskBudgetExceeded",
            Self::PrimaryFallbackConflict => "PrimaryFallbackConflict",
            Self::BoundaryClarifyRequired => "BoundaryClarifyRequired",
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
            | Self::InvalidArgumentValue
            | Self::UnresolvedTemplateArg
            | Self::InvalidDependsOn
            | Self::PrimaryFallbackConflict
            | Self::BoundaryClarifyRequired => FailureAttribution::ModelError,
            Self::DefaultCreationTargetApplied
            | Self::RecipeInspectBeforeMutateRequired
            | Self::RecipeValidationAfterMutateRequired
            | Self::RecipeTargetScopeRequired => FailureAttribution::CodeGap,
            Self::ConfirmationRequired | Self::SandboxPolicyDenied | Self::RiskBudgetExceeded => {
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
            Self::InvalidArgumentValue => "verify_invalid_argument_value",
            Self::DefaultCreationTargetApplied => "verify_default_creation_target_applied",
            Self::UnresolvedTemplateArg => "verify_unresolved_template_arg",
            Self::InvalidDependsOn => "verify_invalid_depends_on",
            Self::ConfirmationRequired => "verify_confirmation_required",
            Self::SandboxPolicyDenied => "verify_sandbox_policy_denied",
            Self::RiskBudgetExceeded => "verify_risk_budget_exceeded",
            Self::PrimaryFallbackConflict => "verify_primary_fallback_conflict",
            Self::BoundaryClarifyRequired => "verify_boundary_clarify_required",
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
            Self::InvalidArgumentValue => "invalid_argument_value",
            Self::DefaultCreationTargetApplied => "default_creation_target_applied",
            Self::UnresolvedTemplateArg => "unresolved_template_arg",
            Self::InvalidDependsOn => "invalid_depends_on",
            Self::ConfirmationRequired => "confirmation_required",
            Self::SandboxPolicyDenied => "sandbox_policy_denied",
            Self::RiskBudgetExceeded => "risk_budget_exceeded",
            Self::PrimaryFallbackConflict => "primary_fallback_conflict",
            Self::BoundaryClarifyRequired => "boundary_clarify_required",
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
            Self::InvalidArgumentValue => "clawd.verify.invalid_argument_value",
            Self::DefaultCreationTargetApplied => "clawd.verify.default_creation_target_applied",
            Self::UnresolvedTemplateArg => "clawd.verify.unresolved_template_arg",
            Self::InvalidDependsOn => "clawd.verify.invalid_depends_on",
            Self::ConfirmationRequired => "clawd.verify.confirmation_required",
            Self::SandboxPolicyDenied => "clawd.verify.sandbox_policy_denied",
            Self::RiskBudgetExceeded => "clawd.verify.risk_budget_exceeded",
            Self::PrimaryFallbackConflict => "clawd.verify.primary_fallback_conflict",
            Self::BoundaryClarifyRequired => "clawd.verify.boundary_clarify_required",
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
    pub(crate) output_contract: Option<&'a crate::IntentOutputContract>,
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
    pub(crate) capability_resolutions: Vec<VerifiedCapabilityResolution>,
}

#[derive(Debug, Clone)]
pub(crate) struct VerifiedCapabilityResolution {
    pub(crate) plan_step_index: usize,
    pub(crate) plan_step_id: String,
    pub(crate) record: crate::capability_resolver::CapabilityResolutionRecord,
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

fn manifest_required_args(state: &AppState, normalized_skill: &str) -> Vec<String> {
    if let Some(tool) = state.mcp_tool(normalized_skill) {
        return tool.required_args;
    }
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
    let action = args
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(|value| value.as_str())
        .map(normalize_schema_token)
        .filter(|value| !value.is_empty());
    state
        .skill_manifest(normalized_skill)
        .and_then(|manifest| {
            claw_core::skill_registry::select_planner_capability_mapping(
                &manifest.planner_capabilities,
                action.as_deref(),
            )
            .map(|mapping| mapping.required.clone())
        })
        .unwrap_or_default()
}

fn action_scoped_risk_level(
    state: &AppState,
    normalized_skill: &str,
    args: &serde_json::Value,
) -> Option<SkillRiskLevel> {
    if let Some(tool) = state.mcp_tool(normalized_skill) {
        return Some(match tool.policy.risk_level.as_str() {
            "low" => SkillRiskLevel::Low,
            "medium" => SkillRiskLevel::Medium,
            "high" => SkillRiskLevel::High,
            _ => SkillRiskLevel::Unknown,
        });
    }
    let action = args
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(|value| value.as_str())
        .map(normalize_schema_token)
        .filter(|value| !value.is_empty());
    state.skill_manifest(normalized_skill).and_then(|manifest| {
        claw_core::skill_registry::select_planner_capability_mapping(
            &manifest.planner_capabilities,
            action.as_deref(),
        )
        .and_then(|mapping| mapping.risk_level)
    })
}

fn registry_declares_non_mutating_planner_action(
    state: &AppState,
    normalized_skill: &str,
    args: &serde_json::Value,
) -> bool {
    if let Some(tool) = state.mcp_tool(normalized_skill) {
        return matches!(tool.policy.effect.as_str(), "observe" | "validate");
    }
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
            claw_core::skill_registry::select_planner_capability_mapping(
                &manifest.planner_capabilities,
                Some(action.as_str()),
            )
            .is_some_and(|mapping| {
                matches!(
                    mapping.effect,
                    Some(PlannerCapabilityEffect::Observe | PlannerCapabilityEffect::Validate)
                )
            })
        })
}

fn effective_step_risk_level(
    state: &AppState,
    normalized_skill: &str,
    args: &serde_json::Value,
) -> SkillRiskLevel {
    if crate::execution_recipe::dry_run_observes_only_action(normalized_skill, args) {
        return SkillRiskLevel::Low;
    }
    if normalized_skill == "run_cmd"
        && crate::execution_recipe::action_targets_external_workspace(state, normalized_skill, args)
    {
        return SkillRiskLevel::High;
    }
    if validation_run_cmd_can_run_autonomously(state, normalized_skill, args) {
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
            | VerifyIssueKind::SandboxPolicyDenied
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
    for violation in
        crate::schema_contract::executable_enum_violations(state, normalized_skill, &step.args)
    {
        issues.push(VerifyIssue {
            step_id: step.step_id.clone(),
            kind: VerifyIssueKind::InvalidArgumentValue,
            detail: format!(
                "error_code=invalid_argument_value field={} constraint=enum",
                violation.field
            ),
            missing_fields: Vec::new(),
        });
    }
    for violation in crate::schema_contract::executable_type_constraint_violations(
        state,
        normalized_skill,
        &step.args,
    ) {
        issues.push(VerifyIssue {
            step_id: step.step_id.clone(),
            kind: VerifyIssueKind::InvalidArgumentValue,
            detail: format!(
                "error_code=invalid_argument_value field={} constraint=type expected={}",
                violation.field, violation.expected
            ),
            missing_fields: Vec::new(),
        });
    }
    for violation in crate::schema_contract::executable_unknown_argument_violations(
        state,
        normalized_skill,
        &step.args,
    ) {
        issues.push(VerifyIssue {
            step_id: step.step_id.clone(),
            kind: VerifyIssueKind::InvalidArgumentValue,
            detail: format!(
                "error_code=invalid_argument_value field={} constraint=declared_property",
                violation.field
            ),
            missing_fields: Vec::new(),
        });
    }
    for violation in crate::schema_contract::executable_nested_required_constraint_violations(
        state,
        normalized_skill,
        &step.args,
    ) {
        issues.push(VerifyIssue {
            step_id: step.step_id.clone(),
            kind: VerifyIssueKind::MissingRequiredArg,
            detail: format!(
                "error_code=missing_required_arg field={} constraint=schema_required",
                violation.field
            ),
            missing_fields: vec![violation.field],
        });
    }
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

pub(crate) fn issue_blocks_in_enforce(kind: VerifyIssueKind) -> bool {
    matches!(
        kind,
        VerifyIssueKind::SkillNotVisible
            | VerifyIssueKind::CapabilityUnavailable
            | VerifyIssueKind::MissingRequiredArg
            | VerifyIssueKind::InvalidArgumentValue
            | VerifyIssueKind::UnresolvedTemplateArg
            | VerifyIssueKind::InvalidDependsOn
            | VerifyIssueKind::PrimaryFallbackConflict
            | VerifyIssueKind::SandboxPolicyDenied
            | VerifyIssueKind::RiskBudgetExceeded
            | VerifyIssueKind::BoundaryClarifyRequired
            | VerifyIssueKind::RecipeInspectBeforeMutateRequired
            | VerifyIssueKind::RecipeValidationAfterMutateRequired
            | VerifyIssueKind::RecipeTargetScopeRequired
            | VerifyIssueKind::ContractActionRejected
            | VerifyIssueKind::ContractMissing
            | VerifyIssueKind::ContractPolicyViolation
    )
}

fn output_contract_requires_policy(output_contract: Option<&crate::IntentOutputContract>) -> bool {
    output_contract
        .map(|contract| {
            !contract.does_not_request_exact_command_output()
                || contract.requires_content_evidence
                || contract.delivery_required
        })
        .unwrap_or(false)
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
    let execution_policy = crate::task_execution_policy::effective_policy_for_task(state, task);
    let mut effective_plan_result = input.plan_result.clone();
    let capability_resolutions = resolve_capability_plan_steps(state, &mut effective_plan_result);
    let mut issues = Vec::new();
    apply_default_creation_targets(state, task, &mut effective_plan_result, &mut issues);
    let visible_skills: HashSet<String> = state
        .planner_available_skills_for_task(task)
        .into_iter()
        .collect();
    let all_step_ids: HashSet<String> = effective_plan_result
        .steps
        .iter()
        .map(|step| step.step_id.clone())
        .collect();
    let mut needs_confirmation = false;
    let output_contract_policy_missing = input
        .output_contract
        .filter(|_| output_contract_requires_policy(input.output_contract))
        .is_some_and(|contract| {
            crate::evidence_policy::final_answer_shape_for_output_contract(contract).is_none()
        });
    if output_contract_policy_missing {
        issues.push(VerifyIssue {
            step_id: "route".to_string(),
            kind: VerifyIssueKind::ContractMissing,
            detail: "error_code=evidence_policy_entry_missing final_answer_shape=missing"
                .to_string(),
            missing_fields: Vec::new(),
        });
    }

    let mut template_scope = TemplatePlaceholderScope::default();
    let unbound_locator_boundary =
        context_bundle_has_redacted_workspace_child_locator(input.context_bundle_summary)
            || context_bundle_has_redacted_workspace_child_locator(Some(&input.plan_result.goal));
    for (idx, step) in effective_plan_result.steps.iter().enumerate() {
        if step.action_type == "call_capability" {
            if unbound_locator_boundary
                && step_reads_path_content_under_unbound_locator(
                    step,
                    &step.skill,
                    &state.skill_rt.workspace_root,
                )
            {
                push_unbound_locator_boundary_clarify_issue(&mut issues, &step.step_id);
            }
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
                && state.mcp_tool(&normalized_skill).is_none()
            {
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::SkillNotVisible,
                    detail: format!("skill `{normalized_skill}` is not in planner visible skills"),
                    missing_fields: Vec::new(),
                });
            }
            verify_step_args(state, step, &normalized_skill, &template_scope, &mut issues);
            if unbound_locator_boundary
                && step_reads_path_content_under_unbound_locator(
                    step,
                    &normalized_skill,
                    &state.skill_rt.workspace_root,
                )
            {
                push_unbound_locator_boundary_clarify_issue(&mut issues, &step.step_id);
            }
            if output_contract_requires_policy(input.output_contract)
                && !output_contract_policy_missing
                && crate::evidence_policy::ActionRef::from_skill_args(&normalized_skill, &step.args)
                    .is_none()
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
            let autonomous_workspace_fs_mutation =
                workspace_filesystem_mutation_can_run_autonomously(
                    state,
                    &normalized_skill,
                    &step.args,
                );
            let autonomous_validation_run_cmd =
                validation_run_cmd_can_run_autonomously(state, &normalized_skill, &step.args);
            let step_risk = effective_step_risk_level(state, &normalized_skill, &step.args);
            let effect = crate::execution_recipe::classify_skill_action_effect(
                state,
                &normalized_skill,
                &step.args,
            );
            let sandbox_denial =
                step_sandbox_denial_reason(state, execution_policy, &normalized_skill, &step.args);
            if let Some(reason) = sandbox_denial {
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::SandboxPolicyDenied,
                    detail: format!("reason_code={reason}"),
                    missing_fields: Vec::new(),
                });
            }
            let risk_requires_confirmation = !safe_autonomous_creation
                && !autonomous_workspace_fs_mutation
                && !autonomous_validation_run_cmd
                && !registry_declares_non_mutating_planner_action(
                    state,
                    &normalized_skill,
                    &step.args,
                )
                && (state.skill_invocation_requires_confirmation_policy(
                    &normalized_skill,
                    Some(&step.args),
                ) || is_confirmation_like_skill(&normalized_skill)
                    || high_risk_side_effect_requires_confirmation(effect, step_risk, &step.args));
            let step_requires_confirmation = execution_policy.approval_required(
                risk_requires_confirmation,
                effective_plan_result.needs_confirmation,
                effect.mutates || matches!(step_risk, SkillRiskLevel::High),
            );
            if sandbox_denial.is_none() && step_requires_confirmation {
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

    let confirmation_step_ids = crate::approval_grant::confirmation_step_ids(&issues);
    let mut approval_grant_decision = None;
    if matches!(mode, VerifyMode::Enforce)
        && needs_confirmation
        && !issues
            .iter()
            .any(|issue| issue_blocks_in_enforce(issue.kind))
    {
        if let Some(binding) = crate::approval_grant::binding_for_confirmation_steps(
            state,
            &effective_plan_result.steps,
            &confirmation_step_ids,
        ) {
            let outcome = crate::repo::consume_task_approval_grant(state, &task.task_id, &binding)
                .unwrap_or(crate::repo::TaskApprovalConsumeOutcome::Conflict);
            approval_grant_decision = Some(outcome.decision_json(&binding));
            if outcome == crate::repo::TaskApprovalConsumeOutcome::Consumed {
                issues.retain(|issue| issue.kind != VerifyIssueKind::ConfirmationRequired);
                needs_confirmation = false;
            } else if let Ok(Some(scope_grant)) =
                crate::repo::match_approval_scope_grant(state, task, &binding)
            {
                approval_grant_decision = Some(scope_grant.decision_json(&binding));
                issues.retain(|issue| issue.kind != VerifyIssueKind::ConfirmationRequired);
                needs_confirmation = false;
            }
        }
    }

    let shadow_blocked_reason = first_shadow_blocked_reason(&issues);
    let blocked_reason = if matches!(mode, VerifyMode::Enforce) {
        shadow_blocked_reason.clone()
    } else {
        None
    };

    let approved = blocked_reason.is_none();
    let approved_steps = effective_plan_result.steps.clone();
    let mut permission_decision = verify_permission_decision_json(
        state,
        execution_policy,
        &effective_plan_result,
        mode,
        approved,
        needs_confirmation,
        blocked_reason.as_deref(),
        shadow_blocked_reason.as_deref(),
        &issues,
    );
    if let Some(grant_decision) = approval_grant_decision {
        if !needs_confirmation {
            crate::approval_grant::apply_consumed_grant_to_permission_decision(
                &mut permission_decision,
                &confirmation_step_ids,
                grant_decision,
            );
        } else {
            permission_decision["approval_grant"] = grant_decision;
        }
    }
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
        capability_resolutions,
    }
}

fn resolve_capability_plan_steps(
    state: &AppState,
    plan_result: &mut PlanResult,
) -> Vec<VerifiedCapabilityResolution> {
    let mut resolutions = Vec::new();
    for (plan_step_index, step) in plan_result.steps.iter_mut().enumerate() {
        if step.action_type != "call_capability" {
            continue;
        }
        let (resolved, record) =
            crate::capability_resolver::resolve_capability_action_with_record_for_state(
                state,
                &step.skill,
                step.args.clone(),
            );
        resolutions.push(VerifiedCapabilityResolution {
            plan_step_index,
            plan_step_id: step.step_id.clone(),
            record,
        });
        let Some(resolved) = resolved else {
            continue;
        };
        let resolved =
            crate::agent_engine::normalize_resolved_planner_action_for_verifier(state, resolved);
        *step = crate::plan_step_from_agent_action(
            &resolved,
            step.step_id.clone(),
            step.depends_on.clone(),
            step.why.clone(),
        );
    }
    resolutions
}

pub(crate) fn skill_sandbox_denial_reason(
    state: &AppState,
    task: Option<&ClaimedTask>,
    normalized_skill: &str,
    args: &Value,
) -> Option<&'static str> {
    permission::step_sandbox_denial_reason(
        state,
        task.map(|task| crate::task_execution_policy::effective_policy_for_task(state, task))
            .unwrap_or_else(|| crate::task_execution_policy::configured_policy(state)),
        normalized_skill,
        args,
    )
}

pub(crate) fn preview_command_permission_decision(
    state: &AppState,
    command: &str,
    cwd: Option<&str>,
    sudo_allowed: bool,
) -> Value {
    permission::preview_command_permission_decision_json(state, command, cwd, sudo_allowed)
}

fn planner_internal_tool_is_visible(normalized_skill: &str) -> bool {
    matches!(normalized_skill, "subagent")
}

#[cfg(test)]
#[path = "verifier_approval_tests.rs"]
mod approval_tests;
#[cfg(test)]
#[path = "verifier_permission_tests.rs"]
mod permission_tests;
#[cfg(test)]
#[path = "verifier_schema_tests.rs"]
mod schema_tests;
#[cfg(test)]
#[path = "verifier_tests.rs"]
pub(super) mod tests;
