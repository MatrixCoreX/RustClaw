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

fn respond_call(mut arguments: Value) -> ModelToolCall {
    if let Some(arguments) = arguments.as_object_mut() {
        arguments.entry("fields").or_insert_with(|| json!([]));
        arguments
            .entry("observed_fields")
            .or_insert_with(|| json!([]));
        arguments
            .entry("exact_field_count")
            .or_insert_with(|| json!(0));
    }
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
fn planner_projects_unknown_capability_data_without_dropping_machine_fields() {
    let mut loop_state = LoopState::default();
    loop_state.last_output = Some("IMAGE_GENERATE_DRY_RUN".to_string());
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "image_generate",
            Some("preview_generate".to_string()),
            json!({
                "output": "IMAGE_GENERATE_DRY_RUN",
                "extra": {
                    "provider": "minimax",
                    "model": "image-01",
                    "planned_outputs": [{
                        "type": "image_file",
                        "path": "document/media_dry_run/status.png"
                    }],
                    "async_contract": {
                        "status": "accepted",
                        "poll_after_seconds": 5
                    },
                    "api_key": "secret-value-must-not-reach-model"
                }
            }),
        ));

    let observation = planner_last_observation(&loop_state);

    assert!(observation.starts_with("capability_result_observation="));
    assert!(observation.contains("\"provider\":\"minimax\""));
    assert!(observation.contains("\"model\":\"image-01\""));
    assert!(observation.contains("\"planned_outputs\""));
    assert!(observation.contains("\"async_contract\""));
    assert!(!observation.contains("secret-value-must-not-reach-model"));
    assert!(!observation.eq("IMAGE_GENERATE_DRY_RUN"));
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
fn native_respond_materializes_exact_structured_object_fields() {
    let actions = actions_from_native_turn(
        &turn(
            vec![respond_call(json!({
                "shape": "object",
                "content": "",
                "items": [],
                "exact_item_count": 0,
                "fields": [
                    {"name": "provider", "value_json": "\"minimax\""},
                    {
                        "name": "async_contract",
                        "value_json": "{\"status\":\"accepted\",\"poll_after_seconds\":5}"
                    }
                ],
                "exact_field_count": 2
            }))],
            "",
        ),
        &callable_capabilities(),
    )
    .expect("object response");

    let AgentAction::Respond { content } = &actions[0] else {
        panic!("expected terminal response");
    };
    let content: Value = serde_json::from_str(content).expect("materialized object json");
    assert_eq!(content["provider"], "minimax");
    assert_eq!(content["async_contract"]["status"], "accepted");
    assert_eq!(content["async_contract"]["poll_after_seconds"], 5);
}

#[test]
fn native_respond_canonicalizes_only_equivalent_redundant_object_payloads() {
    let actions = actions_from_native_turn(
        &turn(
            vec![respond_call(json!({
                "shape": "object",
                "content": "{\"value\":\"minimax\",\"field_path\":\"llm.selected_vendor\"}",
                "fields": [
                    {"name": "field_path", "value_json": "\"llm.selected_vendor\""},
                    {"name": "value", "value_json": "\"minimax\""}
                ],
                "exact_field_count": 2
            }))],
            "",
        ),
        &callable_capabilities(),
    )
    .expect("equivalent redundant object response");
    let AgentAction::Respond { content } = &actions[0] else {
        panic!("expected terminal response");
    };
    assert_eq!(
        serde_json::from_str::<Value>(content).expect("materialized object"),
        json!({"field_path": "llm.selected_vendor", "value": "minimax"})
    );

    let contradictory = turn(
        vec![respond_call(json!({
            "shape": "object",
            "content": "{\"field_path\":\"llm.selected_vendor\",\"value\":\"other\"}",
            "fields": [
                {"name": "field_path", "value_json": "\"llm.selected_vendor\""},
                {"name": "value", "value_json": "\"minimax\""}
            ],
            "exact_field_count": 2
        }))],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&contradictory, &callable_capabilities())
            .expect_err("contradictory redundant object rejected"),
        "native_respond_object_non_field_payload"
    );
}

