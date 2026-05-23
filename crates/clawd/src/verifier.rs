use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};

use claw_core::skill_registry::{PrimaryFallbackRole, SkillRiskLevel};

use crate::{AppState, ClaimedTask, PlanResult, PlanStep};

#[allow(dead_code)]
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
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VerifyIssue {
    pub(crate) step_id: String,
    pub(crate) kind: VerifyIssueKind,
    pub(crate) detail: String,
}

pub(crate) struct VerifyInput<'a> {
    pub(crate) route_result: Option<&'a crate::RouteResult>,
    pub(crate) request_text: Option<&'a str>,
    pub(crate) context_bundle_summary: Option<&'a str>,
    pub(crate) plan_result: &'a PlanResult,
    pub(crate) execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct VerifyResult {
    pub(crate) mode: VerifyMode,
    pub(crate) approved: bool,
    pub(crate) blocked_reason: Option<String>,
    pub(crate) shadow_blocked_reason: Option<String>,
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

fn effective_step_risk_level(
    state: &AppState,
    normalized_skill: &str,
    args: &serde_json::Value,
) -> SkillRiskLevel {
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
        let primary_count = entries
            .iter()
            .filter(|(_, _, role)| matches!(role, PrimaryFallbackRole::Primary))
            .count();
        let fallback_count = entries
            .iter()
            .filter(|(_, _, role)| matches!(role, PrimaryFallbackRole::Fallback))
            .count();

        if primary_count > 0 && fallback_count > 0 {
            push_group_conflict_issues(
                issues,
                &group,
                &entries,
                "both primary and fallback steps are present in the same plan".to_string(),
            );
            continue;
        }
        if primary_count > 1 {
            push_group_conflict_issues(
                issues,
                &group,
                &entries,
                "multiple primary steps are present in the same group".to_string(),
            );
            continue;
        }
        if fallback_count > 1 {
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
        });
        return;
    };
    for key in &required {
        if !required_arg_satisfied(obj, key) {
            issues.push(VerifyIssue {
                step_id: step.step_id.clone(),
                kind: VerifyIssueKind::MissingRequiredArg,
                detail: format!("skill `{normalized_skill}` missing required arg `{key}`"),
            });
        }
    }
}

#[derive(Debug, Clone, Default)]
struct TemplatePlaceholderScope {
    exact_refs: HashSet<String>,
    indexable_refs: HashSet<String>,
}

impl TemplatePlaceholderScope {
    fn register_step_output(&mut self, step: &PlanStep, step_number: usize) {
        self.exact_refs.insert("last_output".to_string());
        self.exact_refs.insert(format!("s{step_number}.output"));
        self.exact_refs
            .insert(format!("{}.last_output", step.step_id.trim()));
        self.indexable_refs.insert("last_output".to_string());
        self.indexable_refs.insert(format!("s{step_number}"));
        self.indexable_refs.insert(step.step_id.trim().to_string());
    }

    fn allows(&self, raw_ref: &str) -> bool {
        let reference = raw_ref.trim();
        if reference.is_empty() {
            return false;
        }
        if self.exact_refs.contains(reference) {
            return true;
        }
        placeholder_indexable_base(reference).is_some_and(|base| self.indexable_refs.contains(base))
    }
}

fn placeholder_indexable_base(reference: &str) -> Option<&str> {
    let dot = reference.find('.');
    let bracket = reference.find('[');
    let split_at = match (dot, bracket) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return None,
    };
    let base = reference[..split_at].trim();
    (!base.is_empty()).then_some(base)
}

fn extract_template_refs(text: &str) -> Option<Vec<String>> {
    let mut refs = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            return None;
        };
        let reference = after_start[..end].trim();
        if reference.is_empty() {
            return None;
        }
        refs.push(reference.to_string());
        rest = &after_start[end + 2..];
    }
    Some(refs)
}

fn value_contains_unresolved_template(
    value: &serde_json::Value,
    template_scope: &TemplatePlaceholderScope,
) -> bool {
    match value {
        serde_json::Value::String(text) => {
            let text = text.trim();
            if !(text.contains("{{") || text.contains("}}")) {
                return false;
            }
            let Some(refs) = extract_template_refs(text) else {
                return true;
            };
            refs.is_empty()
                || refs
                    .iter()
                    .any(|reference| !template_scope.allows(reference))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| value_contains_unresolved_template(item, template_scope)),
        serde_json::Value::Object(map) => map
            .values()
            .any(|item| value_contains_unresolved_template(item, template_scope)),
        _ => false,
    }
}

