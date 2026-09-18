use super::*;
use claw_core::model_turn::ModelContentPart;

#[test]
fn initial_native_request_requires_protocol_but_keeps_answer_and_clarification() {
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &[],
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    );
    assert_eq!(request.tool_choice, ModelToolChoice::Required);
    let respond = request
        .tools
        .iter()
        .find(|tool| tool.name == NATIVE_RESPOND_TOOL)
        .expect("terminal response is available without domain capabilities");
    assert!(respond.strict);
    assert_eq!(
        respond.input_schema["properties"]["terminal_intent"]["enum"],
        json!(["answer", "clarify"]),
    );
    for intent in ["answer", "clarify"] {
        let mut arguments = json!({
            "terminal_intent": intent, "shape": "free_text",
            "content": "Model-generated response", "items": [],
            "exact_item_count": 0, "fields": [], "observed_fields": [],
            "exact_field_count": 0
        });
        if intent == "clarify" {
            arguments["missing_slot"] = json!("path");
            arguments["clarify_reason_code"] = json!("missing_required_input");
        }
        let response = ModelTurnResponse {
            text: String::new(),
            tool_calls: vec![ModelToolCall {
                id: format!("terminal-{intent}"),
                name: NATIVE_RESPOND_TOOL.to_string(),
                arguments,
            }],
            usage: None,
            finish_reason: claw_core::model_turn::ModelFinishReason::ToolCalls,
            reasoning_metadata: Default::default(),
            events: Vec::new(),
        };
        let actions = actions_from_native_turn(&response, &[]).unwrap();
        assert!(matches!(actions.as_slice(), [AgentAction::Respond { .. }]));
    }
}

#[test]
fn rejected_arguments_allow_reselection_without_expanding_the_catalog() {
    let capability = "filesystem.append_text";
    let name = native_capability_leaf_tool_name(capability);
    let schema = json!({
        "type": "object",
        "required": ["path", "content"],
        "properties": {
            "path": {"type": "string"},
            "content": {"type": "string", "minLength": 1}
        },
        "additionalProperties": false
    });
    let mut request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &[],
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    );
    for (tool_name, input_schema) in [
        (name.clone(), schema.clone()),
        (
            native_capability_leaf_tool_name("filesystem.remove_path"),
            json!({
                "type": "object", "required": ["path"],
                "properties": {"path": {"type": "string"}},
                "additionalProperties": false
            }),
        ),
    ] {
        request.tools.push(ModelToolDefinition {
            name: tool_name,
            description: String::new(),
            input_schema,
            strict: true,
        });
    }
    let malformed = ModelTurnResponse {
        text: String::new(),
        tool_calls: vec![ModelToolCall {
            id: "invalid-before-dispatch".to_string(),
            name: name.clone(),
            arguments: json!({"path": "tmp/note.txt", "content": ""}),
        }],
        usage: None,
        finish_reason: claw_core::model_turn::ModelFinishReason::ToolCalls,
        reasoning_metadata: Default::default(),
        events: Vec::new(),
    };
    let groups = BTreeMap::from([(name, BTreeSet::from([capability.to_string()]))]);
    let schemas = BTreeMap::from([(capability.to_string(), schema)]);
    let error = actions_from_native_turn_with_schemas(
        &malformed,
        &[capability.to_string()],
        &groups,
        &schemas,
        None,
    )
    .expect_err("empty content is rejected before execution");
    let signal = native_contract_repair_signal_for_turn(
        &error,
        &malformed,
        &request,
        &[],
        &[],
        &groups,
        &schemas,
        None,
        &[capability.to_string()],
    );
    let observation: Value = serde_json::from_str(&signal).unwrap();
    assert_eq!(
        observation["protocol_observation"]["exact_failed_tool"],
        true
    );
    let repaired = native_contract_retry_request(&request, &signal);
    assert_eq!(repaired.tools, request.tools);
    assert_eq!(repaired.tool_choice, ModelToolChoice::Required);
    assert_eq!(repaired.metadata, request.metadata);
    assert_eq!(
        repaired.messages.last().unwrap().content,
        vec![ModelContentPart::Text { text: signal }]
    );
    assert!(
        actions_from_native_turn_with_schemas(
            &malformed,
            &[capability.to_string()],
            &groups,
            &schemas,
            None,
        )
        .is_err(),
        "repair must not make invalid arguments executable"
    );
}