#[test]
fn native_respond_projects_exact_fields_from_successful_capability_observation() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "image.preview_generate",
            Some("preview_generate".to_string()),
            json!({
                "output": "dry_run",
                "extra": {
                    "provider": "minimax",
                    "model": "image-01",
                    "planned_outputs": [{
                        "path": "document/media_dry_run/status.png",
                        "type": "image_file"
                    }],
                    "async_contract": {
                        "status": "accepted",
                        "poll_after_seconds": 5
                    }
                }
            }),
        ));
    let native_turn = turn(
        vec![respond_call(json!({
            "shape": "observed_object",
            "content": "",
            "items": [],
            "exact_item_count": 0,
            "fields": [],
            "observed_fields": [
                {
                    "name": "provider",
                    "capability": "image.preview_generate",
                    "path": "data.extra.provider"
                },
                {
                    "name": "model",
                    "capability": "image.preview_generate",
                    "path": "data.extra.model"
                },
                {
                    "name": "planned_outputs",
                    "capability": "image.preview_generate",
                    "path": "data.extra.planned_outputs"
                },
                {
                    "name": "async_contract",
                    "capability": "image.preview_generate",
                    "path": "data.extra.async_contract"
                }
            ],
            "exact_field_count": 0
        }))],
        "",
    );

    let actions = actions_from_native_turn_with_groups(
        &native_turn,
        &callable_capabilities(),
        &BTreeMap::new(),
        Some(&loop_state),
    )
    .expect("observed object response");
    let AgentAction::Respond { content } = &actions[0] else {
        panic!("expected terminal response");
    };
    let content: Value = serde_json::from_str(content).expect("projected object");
    assert_eq!(content["provider"], "minimax");
    assert_eq!(content["model"], "image-01");
    assert_eq!(
        content["planned_outputs"][0]["path"],
        "document/media_dry_run/status.png"
    );
    assert_eq!(content["async_contract"]["poll_after_seconds"], 5);

    let contradictory_count = turn(
        vec![respond_call(json!({
            "shape": "observed_object",
            "content": "",
            "items": [],
            "exact_item_count": 0,
            "fields": [],
            "observed_fields": [{
                "name": "provider",
                "capability": "image.preview_generate",
                "path": "data.extra.provider"
            }],
            "exact_field_count": 2
        }))],
        "",
    );
    assert_eq!(
        actions_from_native_turn_with_groups(
            &contradictory_count,
            &callable_capabilities(),
            &BTreeMap::new(),
            Some(&loop_state),
        )
        .expect_err("non-neutral contradictory count rejected"),
        "native_respond_observed_object_count_mismatch"
    );
}

#[test]
fn native_respond_rejects_unobserved_or_invalid_field_references() {
    let mut failed_loop_state = LoopState::default();
    let mut failed = CapabilityResultEnvelope::ok(
        "image.preview_generate",
        Some("preview_generate".to_string()),
        json!({"extra": {"provider": "minimax"}}),
    );
    failed.status = claw_core::capability_result::CapabilityResultStatus::Error;
    failed_loop_state.capability_results.push(failed);

    let observed_turn = |path: &str| {
        turn(
            vec![respond_call(json!({
                "shape": "observed_object",
                "content": "",
                "items": [],
                "exact_item_count": 0,
                "fields": [],
                "observed_fields": [{
                    "name": "provider",
                    "capability": "image.preview_generate",
                    "path": path
                }],
                "exact_field_count": 1
            }))],
            "",
        )
    };

    assert_eq!(
        actions_from_native_turn_with_groups(
            &observed_turn("provider"),
            &callable_capabilities(),
            &BTreeMap::new(),
            Some(&failed_loop_state),
        )
        .expect_err("failed observations cannot authorize projection"),
        "native_respond_observed_capability_result_missing"
    );
    assert_eq!(
        actions_from_native_turn_with_groups(
            &observed_turn("provider"),
            &callable_capabilities(),
            &BTreeMap::new(),
            None,
        )
        .expect_err("missing loop observation state rejected"),
        "native_respond_observation_state_missing"
    );
    assert_eq!(
        actions_from_native_turn_with_groups(
            &observed_turn("provider value"),
            &callable_capabilities(),
            &BTreeMap::new(),
            Some(&LoopState::default()),
        )
        .expect_err("natural-language source reference rejected"),
        "native_respond_observed_path_invalid"
    );
}

