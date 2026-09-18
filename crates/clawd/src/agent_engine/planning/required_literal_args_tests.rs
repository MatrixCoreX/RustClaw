use super::*;

#[test]
fn required_literal_args_native_call_keeps_supplied_bytes() {
    let capability = "fixture.literal";
    let groups = BTreeMap::from([(
        "call_fixture_literal".to_string(),
        BTreeSet::from([capability.to_string()]),
    )]);
    let schemas = BTreeMap::from([(
        capability.to_string(),
        serde_json::json!({
            "type": "object", "required": ["payload"],
            "properties": {"payload": {"type": "string", "minLength": 0}}
        }),
    )]);
    for payload in ["", "\n", "\r\n", " \t"] {
        let arguments = serde_json::json!({"payload": payload});
        let action = action_from_native_tool_call_with_schemas(
            &ModelToolCall {
                id: "literal-input".to_string(),
                name: "call_fixture_literal".to_string(),
                arguments: arguments.clone(),
            },
            &groups,
            &schemas,
            None,
        )
        .expect("literal call admitted");
        match action {
            AgentAction::CallCapability { args, .. } => assert_eq!(args, arguments),
            _ => panic!("expected capability action"),
        }
    }
}

#[test]
fn required_literal_args_preserve_explicit_empty_string_schema() {
    let schema = serde_json::json!({
        "type": "object", "required": ["payload"],
        "properties": {"payload": {"type": "string", "minLength": 0}}
    });
    for payload in ["", "\n", "\r\n", " \t", "alpha\n"] {
        assert!(
            !schema_has_missing_required_fields(&schema, &serde_json::json!({"payload": payload})),
            "literal payload {payload:?} must remain a supplied value"
        );
    }
    assert!(schema_has_missing_required_fields(
        &schema,
        &serde_json::json!({})
    ));
    assert!(schema_has_missing_required_fields(
        &schema,
        &serde_json::json!({"payload": null})
    ));
}

#[test]
fn required_literal_args_keep_unopted_identifiers_nonblank() {
    for field_schema in [
        serde_json::json!({"type": "string"}),
        serde_json::json!({"type": "string", "minLength": 1}),
        serde_json::json!({"type": "object", "minLength": 0}),
    ] {
        let schema = serde_json::json!({
            "type": "object", "required": ["locator"],
            "properties": {"locator": field_schema}
        });
        for value in ["", "\n", " \t"] {
            assert!(schema_has_missing_required_fields(
                &schema,
                &serde_json::json!({"locator": value})
            ));
        }
    }
}
