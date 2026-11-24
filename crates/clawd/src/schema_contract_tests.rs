use serde_json::json;

use super::{
    enum_constraint_violations, type_constraint_violations, unknown_argument_violations,
    value_constraint_violations,
};

#[test]
fn explicit_unknown_fields_honor_open_defaults_and_preserve_legacy_checks() {
    let open = json!({"type":"object","properties":{"known":{"type":"string"}}});
    let args = json!({"known":"value","extra":false});
    assert!(super::explicit_unknown_argument_violations(&open, &args).is_empty());
    assert_eq!(unknown_argument_violations(&open, &args).len(), 1);
    let mut closed = open;
    closed["additionalProperties"] = json!(false);
    assert_eq!(
        super::explicit_unknown_argument_violations(&closed, &args).len(),
        1
    );
    assert_eq!(unknown_argument_violations(&closed, &args).len(), 1);
}

#[test]
fn explicit_unknown_fields_cover_nested_objects_arrays_and_empty_closed_objects() {
    let schema = json!({"type":"object","additionalProperties":false,"properties":{
        "open":{"type":"object","properties":{"known":{}}},
        "closed":{"type":"object","additionalProperties":false,"properties":{"known":{}}},
        "items":{"type":"array","items":{"type":"object","additionalProperties":false,"properties":{"known":{}}}},
        "empty":{"type":"object","additionalProperties":false}}});
    let args =
        json!({"open":{"extra":1},"closed":{"extra":1},"items":[{"extra":1}],"empty":{"extra":1}});
    let fields = super::explicit_unknown_argument_violations(&schema, &args)
        .into_iter()
        .map(|v| v.field)
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        vec!["closed.extra", "empty.extra", "items[0].extra"]
    );
}

#[test]
fn required_literal_args_do_not_bypass_other_schema_constraints() {
    let field = json!({"type": "string", "minLength": 0, "enum": ["accepted"]});
    assert!(super::required_value_is_present(Some(&field), &json!("\n")));
    let schema = json!({"type": "object", "properties": {"payload": field}});
    assert_eq!(
        enum_constraint_violations(&schema, &json!({"payload": "\n"})).len(),
        1
    );
    assert_eq!(
        type_constraint_violations(&schema, &json!({"payload": false})).len(),
        1
    );
    for value in [json!([]), json!({}), json!(null)] {
        assert!(!super::required_value_is_present(None, &value));
    }
    assert!(super::required_value_is_present(
        Some(&json!({"type": "null"})),
        &json!(null)
    ));
}

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

#[test]
fn nested_object_and_array_constraints_report_stable_paths() {
    let schema = json!({
        "properties": {
            "children": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "role": {"type": "string", "enum": ["review", "test"]},
                        "objective": {"type": "string"}
                    }
                }
            }
        }
    });
    let args = json!({
        "children": [
            {"role": "writer", "objective": 7, "task": "undeclared"}
        ]
    });

    assert_eq!(
        enum_constraint_violations(&schema, &args)[0].field,
        "children[0].role"
    );
    let types = type_constraint_violations(&schema, &args);
    assert_eq!(types[0].field, "children[0].objective");
    assert_eq!(types[0].expected, "string");
    assert_eq!(
        unknown_argument_violations(&schema, &args)[0].field,
        "children[0].task"
    );
}

#[test]
fn value_constraints_cover_strings_numbers_and_array_sizes() {
    let schema = json!({
        "properties": {
            "cursor": {
                "type": "string",
                "minLength": 3,
                "maxLength": 32,
                "pattern": "^office-v1:"
            },
            "limit": {"type": "integer", "minimum": 1, "maximum": 1000},
            "operations": {"type": "array", "minItems": 1, "maxItems": 2}
        }
    });
    let violations = value_constraint_violations(
        &schema,
        &json!({"cursor": "first", "limit": 0, "operations": []}),
    );

    assert_eq!(
        violations
            .iter()
            .map(|violation| (violation.field.as_str(), violation.constraint.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("cursor", "pattern"),
            ("limit", "minimum"),
            ("operations", "min_items")
        ]
    );
}
