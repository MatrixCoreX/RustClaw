use std::collections::{HashMap, HashSet};

use claw_core::skill_registry::PrimaryFallbackRole;

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
    MissingRequiredArg,
    InvalidDependsOn,
    ConfirmationRequired,
    PrimaryFallbackConflict,
    RouteClarifyRequired,
    RecipeInspectBeforeMutateRequired,
    RecipeValidationAfterMutateRequired,
    RecipeTargetScopeRequired,
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
            Self::MissingRequiredArg => "MissingRequiredArg",
            Self::InvalidDependsOn => "InvalidDependsOn",
            Self::ConfirmationRequired => "ConfirmationRequired",
            Self::PrimaryFallbackConflict => "PrimaryFallbackConflict",
            Self::RouteClarifyRequired => "RouteClarifyRequired",
            Self::RecipeInspectBeforeMutateRequired => "RecipeInspectBeforeMutateRequired",
            Self::RecipeValidationAfterMutateRequired => "RecipeValidationAfterMutateRequired",
            Self::RecipeTargetScopeRequired => "RecipeTargetScopeRequired",
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
        "run_cmd" | "write_file" | "remove_file" | "make_dir" | "schedule"
    )
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
    issues: &mut Vec<VerifyIssue>,
) {
    let manifest_required = manifest_required_args(state, normalized_skill);
    let fallback_required = required_args_for_skill(normalized_skill);
    let required: Vec<String> = if manifest_required.is_empty() {
        fallback_required
            .iter()
            .map(|key| (*key).to_string())
            .collect()
    } else {
        manifest_required
    };
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
        let missing = obj
            .get(key)
            .map(|v| {
                (v.is_string() && v.as_str().map(str::trim).unwrap_or("").is_empty()) || v.is_null()
            })
            .unwrap_or(true);
        if missing {
            issues.push(VerifyIssue {
                step_id: step.step_id.clone(),
                kind: VerifyIssueKind::MissingRequiredArg,
                detail: format!("skill `{normalized_skill}` missing required arg `{key}`"),
            });
        }
    }
}

fn issue_blocks_in_enforce(kind: VerifyIssueKind) -> bool {
    matches!(
        kind,
        VerifyIssueKind::SkillNotVisible
            | VerifyIssueKind::MissingRequiredArg
            | VerifyIssueKind::InvalidDependsOn
            | VerifyIssueKind::PrimaryFallbackConflict
            | VerifyIssueKind::RouteClarifyRequired
            | VerifyIssueKind::RecipeInspectBeforeMutateRequired
            | VerifyIssueKind::RecipeValidationAfterMutateRequired
            | VerifyIssueKind::RecipeTargetScopeRequired
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

fn marker_candidate_from_text(rest: &str) -> Option<String> {
    let trimmed =
        rest.trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, ':' | '：' | '='));
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return None;
    };
    if matches!(first, '"' | '\'' | '`' | '“' | '”' | '‘' | '’') {
        let quote = first;
        let tail = &trimmed[first.len_utf8()..];
        let end = tail.find(quote).unwrap_or(tail.len());
        let value = tail[..end].trim();
        return (!value.is_empty()).then(|| value.to_string());
    }
    let value = trimmed
        .chars()
        .take_while(|ch| {
            !ch.is_whitespace()
                && !matches!(
                    ch,
                    ',' | '，'
                        | ';'
                        | '；'
                        | '.'
                        | '。'
                        | '!'
                        | '！'
                        | '?'
                        | '？'
                        | ')'
                        | '）'
                        | '('
                        | '（'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                )
        })
        .collect::<String>()
        .trim()
        .to_string();
    (!value.is_empty()).then_some(value)
}

pub(crate) fn extract_expected_http_marker(
    route_result: Option<&crate::RouteResult>,
    request_text: Option<&str>,
) -> Option<String> {
    let route_texts = route_result.map(|route| {
        vec![
            route.resolved_intent.as_str(),
            route.route_reason.as_str(),
            route.output_contract.locator_hint.as_str(),
        ]
    });
    let mut texts = Vec::new();
    if let Some(route_texts) = route_texts {
        texts.extend(route_texts);
    }
    if let Some(request_text) = request_text.map(str::trim).filter(|text| !text.is_empty()) {
        texts.push(request_text);
    }
    for text in texts {
        let lower = text.to_ascii_lowercase();
        for keyword in ["contains ", "containing ", "contain "] {
            if let Some(idx) = lower.find(keyword) {
                if let Some(marker) = marker_candidate_from_text(&text[idx + keyword.len()..]) {
                    return Some(marker);
                }
            }
        }
        for keyword in ["包含", "含有"] {
            if let Some(idx) = text.find(keyword) {
                if let Some(marker) = marker_candidate_from_text(&text[idx + keyword.len()..]) {
                    return Some(marker);
                }
            }
        }
    }
    None
}