fn step_can_produce_output_for_template_scope(step: &PlanStep) -> bool {
    matches!(
        step.action_type.as_str(),
        "call_skill" | "call_tool" | "synthesize_answer"
    )
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
    )
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
        && !matches!(
            recipe.profile,
            crate::execution_recipe::ExecutionRecipeProfile::ConfigChange
                | crate::execution_recipe::ExecutionRecipeProfile::CodeChange
                | crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring
        );
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
        }
    }

    if let Some(step_id) = first_mutation_step_id {
        issues.push(VerifyIssue {
            step_id,
            kind: VerifyIssueKind::RecipeInspectBeforeMutateRequired,
            detail: "ops_closed_loop requires at least one inspect/read-only evidence step before mutating".to_string(),
        });
    }

    let validation_satisfied = if matches!(
        recipe.profile,
        crate::execution_recipe::ExecutionRecipeProfile::ConfigChange
            | crate::execution_recipe::ExecutionRecipeProfile::CodeChange
            | crate::execution_recipe::ExecutionRecipeProfile::SkillAuthoring
    ) {
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
                });
            } else if let Some(step_id) = target_scope_conflict_step_id {
                issues.push(VerifyIssue {
                    step_id,
                    kind: VerifyIssueKind::RecipeTargetScopeRequired,
                    detail: crate::execution_recipe::target_scope_detail_for_recipe(recipe)
                        .to_string(),
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

fn request_prefers_english(request_text: Option<&str>) -> bool {
    request_text
        .map(crate::language_policy::request_language_hint)
        .is_some_and(|hint| hint == "en")
}

fn unresolved_template_response_step(request_text: Option<&str>) -> PlanStep {
    let content = if request_prefers_english(request_text) {
        "I need the concrete input before I can continue. Please provide the JSON array, file path, or previous result to use.".to_string()
    } else {
        "我还缺少要处理的具体内容。请直接提供 JSON 数组、文件路径或上一条结果后，我再继续处理。"
            .to_string()
    };
    PlanStep {
        step_id: "verify_unresolved_template_response".to_string(),
        action_type: "respond".to_string(),
        skill: "respond".to_string(),
        args: serde_json::json!({ "content": content }),
        depends_on: Vec::new(),
        why: "unresolved template placeholder in executable step args".to_string(),
    }
}

fn unresolved_capability_response_step(request_text: Option<&str>, capability: &str) -> PlanStep {
    let content = if request_prefers_english(request_text) {
        format!("I cannot execute this yet because the runtime capability is not available: `{capability}`.")
    } else {
        format!("暂时无法执行：运行时能力未解析或不可用：`{capability}`。")
    };
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

    let route_requires_clarify_before_tools = input
        .route_result
        .map(|route| route.needs_clarify)
        .unwrap_or(false)
        && input.plan_result.steps.iter().any(|step| {
            matches!(
                step.action_type.as_str(),
                "call_skill" | "call_tool" | "call_capability"
            )
        });
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
    let mut needs_confirmation = false;
    if route_requires_clarify_before_tools {
        issues.push(VerifyIssue {
            step_id: "route".to_string(),
            kind: VerifyIssueKind::RouteClarifyRequired,
            detail: format!(
                "route requires clarify before execution; context={}",
                input.context_bundle_summary.unwrap_or("<none>")
            ),
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
            });
        } else if matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
            let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
            if !visible_skills.contains(&normalized_skill) {
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::SkillNotVisible,
                    detail: format!("skill `{normalized_skill}` is not in planner visible skills"),
                });
            }
            verify_step_args(state, step, &normalized_skill, &template_scope, &mut issues);
            if let Some(policy) = crate::contract_matrix::action_policy_for_output_contract(
                input.route_result.map(|route| &route.output_contract),
                &normalized_skill,
                &step.args,
            ) {
                if !policy.is_allowed() {
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
                    });
                }
            }
            let safe_autonomous_creation =
                safe_autonomous_creation_step(state, &normalized_skill, &step.args);
            let step_risk = effective_step_risk_level(state, &normalized_skill, &step.args);
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
                });
            }
            if !confirmation_already_granted
                && !safe_autonomous_creation
                && (state.skill_invocation_requires_confirmation_policy(
                    &normalized_skill,
                    Some(&step.args),
                ) || is_confirmation_like_skill(&normalized_skill))
            {
                needs_confirmation = true;
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::ConfirmationRequired,
                    detail: format!("skill `{normalized_skill}` may require explicit confirmation"),
                });
            }
        }

        for dep in &step.depends_on {
            if !all_step_ids.contains(dep) {
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::InvalidDependsOn,
                    detail: format!("depends_on references missing step `{dep}`"),
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
        approved_steps,
        needs_confirmation,
        rewritten_steps,
        issues,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, RwLock};

    use claw_core::config::{AgentConfig, ToolsConfig};
    use claw_core::skill_registry::SkillsRegistry;

    use serde_json::json;

    use super::{verify_plan, VerifyInput, VerifyIssueKind, VerifyMode};
    use crate::{
        AgentRuntimeConfig, AppState, ClaimedTask, PlanKind, PlanResult, PlanStep, RouteResult,
        ScheduleKind, SkillViewsSnapshot, ToolsPolicy,
    };

    fn test_registry() -> SkillsRegistry {
        let toml = r#"
[[skills]]
name = "read_file"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = false
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["command"], properties = { command = { type = "string" } } }

[[skills]]
name = "list_dir"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = false
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["path", "content"], properties = { path = { type = "string" }, content = { type = "string" } } }

[[skills]]
name = "make_dir"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "remove_file"
enabled = true
kind = "builtin"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["path"], properties = { path = { type = "string" } } }

[[skills]]
name = "fs_basic"
enabled = true
kind = "builtin"
planner_kind = "tool"
output_kind = "text"
side_effect = true
auto_invocable = true
input_schema = { type = "object", required = ["action"], properties = { action = { type = "string" }, path = { type = "string" }, paths = { type = "array", items = { type = "string" } } } }
planner_capabilities = [
  { name = "filesystem.stat_paths", action = "stat_paths", effect = "observe", required = ["path|paths"] },
  { name = "filesystem.read_text_range", action = "read_text_range", effect = "observe", required = ["path"] },
  { name = "filesystem.remove_path", action = "remove_path", effect = "mutate", required = ["path"], risk_level = "high" },
]

