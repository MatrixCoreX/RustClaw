use super::*;
use serde_json::json;

#[test]
fn typed_native_arguments_reject_invalid_enum_type_and_bounds_before_dispatch() {
    let schema = json!({"type":"object","properties":{
        "mode":{"type":"string","enum":["head","tail","range"]},
        "start_line":{"type":"integer","minimum":1}
    }});
    assert!(native_argument_values_valid(
        &schema,
        &json!({"mode":"range","start_line":1})
    ));
    for args in [
        json!({"mode":"full"}),
        json!({"start_line":"one"}),
        json!({"start_line":0}),
    ] {
        assert!(!native_argument_values_valid(&schema, &args));
    }
}

#[test]
fn direct_schema_projection_keeps_argument_validation_and_package_semantics() {
    let capability = "fixture.read".to_string();
    let annotation = format!("schema:planner_capability_arguments_v1; capability={capability}");
    let original = json!({"type":"object","description":annotation,
        "properties":{"mode":{"type":"string","enum":["range"],
            "description":" Preserve Unicode α and whitespace.\n"},
            "start_line":{"type":"integer","minimum":1,"maximum":5}},
        "required":["mode"],"additionalProperties":false});
    let schemas = std::collections::BTreeMap::from([(capability.clone(), original.clone())]);
    let tool = native_capability_tool_definition(
        "call_fixture_read",
        "fixture purpose",
        std::slice::from_ref(&capability),
        &schemas,
    );
    let mut restored = tool.input_schema.clone();
    assert_eq!(tool.description, "fixture purpose");
    assert!(restored.get("description").is_none());
    restored["description"] = json!(annotation);
    assert_eq!(restored, original);
    assert_eq!(schemas[&capability], original);
    for (args, valid) in [
        (json!({"mode":"range","start_line":1}), true),
        (json!({"mode":"all"}), false),
        (json!({"mode":"range","start_line":0}), false),
        (json!({"mode":"range","start_line":6}), false),
        (json!({"mode":"range","start_line":"1"}), false),
    ] {
        assert_eq!(native_argument_values_valid(&original, &args), valid);
        assert_eq!(
            native_argument_values_valid(&tool.input_schema, &args),
            valid
        );
    }
    for description in [
        "Package-authored semantic instructions.",
        "schema:planner_capability_arguments_v1; capability=other.read",
    ] {
        let mut package = original.clone();
        package["description"] = json!(description);
        let schemas = std::collections::BTreeMap::from([(capability.clone(), package.clone())]);
        let projected = native_capability_tool_definition(
            "call_fixture_read",
            "fixture purpose",
            std::slice::from_ref(&capability),
            &schemas,
        );
        assert_eq!(projected.input_schema, package);
    }
}