#[test]
fn native_respond_rejects_invalid_or_duplicate_object_fields() {
    let invalid_json = turn(
        vec![respond_call(json!({
            "shape": "object",
            "content": "",
            "items": [],
            "exact_item_count": 0,
            "fields": [{"name": "provider", "value_json": "minimax"}],
            "exact_field_count": 1
        }))],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&invalid_json, &callable_capabilities())
            .expect_err("invalid JSON lexical value rejected"),
        "native_respond_object_field_json_invalid"
    );

    let duplicate = turn(
        vec![respond_call(json!({
            "shape": "object",
            "content": "",
            "items": [],
            "exact_item_count": 0,
            "fields": [
                {"name": "provider", "value_json": "\"minimax\""},
                {"name": "provider", "value_json": "\"other\""}
            ],
            "exact_field_count": 2
        }))],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&duplicate, &callable_capabilities())
            .expect_err("duplicate object field rejected"),
        "native_respond_object_field_duplicate"
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
        &BTreeMap::new(),
        &[],
        &[],
        &[],
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
    assert_eq!(request.tools.len(), 2);
    assert_eq!(request.tools[1].name, "respond");
    assert_eq!(
        request.tools[1].input_schema["properties"]["shape"]["enum"],
        json!(["free_text", "list", "object", "observed_object"])
    );
    assert_eq!(
        request.tools[1].input_schema["properties"]["observed_fields"]["items"]["required"],
        json!(["name", "capability", "path"])
    );
}

#[test]
fn native_request_exposes_registry_groups_as_distinct_tools() {
    let groups = vec![crate::capability_map::PlannerNativeCapabilityGroup {
        skill_name: "doc_parse".to_string(),
        tool_name: "call_doc_parse".to_string(),
        description: "runtime_capability_group_v1; semantic_tags=document_summary".to_string(),
        capability_names: vec!["doc_parse".to_string()],
        capability_argument_schemas: BTreeMap::from([(
            "doc_parse".to_string(),
            json!({
                "type": "object",
                "required": ["path"],
                "properties": {"path": {"type": "string"}},
                "additionalProperties": false
            }),
        )]),
    }];
    let callable = vec!["doc_parse".to_string(), "mcp.dynamic".to_string()];
    let mcp_schemas = BTreeMap::from([(
        "mcp.dynamic".to_string(),
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {"query": {"type": "string"}},
            "additionalProperties": false
        }),
    )]);
    let request = native_planner_request(
        "protocol",
        "current turn",
        None,
        &callable,
        &mcp_schemas,
        &groups,
        &groups,
        &[],
    );

    assert_eq!(request.tools.len(), 3);
    assert_eq!(request.tools[0].name, "call_capability");
    assert_eq!(
        request.tools[0].input_schema["oneOf"][0]["properties"]["capability"]["enum"],
        json!(["mcp.dynamic"])
    );
    assert_eq!(
        request.tools[0].input_schema["oneOf"][0]["properties"]["args"]["required"],
        json!(["query"])
    );
    assert_eq!(request.tools[1].name, "call_doc_parse");
    assert!(request.tools[1].description.contains("document_summary"));
    assert_eq!(
        request.tools[1].input_schema["oneOf"][0]["properties"]["capability"]["enum"],
        json!(["doc_parse"])
    );
    assert_eq!(
        request.tools[1].input_schema["oneOf"][0]["properties"]["args"]["required"],
        json!(["path"])
    );
    assert_eq!(
        request.tools[1].input_schema["oneOf"][0]["properties"]["args"]["additionalProperties"],
        json!(false)
    );
    assert_eq!(request.tools[2].name, "respond");

    let registry_only_request = native_planner_request(
        "protocol",
        "current turn",
        None,
        &["doc_parse".to_string()],
        &BTreeMap::new(),
        &groups,
        &groups,
        &[],
    );
    assert_eq!(registry_only_request.tools.len(), 2);
    assert_eq!(registry_only_request.tools[0].name, "call_doc_parse");
    assert_eq!(registry_only_request.tools[1].name, "respond");
    assert!(registry_only_request
        .tools
        .iter()
        .all(|tool| tool.name != "call_capability"));

    let group_map = BTreeMap::from([(
        "call_doc_parse".to_string(),
        BTreeSet::from(["doc_parse".to_string()]),
    )]);
    let actions = actions_from_native_turn_with_groups(
        &turn(
            vec![ModelToolCall {
                id: "group-call".to_string(),
                name: "call_doc_parse".to_string(),
                arguments: json!({"capability": "doc_parse", "args": {"path": "README.md"}}),
            }],
            "",
        ),
        &callable,
        &group_map,
        None,
    )
    .expect("group action");
    assert!(matches!(
        &actions[0],
        AgentAction::CallCapability { capability, .. } if capability == "doc_parse"
    ));
}

