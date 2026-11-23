use super::*;

fn response_call() -> ModelToolCall {
    ModelToolCall {
        id: "response-shape-fixture".into(),
        name: NATIVE_RESPOND_TOOL.into(),
        arguments: json!({"terminal_intent":"answer","conversation_relation":"continue_current",
            "shape":"free_text","content":"observed fixture value","items":{},
            "fields":{},"observed_fields":{},"exact_item_count":0,
            "exact_field_count":0,"exact_visible_line_count":1}),
    }
}

fn planner_request() -> ModelTurnRequest {
    native_planner_request(
        "protocol",
        "turn",
        Some(30),
        &[],
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    )
}

#[test]
fn response_repair_omits_only_empty_inactive_slots_and_keeps_validated_controls() {
    let original = response_call();
    let request = planner_request();
    let schema = &request
        .tools
        .iter()
        .find(|tool| tool.name == NATIVE_RESPOND_TOOL)
        .unwrap()
        .input_schema;
    assert_eq!(
        action_from_native_respond_call(&original, None).unwrap_err(),
        "native_respond_items_not_array"
    );
    let (schema, context) = inactive_response_collection_repair(
        "native_respond_items_not_array",
        &original,
        schema,
        None,
    )
    .unwrap();
    assert_eq!(
        context["omitted_inactive_fields"],
        json!(["items", "fields", "observed_fields"])
    );
    for field in ["items", "fields", "observed_fields"] {
        assert!(schema["properties"].get(field).is_none());
        assert!(!schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!(field)));
    }
    assert_eq!(schema["properties"]["shape"]["enum"], json!(["free_text"]));
    assert_eq!(
        schema["properties"]["conversation_relation"]["enum"],
        json!(["continue_current"])
    );
    let mut returned = original.clone();
    for field in ["items", "fields", "observed_fields"] {
        returned.arguments.as_object_mut().unwrap().remove(field);
    }
    assert!(action_from_native_respond_call(&returned, None).is_ok());
    // This is a retry schema, not a permissive decoder for the rejected call.
    assert!(action_from_native_respond_call(&original, None).is_err());
    assert_eq!(original.arguments["items"], json!({}));
}

#[test]
fn response_repair_does_not_hide_business_values_or_invalid_control_fields() {
    let request = planner_request();
    let schema = &request.tools.last().unwrap().input_schema;
    for (field, value) in [
        ("fields", json!([{"name":"extra","value_json":"17"}])),
        ("observed_fields", json!({"name":"extra"})),
        ("exact_field_count", json!(1)),
        ("conversation_relation", json!("unknown")),
        ("terminal_intent", json!("clarify")),
        ("exact_visible_line_count", json!(2)),
        ("content", json!("")),
    ] {
        let mut call = response_call();
        call.arguments[field] = value;
        assert!(
            inactive_response_collection_repair(
                "native_respond_items_not_array",
                &call,
                schema,
                None
            )
            .is_none(),
            "{field}"
        );
    }
    let mut list = response_call();
    list.arguments["shape"] = json!("list");
    assert!(inactive_response_collection_repair(
        "native_respond_items_not_array",
        &list,
        schema,
        None
    )
    .is_none());
    let mut other = response_call();
    other.name = "call_capability".into();
    assert!(inactive_response_collection_repair(
        "native_respond_items_not_array",
        &other,
        schema,
        None
    )
    .is_none());
}

#[test]
fn response_retry_uses_reduced_runtime_schema_without_exposing_execution_tools() {
    let request = planner_request();
    let call = response_call();
    let turn = ModelTurnResponse {
        text: String::new(),
        tool_calls: vec![call.clone()],
        usage: None,
        finish_reason: claw_core::model_turn::ModelFinishReason::ToolCalls,
        reasoning_metadata: Default::default(),
        events: Vec::new(),
    };
    let signal = native_contract_repair_signal_for_turn(
        "native_respond_items_not_array",
        &turn,
        &request,
        &[],
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
        None,
        &[],
    );
    let observation: Value = serde_json::from_str(&signal).unwrap();
    let repaired = native_contract_retry_request(&request, &signal);
    assert_eq!(repaired.tools.len(), 1);
    assert_eq!(repaired.tools[0].name, NATIVE_RESPOND_TOOL);
    assert_eq!(repaired.tool_choice, ModelToolChoice::Required);
    assert_eq!(
        repaired.tools[0].input_schema,
        observation["protocol_observation"]["argument_constraints"]["exact_call_schema"]
    );
    assert_eq!(
        repaired.tools[0].input_schema["additionalProperties"],
        false
    );
    assert!(repaired.tools[0].input_schema["properties"]
        .get("items")
        .is_none());
    assert!(request.tools[0].input_schema["properties"]
        .get("items")
        .is_some());
    assert_eq!(
        observation["protocol_observation"]["required_argument_fields"],
        repaired.tools[0].input_schema["required"]
    );
    assert_eq!(
        observation["protocol_observation"]["argument_constraints"]["response_shape_contract"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    for unsupported_version in [Value::Null, json!(2), json!("1")] {
        let mut unsupported = observation.clone();
        unsupported["protocol_observation"]["response_schema_projection"]["schema_version"] =
            unsupported_version;
        let retry = native_contract_retry_request(&request, &unsupported.to_string());
        assert_eq!(retry.tools.len(), 1);
        assert_eq!(retry.tools[0].input_schema, request.tools[0].input_schema);
    }
}

#[test]
fn response_repair_keeps_active_object_field_array_required() {
    let request = planner_request();
    let schema = &request.tools[0].input_schema;
    let mut call = response_call();
    call.arguments["shape"] = json!("object");
    call.arguments["content"] = json!("");
    call.arguments["exact_visible_line_count"] = json!(0);
    call.arguments["fields"] = json!([{"name":"result","value_json":"17"}]);
    call.arguments["exact_field_count"] = json!(1);
    let (projected, _) =
        inactive_response_collection_repair("native_respond_items_not_array", &call, schema, None)
            .unwrap();
    assert_eq!(projected["properties"]["fields"]["type"], "array");
    assert!(projected["required"]
        .as_array()
        .unwrap()
        .contains(&json!("fields")));
    assert!(projected["properties"].get("items").is_none());
}
