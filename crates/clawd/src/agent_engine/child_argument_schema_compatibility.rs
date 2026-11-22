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
        let Some(schema) = registry.all_names().into_iter().find_map(|skill| {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_strings_are_rejected_without_coercing_the_constraint() {
        let host = json!({"type":"object","properties":{"line":{"type":"integer"},"code":{"type":"string"}}});
        let proposal = json!({"properties":{"line":{"enum":["12"]},"code":{"enum":["12"]}}});
        let original = proposal.clone();
        let mut errors = Vec::new();
        compare(&proposal, &host, "", 0, &mut errors);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["path"], "/properties/line/enum");
        assert_eq!(errors[0]["expected_type"], "integer");
        assert_eq!(proposal, original);
        errors.clear();
        compare(
            &json!({"properties":{"line":{"enum":[12]}}}),
            &host,
            "",
            0,
            &mut errors,
        );
        assert!(errors.is_empty());
    }
    #[test]
    fn nullable_and_nested_literal_types_are_checked_with_bounded_diagnostics() {
        let host =
            json!({"properties":{"entries":{"type":"array","items":{"type":["integer","null"]}}}});
        for (values, valid) in [
            (json!([12, null]), true),
            (json!(["12"]), false),
            (json!([true]), false),
        ] {
            let mut errors = Vec::new();
            compare(
                &json!({"properties":{"entries":{"items":{"enum":values}}}}),
                &host,
                "",
                0,
                &mut errors,
            );
            assert_eq!(errors.is_empty(), valid);
        }
        let host = json!({"properties":(0..64).map(|i|(format!("p{i}"),json!({"type":"integer"}))).collect::<serde_json::Map<_,_>>()});
        let constraint = json!({"properties":(0..64).map(|i|(format!("p{i}"),json!({"enum":["1"]}))).collect::<serde_json::Map<_,_>>()});
        let mut errors = Vec::new();
        compare(&constraint, &host, "", 0, &mut errors);
        assert_eq!(errors.len(), 16);
    }
}
