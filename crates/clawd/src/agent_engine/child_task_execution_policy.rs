use serde_json::{json, Value};
use std::collections::BTreeSet;

use super::{AppState, ClaimedTask};

const MAX_CHILD_ALLOWED_CAPABILITIES: usize = 32;

struct SelectedCapability {
    canonical_skill: String,
    name: String,
    action: Option<String>,
    effect: String,
    policy: Value,
}

pub(super) fn child_task_execution_policy_error(
    state: &AppState,
    task: &ClaimedTask,
    normalized_skill: &str,
    args: &Value,
) -> Option<String> {
    let payload = serde_json::from_str::<Value>(&task.payload_json).ok()?;
    if !crate::repo::child_tasks::is_child_subagent_payload(&payload) {
        return None;
    }

    let contract = payload.get("child_task_contract")?;
    let permission_profile = contract
        .get("permission_profile")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let (allowed_capabilities, invalid_capability_count, capability_limit_exceeded) =
        child_allowed_capabilities(contract);
    let allowed_capabilities = canonicalize_allowed_capabilities(state, allowed_capabilities);
    let selected = selected_capability(state, normalized_skill, args);
    let mut violations = Vec::new();

    if invalid_capability_count > 0 {
        violations.push("invalid_allowed_capability");
    }
    if capability_limit_exceeded {
        violations.push("allowed_capability_limit_exceeded");
    }
    if allowed_capabilities.is_empty() {
        violations.push("allowed_capabilities_missing");
    }
    match selected.as_ref() {
        None => violations.push("capability_unresolved"),
        Some(capability) if !allowed_capabilities.contains(&capability.name) => {
            violations.push("capability_not_allowed");
        }
        Some(_) => {}
    }

    match (permission_profile, selected.as_ref()) {
        ("read_only", Some(capability)) => {
            append_read_only_violations(capability, &mut violations);
        }
        ("local_worktree", Some(capability)) => {
            append_local_worktree_violations(state, capability, &mut violations);
        }
        ("read_only" | "local_worktree", None) => {}
        _ => violations.push("permission_profile_unsupported"),
    }
    violations.sort_unstable();
    violations.dedup();
    if violations.is_empty() {
        return None;
    }

    let child_task_id = contract
        .get("child_task_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let parent_task_id = contract
        .get("parent_task_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let selected_capability = selected.as_ref().map(|capability| capability.name.as_str());
    let canonical_skill = selected
        .as_ref()
        .map(|capability| capability.canonical_skill.as_str())
        .unwrap_or(normalized_skill);
    let action = selected
        .as_ref()
        .and_then(|capability| capability.action.as_deref());
    let capability_policy = selected
        .as_ref()
        .map(|capability| capability.policy.clone())
        .unwrap_or(Value::Null);

    Some(crate::skills::structured_skill_error_from_parts(
        normalized_skill,
        "child_task_policy_violation",
        "child_task_policy_violation",
        None,
        Some(json!({
            "reason_code": "child_task_policy_violation",
            "failure_attribution":
                crate::evidence_policy::FailureAttribution::PermissionDenied.as_str(),
            "decision":
                crate::evidence_policy::ActionPolicyDecision::RejectedForbidden.as_str(),
            "policy_decision": crate::policy_decision::PolicyDecision::Deny.as_token(),
            "owner_layer": "child_task_execution_policy",
            "parent_task_id": parent_task_id,
            "child_task_id": child_task_id,
            "permission_profile": permission_profile,
            "canonical_skill": canonical_skill,
            "action": action,
            "selected_capability": selected_capability,
            "allowed_capabilities": allowed_capabilities,
            "invalid_allowed_capability_count": invalid_capability_count,
            "violations": violations,
            "capability_policy": capability_policy,
        })),
    ))
}

fn child_allowed_capabilities(contract: &Value) -> (BTreeSet<String>, usize, bool) {
    let Some(items) = contract
        .pointer("/scope/allowed_capabilities")
        .and_then(Value::as_array)
    else {
        return (BTreeSet::new(), 0, false);
    };
    let capability_limit_exceeded = items.len() > MAX_CHILD_ALLOWED_CAPABILITIES;
    let mut allowed = BTreeSet::new();
    let mut invalid_count = 0;
    for value in items.iter().take(MAX_CHILD_ALLOWED_CAPABILITIES) {
        let Some(token) = value.as_str().map(str::trim) else {
            invalid_count += 1;
            continue;
        };
        if !is_machine_capability_token(token) {
            invalid_count += 1;
            continue;
        }
        allowed.insert(token.to_string());
    }
    (allowed, invalid_count, capability_limit_exceeded)
}

fn canonicalize_allowed_capabilities(
    state: &AppState,
    allowed: BTreeSet<String>,
) -> BTreeSet<String> {
    let registry = state.get_skills_registry();
    allowed
        .into_iter()
        .map(|capability| {
            registry
                .as_ref()
                .and_then(|registry| {
                    registry
                        .canonical_planner_capability_name(&capability)
                        .map(ToString::to_string)
                })
                .unwrap_or(capability)
        })
        .collect()
}

fn is_machine_capability_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value.chars().all(|ch| {
            ch.is_ascii_lowercase()
                || ch.is_ascii_digit()
                || matches!(ch, '_' | '-' | '.' | ':' | '/')
        })
}

fn selected_capability(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> Option<SelectedCapability> {
    let canonical_skill = state.resolve_canonical_skill_name(normalized_skill);
    let action = normalized_action(args);
    if let Some(tool) = state.mcp_tool(&canonical_skill) {
        return Some(SelectedCapability {
            name: tool.capability,
            canonical_skill,
            action,
            effect: tool.policy.effect.clone(),
            policy: tool.policy.permission_policy_json(),
        });
    }
    let manifest = state.skill_manifest(&canonical_skill)?;
    let mapping = claw_core::skill_registry::select_planner_capability_mapping(
        &manifest.planner_capabilities,
        action.as_deref(),
    )?;
    Some(SelectedCapability {
        canonical_skill,
        name: mapping.name.clone(),
        action,
        effect: mapping
            .effect
            .map(|effect| effect.as_token().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        policy: json!({
            "source": "registry_capability_policy",
            "effect": mapping.effect.map(|effect| effect.as_token()),
            "isolation_profile": mapping
                .isolation_profile
                .map(|profile| profile.as_token()),
            "network_access": mapping.network_access,
            "filesystem_write": mapping.filesystem_write,
            "external_publish": mapping.external_publish,
            "credential_access": mapping.credential_access,
            "subprocess": mapping.subprocess,
            "package_install": mapping.package_install,
            "privilege_escalation": mapping.privilege_escalation,
        }),
    })
}

fn normalized_action(args: &Value) -> Option<String> {
    args.get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
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
        })
}

fn append_read_only_violations(
    capability: &SelectedCapability,
    violations: &mut Vec<&'static str>,
) {
    if !matches!(capability.effect.as_str(), "observe" | "validate") {
        violations.push("read_only_effect_required");
    }
    for field in [
        "network_access",
        "filesystem_write",
        "external_publish",
        "credential_access",
        "subprocess",
        "package_install",
        "privilege_escalation",
    ] {
        if capability.policy.get(field).and_then(Value::as_bool) == Some(true) {
            violations.push(field);
        }
    }
}

fn append_local_worktree_violations(
    state: &AppState,
    capability: &SelectedCapability,
    violations: &mut Vec<&'static str>,
) {
    if crate::execution_isolation::execution_isolation_root_profile(&state.skill_rt.workspace_root)
        .as_deref()
        != Some("local_worktree")
    {
        violations.push("child_worktree_binding_required");
    }
    for field in [
        "network_access",
        "external_publish",
        "credential_access",
        "package_install",
        "privilege_escalation",
    ] {
        if capability.policy.get(field).and_then(Value::as_bool) == Some(true) {
            violations.push(field);
        }
    }
    let mutates_workspace = capability.effect == "mutate"
        || capability
            .policy
            .get("filesystem_write")
            .and_then(Value::as_bool)
            == Some(true);
    if mutates_workspace {
        let profile = capability
            .policy
            .get("isolation_profile")
            .and_then(Value::as_str);
        if !matches!(profile, Some("local_worktree" | "local_current_workspace")) {
            violations.push("local_worktree_isolation_required");
        }
    }
}
