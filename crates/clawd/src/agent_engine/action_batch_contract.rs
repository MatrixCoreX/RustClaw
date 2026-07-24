use serde_json::Value;

use crate::{AgentAction, AppState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SameTurnActionClass {
    IndependentRead,
    MaterialBoundary,
    Discussion,
}

fn value_contains_runtime_reference(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            let mut remainder = text.as_str();
            while let Some(open) = remainder.find("{{") {
                let after_open = &remainder[open + 2..];
                let Some(close) = after_open.find("}}") else {
                    return false;
                };
                let token = after_open[..close].trim();
                if !token.is_empty()
                    && token.len() <= 160
                    && token.chars().all(|ch| {
                        ch.is_ascii_alphanumeric()
                            || matches!(
                                ch,
                                '_' | '-' | '.' | '[' | ']' | '"' | '\'' | ' ' | '\t' | '\n'
                            )
                    })
                {
                    return true;
                }
                remainder = &after_open[close + 2..];
            }
            false
        }
        Value::Array(items) => items.iter().any(value_contains_runtime_reference),
        Value::Object(fields) => fields.values().any(value_contains_runtime_reference),
        _ => false,
    }
}

fn resolved_executable_action(state: &AppState, action: &AgentAction) -> Option<AgentAction> {
    match action {
        AgentAction::CallCapability { .. } => {
            let resolved =
                crate::capability_resolver::resolve_agent_action_for_state(state, action.clone());
            (!matches!(resolved, AgentAction::CallCapability { .. })).then_some(resolved)
        }
        AgentAction::CallTool { .. } | AgentAction::CallSkill { .. } => Some(action.clone()),
        AgentAction::Think { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. } => None,
    }
}

fn policy_proves_parallel_read(state: &AppState, executable: &str, args: &Value) -> bool {
    if let Some(tool) = state.mcp_tool(executable) {
        return tool.policy.effect == "observe"
            && tool.policy.idempotent
            && !tool.policy.filesystem_write
            && !tool.policy.external_publish
            && !tool.policy.subprocess
            && !tool.policy.package_install
            && !tool.policy.privilege_escalation;
    }
    let canonical = state.resolve_canonical_skill_name(executable);
    let Some(manifest) = state.skill_manifest(&canonical) else {
        return false;
    };
    let action = args.get("action").and_then(Value::as_str);
    let Some(mapping) = claw_core::skill_registry::select_planner_capability_mapping(
        &manifest.planner_capabilities,
        action,
    ) else {
        return false;
    };
    matches!(
        mapping.effect,
        Some(claw_core::skill_registry::PlannerCapabilityEffect::Observe)
    ) && mapping.idempotent == Some(true)
        && mapping.filesystem_write != Some(true)
        && mapping.external_publish != Some(true)
        && mapping.subprocess != Some(true)
        && mapping.package_install != Some(true)
        && mapping.privilege_escalation != Some(true)
        && mapping.async_adapter_kind.is_none()
        && !matches!(
            mapping.execution_mode,
            Some(
                claw_core::skill_registry::CapabilityExecutionMode::AsyncPreferred
                    | claw_core::skill_registry::CapabilityExecutionMode::AsyncRequired
            )
        )
}

fn classify_action(state: &AppState, action: &AgentAction) -> SameTurnActionClass {
    let Some(resolved) = resolved_executable_action(state, action) else {
        return match action {
            AgentAction::Think { .. }
            | AgentAction::SynthesizeAnswer { .. }
            | AgentAction::Respond { .. } => SameTurnActionClass::Discussion,
            AgentAction::CallCapability { .. }
            | AgentAction::CallTool { .. }
            | AgentAction::CallSkill { .. } => SameTurnActionClass::MaterialBoundary,
        };
    };
    let (executable, args) = match &resolved {
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        _ => return SameTurnActionClass::MaterialBoundary,
    };
    if executable == super::capability_discovery::RUNTIME_CAPABILITY_LOADER_TOOL {
        return SameTurnActionClass::MaterialBoundary;
    }
    if !policy_proves_parallel_read(state, executable, args) {
        return SameTurnActionClass::MaterialBoundary;
    }
    if value_contains_runtime_reference(args) {
        return SameTurnActionClass::MaterialBoundary;
    }
    let canonical = state.resolve_canonical_skill_name(executable);
    let effect = crate::execution_recipe::classify_skill_action_effect(state, &canonical, args);
    if effect.observes && !effect.mutates && !effect.validates {
        SameTurnActionClass::IndependentRead
    } else {
        SameTurnActionClass::MaterialBoundary
    }
}

pub(super) fn independent_read_batch_prefix_len(
    state: &AppState,
    actions: &[AgentAction],
    execution_limit: usize,
) -> usize {
    let count = actions
        .iter()
        .take(execution_limit.max(1))
        .take_while(|action| classify_action(state, action) == SameTurnActionClass::IndependentRead)
        .count();
    if count >= 2 {
        count
    } else {
        0
    }
}

pub(super) fn planner_action_dependencies(
    state: Option<&AppState>,
    actions: &[AgentAction],
) -> Vec<Vec<String>> {
    let Some(state) = state else {
        let mut previous_actionable: Option<String> = None;
        return actions
            .iter()
            .enumerate()
            .map(|(index, action)| {
                let dependencies = previous_actionable
                    .as_ref()
                    .map(|step_id| vec![step_id.clone()])
                    .unwrap_or_default();
                if !matches!(action, AgentAction::Think { .. }) {
                    previous_actionable = Some(format!("step_{}", index + 1));
                }
                dependencies
            })
            .collect();
    };

    let mut barrier: Option<String> = None;
    let mut open_parallel_reads: Vec<String> = Vec::new();
    let mut dependencies = Vec::with_capacity(actions.len());
    for (index, action) in actions.iter().enumerate() {
        let step_id = format!("step_{}", index + 1);
        match classify_action(state, action) {
            SameTurnActionClass::IndependentRead => {
                dependencies.push(barrier.iter().cloned().collect());
                open_parallel_reads.push(step_id);
            }
            SameTurnActionClass::Discussion if matches!(action, AgentAction::Think { .. }) => {
                dependencies.push(
                    open_parallel_reads
                        .last()
                        .cloned()
                        .or_else(|| barrier.clone())
                        .into_iter()
                        .collect(),
                );
            }
            SameTurnActionClass::MaterialBoundary | SameTurnActionClass::Discussion => {
                let prior = if open_parallel_reads.is_empty() {
                    barrier.iter().cloned().collect()
                } else {
                    open_parallel_reads.clone()
                };
                dependencies.push(prior);
                barrier = Some(step_id);
                open_parallel_reads.clear();
            }
        }
    }
    dependencies
}

pub(super) fn return_control_boundary_after_action(
    state: &AppState,
    actions: &[AgentAction],
    current_index: usize,
    execution_limit: usize,
) -> Option<&'static str> {
    let current = actions.get(current_index)?;
    match classify_action(state, current) {
        SameTurnActionClass::IndependentRead => {
            let next = actions
                .get(current_index + 1)
                .filter(|_| current_index + 1 < execution_limit);
            if next.is_some_and(|action| {
                classify_action(state, action) == SameTurnActionClass::IndependentRead
            }) {
                None
            } else {
                Some("independent_read_batch_observed")
            }
        }
        SameTurnActionClass::MaterialBoundary => Some("material_action_observed"),
        SameTurnActionClass::Discussion => None,
    }
}

#[cfg(test)]
#[path = "action_batch_contract_tests.rs"]
mod tests;
