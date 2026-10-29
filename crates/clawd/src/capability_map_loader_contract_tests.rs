use super::*;

#[test]
fn planner_hint_uses_the_exact_native_load_schema_and_visible_groups() {
    let groups = vec!["fixture_reader".to_string(), "fixture_writer".to_string()];
    let native = tool_definition(&groups);
    let hint = planner_hint(&groups);
    let prefix = format!(
        "capability_group_loader=call_tool:{}; args_schema=",
        native.name
    );
    let raw = hint
        .strip_prefix(&prefix)
        .unwrap()
        .strip_suffix("; selected_scopes_persist=true")
        .unwrap();
    let schema: Value = serde_json::from_str(raw).unwrap();
    assert_eq!(schema, native.input_schema["oneOf"][2]);
    assert_eq!(schema["required"], json!(["groups"]));
    assert_eq!(
        schema["properties"]["groups"]["items"]["enum"],
        json!(groups)
    );
    assert_eq!(schema["properties"]["groups"]["minItems"], 1);
    assert_eq!(schema["properties"]["op"]["enum"], json!(["load_groups"]));
    assert_eq!(schema["additionalProperties"], false);
    assert!(schema["properties"].get("nonempty_groups").is_none());
    assert!(native.strict);
}

#[test]
fn search_and_expand_keep_distinct_required_machine_fields() {
    let native = tool_definition(&[]);
    let variants = native.input_schema["oneOf"].as_array().unwrap();
    assert_eq!(variants.len(), 3);
    assert_eq!(variants[0]["required"], json!(["op", "query"]));
    assert_eq!(variants[1]["required"], json!(["op", "capability_refs"]));
    assert_eq!(variants[0]["properties"]["op"]["enum"], json!(["search"]));
    assert_eq!(variants[1]["properties"]["op"]["enum"], json!(["expand"]));
}