[[skills]]
name = "primary_reader"
enabled = true
kind = "runner"
output_kind = "text"
group = "reader"
primary_fallback_role = "primary"

	[[skills]]
	name = "fallback_reader"
	enabled = true
	kind = "runner"
	output_kind = "text"
	group = "reader"
	primary_fallback_role = "fallback"

	[[skills]]
	name = "photo_organize"
	enabled = true
	kind = "runner"
	output_kind = "text"
	risk_level = "high"
	auto_invocable = false
	requires_confirmation = true
	side_effect = true
	confirmation_exempt_when = [
	  { action = "prepare" },
	  { action = "organize", mode = "plan" },
	]
	"#;
        let path = std::env::temp_dir().join(format!(
            "verifier_registry_{}_{}_{}.toml",
            std::process::id(),
            crate::now_ts_u64(),
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, toml).expect("write registry");
        let registry = SkillsRegistry::load_from_path(&path).expect("load registry");
        let _ = std::fs::remove_file(path);
        registry
    }

    fn test_state() -> AppState {
        let registry = Arc::new(test_registry());
        let skills_list = Arc::new(
            [
                "read_file",
                "run_cmd",
                "list_dir",
                "write_file",
                "make_dir",
                "fs_basic",
                "primary_reader",
                "fallback_reader",
                "photo_organize",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<HashSet<_>>(),
        );
        let agents_by_id = HashMap::from([(
            crate::DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            core: crate::CoreServices {
                agents_by_id: Arc::new(agents_by_id),
                skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                    registry: Some(registry),
                    skills_list,
                }))),
                ..crate::CoreServices::test_default()
            },
            skill_rt: crate::SkillRuntime {
                locator_scan_max_depth: 3,
                locator_scan_max_files: 200,
                tools_policy: Arc::new(
                    ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
                ),
                ..crate::SkillRuntime::test_default()
            },
            policy: crate::PolicyConfig::test_default(),
            worker: crate::WorkerConfig::test_default(),
            metrics: crate::TaskMetricsRegistry::default(),
            channels: crate::ChannelConfig::default(),
            reload_ctx: crate::ReloadContext::default(),
            ask_states: crate::AskStateRegistry::default(),
        }
    }

    fn test_task() -> ClaimedTask {
        ClaimedTask {
            task_id: "task-verify".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "telegram".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        }
    }

    fn route_result(needs_clarify: bool) -> RouteResult {
        route_result_with_risk(needs_clarify, crate::RiskCeiling::Unknown)
    }

    fn route_result_with_semantic(semantic_kind: crate::OutputSemanticKind) -> RouteResult {
        let mut route = route_result(false);
        route.output_contract = crate::IntentOutputContract {
            semantic_kind,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            ..Default::default()
        };
        route
    }

    fn route_result_with_risk(
        needs_clarify: bool,
        risk_ceiling: crate::RiskCeiling,
    ) -> RouteResult {
        RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "test".to_string(),
            needs_clarify,
            route_reason: "test".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: vec!["read_file".to_string()],
            risk_ceiling,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        }
    }

    fn plan_result(steps: Vec<PlanStep>) -> PlanResult {
        PlanResult {
            goal: "test".to_string(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            steps,
            planner_notes: String::new(),
            plan_kind: PlanKind::Single,
            raw_plan_text: String::new(),
        }
    }

    #[test]
    fn observe_mode_keeps_route_clarify_as_shadow_only() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(true)),
                request_text: None,
                context_bundle_summary: Some("need more info"),
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "README.md" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.approved);
        assert!(result.blocked_reason.is_none());
        assert!(matches!(
            result.issues.first().map(|issue| issue.kind),
            Some(VerifyIssueKind::RouteClarifyRequired)
        ));
        assert!(result.shadow_blocked_reason.is_some());
    }

    #[test]
    fn observe_mode_rewrites_unresolved_template_args_to_response() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(true)),
                request_text: Some("帮我转成表格"),
                context_bundle_summary: Some("needs concrete JSON array"),
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "{{last_output}}" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.approved);
        assert!(result.shadow_blocked_reason.is_some());
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::RouteClarifyRequired)));
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::UnresolvedTemplateArg)));
        assert_eq!(result.rewritten_steps.len(), 1);
        assert_eq!(result.rewritten_steps[0].action_type, "respond");
        assert!(result.rewritten_steps[0]
            .args
            .get("content")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .contains("具体内容"));
    }

    #[test]
    fn observe_mode_rewrites_unresolved_call_capability_to_response() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("帮我查一下"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_capability".to_string(),
                    skill: "unknown.example".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::ObserveOnly,
        );

        assert!(result.approved);
        assert!(result.shadow_blocked_reason.is_some());
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::CapabilityUnavailable)));
        assert_eq!(result.rewritten_steps.len(), 1);
        assert_eq!(result.rewritten_steps[0].action_type, "respond");
        assert!(result.rewritten_steps[0]
            .args
            .get("content")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .contains("unknown.example"));
    }

    #[test]
    fn enforce_mode_blocks_unresolved_call_capability() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_capability".to_string(),
                    skill: "unknown.example".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );

        assert!(!result.approved);
        assert!(result.blocked_reason.is_some());
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::CapabilityUnavailable)));
    }

    #[test]
    fn observe_mode_allows_prior_output_template_in_later_args() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(true)),
                request_text: Some(
                    "查看 logs 目录，把里面的日志文件名整理到 logs_inventory.txt，然后把文件发给我。",
                ),
                context_bundle_summary: Some("auto_locator_path=/home/guagua/rustclaw/logs"),
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "list_dir".to_string(),
                        args: json!({ "path": "/home/guagua/rustclaw/logs" }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "write_file".to_string(),
                        args: json!({
                            "path": "/home/guagua/rustclaw/logs_inventory.txt",
                            "content": "{{last_output}}"
                        }),
                        depends_on: vec!["step_1".to_string()],
                        why: String::new(),
                    },
                ]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::ObserveOnly,
        );

        assert!(result.approved);
        assert!(!result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::UnresolvedTemplateArg)));
        assert!(result.rewritten_steps.is_empty());
    }

    #[test]
    fn enforce_mode_blocks_missing_required_arg() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );
        assert!(!result.approved);
        assert!(matches!(
            result.issues.first().map(|issue| issue.kind),
            Some(VerifyIssueKind::MissingRequiredArg)
        ));
        assert!(result
            .blocked_reason
            .as_deref()
            .unwrap_or_default()
            .contains("missing required arg"));
    }

    #[test]
    fn enforce_mode_blocks_action_scoped_required_arg() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: json!({"action": "read_text_range"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );
        assert!(!result.approved);
        assert!(result.issues.iter().any(|issue| matches!(
            issue.kind,
            VerifyIssueKind::MissingRequiredArg
        ) && issue.detail.contains("`path`")));
    }

    #[test]
    fn enforce_mode_accepts_action_scoped_alternative_arg() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: json!({"action": "stat_paths", "path": "README.md"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );
        assert!(result.approved, "issues: {:?}", result.issues);
        assert!(!result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
    }

    #[test]
    fn enforce_mode_blocks_mutation_above_low_risk_ceiling() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result_with_risk(false, crate::RiskCeiling::Low)),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({"path": "out.txt", "content": "hello"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );
        assert!(!result.approved);
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::RiskBudgetExceeded)));
    }

    #[test]
    fn observe_mode_records_contract_action_rejection_for_structured_route() {
        let state = test_state();
        let task = test_task();
        let route = route_result_with_semantic(crate::OutputSemanticKind::FileNames);
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({"command": "ls"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::ObserveOnly,
        );

        assert!(result.approved);
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractActionRejected)));
        assert!(result
            .shadow_blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("rejected by contract")));
    }

    #[test]
    fn enforce_mode_allows_low_risk_action_under_low_risk_ceiling() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result_with_risk(false, crate::RiskCeiling::Low)),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: json!({"action": "stat_paths", "paths": ["README.md"]}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );
        assert!(result.approved, "issues: {:?}", result.issues);
        assert!(!result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::RiskBudgetExceeded)));
    }

    #[test]
    fn enforce_mode_blocks_skill_not_visible() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "totally_fake_skill".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );
        assert!(!result.approved);
        assert!(result
            .issues
            .iter()
            .any(|issue| { matches!(issue.kind, VerifyIssueKind::SkillNotVisible) }));
    }

    #[test]
    fn enforce_mode_blocks_primary_fallback_conflict() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "primary_reader".to_string(),
                        args: json!({}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "fallback_reader".to_string(),
                        args: json!({}),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                ]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );
        assert!(!result.approved);
        assert!(result
            .issues
            .iter()
            .any(|issue| { matches!(issue.kind, VerifyIssueKind::PrimaryFallbackConflict) }));
    }

    #[test]
    fn resume_execute_route_skips_confirmation_requirement() {
        let state = test_state();
        let task = test_task();
        let mut resumed_route = route_result(false);
        resumed_route.resume_behavior = crate::ResumeBehavior::ResumeExecute;
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&resumed_route),
                request_text: None,
                context_bundle_summary: Some("resume"),
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({ "command": "pwd" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );
        assert!(result.approved);
        assert!(!result.needs_confirmation);
        assert!(!result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    }

    #[test]
    fn confirmation_exempt_invocation_skips_confirmation_requirement() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: None,
                context_bundle_summary: Some("photo preview"),
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "photo_organize".to_string(),
                    args: json!({ "action": "organize", "mode": "plan" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );
        assert!(result.approved);
        assert!(!result.needs_confirmation);
        assert!(!result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    }

    #[test]
    fn safe_make_dir_missing_path_defaults_under_workspace_without_confirmation() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("帮我创建一个文件夹"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "make_dir".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );

        assert!(result.approved);
        assert!(!result.needs_confirmation);
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
        assert!(!result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
        let path = result.approved_steps[0]
            .args
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        assert!(path.starts_with(state.skill_rt.workspace_root.to_string_lossy().as_ref()));
        assert!(path.contains("rustclaw-created-dir-taskveri"));
    }

    #[test]
    fn safe_write_file_relative_path_anchors_under_workspace_without_confirmation() {
        let state = test_state();
        let task = test_task();
        let filename = format!("rustclaw-autonomy-{}.txt", uuid::Uuid::new_v4());
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("把结果写到文件"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": filename, "content": "ok" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );

        assert!(result.approved);
        assert!(!result.needs_confirmation);
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
        assert!(!result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
        let path = result.approved_steps[0]
            .args
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        assert!(path.starts_with(state.skill_rt.workspace_root.to_string_lossy().as_ref()));
        assert!(path.ends_with(".txt"));
    }

    #[test]
    fn dangerous_remove_file_missing_path_blocks_without_default_target() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("delete it"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "remove_file".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );

        assert!(!result.approved);
        assert!(!result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
    }

    #[test]
    fn dangerous_fs_basic_remove_path_missing_path_blocks_without_default_target() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("remove that path"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: json!({ "action": "remove_path" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );

        assert!(!result.approved);
        assert!(!result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::DefaultCreationTargetApplied)));
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
    }

    #[test]
    fn destructive_run_cmd_requires_confirmation_without_resume() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("remove temp files"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({ "command": "rm -rf /tmp/rustclaw-verifier-test" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );

        assert!(result.approved);
        assert!(result.needs_confirmation);
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    }

    #[test]
    fn non_exempt_invocation_still_requires_confirmation() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: None,
                context_bundle_summary: Some("photo move"),
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "photo_organize".to_string(),
                    args: json!({ "action": "organize", "mode": "move" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        );
        assert!(result.approved);
        assert!(result.needs_confirmation);
        assert!(result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
    }

    #[test]
    fn ops_recipe_requires_inspect_before_mutate() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({ "command": "systemctl restart sing-box" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        inspect_first: true,
                        validation_required: true,
                        max_repairs: 2,
                        ..Default::default()
                    },
                ),
            },
            VerifyMode::Enforce,
        );
        assert!(!result.approved);
        assert!(result.issues.iter().any(|issue| {
            matches!(
                issue.kind,
                VerifyIssueKind::RecipeInspectBeforeMutateRequired
            )
        }));
    }

    #[test]
    fn ops_recipe_requires_validation_after_mutate() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "read_file".to_string(),
                        args: json!({ "path": "configs/config.toml" }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({ "command": "systemctl restart sing-box" }),
                        depends_on: vec!["s1".to_string()],
                        why: String::new(),
                    },
                ]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        inspect_first: true,
                        validation_required: true,
                        max_repairs: 2,
                        ..Default::default()
                    },
                ),
            },
            VerifyMode::Enforce,
        );
        assert!(!result.approved);
        assert!(result.issues.iter().any(|issue| {
            matches!(
                issue.kind,
                VerifyIssueKind::RecipeValidationAfterMutateRequired
            )
        }));
    }

    #[test]
    fn code_change_recipe_requires_profile_specific_verification() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("修复当前仓库里的 clawd 入口逻辑，并验证编译或测试通过。"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "read_file".to_string(),
                        args: json!({ "path": "crates/clawd/src/main.rs" }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "write_file".to_string(),
                        args: json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
                        depends_on: vec!["s1".to_string()],
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s3".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "read_file".to_string(),
                        args: json!({ "path": "crates/clawd/src/main.rs" }),
                        depends_on: vec!["s2".to_string()],
                        why: String::new(),
                    },
                ]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                        target_scope:
                            crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                        inspect_first: true,
                        validation_required: true,
                        max_repairs: 2,
                    },
                ),
            },
            VerifyMode::Enforce,
        );
        assert!(!result.approved);
        let issue = result
            .issues
            .iter()
            .find(|issue| {
                matches!(
                    issue.kind,
                    VerifyIssueKind::RecipeValidationAfterMutateRequired
                )
            })
            .expect("expected code_change validation issue");
        assert!(issue
            .detail
            .contains("code_change requires compile/test/build or runtime verification"));
    }

    #[test]
    fn code_change_recipe_accepts_structured_cargo_check_verification() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("修复当前仓库里的 clawd 入口逻辑，并验证编译通过。"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s0".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "read_file".to_string(),
                        args: json!({ "path": "crates/clawd/src/main.rs" }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "write_file".to_string(),
                        args: json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
                        depends_on: vec!["s0".to_string()],
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({
                            "command": "cargo check -p clawd",
                            "_clawd_validation": {
                                "profile": "code_change",
                                "validator_type": "build",
                                "validated_target": "clawd"
                            }
                        }),
                        depends_on: vec!["s1".to_string()],
                        why: String::new(),
                    },
                ]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                        target_scope:
                            crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                        inspect_first: true,
                        validation_required: true,
                        max_repairs: 2,
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.issues.iter().all(|issue| {
            !matches!(
                issue.kind,
                VerifyIssueKind::RecipeValidationAfterMutateRequired
                    | VerifyIssueKind::RecipeInspectBeforeMutateRequired
            )
        }));
    }

    #[test]
    fn code_change_recipe_rejects_unstructured_cargo_check_verification() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("修复当前仓库里的 clawd 入口逻辑，并验证编译通过。"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s0".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "read_file".to_string(),
                        args: json!({ "path": "crates/clawd/src/main.rs" }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "write_file".to_string(),
                        args: json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
                        depends_on: vec!["s0".to_string()],
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({ "command": "cargo check -p clawd" }),
                        depends_on: vec!["s1".to_string()],
                        why: String::new(),
                    },
                ]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                        target_scope:
                            crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                        inspect_first: true,
                        validation_required: true,
                        max_repairs: 2,
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.issues.iter().any(|issue| matches!(
            issue.kind,
            VerifyIssueKind::RecipeValidationAfterMutateRequired
        )));
    }

    #[test]
    fn code_change_recipe_accepts_structured_custom_validation_step() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("修复当前仓库里的脚本，并运行自定义检查脚本验证通过。"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s0".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "read_file".to_string(),
                        args: json!({ "path": "scripts/check.sh" }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "write_file".to_string(),
                        args: json!({ "path": "scripts/check.sh", "content": "#!/usr/bin/env bash\nexit 0\n" }),
                        depends_on: vec!["s0".to_string()],
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({
                            "command": "bash scripts/check.sh",
                            "_clawd_validation": {
                                "profile": "code_change",
                                "validator_type": "custom",
                                "validated_target": "scripts/check.sh"
                            }
                        }),
                        depends_on: vec!["s1".to_string()],
                        why: String::new(),
                    },
                ]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                        target_scope:
                            crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                        inspect_first: true,
                        validation_required: true,
                        max_repairs: 2,
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.issues.iter().all(|issue| {
            !matches!(
                issue.kind,
                VerifyIssueKind::RecipeValidationAfterMutateRequired
                    | VerifyIssueKind::RecipeInspectBeforeMutateRequired
            )
        }));
    }

    #[test]
    fn current_repo_scope_rejects_external_target_plan() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("修复当前仓库里的 clawd 入口逻辑，不要动仓库外项目。"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": "/opt/other-project/main.rs", "content": "fn main() {}\n" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                        target_scope:
                            crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
                        inspect_first: false,
                        validation_required: false,
                        max_repairs: 2,
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.issues.iter().any(|issue| {
            matches!(issue.kind, VerifyIssueKind::RecipeTargetScopeRequired)
                && issue
                    .detail
                    .contains("current_repo scope must stay inside the current workspace")
        }));
    }

    #[test]
    fn external_workspace_scope_requires_explicit_external_target_plan() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("去当前仓库外的另一个项目修问题。"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({ "path": "crates/clawd/src/main.rs", "content": "fn main() {}\n" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                        target_scope:
                            crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
                        inspect_first: false,
                        validation_required: false,
                        max_repairs: 2,
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.issues.iter().any(|issue| {
            matches!(issue.kind, VerifyIssueKind::RecipeTargetScopeRequired)
                && issue
                    .detail
                    .contains("external_workspace scope requires an explicit external path")
        }));
    }

    #[test]
    fn greenfield_scope_requires_creation_plan() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("从零做一个新脚本并验证。"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({ "command": "cargo check -p clawd" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                        target_scope:
                            crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
                        inspect_first: false,
                        validation_required: false,
                        max_repairs: 2,
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.issues.iter().any(|issue| {
            matches!(issue.kind, VerifyIssueKind::RecipeTargetScopeRequired)
                && issue
                    .detail
                    .contains("greenfield scope requires creating a new file")
        }));
    }

    #[test]
    fn external_workspace_scope_accepts_explicit_external_path_plan() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("去另一个目录修问题，并验证通过。"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "read_file".to_string(),
                        args: json!({ "path": "/opt/other-project/src/main.rs" }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "write_file".to_string(),
                        args: json!({ "path": "/opt/other-project/src/main.rs", "content": "fn main() {}\n" }),
                        depends_on: vec!["s1".to_string()],
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s3".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({
                            "command": "cd /opt/other-project && cargo check",
                            "_clawd_validation": {
                                "profile": "code_change",
                                "validator_type": "build",
                                "validated_target": "/opt/other-project"
                            }
                        }),
                        depends_on: vec!["s2".to_string()],
                        why: String::new(),
                    },
                ]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                        target_scope:
                            crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
                        inspect_first: true,
                        validation_required: true,
                        max_repairs: 2,
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.issues.iter().all(|issue| {
            !matches!(
                issue.kind,
                VerifyIssueKind::RecipeTargetScopeRequired
                    | VerifyIssueKind::RecipeValidationAfterMutateRequired
                    | VerifyIssueKind::RecipeInspectBeforeMutateRequired
            )
        }));
    }

    #[test]
    fn external_workspace_scope_persisted_target_allows_followup_validation_plan() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("继续修外部工作区里的项目，并验证通过。"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "cargo check",
                        "_clawd_validation": {
                            "profile": "code_change",
                            "validator_type": "build",
                            "validated_target": "external_workspace"
                        }
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope:
                        crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
                    phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                    saw_inspect: true,
                    saw_mutation: true,
                    saw_external_target: true,
                    ..Default::default()
                },
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.issues.iter().all(|issue| {
            !matches!(
                issue.kind,
                VerifyIssueKind::RecipeTargetScopeRequired
                    | VerifyIssueKind::RecipeValidationAfterMutateRequired
                    | VerifyIssueKind::RecipeInspectBeforeMutateRequired
            )
        }));
    }

    #[test]
    fn greenfield_scope_persisted_creation_allows_followup_validation_plan() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: Some("继续验证刚创建的新项目。"),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "cargo check -p clawd",
                        "_clawd_validation": {
                            "profile": "code_change",
                            "validator_type": "build",
                            "validated_target": "greenfield_project"
                        }
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    profile: crate::execution_recipe::ExecutionRecipeProfile::CodeChange,
                    target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
                    phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                    saw_inspect: true,
                    saw_mutation: true,
                    saw_greenfield_creation: true,
                    ..Default::default()
                },
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.issues.iter().all(|issue| {
            !matches!(
                issue.kind,
                VerifyIssueKind::RecipeTargetScopeRequired
                    | VerifyIssueKind::RecipeValidationAfterMutateRequired
                    | VerifyIssueKind::RecipeInspectBeforeMutateRequired
            )
        }));
    }

    #[test]
    fn ops_recipe_rewrites_combined_run_cmd_into_apply_then_validate() {
        let state = test_state();
        let task = test_task();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route_result(false)),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "cd /tmp/demo && nohup python3 -m http.server 51179 --bind 127.0.0.1 > /dev/null 2>&1 & sleep 2 && curl -s http://127.0.0.1:51179/ | grep -q 'ops-demo-ok' && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        inspect_first: false,
                        validation_required: true,
                        max_repairs: 2,
                        ..Default::default()
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
        assert_eq!(result.rewritten_steps.len(), 2);
        assert_eq!(result.rewritten_steps[0].step_id, "s1");
        assert_eq!(result.rewritten_steps[1].step_id, "s1__validate");
        assert_eq!(
            result.rewritten_steps[0].args.get("command").and_then(|v| v.as_str()),
            Some(
                "cd /tmp/demo && nohup python3 -m http.server 51179 --bind 127.0.0.1 > /dev/null 2>&1 &"
            )
        );
        assert_eq!(
            result.rewritten_steps[1].args.get("command").and_then(|v| v.as_str()),
            Some(
                "sleep 2 && curl -s http://127.0.0.1:51179/ | grep -q 'ops-demo-ok' && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
            )
        );
        assert_eq!(result.rewritten_steps[1].depends_on, vec!["s1".to_string()]);
        assert!(result.rewritten_steps[0]
            .args
            .get("timeout_seconds")
            .is_none());
        assert!(result.rewritten_steps[1]
            .args
            .get("timeout_seconds")
            .is_none());
    }

    #[test]
    fn ops_recipe_split_does_not_infer_success_marker_from_request_text() {
        let state = test_state();
        let task = test_task();
        let mut route = route_result(false);
        route.resolved_intent =
            "start local http service and verify homepage contains ops-demo-ok".to_string();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: Some(
                    "Start a static HTTP server in the background, then use curl to verify that the homepage contains ops-demo-ok; when validation passes, explicitly output VALIDATION_PASSED and finish immediately.",
                ),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "cd /tmp/demo && nohup python3 -m http.server 51179 --bind 127.0.0.1 > /dev/null 2>&1 & sleep 2 && curl -s http://127.0.0.1:51179/"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        inspect_first: false,
                        validation_required: true,
                        max_repairs: 2,
                    ..Default::default()
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
        assert_eq!(result.rewritten_steps.len(), 2);
        assert_eq!(
            result.rewritten_steps[1]
                .args
                .get("command")
                .and_then(|value| value.as_str()),
            Some("sleep 2 && curl -s http://127.0.0.1:51179/")
        );
    }

    #[test]
    fn ops_recipe_does_not_infer_http_expect_contains_marker_from_route_text() {
        let state = test_state();
        let task = test_task();
        let mut route = route_result(false);
        route.resolved_intent =
            "verify local http service homepage contains ops-repair-ok and repair if needed"
                .to_string();
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "http_basic".to_string(),
                    args: json!({
                        "action": "get",
                        "url": "http://127.0.0.1:51179/"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        inspect_first: true,
                        validation_required: true,
                        max_repairs: 2,
                        ..Default::default()
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.rewritten_steps.is_empty());
        assert_eq!(result.approved_steps.len(), 1);
        assert!(result.approved_steps[0]
            .args
            .get("expect_contains")
            .is_none());
    }

    #[test]
    fn ops_recipe_does_not_infer_http_expect_contains_marker_from_request_text() {
        let state = test_state();
        let task = test_task();
        let route = route_result(false);
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: Some(
                    "First verify whether the local static HTTP service serves a homepage containing ops-repair-ok. If verification fails, repair it and verify again until it passes.",
                ),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "http_basic".to_string(),
                    args: json!({
                        "action": "get",
                        "url": "http://127.0.0.1:51179/"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        inspect_first: true,
                        validation_required: true,
                        max_repairs: 2,
                    ..Default::default()
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
        assert!(result.rewritten_steps.is_empty());
        assert_eq!(result.approved_steps.len(), 1);
        assert!(result.approved_steps[0]
            .args
            .get("expect_contains")
            .is_none());
    }

    #[test]
    fn ops_recipe_repair_round_plan_stays_valid_after_failed_http_preflight() {
        let state = test_state();
        let task = test_task();
        let mut route = route_result(false);
        route.resolved_intent =
            "verify local http service homepage contains ops-repair-ok and repair if needed"
                .to_string();
        route.resume_behavior = crate::ResumeBehavior::ResumeExecute;
        let initial_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
            crate::execution_recipe::ExecutionRecipeSpec {
                kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                inspect_first: true,
                validation_required: true,
                max_repairs: 2,
                ..Default::default()
            },
        );
        let inspect_result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "http_basic".to_string(),
                    args: json!({
                        "action": "get",
                        "url": "http://127.0.0.1:51179/",
                        "expect_contains": "ops-repair-ok"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: initial_recipe,
            },
            VerifyMode::ObserveOnly,
        );
        let inspect_step = &inspect_result.approved_steps[0];
        let raw_effect = crate::execution_recipe::classify_skill_action_effect(
            &state,
            &inspect_step.skill,
            &inspect_step.args,
        );
        let effective_effect =
            crate::execution_recipe::effective_action_effect_for_recipe(initial_recipe, raw_effect);
        let validation = crate::execution_recipe::assess_validation_output(
            &state,
            &inspect_step.skill,
            &inspect_step.args,
            "status=200\nops-repair-bad\n",
        );
        assert!(matches!(
            validation,
            crate::execution_recipe::ValidationObservation::Failed(_)
        ));
        assert!(effective_effect.observes);
        assert!(!effective_effect.validates);

        let mut repair_recipe = initial_recipe;
        crate::execution_recipe::apply_action_effect_failure(&mut repair_recipe, effective_effect);
        assert_eq!(
            crate::execution_recipe::stop_signal_for_validation_failure(&repair_recipe),
            "recoverable_failure_continue_round"
        );
        assert!(repair_recipe.saw_inspect);
        assert_eq!(
            repair_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Apply
        );

        let repair_result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "read_file".to_string(),
                        args: json!({ "path": "document/nl_ops_http_demo/index.html" }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s3".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({
                            "command": "printf 'ops-repair-ok\\n' > document/nl_ops_http_demo/index.html"
                        }),
                        depends_on: vec!["s2".to_string()],
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s4".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({
                            "command": "curl -s http://127.0.0.1:51179/ | grep -q 'ops-repair-ok' && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
                        }),
                        depends_on: vec!["s3".to_string()],
                        why: String::new(),
                    },
                ]),
                execution_recipe: repair_recipe,
            },
            VerifyMode::Enforce,
        );
        assert!(repair_result.approved, "issues: {:?}", repair_result.issues);
        assert!(repair_result.blocked_reason.is_none());
        assert_eq!(repair_result.approved_steps.len(), 3);
        assert!(repair_result.rewritten_steps.is_empty());
        assert_eq!(
            repair_result.approved_steps[2]
                .args
                .get("command")
                .and_then(|value| value.as_str()),
            Some(
                "curl -s http://127.0.0.1:51179/ | grep -q 'ops-repair-ok' && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
            )
        );
    }

    #[test]
    fn ops_recipe_service_repair_round_plan_stays_valid_after_failed_status_preflight() {
        let state = test_state();
        let task = test_task();
        let mut route = route_result(false);
        route.resolved_intent = "repair sing-box and verify the service is running".to_string();
        route.resume_behavior = crate::ResumeBehavior::ResumeExecute;
        let initial_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
            crate::execution_recipe::ExecutionRecipeSpec {
                kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                inspect_first: true,
                validation_required: true,
                max_repairs: 2,
                ..Default::default()
            },
        );
        let inspect_step = PlanStep {
            step_id: "s1".to_string(),
            action_type: "call_skill".to_string(),
            skill: "run_cmd".to_string(),
            args: json!({ "command": "systemctl status sing-box" }),
            depends_on: Vec::new(),
            why: String::new(),
        };
        let inspect_result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![inspect_step.clone()]),
                execution_recipe: initial_recipe,
            },
            VerifyMode::ObserveOnly,
        );
        assert!(inspect_result.approved);
        assert_eq!(inspect_result.approved_steps.len(), 1);
        assert_eq!(
            inspect_result.approved_steps[0].step_id,
            inspect_step.step_id
        );
        assert_eq!(inspect_result.approved_steps[0].skill, inspect_step.skill);
        assert_eq!(
            inspect_result.approved_steps[0]
                .args
                .get("command")
                .and_then(|value| value.as_str()),
            Some("systemctl status sing-box")
        );

        let raw_effect = crate::execution_recipe::classify_skill_action_effect(
            &state,
            "run_cmd",
            &json!({ "command": "systemctl status sing-box" }),
        );
        let effective_effect =
            crate::execution_recipe::effective_action_effect_for_recipe(initial_recipe, raw_effect);
        let validation = crate::execution_recipe::assess_validation_output(
            &state,
            "run_cmd",
            &json!({ "command": "systemctl status sing-box" }),
            "inactive (dead)\n",
        );
        assert!(matches!(
            validation,
            crate::execution_recipe::ValidationObservation::Failed(_)
        ));
        assert!(effective_effect.observes);
        assert!(!effective_effect.validates);

        let mut repair_recipe = initial_recipe;
        crate::execution_recipe::apply_action_effect_failure(&mut repair_recipe, effective_effect);
        assert_eq!(
            crate::execution_recipe::stop_signal_for_validation_failure(&repair_recipe),
            "recoverable_failure_continue_round"
        );
        assert!(repair_recipe.saw_inspect);
        assert_eq!(
            repair_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Apply
        );

        let repair_result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({ "command": "systemctl restart sing-box" }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s3".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "run_cmd".to_string(),
                        args: json!({ "command": "systemctl is-active sing-box" }),
                        depends_on: vec!["s2".to_string()],
                        why: String::new(),
                    },
                ]),
                execution_recipe: repair_recipe,
            },
            VerifyMode::Enforce,
        );
        assert!(repair_result.approved, "issues: {:?}", repair_result.issues);
        assert!(repair_result.blocked_reason.is_none());
        assert_eq!(repair_result.approved_steps.len(), 2);
        assert!(repair_result.rewritten_steps.is_empty());
        assert_eq!(
            repair_result.approved_steps[1]
                .args
                .get("command")
                .and_then(|value| value.as_str()),
            Some("systemctl is-active sing-box")
        );
    }

    #[test]
    fn ops_recipe_repair_round_rewrites_combined_run_cmd_plan() {
        let state = test_state();
        let task = test_task();
        let mut route = route_result(false);
        route.resolved_intent =
            "repair local demo file and verify it contains ops-repair-ok".to_string();
        route.resume_behavior = crate::ResumeBehavior::ResumeExecute;
        let mut repair_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
            crate::execution_recipe::ExecutionRecipeSpec {
                kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                inspect_first: true,
                validation_required: true,
                max_repairs: 2,
                ..Default::default()
            },
        );
        repair_recipe.saw_inspect = true;
        repair_recipe.phase = crate::execution_recipe::ExecutionRecipePhase::Apply;

        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "printf 'ops-repair-ok\\n' > document/nl_ops_http_demo/index.html & sleep 1 && grep -q 'ops-repair-ok' document/nl_ops_http_demo/index.html && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: repair_recipe,
            },
            VerifyMode::Enforce,
        );
        assert!(result.approved, "issues: {:?}", result.issues);
        assert!(result.blocked_reason.is_none());
        assert_eq!(result.rewritten_steps.len(), 2);
        assert_eq!(result.rewritten_steps[0].step_id, "s2");
        assert_eq!(result.rewritten_steps[1].step_id, "s2__validate");
        assert_eq!(
            result.rewritten_steps[0]
                .args
                .get("command")
                .and_then(|value| value.as_str()),
            Some("printf 'ops-repair-ok\\n' > document/nl_ops_http_demo/index.html &")
        );
        assert_eq!(
            result.rewritten_steps[1]
                .args
                .get("command")
                .and_then(|value| value.as_str()),
            Some(
                "sleep 1 && grep -q 'ops-repair-ok' document/nl_ops_http_demo/index.html && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
            )
        );
        assert_eq!(result.rewritten_steps[1].depends_on, vec!["s2".to_string()]);
    }

    #[test]
    fn ops_recipe_apply_phase_skips_leading_validation_before_mutation() {
        let state = test_state();
        let task = test_task();
        let mut route = route_result(false);
        route.resolved_intent =
            "验证首页包含 ops-repair-ok，失败就修复 document/nl_ops_http_demo/index.html 后重试"
                .to_string();
        let mut apply_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
            crate::execution_recipe::ExecutionRecipeSpec {
                kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                inspect_first: true,
                validation_required: true,
                max_repairs: 2,
                ..Default::default()
            },
        );
        apply_recipe.saw_inspect = true;
        apply_recipe.phase = crate::execution_recipe::ExecutionRecipePhase::Apply;
        let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: Some(
                    "先验证首页是否包含 ops-repair-ok，如果失败就修复 document/nl_ops_http_demo/index.html，然后再次验证直到通过。",
                ),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "http_basic".to_string(),
                        args: json!({ "action": "get", "url": "http://127.0.0.1:51179/" }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "read_file".to_string(),
                        args: json!({ "path": "document/nl_ops_http_demo/index.html" }),
                        depends_on: vec!["s1".to_string()],
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s3".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "write_file".to_string(),
                        args: json!({
                            "path": "document/nl_ops_http_demo/index.html",
                            "content": "ops-repair-ok\n"
                        }),
                        depends_on: vec!["s2".to_string()],
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s4".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "http_basic".to_string(),
                        args: json!({ "action": "get", "url": "http://127.0.0.1:51179/" }),
                        depends_on: vec!["s3".to_string()],
                        why: String::new(),
                    },
                ]),
                execution_recipe: apply_recipe,
            },
            VerifyMode::ObserveOnly,
        );
        assert_eq!(result.rewritten_steps.len(), 3);
        assert_eq!(result.rewritten_steps[0].step_id, "s2");
        assert_eq!(result.rewritten_steps[1].step_id, "s3");
        assert_eq!(result.rewritten_steps[2].step_id, "s4");
    }
}
