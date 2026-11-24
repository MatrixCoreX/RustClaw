use super::super::native_capability_tools::native_argument_values_valid;
use super::*;

fn text_call() -> ModelToolCall {
    ModelToolCall {
        id: "wire-test".into(),
        name: NATIVE_RESPOND_TOOL.into(),
        arguments: json!({
            "terminal_intent":"answer","conversation_relation":"start_followup","shape":"free_text",
            "content":"observed value","exact_item_count":0,"exact_field_count":0,"exact_visible_line_count":0
        }),
    }
}

#[test]
fn inactive_collections_can_be_omitted_without_repair_or_runtime_mutation() {
    let request =
        native_planner_request("system", "user", None, &[], &BTreeMap::new(), &[], &[], &[]);
    let tool = request
        .tools
        .iter()
        .find(|t| t.name == NATIVE_RESPOND_TOOL)
        .unwrap();
    assert!(!tool.strict);
    for field in ["items", "fields", "observed_fields"] {
        assert!(!tool.input_schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!(field)));
        assert_eq!(tool.input_schema["properties"][field]["type"], "array");
    }
    let call = text_call();
    assert!(native_argument_values_valid(
        &tool.input_schema,
        &call.arguments
    ));
    assert!(action_from_native_respond_call(&call, None).is_ok());
}

#[test]
fn optional_wire_fields_do_not_relax_active_shapes_types_counts_or_control() {
    let request =
        native_planner_request("system", "user", None, &[], &BTreeMap::new(), &[], &[], &[]);
    let tool = request
        .tools
        .iter()
        .find(|t| t.name == NATIVE_RESPOND_TOOL)
        .unwrap();
    for field in ["items", "fields", "observed_fields"] {
        let mut call = text_call();
        call.arguments[field] = json!({});
        assert!(!native_argument_values_valid(
            &tool.input_schema,
            &call.arguments
        ));
        assert!(action_from_native_respond_call(&call, None).is_err());
    }
    for shape in ["list", "object", "observed_object"] {
        let mut call = text_call();
        call.arguments["shape"] = json!(shape);
        call.arguments["content"] = json!("");
        call.arguments["exact_item_count"] = json!(if shape == "list" { 1 } else { 0 });
        call.arguments["exact_field_count"] = json!(if shape == "list" { 0 } else { 1 });
        assert!(action_from_native_respond_call(&call, None).is_err());
    }
    let mut call = text_call();
    call.arguments["items"] = json!(["unexpected"]);
    assert!(action_from_native_respond_call(&call, None).is_err());
    call = text_call();
    call.arguments
        .as_object_mut()
        .unwrap()
        .remove("conversation_relation");
    assert!(action_from_native_respond_call(&call, None).is_err());
}
