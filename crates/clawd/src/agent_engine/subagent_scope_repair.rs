//! Retain typed delegation restrictions while repairing a rejected plan.
//! These records can only narrow admission; they never authorize execution.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use super::skill_execution::child_task_execution_policy::{
    normalize_capability_argument_constraints, registry_argument_constraints_issues,
    valid_capability_argument_constraints,
};

const MAX_NODES: usize = 32;
const MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct ScopeRepairState {
    revision: u64,
    nodes: BTreeMap<String, Value>,
    overflowed: bool,
}

impl ScopeRepairState {
    fn for_revision(&mut self, revision: u64) {
        if self.revision != revision {
            *self = Self {
                revision,
                ..Self::default()
            };
        }
    }

    pub(super) fn restore(value: &Value, revision: u64) -> Self {
        if value.is_null() {
            return Self {
                revision,
                ..Self::default()
            };
        }
        let decoded = (value.to_string().len() <= MAX_BYTES)
            .then(|| serde_json::from_value::<Self>(value.clone()).ok())
            .flatten();
        match decoded {
            Some(state)
                if state.nodes.len() <= MAX_NODES
                    && state.nodes.iter().all(|(node, rules)| {
                        !node.is_empty()
                            && node.len() <= 160
                            && valid_rules(rules)
                            && rules.as_array().is_some_and(|a| !a.is_empty())
                    }) =>
            {
                state
            }
            _ => Self {
                revision,
                overflowed: true,
                ..Self::default()
            },
        }
    }

    fn remember(&mut self, args: &Value, revision: u64, state: Option<&crate::AppState>) {
        self.for_revision(revision);
        for (node, child, inherited, _) in children(args) {
            let Some(rules) = child
                .get("capability_argument_constraints")
                .or_else(|| inherited.and_then(|v| v.get("capability_argument_constraints")))
                .and_then(|v| normalize_capability_argument_constraints(v).ok())
                .filter(valid_rules)
                .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
                .filter(|v| {
                    state.is_none_or(|state| {
                        registry_argument_constraints_issues(state, v).is_empty()
                    })
                })
            else {
                continue;
            };
            // Never replace an earlier restriction with a later repair proposal.
            if self.nodes.contains_key(&node) {
                continue;
            }
            if self.nodes.len() == MAX_NODES {
                self.overflowed = true;
                break;
            }
            self.nodes
                .insert(node.clone(), canonical_rules(rules, state));
            if serde_json::to_string(self).map_or(true, |s| s.len() > MAX_BYTES) {
                self.nodes.remove(&node);
                self.overflowed = true;
                break;
            }
        }
    }

    pub(super) fn admission_issues(
        &mut self,
        args: &Value,
        revision: u64,
        state: Option<&crate::AppState>,
    ) -> Vec<Value> {
        self.for_revision(revision);
        if self.overflowed {
            return vec![json!({"path":"/capability_argument_constraints",
                "reason":"scope_repair_state_unavailable"})];
        }
        let mut issues = Vec::new();
        for (node, child, inherited, prefix) in children(args) {
            let Some(required) = self.nodes.get(&node).and_then(Value::as_array) else {
                continue;
            };
            let supplied = child
                .get("capability_argument_constraints")
                .or_else(|| inherited.and_then(|v| v.get("capability_argument_constraints")))
                .and_then(|v| normalize_capability_argument_constraints(v).ok())
                .map(|v| canonical_rules(v, state));
            let allowed = child
                .get("allowed_capabilities")
                .or_else(|| inherited.and_then(|v| v.get("allowed_capabilities")))
                .and_then(Value::as_array)
                .map(|caps| {
                    caps.iter()
                        .filter_map(Value::as_str)
                        .map(|cap| canonical_capability(cap, state))
                        .collect::<Vec<_>>()
                });
            for rule in required {
                if !allowed.as_ref().is_some_and(|caps| {
                    rule["capability"]
                        .as_str()
                        .is_some_and(|cap| caps.iter().any(|v| v == cap))
                }) {
                    continue;
                }
                if !supplied
                    .as_ref()
                    .and_then(Value::as_array)
                    .is_some_and(|rules| rules.contains(rule))
                {
                    issues.push(
                        json!({"path":format!("{prefix}/capability_argument_constraints"),
                        "node_id":node,"reason":"prior_argument_scope_must_be_preserved",
                        "required_constraint":rule}),
                    );
                }
            }
        }
        issues
    }
}

