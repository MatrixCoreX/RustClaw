use std::path::{Component, Path};

use claw_core::capability_result::{CapabilityResultEnvelope, CapabilityResultStatus};
use serde_json::{json, Value};

use super::{AgentLoopGuardPolicy, LoopState};
use crate::{AgentAction, AppState, ClaimedTask};

const PREFIX: &str = "workspace_revision_v1:";

pub(super) fn execution_fingerprint(
    state: &AppState,
    task: &ClaimedTask,
    policy: &AgentLoopGuardPolicy,
    loop_state: &LoopState,
    action: &AgentAction,
) -> String {
    let base = super::action_fingerprint_for_policy(state, policy, action);
    let Some(path) = local_primitive_path(state, action) else {
        return base;
    };
    revision_fingerprint(&base, &path, &task.task_id, &loop_state.capability_results)
}

pub(super) fn local_primitive_path(state: &AppState, action: &AgentAction) -> Option<String> {
    let resolved =
        crate::capability_resolver::resolve_agent_action_for_state(state, action.clone());
    let (name, args) = match resolved {
        AgentAction::CallTool { tool, args } => (tool, args),
        AgentAction::CallSkill { skill, args } => (skill, args),
        _ => return None,
    };
    let name = state.resolve_canonical_skill_name(&name);
    let (name, args) = crate::virtual_tools::canonicalize_legacy_tool_call(&name, args.clone())
        .map(|call| (call.tool, call.args))
        .unwrap_or((name, args));
    let rewritten = crate::virtual_tools::rewrite_virtual_tool_call(&name, args.clone()).ok()?;
    let (name, args) = rewritten
        .map(|call| (call.runtime_tool, call.runtime_args))
        .unwrap_or((name, args));
    // Only host file primitives participate; arbitrary process/network effects
    // keep their task-wide idempotency keys even if they return similar JSON.
    if !matches!(name.as_str(), "write_file" | "make_dir" | "remove_file")
        && !(name == "system_basic"
            && args.get("action").and_then(Value::as_str) == Some("read_range"))
    {
        return None;
    }
    let requested = Path::new(args.get("path")?.as_str()?);
    let root = &state.skill_rt.workspace_root;
    let canonical_root = root.canonicalize().ok()?;
    let relative = if requested.is_absolute() {
        requested
            .strip_prefix(root)
            .or_else(|_| requested.strip_prefix(&canonical_root))
            .ok()?
    } else {
        requested
    };
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn original_fingerprint(value: &str) -> Option<String> {
    let encoded = value.strip_prefix(PREFIX)?;
    serde_json::from_str::<Value>(encoded)
        .ok()?
        .get("action")?
        .as_str()
        .map(str::to_string)
}

fn revision_fingerprint(
    base: &str,
    path: &str,
    task_id: &str,
    results: &[CapabilityResultEnvelope],
) -> String {
    let trusted = |result: &&CapabilityResultEnvelope| {
        result.status == CapabilityResultStatus::Ok
            && result.provenance.get("source").and_then(Value::as_str) == Some("runtime_step")
            && result.provenance.get("task_id").and_then(Value::as_str) == Some(task_id)
    };
    let Some((index, previous)) = results.iter().enumerate().rev().find(|(_, result)| {
        trusted(result)
            && result
                .provenance
                .get("action_fingerprint")
                .and_then(Value::as_str)
                .is_some_and(|fingerprint| {
                    fingerprint == base
                        || original_fingerprint(fingerprint).as_deref() == Some(base)
                })
    }) else {
        return base.to_string();
    };
    let previous_key = previous.provenance["action_fingerprint"]
        .as_str()
        .unwrap_or(base);
    for result in results[index + 1..].iter().rev().filter(trusted) {
        let Some(output) = result.data.get("output") else {
            continue;
        };
        if result.effect.as_deref() != Some("mutate")
            || result
                .provenance
                .get("host_workspace_primitive")
                .and_then(Value::as_bool)
                != Some(true)
            || output.get("source").and_then(Value::as_str) != Some("workspace_mutation")
            || output.get("isolation_root").and_then(Value::as_str) != Some("workspace://current")
            || output.get("state").and_then(Value::as_str) != Some("applied")
            || output.get("target_path").and_then(Value::as_str) != Some(path)
        {
            continue;
        }
        let Some(revision) = output
            .get("mutation_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(step_id) = result
            .provenance
            .get("step_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        return format!(
            "{PREFIX}{}",
            json!({"action": base, "revision": revision, "step_id": step_id})
        );
    }
    previous_key.to_string()
}

#[cfg(test)]
#[path = "workspace_action_revision_tests.rs"]
mod tests;
