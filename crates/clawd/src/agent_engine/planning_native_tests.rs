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

fn respond_call(arguments: Value) -> ModelToolCall {
    ModelToolCall {
        id: "respond-1".to_string(),
        name: "respond".to_string(),
        arguments,
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
fn native_terminal_text_requires_the_structured_respond_tool() {
    assert_eq!(
        actions_from_native_turn(&turn(Vec::new(), "Done."), &callable_capabilities())
            .expect_err("bare terminal text rejected"),
        "native_plan_respond_tool_required"
    );
}

#[test]
fn native_respond_maps_free_text_contract_to_terminal_action() {
    let actions = actions_from_native_turn(
        &turn(
            vec![respond_call(json!({
                "shape": "free_text",
                "content": "Done.",
                "items": [],
                "exact_item_count": 0
            }))],
            "",
        ),
        &callable_capabilities(),
    )
    .expect("terminal action");

    assert!(matches!(
        &actions[0],
        AgentAction::Respond { content } if content == "Done."
    ));
}

#[test]
fn native_respond_preserves_single_scalar_without_a_list_marker() {
    let actions = actions_from_native_turn(
        &turn(
            vec![respond_call(json!({
                "shape": "free_text",
                "content": "RC-CONT-CN-0428-A",
                "items": [],
                "exact_item_count": 0
            }))],
            "",
        ),
        &callable_capabilities(),
    )
    .expect("scalar response");

    assert!(matches!(
        &actions[0],
        AgentAction::Respond { content } if content == "RC-CONT-CN-0428-A"
    ));
}

#[test]
fn native_respond_renders_only_the_exact_structured_list_items() {
    let actions = actions_from_native_turn(
        &turn(
            vec![respond_call(json!({
                "shape": "list",
                "content": "",
                "items": ["first", "second", "third"],
                "exact_item_count": 3
            }))],
            "",
        ),
        &callable_capabilities(),
    )
    .expect("list response");

    assert!(matches!(
        &actions[0],
        AgentAction::Respond { content }
            if content == "1. first\n2. second\n3. third"
    ));
}

#[test]
fn native_respond_rejects_list_count_mismatch_and_extra_content() {
    let count_mismatch = turn(
        vec![respond_call(json!({
            "shape": "list",
            "content": "",
            "items": ["first", "second"],
            "exact_item_count": 3
        }))],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&count_mismatch, &callable_capabilities())
            .expect_err("count mismatch rejected"),
        "native_respond_list_count_mismatch"
    );

    let extra_content = turn(
        vec![respond_call(json!({
            "shape": "list",
            "content": "preface",
            "items": ["first"],
            "exact_item_count": 1
        }))],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&extra_content, &callable_capabilities())
            .expect_err("list preface rejected"),
        "native_respond_list_content_not_empty"
    );
}

#[test]
fn native_respond_cannot_be_mixed_with_runtime_actions() {
    let mixed = turn(
        vec![
            ModelToolCall {
                id: "call-1".to_string(),
                name: "call_capability".to_string(),
                arguments: json!({
                    "capability": "fs.read",
                    "args": {"path": "README.md"}
                }),
            },
            respond_call(json!({
                "shape": "free_text",
                "content": "Done.",
                "items": [],
                "exact_item_count": 0
            })),
        ],
        "",
    );

    assert_eq!(
        actions_from_native_turn(&mixed, &callable_capabilities())
            .expect_err("mixed terminal and executable actions rejected"),
        "native_respond_mixed_actions"
    );
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
        "config.guard(purpose=authoritative validation,semantic_tags=config_safety)",
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
    assert!(
        request.tools[0].input_schema["properties"]["capability"]["description"]
            .as_str()
            .expect("capability description")
            .contains("runtime_leaf_capability_contracts_v1=config.guard")
    );
    assert_eq!(request.tools.len(), 2);
    assert_eq!(request.tools[1].name, "respond");
    assert_eq!(
        request.tools[1].input_schema["properties"]["shape"]["enum"],
        json!(["free_text", "list"])
    );
}

#[test]
fn native_contract_retry_scopes_required_tool_and_adds_machine_observation() {
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
        "",
    );
    let signal = native_contract_repair_signal("native_plan_capability_missing");
    let repaired = native_contract_retry_request(&request, &signal);

    assert_eq!(repaired.tools.len(), 1);
    assert_eq!(repaired.tools[0].name, "call_capability");
    assert_eq!(repaired.tool_choice, ModelToolChoice::Required);
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

#[test]
fn native_response_contract_retry_targets_the_respond_schema() {
    let signal = native_contract_repair_signal("native_respond_list_count_mismatch");
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
        "",
    );
    let repaired = native_contract_retry_request(&request, &signal);
    let observation: Value = serde_json::from_str(&signal).expect("machine observation json");

    assert_eq!(repaired.tools.len(), 1);
    assert_eq!(repaired.tools[0].name, "respond");
    assert_eq!(repaired.tool_choice, ModelToolChoice::Required);
    assert_eq!(observation["protocol_observation"]["tool_name"], "respond");
    assert_eq!(
        observation["protocol_observation"]["required_argument_fields"],
        json!(["shape", "content", "items", "exact_item_count"])
    );
    assert_eq!(
        observation["protocol_observation"]["next_action"],
        "retry_native_respond_call"
    );
}

#[test]
fn native_contract_repair_supports_two_bounded_protocol_transitions() {
    assert_eq!(MAX_NATIVE_CONTRACT_REPAIR_ATTEMPTS, 2);

    let capability_signal = native_contract_repair_signal("native_plan_capability_missing");
    let respond_signal = native_contract_repair_signal("native_plan_respond_tool_required");
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
        "",
    );

    let capability_retry = native_contract_retry_request(&request, &capability_signal);
    let respond_retry = native_contract_retry_request(&request, &respond_signal);
    assert_eq!(capability_retry.tools.len(), 1);
    assert_eq!(capability_retry.tools[0].name, "call_capability");
    assert_eq!(respond_retry.tools.len(), 1);
    assert_eq!(respond_retry.tools[0].name, "respond");
    assert_eq!(capability_retry.tool_choice, ModelToolChoice::Required);
    assert_eq!(respond_retry.tool_choice, ModelToolChoice::Required);

    let notes = native_contract_repair_notes(&[
        "native_plan_capability_missing".to_string(),
        "native_plan_respond_tool_required".to_string(),
    ]);
    assert_eq!(
        notes,
        "native_contract_repair_reason_codes=native_plan_capability_missing,native_plan_respond_tool_required"
    );
}

#[test]
fn native_contract_repair_notes_are_empty_without_a_retry() {
    assert!(native_contract_repair_notes(&[]).is_empty());
}
