use std::path::{Component, Path};

use serde_json::{json, Value};

use super::*;

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
    let dry_run_observe_only =
        crate::execution_recipe::dry_run_observes_only_action(&normalized_skill, &step.args);
    let registry_non_mutating_action =
        registry_declares_non_mutating_planner_action(state, &normalized_skill, &step.args);
    let autonomous_workspace_fs_mutation =
        workspace_filesystem_mutation_can_run_autonomously(state, &normalized_skill, &step.args);
    let autonomous_validation_run_cmd =
        validation_run_cmd_can_run_autonomously(state, &normalized_skill, &step.args);
    let requires_confirmation = !dry_run_observe_only
        && !registry_non_mutating_action
        && !autonomous_workspace_fs_mutation
        && !autonomous_validation_run_cmd
        && (state
            .skill_invocation_requires_confirmation_policy(&normalized_skill, Some(&step.args))
            || is_confirmation_like_skill(&normalized_skill)
            || high_risk_side_effect_requires_confirmation(effect, risk_level, &step.args));
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
                            "isolation_profile": mapping.isolation_profile.map(|profile| profile.as_token()),
                            "network_access": mapping.network_access,
                            "filesystem_write": mapping.filesystem_write,
                            "external_publish": mapping.external_publish,
                            "credential_access": mapping.credential_access,
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
    let sandbox_profile = verifier_sandbox_profile_token(registry_policy.as_ref(), effect);

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
        "sandbox_profile": sandbox_profile.clone(),
        "sandbox": verifier_sandbox_summary(registry_policy.as_ref(), &sandbox_profile, effect),
        "workspace_scope": verifier_workspace_scope_summary(
            &step.args,
            &state.skill_rt.workspace_root,
            crate::execution_recipe::action_targets_external_workspace(
                state,
                &normalized_skill,
                &step.args
            )
        ),
        "registry_policy": registry_policy,
    })
}