#[test]
fn native_respond_tool_requires_runtime_observation_for_machine_evidence() {
    let request = native_planner_request(
        "system",
        "user",
        None,
        &callable_capabilities(),
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    );
    let respond = request
        .tools
        .iter()
        .find(|tool| tool.name == "respond")
        .expect("respond tool");

    assert!(respond.description.contains("does not execute or simulate"));
    assert!(respond
        .description
        .contains("prior matching capability result"));
    assert!(respond
        .description
        .contains("domain parse/normalize/validate/preview"));
    assert!(respond
        .description
        .contains("not a substitute for the disclosed domain capability"));
    assert!(respond.description.contains("checkpoint"));
    assert!(respond.description.contains("verification"));
}

#[test]
fn native_request_loads_hidden_registry_groups_before_they_are_callable() {
    let groups = vec![crate::capability_map::PlannerNativeCapabilityGroup {
        skill_name: "doc_parse".to_string(),
        tool_name: "call_doc_parse".to_string(),
        description: "runtime_capability_group_v1".to_string(),
        capability_names: vec!["doc_parse".to_string()],
        capability_argument_schemas: BTreeMap::from([(
            "doc_parse".to_string(),
            json!({
                "type": "object",
                "required": ["path"],
                "properties": {"path": {"type": "string"}},
                "additionalProperties": false
            }),
        )]),
    }];
    let callable = vec!["doc_parse".to_string(), "mcp.dynamic".to_string()];
    let disclosed = disclosed_callable_capability_names(&callable, &groups, &[]);
    assert_eq!(disclosed, vec!["mcp.dynamic".to_string()]);

    let request = native_planner_request(
        "protocol",
        "current turn",
        None,
        &callable,
        &BTreeMap::new(),
        &groups,
        &[],
        &["doc_parse".to_string()],
    );
    assert_eq!(
        request
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec!["call_capability", "load_capability_groups", "respond"]
    );
    assert_eq!(
        request.tools[1].input_schema["properties"]["groups"]["items"]["enum"],
        json!(["doc_parse"])
    );

    let actions = actions_from_native_turn_with_groups(
        &turn(
            vec![ModelToolCall {
                id: "load-doc-parse".to_string(),
                name: "load_capability_groups".to_string(),
                arguments: json!({"groups": ["doc_parse"]}),
            }],
            "",
        ),
        &disclosed,
        &BTreeMap::new(),
        None,
    )
    .expect("loader action");
    assert!(matches!(
        &actions[0],
        AgentAction::CallTool { tool, args }
            if tool == "load_capability_groups" && args == &json!({"groups": ["doc_parse"]})
    ));

    let hidden_direct = turn(
        vec![ModelToolCall {
            id: "hidden-direct".to_string(),
            name: "call_capability".to_string(),
            arguments: json!({"capability": "doc_parse", "args": {}}),
        }],
        "",
    );
    assert_eq!(
        actions_from_native_turn_with_groups(&hidden_direct, &disclosed, &BTreeMap::new(), None,)
            .expect_err("hidden registry capability must not bypass loading"),
        "native_plan_capability_not_in_runtime_catalog"
    );
}