fn canonical_capability(capability: &str, state: Option<&crate::AppState>) -> String {
    state
        .and_then(|state| state.get_skills_registry())
        .and_then(|registry| {
            registry
                .canonical_planner_capability_name(capability)
                .map(str::to_string)
        })
        .unwrap_or_else(|| capability.to_string())
}

fn canonical_rules(mut rules: Value, state: Option<&crate::AppState>) -> Value {
    for rule in rules.as_array_mut().into_iter().flatten() {
        if let Some(name) = rule["capability"].as_str() {
            rule["capability"] = json!(canonical_capability(name, state));
        }
        if let Some(schemas) = rule.get_mut("allowed_args").and_then(Value::as_array_mut) {
            for schema in schemas.iter_mut() {
                canonical_schema(schema);
            }
            schemas.sort_by_key(Value::to_string);
        }
    }
    rules
}

fn canonical_schema(schema: &mut Value) {
    // Ordering is irrelevant for schema alternatives and required keys, but
    // arrays inside enum literals are business values and must stay untouched.
    for key in ["required", "enum"] {
        if let Some(items) = schema.get_mut(key).and_then(Value::as_array_mut) {
            items.sort_by_key(Value::to_string);
        }
    }
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        for property in properties.values_mut() {
            canonical_schema(property);
        }
    }
    if let Some(items) = schema.get_mut("items") {
        canonical_schema(items);
    }
}

fn valid_rules(rules: &Value) -> bool {
    let allowed = rules
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["capability"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    valid_capability_argument_constraints(rules, &allowed)
}

fn children(args: &Value) -> Vec<(String, &Value, Option<&Value>, String)> {
    let node = |child: &Value, index: usize| {
        child
            .get("node_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("node_{index}"))
    };
    match args.get("children").and_then(Value::as_array) {
        Some(items) => items
            .iter()
            .take(16)
            .enumerate()
            .map(|(i, child)| (node(child, i), child, Some(args), format!("/children/{i}")))
            .collect(),
        None => vec![(node(args, 0), args, None, String::new())],
    }
}

pub(super) fn remember_rejected_plan(
    state: &crate::AppState,
    loop_state: &mut super::LoopState,
    plan: &crate::PlanResult,
) {
    for step in &plan.steps {
        let Some(action) = step.to_agent_action() else {
            continue;
        };
        let resolved = crate::capability_resolver::resolve_agent_action_for_state(state, action);
        let (name, args) = match &resolved {
            crate::AgentAction::CallTool { tool, args } => (tool, args),
            crate::AgentAction::CallSkill { skill, args } => (skill, args),
            _ => continue,
        };
        let Some(registry) = state.get_skills_registry() else {
            continue;
        };
        let Some(name) = registry.resolve_canonical(name) else {
            continue;
        };
        if registry.is_builtin(name) && registry.has_semantic_tag(name, "agent_delegation") {
            loop_state.subagent_scope_repairs.remember(
                args,
                loop_state.conversation_input_revision,
                Some(state),
            );
        }
    }
}

pub(super) fn remember_args(
    state: &crate::AppState,
    loop_state: &mut super::LoopState,
    args: &Value,
) {
    loop_state.subagent_scope_repairs.remember(
        args,
        loop_state.conversation_input_revision,
        Some(state),
    );
}

#[cfg(test)]
#[path = "subagent_scope_repair_tests.rs"]
mod tests;
