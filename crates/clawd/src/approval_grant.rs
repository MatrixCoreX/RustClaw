use std::collections::HashSet;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::{AppState, PlanStep};

pub(crate) const APPROVAL_GRANT_TTL_SECONDS: i64 = 900;
pub(crate) const APPROVAL_SCOPE_GRANT_TTL_SECONDS: i64 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovalDecision {
    ApproveOnce,
    AlwaysForScope,
    Deny,
}

impl ApprovalDecision {
    pub(crate) fn parse_token(value: &str) -> Option<Self> {
        match value.trim() {
            "approve_once" => Some(Self::ApproveOnce),
            "always_for_scope" => Some(Self::AlwaysForScope),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }

    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::ApproveOnce => "approve_once",
            Self::AlwaysForScope => "always_for_scope",
            Self::Deny => "deny",
        }
    }

    pub(crate) fn grants_execution(self) -> bool {
        matches!(self, Self::ApproveOnce | Self::AlwaysForScope)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ApprovalScopeEntry {
    pub(crate) capability: String,
    pub(crate) action: String,
    pub(crate) effect: String,
    pub(crate) resource_kind: String,
    pub(crate) resources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ApprovalScopeBinding {
    pub(crate) scope_kind: String,
    pub(crate) scope_fingerprint: String,
    pub(crate) entries: Vec<ApprovalScopeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovalBinding {
    pub(crate) action_fingerprint: String,
    pub(crate) arguments_hash: String,
    pub(crate) action_count: usize,
    pub(crate) targets: Vec<String>,
    pub(crate) scope: Option<ApprovalScopeBinding>,
}

pub(crate) fn binding_for_confirmation_steps(
    state: &AppState,
    steps: &[PlanStep],
    confirmation_step_ids: &[String],
) -> Option<ApprovalBinding> {
    let confirmation_step_ids = confirmation_step_ids
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>();
    if confirmation_step_ids.is_empty() {
        return None;
    }

    let mut action_bindings = Vec::new();
    let mut argument_bindings = Vec::new();
    let mut targets = Vec::new();
    let mut scope_entries = Vec::new();
    let mut scope_grantable = true;
    for step in steps
        .iter()
        .filter(|step| confirmation_step_ids.contains(step.step_id.trim()))
    {
        let canonical_step = canonical_approval_step(state, step);
        let step = &canonical_step;
        let target = normalized_step_target(state, step);
        if target.is_empty() {
            continue;
        }
        action_bindings.push(json!({
            "action_type": step.action_type.trim().to_ascii_lowercase(),
            "target": target,
        }));
        argument_bindings.push(canonical_approval_arguments(state, step));
        targets.push(target);
        match scope_entry_for_step(state, step) {
            Some(entry) => scope_entries.push(entry),
            None => scope_grantable = false,
        }
    }
    if action_bindings.is_empty() {
        return None;
    }

    let scope = if scope_grantable && scope_entries.len() == targets.len() {
        let scope_value = serde_json::to_value(&scope_entries).ok()?;
        Some(ApprovalScopeBinding {
            scope_kind: "session".to_string(),
            scope_fingerprint: sha256_label(canonical_value(&scope_value).to_string()),
            entries: scope_entries,
        })
    } else {
        None
    };
    Some(ApprovalBinding {
        action_fingerprint: sha256_label(
            canonical_value(&Value::Array(action_bindings)).to_string(),
        ),
        arguments_hash: sha256_label(canonical_value(&Value::Array(argument_bindings)).to_string()),
        action_count: targets.len(),
        targets,
        scope,
    })
}

fn canonical_approval_step(state: &AppState, step: &PlanStep) -> PlanStep {
    if step.action_type != "call_capability" {
        return step.clone();
    }
    let Some(action) = crate::capability_resolver::resolve_capability_action_for_state(
        state,
        &step.skill,
        step.args.clone(),
    ) else {
        return step.clone();
    };
    let action = crate::agent_engine::normalize_resolved_planner_action_for_verifier(state, action);
    crate::plan_step_from_agent_action(
        &action,
        step.step_id.clone(),
        step.depends_on.clone(),
        step.why.clone(),
    )
}

pub(crate) fn pending_approval_request_json(
    task_id: &str,
    binding: &ApprovalBinding,
    now_ts: i64,
) -> Value {
    let allowed_decisions = if binding.scope.is_some() {
        json!(["approve_once", "always_for_scope", "deny"])
    } else {
        json!(["approve_once", "deny"])
    };
    let scope_grant = binding.scope.as_ref().map_or_else(
        || {
            json!({
                "available": false,
                "scope_kind": Value::Null,
                "scope_fingerprint": Value::Null,
                "entries": [],
            })
        },
        |scope| {
            json!({
                "available": true,
                "scope_kind": scope.scope_kind,
                "scope_fingerprint": scope.scope_fingerprint,
                "entries": scope.entries,
                "max_ttl_seconds": APPROVAL_SCOPE_GRANT_TTL_SECONDS,
            })
        },
    );
    json!({
        "schema_version": 1,
        "request_id": format!("approval-{}", uuid::Uuid::new_v4()),
        "task_id": task_id,
        "status": "pending",
        "action_fingerprint": binding.action_fingerprint,
        "arguments_hash": binding.arguments_hash,
        "action_count": binding.action_count,
        "targets": binding.targets,
        "issued_at": now_ts,
        "expires_at": now_ts.saturating_add(APPROVAL_GRANT_TTL_SECONDS),
        "effect": "mutating_or_external_action",
        "reason_code": "explicit_approval_required",
        "reversible": false,
        "next_safe_action": "approve_exact_action_set",
        "allowed_decisions": allowed_decisions,
        "scope_grant": scope_grant,
    })
}

pub(crate) fn confirmation_step_ids(issues: &[crate::verifier::VerifyIssue]) -> Vec<String> {
    issues
        .iter()
        .filter(|issue| issue.kind == crate::verifier::VerifyIssueKind::ConfirmationRequired)
        .map(|issue| issue.step_id.trim().to_string())
        .filter(|step_id| !step_id.is_empty())
        .collect()
}

pub(crate) fn apply_consumed_grant_to_permission_decision(
    permission_decision: &mut Value,
    confirmation_step_ids: &[String],
    grant_decision: Value,
) {
    let step_ids = confirmation_step_ids
        .iter()
        .map(|value| value.trim())
        .collect::<HashSet<_>>();
    if let Some(steps) = permission_decision
        .get_mut("steps")
        .and_then(Value::as_array_mut)
    {
        for step in steps {
            let matches = step
                .get("step_id")
                .and_then(Value::as_str)
                .is_some_and(|step_id| step_ids.contains(step_id.trim()));
            if !matches {
                continue;
            }
            step["decision"] = json!(crate::policy_decision::PolicyDecision::Allow.as_token());
            step["requires_confirmation"] = json!(false);
            step["approval_grant_consumed"] = json!(true);
        }
    }
    permission_decision["approval_grant"] = grant_decision;
}

fn normalized_step_target(state: &AppState, step: &PlanStep) -> String {
    let target = step.skill.trim().to_ascii_lowercase();
    match step.action_type.trim() {
        "call_skill" | "call_tool" => state.resolve_canonical_skill_name(&target),
        _ => target,
    }
}

fn scope_entry_for_step(state: &AppState, step: &PlanStep) -> Option<ApprovalScopeEntry> {
    if !matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
        return None;
    }
    let skill = state.resolve_canonical_skill_name(step.skill.trim());
    if skill == "run_cmd" {
        return None;
    }
    let action = step
        .args
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_machine_token)?;
    let manifest = state.skill_manifest(&skill)?;
    let mapping = claw_core::skill_registry::select_planner_capability_mapping(
        &manifest.planner_capabilities,
        Some(&action),
    )?;
    if mapping.effect.map(|effect| effect.as_token()) != Some("mutate")
        || mapping.filesystem_write != Some(true)
        || mapping.network_access == Some(true)
        || mapping.external_publish == Some(true)
        || mapping.credential_access == Some(true)
        || mapping.package_install == Some(true)
        || mapping.privilege_escalation == Some(true)
    {
        return None;
    }
    let (resource_kind, resources) = scoped_resources(state, &step.args)?;
    Some(ApprovalScopeEntry {
        capability: mapping.name.clone(),
        action: mapping
            .action
            .as_deref()
            .map(normalize_machine_token)
            .unwrap_or(action),
        effect: "mutate".to_string(),
        resource_kind,
        resources,
    })
}

fn scoped_resources(state: &AppState, args: &Value) -> Option<(String, Vec<String>)> {
    let object = args.as_object()?;
    for key in ["path", "paths"] {
        let Some(value) = object.get(key) else {
            continue;
        };
        let values = match value {
            Value::String(value) => vec![value.as_str()],
            Value::Array(values) => values.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        let mut resources = values
            .into_iter()
            .map(|value| normalized_workspace_resource(&state.skill_rt.workspace_root, value))
            .collect::<Option<Vec<_>>>()?;
        resources.sort();
        resources.dedup();
        if !resources.is_empty() {
            return Some(("workspace_path".to_string(), resources));
        }
    }
    for key in ["checkpoint_id", "child_task_id"] {
        let Some(value) = object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        return Some((
            key.to_string(),
            vec![normalize_machine_token(value).chars().take(256).collect()],
        ));
    }
    None
}

fn normalized_workspace_resource(workspace_root: &Path, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let raw_path = Path::new(raw);
    let relative = if raw_path.is_absolute() {
        raw_path.strip_prefix(&root).ok()?
    } else {
        raw_path
    };
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn normalize_machine_token(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace("::", ".")
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            let mut canonical = Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonical_value(&object[key]));
            }
            Value::Object(canonical)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        _ => value.clone(),
    }
}

