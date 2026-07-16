use std::collections::HashSet;

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::{AppState, PlanStep};

pub(crate) const APPROVAL_GRANT_TTL_SECONDS: i64 = 900;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovalBinding {
    pub(crate) action_fingerprint: String,
    pub(crate) arguments_hash: String,
    pub(crate) action_count: usize,
    pub(crate) targets: Vec<String>,
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
    for step in steps
        .iter()
        .filter(|step| confirmation_step_ids.contains(step.step_id.trim()))
    {
        let target = normalized_step_target(state, step);
        if target.is_empty() {
            continue;
        }
        action_bindings.push(json!({
            "action_type": step.action_type.trim().to_ascii_lowercase(),
            "target": target,
        }));
        argument_bindings.push(canonical_value(&step.args));
        targets.push(target);
    }
    if action_bindings.is_empty() {
        return None;
    }

    Some(ApprovalBinding {
        action_fingerprint: sha256_label(
            canonical_value(&Value::Array(action_bindings)).to_string(),
        ),
        arguments_hash: sha256_label(canonical_value(&Value::Array(argument_bindings)).to_string()),
        action_count: targets.len(),
        targets,
    })
}

pub(crate) fn pending_approval_request_json(
    task_id: &str,
    binding: &ApprovalBinding,
    now_ts: i64,
) -> Value {
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

fn sha256_label(value: String) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("sha256:{digest:x}")
}

#[cfg(test)]
#[path = "approval_grant_tests.rs"]
mod tests;
