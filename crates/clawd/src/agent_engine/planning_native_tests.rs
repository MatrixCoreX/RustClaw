use claw_core::{
    capability_result::CapabilityResultEnvelope,
    model_turn::{ModelContentPart, ModelFinishReason, ModelRole, ModelToolCall},
};

use super::*;

fn callable_capabilities() -> Vec<String> {
    vec!["fs.read".to_string(), "process.ps".to_string()]
}

fn turn(tool_calls: Vec<ModelToolCall>, text: &str) -> ModelTurnResponse {
    ModelTurnResponse {
        text: text.to_string(),
        tool_calls,
        usage: None,
        finish_reason: ModelFinishReason::ToolCalls,
        reasoning_metadata: Default::default(),
        events: Vec::new(),
    }
}

#[test]
fn planner_prefers_structured_capability_observation_over_raw_step_output() {
    let mut loop_state = LoopState::default();
    loop_state.last_output = Some("raw socket table".to_string());
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "process_basic",
            Some("port_list".to_string()),
            json!({
                "output": "raw socket table",
                "extra": {
                    "action": "port_list",
                    "platform": "linux",
                    "command_tool": "ss",
                    "listener_count": 2,
                    "all_interface_listener_count": 1,
                    "localhost_listener_count": 1,
                    "internet_reachability": "not_observed",
                    "ports": ["22", "59871"],
                    "all_interface_ports": ["22"],
                    "all_interface_listeners": [{
                        "local_endpoint": "0.0.0.0:22",
                        "port": "22",
                        "bind_scope": "all_interfaces",
                        "process_name": "sshd",
                        "pid": 10
                    }]
                }
            }),
        ));

    let observation = planner_last_observation(&loop_state);

    assert!(observation.starts_with("process_basic.port_list"));
    assert!(observation.contains("port_list.internet_reachability=not_observed"));
    assert!(observation.contains("port_list.all_interface_listener_count=1"));
    assert!(!observation.contains("raw socket table"));
}

#[test]
fn planner_uses_raw_last_output_when_no_structured_projection_exists() {
    let mut loop_state = LoopState::default();
    loop_state.last_output = Some("plain observation".to_string());

    assert_eq!(planner_last_observation(&loop_state), "plain observation");
}

#[test]
fn native_tool_call_maps_only_to_capability_action() {
    let actions = actions_from_native_turn(
        &turn(
            vec![ModelToolCall {
                id: "call-1".to_string(),
                name: "call_capability".to_string(),
                arguments: json!({
                    "capability": "fs.read",
                    "args": {"path": "README.md"}
                }),
            }],
            "",
        ),
        &callable_capabilities(),
    )
    .expect("native action");

    assert_eq!(actions.len(), 1);
    assert!(matches!(
        &actions[0],
        AgentAction::CallCapability { capability, args }
            if capability == "fs.read" && args["path"] == "README.md"
    ));
}

#[test]
fn native_text_is_a_terminal_model_response() {
    let actions = actions_from_native_turn(&turn(Vec::new(), "Done."), &callable_capabilities())
        .expect("terminal action");

    assert!(matches!(
        &actions[0],
        AgentAction::Respond { content } if content == "Done."
    ));
}

#[test]
fn native_tool_rejects_unknown_protocol_name_and_invalid_args() {
    let unknown = turn(
        vec![ModelToolCall {
            id: "call-1".to_string(),
            name: "run_shell_directly".to_string(),
            arguments: json!({}),
        }],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&unknown, &callable_capabilities())
            .expect_err("unknown tool rejected"),
        "native_plan_unknown_tool"
    );

    let invalid = turn(
        vec![ModelToolCall {
            id: "call-2".to_string(),
            name: "call_capability".to_string(),
            arguments: json!({"capability": "fs.read", "args": "README.md"}),
        }],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&invalid, &callable_capabilities())
            .expect_err("invalid args rejected"),
        "native_plan_args_not_object"
    );

    let malformed_transport_arguments = turn(
        vec![ModelToolCall {
            id: "call-3".to_string(),
            name: "call_capability".to_string(),
            arguments: Value::String("{not-json".to_string()),
        }],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&malformed_transport_arguments, &callable_capabilities())
            .expect_err("malformed transport arguments rejected by planner contract"),
        "native_plan_arguments_not_object"
    );
}

#[test]
fn native_tool_rejects_capability_outside_runtime_catalog() {
    let unknown_capability = turn(
        vec![ModelToolCall {
            id: "call-outside-catalog".to_string(),
            name: "call_capability".to_string(),
            arguments: json!({"capability": "process_basic", "args": {"action": "ps"}}),
        }],
        "",
    );

    assert_eq!(
        actions_from_native_turn(&unknown_capability, &callable_capabilities())
            .expect_err("out-of-catalog capability rejected"),
        "native_plan_capability_not_in_runtime_catalog"
    );
}

#[test]
fn native_request_separates_system_protocol_from_user_turn() {
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
    );

    assert_eq!(request.messages.len(), 2);
    assert_eq!(
        request
            .metadata
            .get("provider_timeout_seconds")
            .and_then(serde_json::Value::as_u64),
        Some(90)
    );
    assert_eq!(request.messages[0].role, ModelRole::System);
    assert_eq!(request.messages[1].role, ModelRole::User);
    assert_eq!(
        request.messages[0].content,
        vec![ModelContentPart::Text {
            text: "protocol".to_string()
        }]
    );
    assert_eq!(
        request.messages[1].content,
        vec![ModelContentPart::Text {
            text: "current turn".to_string()
        }]
    );
    assert_eq!(
        request.tools[0].input_schema["properties"]["capability"]["enum"],
        json!(["fs.read", "process.ps"])
    );
}

#[test]
fn native_contract_retry_preserves_tool_schema_and_adds_machine_observation() {
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
    );
    let signal = native_contract_repair_signal("native_plan_capability_missing");
    let repaired = native_contract_retry_request(&request, &signal);

    assert_eq!(repaired.tools, request.tools);
    assert_eq!(repaired.metadata, request.metadata);
    assert_eq!(repaired.messages.len(), 3);
    let observation: Value = serde_json::from_str(&signal).expect("machine observation json");
    assert_eq!(
        observation["protocol_observation"]["error_code"],
        "native_plan_capability_missing"
    );
    assert_eq!(
        observation["protocol_observation"]["required_argument_fields"],
        json!(["capability", "args"])
    );
    assert_eq!(
        repaired.messages[2].content,
        vec![ModelContentPart::Text { text: signal }]
    );
}