pub(super) fn verify_permission_decision_json(
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

pub(super) fn audit_permission_decision(
    state: &AppState,
    task: &ClaimedTask,
    permission_decision: &Value,
) {
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

fn fs_basic_action(args: &Value) -> Option<String> {
    args.as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(Value::as_str)
        .map(normalize_schema_token)
        .filter(|action| !action.is_empty())
}

pub(super) fn workspace_filesystem_mutation_can_run_autonomously(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    if normalized_skill != "fs_basic"
        || !matches!(
            fs_basic_action(args).as_deref(),
            Some("make_dir" | "write_text" | "append_text")
        )
    {
        return false;
    }
    path_args(args)
        .into_iter()
        .any(|path| path_value_is_workspace_scoped(path, &state.skill_rt.workspace_root))
}

pub(super) fn validation_run_cmd_can_run_autonomously(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    if normalized_skill != "run_cmd"
        || crate::execution_recipe::action_targets_external_workspace(state, normalized_skill, args)
    {
        return false;
    }
    let effect =
        crate::execution_recipe::classify_skill_action_effect(state, normalized_skill, args);
    effect.validates && !effect.mutates
}

pub(super) fn route_requires_clarify_before_tools(
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

pub(super) fn context_bundle_has_redacted_workspace_child_locator(summary: Option<&str>) -> bool {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let Some(summary) = summary else {
        return false;
    };
    summary.split(START).skip(1).any(|tail| {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            return false;
        };
        value
            .get("current_request_locator")
            .and_then(|locator| locator.get("resolved_workspace_child_redacted"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
}

fn path_args(args: &Value) -> Vec<&str> {
    let Some(obj) = args.as_object() else {
        return Vec::new();
    };
    ["path", "file_path", "target_path", "requested_path"]
        .into_iter()
        .filter_map(|key| {
            obj.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .collect()
}

fn path_value_is_workspace_scoped(path: &str, workspace_root: &Path) -> bool {
    let candidate = Path::new(path);
    if candidate
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return false;
    }
    if !candidate.is_absolute() {
        return true;
    }
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let target = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());
    target.starts_with(root)
}

fn verifier_workspace_scope_summary(
    args: &Value,
    workspace_root: &Path,
    external_workspace: bool,
) -> Value {
    let paths = path_args(args);
    let untrusted_path_present = paths
        .iter()
        .any(|path| !path_value_is_workspace_scoped(path, workspace_root));
    let scope = if untrusted_path_present || external_workspace {
        "external_or_untrusted"
    } else if paths.is_empty() {
        "unspecified"
    } else {
        "workspace_scoped"
    };
    json!({
        "schema_version": 1,
        "scope": scope,
        "path_arg_count": paths.len(),
        "cwd_present": args
            .get("cwd")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty()),
        "untrusted_path_present": untrusted_path_present,
        "external_workspace": external_workspace,
    })
}

fn verifier_sandbox_profile_token(
    registry_policy: Option<&Value>,
    effect: crate::execution_recipe::ActionEffect,
) -> String {
    registry_policy
        .and_then(|policy| policy.get("isolation_profile"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if effect.mutates {
                "local_current_workspace".to_string()
            } else {
                "read_only_or_validation".to_string()
            }
        })
}

fn verifier_sandbox_summary(
    registry_policy: Option<&Value>,
    sandbox_profile: &str,
    effect: crate::execution_recipe::ActionEffect,
) -> Value {
    json!({
        "schema_version": 1,
        "profile": sandbox_profile,
        "source": if registry_policy
            .and_then(|policy| policy.get("isolation_profile"))
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            "registry_capability_policy"
        } else {
            "effect_default"
        },
        "filesystem_write": registry_policy
            .and_then(|policy| policy.get("filesystem_write"))
            .and_then(Value::as_bool)
            .unwrap_or(effect.mutates),
        "network_access": registry_policy
            .and_then(|policy| policy.get("network_access"))
            .and_then(Value::as_bool),
        "external_publish": registry_policy
            .and_then(|policy| policy.get("external_publish"))
            .and_then(Value::as_bool),
        "credential_access": registry_policy
            .and_then(|policy| policy.get("credential_access"))
            .and_then(Value::as_bool),
    })
}

fn args_have_untrusted_path_value(args: &Value, workspace_root: &Path) -> bool {
    path_args(args)
        .into_iter()
        .any(|path| !path_value_is_workspace_scoped(path, workspace_root))
}

pub(super) fn step_reads_path_content_under_unbound_locator(
    step: &PlanStep,
    normalized_skill: &str,
    workspace_root: &Path,
) -> bool {
    if path_args(&step.args).is_empty()
        || !args_have_untrusted_path_value(&step.args, workspace_root)
    {
        return false;
    }
    match step.action_type.as_str() {
        "call_capability" => action_key_reads_path_content(step.skill.trim()),
        "call_skill" | "call_tool" => {
            let action_key =
                crate::evidence_policy::ActionRef::from_skill_args(normalized_skill, &step.args)
                    .map(|action| action.as_key());
            action_key
                .as_deref()
                .is_some_and(action_key_reads_path_content)
        }
        _ => false,
    }
}

fn action_key_reads_path_content(action_key: &str) -> bool {
    let Some(action_key) =
        crate::evidence_policy::ActionRef::parse(action_key).map(|action| action.as_key())
    else {
        return false;
    };
    matches!(
        action_key.as_str(),
        "filesystem.read_text_range"
            | "filesystem.read_file"
            | "filesystem.grep_text"
            | "fs_basic.read_text_range"
            | "fs_basic.grep_text"
            | "system_basic.read_range"
            | "system_basic.read_file"
            | "read_file"
    )
}

pub(super) fn policy_action_reads_path_content_under_unbound_locator(
    action_key: &str,
    args: &Value,
    workspace_root: &Path,
) -> bool {
    args_have_untrusted_path_value(args, workspace_root)
        && action_key_reads_path_content(action_key)
}

pub(super) fn push_unbound_locator_route_clarify_issue(
    issues: &mut Vec<VerifyIssue>,
    step_id: &str,
) {
    if issues.iter().any(|issue| {
        issue.step_id == step_id
            && matches!(issue.kind, VerifyIssueKind::RouteClarifyRequired)
            && issue.detail.contains("resolved_workspace_child_redacted")
    }) {
        return;
    }
    issues.push(VerifyIssue {
        step_id: step_id.to_string(),
        kind: VerifyIssueKind::RouteClarifyRequired,
        detail: "unbound_locator_requires_clarify; boundary=resolved_workspace_child_redacted"
            .to_string(),
        missing_fields: vec!["execution_target_or_boundary".to_string()],
    });
}

fn route_clarify_can_defer_to_runtime_status_plan(
    state: &AppState,
    route: &crate::RouteResult,
    plan_result: &PlanResult,
) -> bool {
    if !route.output_contract.requires_content_evidence
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