fn canonical_approval_arguments(state: &AppState, step: &PlanStep) -> Value {
    let default_cwd_is_implicit =
        state.resolve_canonical_skill_name(step.skill.trim()) == "run_cmd";
    canonical_approval_value(
        &step.args,
        default_cwd_is_implicit.then_some(state.skill_rt.workspace_root.as_path()),
    )
}

fn canonical_approval_value(value: &Value, implicit_cwd_root: Option<&Path>) -> Value {
    match value {
        Value::Object(object) => {
            let mut canonical = Map::new();
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if key.starts_with("_clawd_") {
                    continue;
                }
                if key.as_str() == "cwd"
                    && implicit_cwd_root.is_some_and(|root| {
                        object[key]
                            .as_str()
                            .is_some_and(|cwd| cwd_resolves_to_workspace_root(root, cwd))
                    })
                {
                    continue;
                }
                canonical.insert(key.clone(), canonical_approval_value(&object[key], None));
            }
            Value::Object(canonical)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| canonical_approval_value(item, None))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn cwd_resolves_to_workspace_root(workspace_root: &Path, raw_cwd: &str) -> bool {
    let raw_cwd = raw_cwd.trim();
    if raw_cwd.is_empty() {
        return false;
    }
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let cwd = Path::new(raw_cwd);
    let resolved = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        root.join(cwd)
    };
    resolved.canonicalize().unwrap_or(resolved) == root
}

fn sha256_label(value: String) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("sha256:{digest:x}")
}

#[cfg(test)]
#[path = "approval_grant_tests.rs"]
mod tests;
