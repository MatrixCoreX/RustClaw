use serde_json::{json, Value};
use std::collections::BTreeSet;

use crate::agent_engine::LoopState;
use crate::ClaimedTask;

pub(in crate::agent_engine) fn valid_capability_call_limits(
    value: &Value,
    allowed: &[String],
) -> bool {
    let Some(items) = value.as_array().filter(|items| items.len() <= 32) else {
        return false;
    };
    let mut seen = BTreeSet::new();
    items.iter().all(|item| {
        let Some(object) = item.as_object() else {
            return false;
        };
        let Some(capability) = item.get("capability").and_then(Value::as_str) else {
            return false;
        };
        object.len() == 2
            && allowed.iter().any(|name| name == capability)
            && seen.insert(capability)
            && item
                .get("max_calls")
                .and_then(Value::as_u64)
                .is_some_and(|limit| limit <= 64)
    })
}

pub(in crate::agent_engine) fn has_scoped_capability_budget(task: &ClaimedTask) -> bool {
    limits(&task.payload_json).is_some_and(|value| value != json!([]))
}

fn limits(payload_json: &str) -> Option<Value> {
    let payload: Value = serde_json::from_str(payload_json).ok()?;
    if !crate::repo::child_tasks::is_child_subagent_payload(&payload) {
        return None;
    }
    payload
        .pointer("/child_task_contract/scope/capability_call_limits")
        .cloned()
}

pub(super) fn capability_budget_error(
    state: &crate::AppState,
    task: &ClaimedTask,
    loop_state: &LoopState,
    capability: &str,
    skill: &str,
) -> Option<String> {
    let limits = limits(&task.payload_json)?;
    let payload: Value = serde_json::from_str(&task.payload_json).ok()?;
    let allowed = payload
        .pointer("/child_task_contract/scope/allowed_capabilities")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !valid_capability_call_limits(&limits, &allowed) {
        return Some(budget_error(
            skill,
            capability,
            "child_capability_budget_invalid",
            0,
            0,
        ));
    }
    let registry = state.get_skills_registry();
    let limit = limits
        .as_array()?
        .iter()
        .filter_map(|item| {
            let name = item.get("capability")?.as_str()?;
            let canonical = registry
                .as_ref()
                .and_then(|registry| registry.canonical_planner_capability_name(name))
                .unwrap_or(name);
            (canonical == capability)
                .then(|| item.get("max_calls").and_then(Value::as_u64))
                .flatten()
        })
        .min()?;
    // Capability results survive checkpoint/resume and include failed calls.
    // Count attempts rather than model prose, replies, or successful reads only.
    let used = loop_state
        .capability_results
        .iter()
        .filter(|result| result.capability == capability)
        .count() as u64;
    (used >= limit).then(|| {
        budget_error(
            skill,
            capability,
            "child_capability_budget_exhausted",
            used,
            limit,
        )
    })
}

pub(in crate::agent_engine) fn planner_budget_context(
    state: &crate::AppState,
    task: &ClaimedTask,
    loop_state: &LoopState,
) -> Option<Value> {
    let limits = limits(&task.payload_json)?;
    let payload: Value = serde_json::from_str(&task.payload_json).ok()?;
    let registry = state.get_skills_registry();
    let remaining=limits.as_array()?.iter().map(|item| {
        let name=item.get("capability").and_then(Value::as_str).unwrap_or_default();
        let canonical=registry.as_ref().and_then(|registry| registry.canonical_planner_capability_name(name)).unwrap_or(name);
        let used=loop_state.capability_results.iter().filter(|r| r.capability==canonical).count() as u64;
        let max_calls=item.get("max_calls").and_then(Value::as_u64).unwrap_or(0);
        json!({"capability":canonical,"max_calls":max_calls,"used_calls":used,"remaining_calls":max_calls.saturating_sub(used)})
    }).collect::<Vec<_>>();
    (!remaining.is_empty()).then(|| json!({
        "schema_version":1,"owner_layer":"child_task_execution_policy",
        "child_task_id":payload.get("child_task_id"),"parent_task_id":payload.get("parent_task_id"),
        "counter_scope":"child_task_across_checkpoint_resume_and_retries",
        "capability_call_limits":remaining,
    }))
}

fn budget_error(skill: &str, capability: &str, code: &str, used: u64, limit: u64) -> String {
    crate::skills::structured_skill_error_from_parts(
        skill,
        code,
        code,
        None,
        Some(json!({
            "owner_layer": "child_task_execution_policy",
            "selected_capability": capability,
            "used_calls": used,
            "max_calls": limit,
            "retryable": false,
            "side_effect_applied": false,
            "recovery_action": "synthesize_from_existing_evidence",
        })),
    )
}

#[cfg(test)]
#[path = "child_capability_budget_tests.rs"]
mod tests;