fn request_requires_success_marker(
    route_result: Option<&crate::RouteResult>,
    request_text: Option<&str>,
) -> bool {
    let mut texts = Vec::new();
    if let Some(route) = route_result {
        texts.push(route.resolved_intent.as_str());
        texts.push(route.route_reason.as_str());
    }
    if let Some(request_text) = request_text.map(str::trim).filter(|text| !text.is_empty()) {
        texts.push(request_text);
    }
    texts
        .into_iter()
        .any(|text| text.to_ascii_uppercase().contains("VALIDATION_PASSED"))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn run_cmd_looks_validation(command_lower: &str) -> bool {
    command_lower.contains("curl ")
        || command_lower.contains("wget ")
        || command_lower.contains("nc ")
        || command_lower.contains("grep")
        || command_lower.contains("systemctl is-active")
        || command_lower.contains("systemctl status")
        || command_lower.contains(" service status")
        || command_lower.contains("service --status-all")
        || command_lower.contains("nginx -t")
        || command_lower.contains("sing-box check")
        || command_lower.contains("ss ")
        || command_lower.contains("lsof ")
}

fn decorate_validation_command_with_success_marker(
    command: &str,
    expected_http_marker: Option<&str>,
    request_requires_marker: bool,
) -> Option<String> {
    if !request_requires_marker {
        return None;
    }
    let lower = command.trim().to_ascii_lowercase();
    if lower.contains("validation_passed") || lower.contains("validation_failed") {
        return None;
    }
    if !run_cmd_looks_validation(&lower) {
        return None;
    }
    if let Some(expected_http_marker) = expected_http_marker {
        if lower.contains("curl ") || lower.contains("wget ") || lower.contains("nc ") {
            return Some(format!(
                "{command} | grep -q {} && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'",
                shell_single_quote(expected_http_marker)
            ));
        }
    }
    Some(format!(
        "{command} && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
    ))
}

fn rewrite_execution_recipe_steps(
    state: &AppState,
    route_result: Option<&crate::RouteResult>,
    request_text: Option<&str>,
    plan_result: &PlanResult,
    recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
) -> Vec<PlanStep> {
    if !matches!(
        recipe.kind,
        crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop
    ) {
        return Vec::new();
    }
    let expected_http_marker = extract_expected_http_marker(route_result, request_text);
    let request_requires_marker = request_requires_success_marker(route_result, request_text);
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
            if effect.validates && !effect.mutates {
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
            && state.resolve_canonical_skill_name(&step.skill) == "http_basic"
        {
            let mut http_step = step.clone();
            if let Some(expected) = expected_http_marker.as_deref() {
                if http_step
                    .args
                    .get("expect_contains")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    if let Some(obj) = http_step.args.as_object_mut() {
                        obj.insert(
                            "expect_contains".to_string(),
                            serde_json::Value::String(expected.to_string()),
                        );
                        changed = true;
                    }
                }
            }
            rewritten.push(http_step);
            continue;
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
                        let validate_command = decorate_validation_command_with_success_marker(
                            &validate_command,
                            expected_http_marker.as_deref(),
                            request_requires_marker,
                        )
                        .unwrap_or(validate_command);
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
                if let Some(decorated_command) = decorate_validation_command_with_success_marker(
                    command,
                    expected_http_marker.as_deref(),
                    request_requires_marker,
                ) {
                    let mut validate_step = step.clone();
                    if let Some(obj) = validate_step.args.as_object_mut() {
                        obj.insert(
                            "command".to_string(),
                            serde_json::Value::String(decorated_command),
                        );
                        obj.remove("timeout_seconds");
                    }
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

fn first_shadow_blocked_reason(issues: &[VerifyIssue]) -> Option<String> {
    issues
        .iter()
        .find(|issue| issue_blocks_in_enforce(issue.kind))
        .map(|issue| issue.detail.clone())
}

pub(crate) fn verify_plan(
    state: &AppState,
    task: &ClaimedTask,
    input: VerifyInput<'_>,
    mode: VerifyMode,
) -> VerifyResult {
    if input
        .route_result
        .map(|route| route.needs_clarify)
        .unwrap_or(false)
        && input
            .plan_result
            .steps
            .iter()
            .any(|step| matches!(step.action_type.as_str(), "call_skill" | "call_tool"))
    {
        let detail = format!(
            "route requires clarify before execution; context={}",
            input.context_bundle_summary.unwrap_or("<none>")
        );
        let shadow_blocked_reason = Some(detail.clone());
        let blocked_reason = matches!(mode, VerifyMode::Enforce).then_some(detail.clone());
        return VerifyResult {
            mode,
            approved: blocked_reason.is_none(),
            blocked_reason,
            shadow_blocked_reason,
            approved_steps: input.plan_result.steps.clone(),
            needs_confirmation: false,
            rewritten_steps: Vec::new(),
            issues: vec![VerifyIssue {
                step_id: "route".to_string(),
                kind: VerifyIssueKind::RouteClarifyRequired,
                detail,
            }],
        };
    }
    let visible_skills: HashSet<String> = state
        .planner_visible_skills_for_task(task)
        .into_iter()
        .collect();
    let all_step_ids: HashSet<String> = input
        .plan_result
        .steps
        .iter()
        .map(|step| step.step_id.clone())
        .collect();
    let confirmation_already_granted = route_has_confirmation_resume(input.route_result);
    let mut issues = Vec::new();
    let mut needs_confirmation = false;

    for step in &input.plan_result.steps {
        if matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
            let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
            if !visible_skills.contains(&normalized_skill) {
                issues.push(VerifyIssue {
                    step_id: step.step_id.clone(),
                    kind: VerifyIssueKind::SkillNotVisible,
                    detail: format!("skill `{normalized_skill}` is not in planner visible skills"),
                });
            }
            verify_step_args(state, step, &normalized_skill, &mut issues);
            if !confirmation_already_granted
                && (state.skill_requires_confirmation_policy(&normalized_skill)
                    || is_confirmation_like_skill(&normalized_skill))
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
    }

    verify_primary_fallback_conflicts(state, input.plan_result, &mut issues);
    verify_execution_recipe(
        state,
        input.plan_result,
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
    let approved_steps = input.plan_result.steps.clone();
    let rewritten_steps = rewrite_execution_recipe_steps(
        state,
        input.route_result,
        input.request_text,
        input.plan_result,
        input.execution_recipe,
    );

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
    

    use claw_core::config::{
        AgentConfig, ToolsConfig,
    };
    use claw_core::skill_registry::SkillsRegistry;
    
    use serde_json::json;
    

    use super::{verify_plan, VerifyInput, VerifyIssueKind, VerifyMode};
    use crate::{
        AgentRuntimeConfig, AppState, ClaimedTask, PlanKind, PlanResult,
        PlanStep, RouteResult, RoutedMode, ScheduleKind,
        SkillViewsSnapshot, ToolsPolicy,
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
            ["read_file", "run_cmd", "primary_reader", "fallback_reader"]
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
        RouteResult {
            routed_mode: RoutedMode::ChatAct,
            resolved_intent: "test".to_string(),
            needs_clarify,
            route_reason: "test".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: vec!["read_file".to_string()],
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
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
    fn code_change_recipe_accepts_cargo_check_verification() {
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
                        args: json!({ "command": "cd /opt/other-project && cargo check" }),
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
                    args: json!({ "command": "cargo check" }),
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
                    args: json!({ "command": "cargo check -p clawd" }),
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
    fn ops_recipe_rewrites_validation_run_cmd_with_explicit_success_marker() {
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
            Some(
                "sleep 2 && curl -s http://127.0.0.1:51179/ | grep -q 'ops-demo-ok' && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
            )
        );
    }

    #[test]
    fn ops_recipe_injects_http_expect_contains_marker() {
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
        assert_eq!(result.rewritten_steps.len(), 1);
        assert_eq!(
            result.rewritten_steps[0]
                .args
                .get("expect_contains")
                .and_then(|value| value.as_str()),
            Some("ops-repair-ok")
        );
    }

    #[test]
    fn ops_recipe_injects_http_expect_contains_marker_from_request_text_fallback() {
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
        assert_eq!(result.rewritten_steps.len(), 1);
        assert_eq!(
            result.rewritten_steps[0]
                .args
                .get("expect_contains")
                .and_then(|value| value.as_str()),
            Some("ops-repair-ok")
        );
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
                        "url": "http://127.0.0.1:51179/"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: initial_recipe,
            },
            VerifyMode::ObserveOnly,
        );
        let inspect_step = &inspect_result.rewritten_steps[0];
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
