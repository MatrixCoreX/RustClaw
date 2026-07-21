use serde_json::json;

use super::{enum_constraint_violations, type_constraint_violations, unknown_argument_violations};

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

#[test]
fn unknown_arguments_are_rejected_but_runtime_metadata_is_allowed() {
    let schema = json!({
        "properties": {
            "action": { "type": "string" },
            "duration": { "type": "integer" }
        }
    });

    let violations = unknown_argument_violations(
        &schema,
        &json!({
            "action": "preview_generate",
            "duration_seconds": 3,
            "language": "zh-CN",
            "_clawd_validation": {"profile": "fixture"}
        }),
    );
    assert_eq!(
        violations
            .iter()
            .map(|violation| violation.field.as_str())
            .collect::<Vec<_>>(),
        vec!["duration_seconds", "language"]
    );
}

#[test]
fn explicitly_open_schemas_allow_extension_arguments() {
    let schema = json!({
        "additionalProperties": true,
        "properties": {
            "action": { "type": "string" }
        }
    });

    assert!(unknown_argument_violations(&schema, &json!({"vendor_extension": true})).is_empty());
}

#[test]
fn type_constraints_reject_wrong_optional_field_types() {
    let schema = json!({
        "properties": {
            "action": { "type": "string" },
            "schedule": { "type": "object" },
            "dry_run": { "type": "boolean" }
        }
    });

    let violations = type_constraint_violations(
        &schema,
        &json!({
            "action": "preview",
            "schedule": "tomorrow at 09:00",
            "dry_run": "true"
        }),
    );
    assert_eq!(
        violations
            .iter()
            .map(|violation| (violation.field.as_str(), violation.expected.as_str()))
            .collect::<Vec<_>>(),
        vec![("dry_run", "boolean"), ("schedule", "object")]
    );
}

#[test]
fn type_constraints_accept_union_and_numeric_types() {
    let schema = json!({
        "properties": {
            "selector": {
                "anyOf": [
                    { "type": "string" },
                    { "type": "array" }
                ]
            },
            "count": { "oneOf": [{ "type": "integer" }, { "type": "null" }] },
            "ratio": { "type": "number" }
        }
    });

    assert!(type_constraint_violations(
        &schema,
        &json!({ "selector": ["a", "b"], "count": 2, "ratio": 0.5 })
    )
    .is_empty());
    assert!(type_constraint_violations(
        &schema,
        &json!({ "selector": "name", "count": null, "ratio": 1 })
    )
    .is_empty());
}
