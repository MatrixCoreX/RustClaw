//! Check frozen enum literals against the host's capability argument types.
//! Reject incompatible proposals; never coerce values or widen the frozen scope.
use serde_json::{json, Value};

pub(super) fn issues(state: &crate::AppState, value: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    let Some(registry) = state.get_skills_registry() else {
        return issues;
    };
    for (i, rule) in value.as_array().into_iter().flatten().take(32).enumerate() {
        let Some(name) = rule.get("capability").and_then(Value::as_str) else {
            continue;
        };
        let canonical = registry
            .canonical_planner_capability_name(name)
            .unwrap_or(name);
        let Some((schema, action)) = registry.all_names().into_iter().find_map(|skill| {
            let mapping = registry
                .planner_capabilities(&skill)
                .iter()
                .find(|m| m.name == canonical)?;
            let manifest = registry.manifest(&skill)?;
            claw_core::skill_registry::planner_capability_argument_schema(
                manifest.input_schema.as_ref(),
                mapping,
            )
            .ok()
            .map(|schema| (schema, mapping.action.clone()))
        }) else {
            continue;
        };
        for (j, constraint) in rule
            .get("allowed_args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(8)
            .enumerate()
        {
            if let Some(action) = action.as_deref() {
                compare_injected_action(
                    constraint,
                    action,
                    &format!("/{i}/allowed_args/{j}"),
                    &mut issues,
                );
            }
            compare(
                constraint,
                &schema,
                &format!("/{i}/allowed_args/{j}"),
                0,
                &mut issues,
            );
            if issues.len() >= 16 {
                return issues;
            }
        }
    }
    issues
}

fn accepts_type(value: &Value, kind: &str) -> bool {
    match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        _ => true,
    }
}

fn compare(constraint: &Value, host: &Value, path: &str, depth: usize, issues: &mut Vec<Value>) {
    if depth > 8 || issues.len() >= 16 {
        return;
    }
    if let (Some(values), Some(types)) = (
        constraint.get("enum").and_then(Value::as_array),
        host.get("type"),
    ) {
        let kinds: Vec<&str> = if let Some(kind) = types.as_str() {
            vec![kind]
        } else {
            types
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect()
        };
        if !kinds.is_empty()
            && values
                .iter()
                .any(|v| !kinds.iter().any(|kind| accepts_type(v, kind)))
        {
            issues.push(json!({"path":format!("{path}/enum"),"reason":"enum_value_incompatible_with_capability_type","expected_type":types}));
        }
    }
    if let Some(properties) = constraint.get("properties").and_then(Value::as_object) {
        for (name, property) in properties.iter().take(64) {
            if let Some(host_property) = host.get("properties").and_then(|p| p.get(name)) {
                let escaped = name.replace('~', "~0").replace('/', "~1");
                compare(
                    property,
                    host_property,
                    &format!("{path}/properties/{escaped}"),
                    depth + 1,
                    issues,
                );
            }
            if issues.len() >= 16 {
                return;
            }
        }
    }
    if let (Some(items), Some(host_items)) = (constraint.get("items"), host.get("items")) {
        compare(
            items,
            host_items,
            &format!("{path}/items"),
            depth + 1,
            issues,
        );
    }
}

// The executor injects the registry-selected action before the frozen scope
// check. Reject an impossible proposal before allocating a durable child;
// never insert a new property into the caller's permission grant.
fn compare_injected_action(constraint: &Value, action: &str, path: &str, issues: &mut Vec<Value>) {
    if issues.len() >= 16 {
        return;
    }
    let property = constraint.get("properties").and_then(|p| p.get("action"));
    let action_value = json!(action);
    let excluded_property = property.is_none()
        && constraint
            .get("additionalProperties")
            .and_then(Value::as_bool)
            == Some(false);
    let excluded_value = property.is_some_and(|schema| {
        !crate::schema_contract::enum_constraint_violations(schema, &action_value).is_empty()
            || !crate::schema_contract::type_constraint_violations(schema, &action_value).is_empty()
            || !crate::schema_contract::value_constraint_violations(schema, &action_value)
                .is_empty()
    });
    let excluded_object = constraint
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|values| {
            !values
                .iter()
                .any(|v| v.get("action") == Some(&action_value))
        });
    if excluded_property || excluded_value || excluded_object {
        issues.push(json!({"path":format!("{path}/properties/action"),
            "reason":"constraint_excludes_registry_injected_argument",
            "argument":"action","expected_value":action}));
    }
}

#[cfg(test)]
#[path = "child_argument_schema_compatibility_tests.rs"]
mod tests;
