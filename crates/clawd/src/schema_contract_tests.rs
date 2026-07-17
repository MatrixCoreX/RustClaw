use serde_json::json;

use super::enum_constraint_violations;

#[test]
fn enum_constraints_accept_exact_machine_values() {
    let schema = json!({
        "properties": {
            "action": { "enum": ["read_text_range", "remove_path"] },
            "mode": { "enum": ["safe", "dry_run"] }
        }
    });

    assert!(enum_constraint_violations(
        &schema,
        &json!({ "action": "remove_path", "mode": "dry_run" })
    )
    .is_empty());
}

#[test]
fn enum_constraints_reject_unknown_or_wrong_typed_values() {
    let schema = json!({
        "properties": {
            "action": { "enum": ["read_text_range", "remove_path"] },
            "attempts": { "enum": [1, 2] }
        }
    });

    let violations = enum_constraint_violations(
        &schema,
        &json!({ "action": "remove_entries", "attempts": "2" }),
    );
    assert_eq!(
        violations
            .iter()
            .map(|violation| violation.field.as_str())
            .collect::<Vec<_>>(),
        vec!["action", "attempts"]
    );
}

#[test]
fn enum_constraints_ignore_unconstrained_and_absent_fields() {
    let schema = json!({
        "properties": {
            "action": { "enum": ["remove_path"] },
            "path": { "type": "string" }
        }
    });

    assert!(enum_constraint_violations(&schema, &json!({ "path": "tmp/example" })).is_empty());
}
