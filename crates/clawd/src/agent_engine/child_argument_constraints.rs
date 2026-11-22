//! Frozen, narrowing-only child argument policy. Never derive grants from prose.
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(in crate::agent_engine) fn argument_constraints_validation_issues(value: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    let Some(rules) = value
        .as_array()
        .filter(|r| r.len() <= 32 && value.to_string().len() <= 16 * 1024)
    else {
        return vec![json!({"path":"","reason":"expected_bounded_constraint_array"})];
    };
    for (i, rule) in rules.iter().enumerate() {
        let Some(schemas) = rule
            .get("allowed_args")
            .and_then(Value::as_array)
            .filter(|s| !s.is_empty() && s.len() <= 8)
        else {
            issues.push(json!({"path":format!("/{i}/allowed_args"),"reason":"expected_nonempty_bounded_schema_array"}));
            continue;
        };
        for (j, schema) in schemas.iter().enumerate() {
            for mut issue in
                crate::repo::child_tasks::bounded_argument_schema_validation_issues(schema)
            {
                let path = issue
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                issue["path"] = json!(format!("/{i}/allowed_args/{j}{path}"));
                issues.push(issue);
                if issues.len() >= 16 {
                    return issues;
                }
            }
        }
    }
    issues.truncate(16);
    issues
}

pub(in crate::agent_engine) fn valid_capability_argument_constraints(
    value: &Value,
    allowed: &[String],
) -> bool {
    let Some(items) = value.as_array().filter(|v| v.len() <= 32) else {
        return false;
    };
    if value.to_string().len() > 16 * 1024 {
        return false;
    }
    let mut seen = BTreeSet::new();
    items.iter().all(|item| {
        let Some(object) = item.as_object() else {
            return false;
        };
        let Some(capability) = item.get("capability").and_then(Value::as_str) else {
            return false;
        };
        object.len() == 2
            && allowed.iter().any(|v| v == capability)
            && seen.insert(capability)
            && item
                .get("allowed_args")
                .and_then(Value::as_array)
                .is_some_and(|schemas| {
                    !schemas.is_empty()
                        && schemas.len() <= 8
                        && schemas
                            .iter()
                            .all(crate::repo::child_tasks::bounded_argument_schema_supported)
                })
    })
}

pub(super) fn append_violations(
    state: &crate::AppState,
    contract: &Value,
    allowed: &BTreeSet<String>,
    capability: &str,
    args: &Value,
    violations: &mut Vec<&'static str>,
) {
    let Some(value) = contract.pointer("/scope/capability_argument_constraints") else {
        return;
    };
    let registry = state.get_skills_registry();
    let canonical = |name: &str| {
        registry
            .as_ref()
            .and_then(|r| r.canonical_planner_capability_name(name))
            .unwrap_or(name)
            .to_string()
    };
    let mut normalized = value.clone();
    if let Some(items) = normalized.as_array_mut() {
        for item in items {
            if let Some(name) = item.get("capability").and_then(Value::as_str) {
                item["capability"] = json!(canonical(name));
            }
        }
    }
    if !valid_capability_argument_constraints(
        &normalized,
        &allowed.iter().cloned().collect::<Vec<_>>(),
    ) {
        violations.push("capability_argument_constraints_invalid");
        return;
    }
    if let Some(rule) = normalized.as_array().and_then(|items| {
        items
            .iter()
            .find(|item| item["capability"].as_str() == Some(capability))
    }) {
        if !rule["allowed_args"].as_array().is_some_and(|schemas| {
            schemas.iter().any(|schema| {
                crate::repo::child_tasks::bounded_argument_schema_matches(schema, args)
            })
        }) {
            violations.push("capability_arguments_out_of_scope");
        }
    }
}

pub(in crate::agent_engine) fn planner_argument_context(
    task: &crate::ClaimedTask,
) -> Option<Value> {
    let payload: Value = serde_json::from_str(&task.payload_json).ok()?;
    if !crate::repo::child_tasks::is_child_subagent_payload(&payload) {
        return None;
    }
    let constraints =
        payload.pointer("/child_task_contract/scope/capability_argument_constraints")?;
    constraints.as_array().filter(|v| !v.is_empty())?;
    Some(
        json!({"schema_version":1,"owner_layer":"child_task_execution_policy",
        "capability_argument_constraints":constraints,"execution_dispatched_on_violation":false}),
    )
}
