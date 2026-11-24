//! Narrow model-visible argument fields using the frozen child contract.
//! This changes discovery only; resolver, policy and attempt budgets remain authoritative.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

pub(super) fn restrict(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    ungrouped: &mut BTreeMap<String, Value>,
    groups: &mut [crate::capability_map::PlannerNativeCapabilityGroup],
) {
    let Some(context) =
        super::super::skill_execution::child_task_execution_policy::planner_argument_context(task)
    else {
        return;
    };
    let Some(rules) = context
        .get("capability_argument_constraints")
        .and_then(Value::as_array)
    else {
        return;
    };
    if rules.len() > 32 || context.to_string().len() > 17 * 1024 {
        return;
    }
    let registry = state.get_skills_registry();
    for rule in rules {
        let (Some(name), Some(variants)) = (
            rule.get("capability").and_then(Value::as_str),
            rule.get("allowed_args").and_then(Value::as_array),
        ) else {
            continue;
        };
        if variants.is_empty()
            || variants.len() > 8
            || !variants
                .iter()
                .all(crate::repo::child_tasks::bounded_argument_schema_supported)
        {
            continue;
        }
        let canonical = registry
            .as_ref()
            .and_then(|r| r.canonical_planner_capability_name(name))
            .unwrap_or(name);
        let scopes = variants.iter().collect::<Vec<_>>();
        for schemas in std::iter::once(&mut *ungrouped).chain(
            groups
                .iter_mut()
                .map(|g| &mut g.capability_argument_schemas),
        ) {
            if let Some(schema) = schemas.get_mut(canonical) {
                project_schema_fields(schema, &scopes, 0);
            }
        }
    }
}

fn project_schema_fields(schema: &mut Value, scopes: &[&Value], depth: usize) {
    if depth > 8 || scopes.is_empty() {
        return;
    }
    let closed = scopes
        .iter()
        .all(|s| s.get("additionalProperties") == Some(&Value::Bool(false)));
    if closed && schema.is_object() {
        schema["additionalProperties"] = Value::Bool(false);
    }
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        if closed {
            let allowed = scopes
                .iter()
                .flat_map(|s| {
                    s.get("properties")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flat_map(|p| p.keys().map(String::as_str))
                })
                .collect::<BTreeSet<_>>();
            properties.retain(|name, _| allowed.contains(name.as_str()));
        }
        for (name, property) in properties {
            // A branch that permits an unspecified field must remain open here.
            if scopes.iter().any(|s| {
                s.get("properties").and_then(|p| p.get(name)).is_none()
                    && s.get("additionalProperties") != Some(&Value::Bool(false))
            }) {
                continue;
            }
            let nested = scopes
                .iter()
                .filter_map(|s| s.get("properties").and_then(|p| p.get(name)))
                .collect::<Vec<_>>();
            project_schema_fields(property, &nested, depth + 1);
        }
    }
    if scopes.iter().all(|s| s.get("items").is_some()) {
        if let Some(items) = schema.get_mut("items") {
            let nested = scopes
                .iter()
                .filter_map(|s| s.get("items"))
                .collect::<Vec<_>>();
            project_schema_fields(items, &nested, depth + 1);
        }
    }
}

#[cfg(test)]
#[path = "child_native_argument_fields_tests.rs"]
mod tests;