#[test]
fn native_group_rejects_capability_from_another_group() {
    let callable = vec!["doc_parse".to_string(), "filesystem.read_text".to_string()];
    let group_map = BTreeMap::from([(
        "call_doc_parse".to_string(),
        BTreeSet::from(["doc_parse".to_string()]),
    )]);
    let error = actions_from_native_turn_with_groups(
        &turn(
            vec![ModelToolCall {
                id: "wrong-group-call".to_string(),
                name: "call_doc_parse".to_string(),
                arguments: json!({"capability": "filesystem.read_text", "args": {"path": "README.md"}}),
            }],
            "",
        ),
        &callable,
        &group_map,
        None,
    )
    .expect_err("cross-group capability rejected");

    assert_eq!(error, "native_plan_capability_not_in_selected_group");
}

#[test]
fn native_contract_retry_scopes_required_tool_and_adds_machine_observation() {
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
        &BTreeMap::new(),
        &[],
        &[],
        &[],
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
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    );
    let repaired = native_contract_retry_request(&request, &signal);
    let observation: Value = serde_json::from_str(&signal).expect("machine observation json");

    assert_eq!(repaired.tools.len(), 1);
    assert_eq!(repaired.tools[0].name, "respond");
    assert_eq!(repaired.tool_choice, ModelToolChoice::Required);
    assert_eq!(observation["protocol_observation"]["tool_name"], "respond");
    assert_eq!(
        observation["protocol_observation"]["required_argument_fields"],
        json!([
            "shape",
            "content",
            "items",
            "exact_item_count",
            "fields",
            "observed_fields",
            "exact_field_count"
        ])
    );
    assert_eq!(
        observation["protocol_observation"]["next_action"],
        "retry_native_respond_call"
    );
}

#[test]
fn native_object_response_schema_and_repair_explain_serialized_json_values() {
    let request = native_planner_request(
        "protocol",
        "current turn",
        Some(90),
        &callable_capabilities(),
        &BTreeMap::new(),
        &[],
        &[],
        &[],
    );
    let respond = request
        .tools
        .iter()
        .find(|tool| tool.name == "respond")
        .expect("respond tool");
    let value_json_description = respond.input_schema["properties"]["fields"]["items"]
        ["properties"]["value_json"]["description"]
        .as_str()
        .expect("value_json description");
    assert!(value_json_description.contains("complete_serialized_json_value_v1"));
    assert!(value_json_description.contains("json_string_requires_surrounding_quotes=true"));

    let signal = native_contract_repair_signal("native_respond_object_field_json_invalid");
    let observation: Value = serde_json::from_str(&signal).expect("machine observation json");
    assert_eq!(
        observation["protocol_observation"]["argument_constraints"]["fields[].value_json"]
            ["encoding"],
        "complete_serialized_json_value"
    );
    assert_eq!(
        observation["protocol_observation"]["argument_constraints"]["fields[].value_json"]
            ["json_string_requires_surrounding_quotes"],
        true
    );
    assert_eq!(
        observation["protocol_observation"]["argument_constraints"]["fields[].value_json"]
            ["malformed_json"],
        "rejected"
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
        &BTreeMap::new(),
        &[],
        &[],
        &[],
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
