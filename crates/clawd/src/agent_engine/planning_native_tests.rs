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
        arguments
            .entry("terminal_intent")
            .or_insert_with(|| json!("answer"));
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
fn planner_runtime_feedback_is_not_hidden_by_an_older_capability_result() {
    let feedback = json!({
        "schema_version": 1,
        "owner_layer": "agent_loop",
        "kind": "answer_verifier_evidence_gap",
        "missing_evidence_fields": ["requested_result"],
        "model_feedback": {"trust": "untrusted_model_output", "retry_instruction": "bounded correction"},
    });
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "fixture_capability",
            None,
            json!({"output": {"observed": true}}),
        ));
    let previous = planner_last_observation(&loop_state);
    loop_state.last_output = Some(feedback.to_string());
    assert_eq!(
        planner_last_observation(&loop_state),
        previous,
        "unbound output must not impersonate runtime feedback"
    );
    loop_state.task_observations.push(feedback.clone());
    assert_eq!(
        serde_json::from_str::<Value>(&planner_last_observation(&loop_state)).unwrap(),
        feedback
    );
    loop_state.last_output = Some("later tool result".to_string());
    assert_eq!(
        planner_last_observation(&loop_state),
        previous,
        "stale runtime feedback must not hide a newer observation"
    );
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
fn native_respond_preserves_structured_clarification_control_fields() {
    let native_turn = turn(
        vec![respond_call(json!({
            "terminal_intent": "clarify",
            "clarify_reason_code": "missing_required_input",
            "missing_slot": "device_price_usd",
            "field_path": "nni.reward_apr.device_price_usd",
            "message_key": "agent.clarify.missing_required_input",
            "shape": "free_text",
            "content": "What is the device price in USD?",
            "items": [],
            "exact_item_count": 0
        }))],
        "",
    );
    let actions = actions_from_native_turn(&native_turn, &callable_capabilities())
        .expect("clarification action");
    let mut plan = build_plan_result_with_notes(
        None,
        "calculate annualized return",
        "{}",
        PlanKind::Native,
        &actions,
        "",
    );

    preserve_native_respond_control_fields(&native_turn, &mut plan);

    assert_eq!(plan.steps[0].args["terminal_intent"], "clarify");
    assert_eq!(plan.steps[0].args["missing_slot"], "device_price_usd");
    assert_eq!(
        plan.steps[0].args["field_path"],
        "nni.reward_apr.device_price_usd"
    );
}

#[test]
fn native_respond_rejects_clarification_without_missing_slot() {
    let error = actions_from_native_turn(
        &turn(
            vec![respond_call(json!({
                "terminal_intent": "clarify",
                "shape": "free_text",
                "content": "What value should I use?",
                "items": [],
                "exact_item_count": 0
            }))],
            "",
        ),
        &callable_capabilities(),
    )
    .expect_err("clarification without a machine slot must be rejected");

    assert_eq!(error, "native_respond_clarify_missing_slot_required");
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
fn native_respond_projects_matching_machine_fields_without_model_path_guessing() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(CapabilityResultEnvelope::ok(
            "filesystem.read_text_range",
            Some("read_text_range".to_string()),
            json!({
                "output": {
                    "path": "/workspace/README.md",
                    "line_count": 844,
                    "first_line": "# Agent Runtime"
                }
            }),
        ));
    let copied = turn(
        vec![respond_call(json!({
            "shape": "object",
            "content": "",
            "items": [],
            "exact_item_count": 0,
            "fields": [
                {"name": "path", "value_json": "\"/workspace/README.md\""},
                {"name": "line_count", "value_json": "844"},
                {"name": "first_line", "value_json": "\"# Agent Runtime\""}
            ],
            "observed_fields": [],
            "exact_field_count": 3
        }))],
        "",
    );

    let actions = actions_from_native_turn_with_groups(
        &copied,
        &["filesystem.read_text_range".to_string()],
        &BTreeMap::new(),
        Some(&loop_state),
    )
    .expect("matching fields are canonicalized from observations");
    let AgentAction::Respond { content } = &actions[0] else {
        panic!("expected terminal response");
    };
    assert_eq!(
        serde_json::from_str::<Value>(content).expect("projected object"),
        json!({
            "path": "/workspace/README.md",
            "line_count": 844,
            "first_line": "# Agent Runtime"
        })
    );

    let authored = turn(
        vec![respond_call(json!({
            "shape": "object",
            "content": "",
            "items": [],
            "exact_item_count": 0,
            "fields": [
                {"name": "summary", "value_json": "\"README has 844 lines\""}
            ],
            "observed_fields": [],
            "exact_field_count": 1
        }))],
        "",
    );
    assert!(
        actions_from_native_turn_with_groups(
            &authored,
            &["filesystem.read_text_range".to_string()],
            &BTreeMap::new(),
            Some(&loop_state),
        )
        .is_ok(),
        "model-authored fields remain valid"
    );
}

#[test]
fn native_respond_rejects_unobserved_or_invalid_field_references() {
    let mut failed_loop_state = LoopState::default();
    let failed = crate::capability_result::failed_execution_envelope(
        "image.preview_generate",
        "step_1",
        &json!({"action": "preview_generate"}),
        &crate::skills::structured_skill_error_from_parts(
            "image_edit",
            "provider_rejected",
            "provider rejected request",
            None,
            Some(json!({"provider": "minimax", "status": "error"})),
        ),
    );
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
            &observed_turn("data.extra.provider"),
            &callable_capabilities(),
            &BTreeMap::new(),
            Some(&failed_loop_state),
        )
        .expect_err("failed result data cannot authorize projection"),
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
fn native_respond_projects_structured_fields_from_failed_capability_observation() {
    let mut loop_state = LoopState::default();
    loop_state
        .capability_results
        .push(crate::capability_result::failed_execution_envelope(
            "x.draft_preview",
            "step_2",
            &json!({"action": "post"}),
            &crate::skills::structured_skill_error_from_parts(
                "x",
                "invalid_input",
                "conflicting flags",
                None,
                Some(json!({
                    "status": "error",
                    "published": false,
                    "would_execute": false,
                    "external_call_count": 0
                })),
            ),
        ));
    let native_turn = turn(
        vec![respond_call(json!({
            "shape": "observed_object",
            "content": "",
            "items": [],
            "exact_item_count": 0,
            "fields": [],
            "observed_fields": [
                {"name": "error_code", "capability": "x.draft_preview", "path": "error.code"},
                {"name": "status", "capability": "x.draft_preview", "path": "status"},
                {"name": "published", "capability": "x.draft_preview", "path": "error.details.structured_error.extra.published"},
                {"name": "would_execute", "capability": "x.draft_preview", "path": "error.details.structured_error.extra.would_execute"},
                {"name": "external_call_count", "capability": "x.draft_preview", "path": "error.details.structured_error.extra.external_call_count"}
            ],
            "exact_field_count": 5
        }))],
        "",
    );

    let actions = actions_from_native_turn_with_groups(
        &native_turn,
        &["x.draft_preview".to_string()],
        &BTreeMap::new(),
        Some(&loop_state),
    )
    .expect("structured failure projection");
    let AgentAction::Respond { content } = &actions[0] else {
        panic!("expected terminal response");
    };
    assert_eq!(
        serde_json::from_str::<Value>(content).expect("projected failure object"),
        json!({
            "error_code": "invalid_input",
            "status": "error",
            "published": false,
            "would_execute": false,
            "external_call_count": 0
        })
    );
}

#[test]
fn native_respond_repairs_plain_scalar_and_rejects_structural_or_duplicate_fields() {
    let plain_scalar = turn(
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
    let actions = actions_from_native_turn(&plain_scalar, &callable_capabilities())
        .expect("plain scalar is normalized as a JSON string");
    let AgentAction::Respond { content } = &actions[0] else {
        panic!("expected terminal response");
    };
    assert_eq!(
        serde_json::from_str::<Value>(content).expect("normalized object"),
        json!({"provider": "minimax"})
    );

    let malformed_structure = turn(
        vec![respond_call(json!({
            "shape": "object",
            "content": "",
            "items": [],
            "exact_item_count": 0,
            "fields": [{"name": "provider", "value_json": "{broken"}],
            "exact_field_count": 1
        }))],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&malformed_structure, &callable_capabilities())
            .expect_err("malformed structural JSON remains rejected"),
        "native_respond_object_field_json_invalid"
    );

    let trailing_quote = turn(
        vec![respond_call(json!({
            "shape": "object",
            "content": "",
            "items": [],
            "exact_item_count": 0,
            "fields": [{"name": "items", "value_json": "[\"one\",\"two\"]\""}],
            "exact_field_count": 1
        }))],
        "",
    );
    let actions = actions_from_native_turn(&trailing_quote, &callable_capabilities())
        .expect("one trailing quote after a complete container is repaired");
    let AgentAction::Respond { content } = &actions[0] else {
        panic!("expected terminal response");
    };
    assert_eq!(
        serde_json::from_str::<Value>(content).expect("normalized object"),
        json!({"items": ["one", "two"]})
    );

    let non_string_json = turn(
        vec![respond_call(json!({
            "shape": "object",
            "content": "",
            "items": [],
            "exact_item_count": 0,
            "fields": [{"name": "provider", "value_json": null}],
            "exact_field_count": 1
        }))],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&non_string_json, &callable_capabilities())
            .expect_err("schema-level null rejected"),
        "native_respond_object_field_value_invalid"
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
fn native_tool_normalizes_only_schema_proven_empty_argument_objects() {
    let capability = "coding_workflow.preview_repair";
    let empty_args_turn = turn(
        vec![ModelToolCall {
            id: "call-empty-args".to_string(),
            name: "call_capability".to_string(),
            arguments: json!({"capability": capability, "args": ""}),
        }],
        "",
    );
    let callable = vec![capability.to_string()];
    let schemas = BTreeMap::from([(
        capability.to_string(),
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    )]);

    let actions = actions_from_native_turn_with_schemas(
        &empty_args_turn,
        &callable,
        &BTreeMap::new(),
        &schemas,
        None,
    )
    .expect("provider empty-object encoding normalized");
    assert!(matches!(
        &actions[0],
        AgentAction::CallCapability {
            capability: selected,
            args
        } if selected == capability && args == &json!({})
    ));

    let required_schema = BTreeMap::from([(
        capability.to_string(),
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {"path": {"type": "string"}},
            "additionalProperties": false
        }),
    )]);
    assert_eq!(
        actions_from_native_turn_with_schemas(
            &empty_args_turn,
            &callable,
            &BTreeMap::new(),
            &required_schema,
            None,
        )
        .expect_err("required args cannot be normalized away"),
        "native_plan_args_not_object"
    );

    let non_empty_string = turn(
        vec![ModelToolCall {
            id: "call-non-empty-args".to_string(),
            name: "call_capability".to_string(),
            arguments: json!({"capability": capability, "args": "unexpected"}),
        }],
        "",
    );
    assert_eq!(
        actions_from_native_turn_with_schemas(
            &non_empty_string,
            &callable,
            &BTreeMap::new(),
            &schemas,
            None,
        )
        .expect_err("non-empty strings remain invalid"),
        "native_plan_args_not_object"
    );
}

#[test]
fn native_tool_normalizes_unambiguous_schema_transport_wrappers() {
    let capability = "web.search_results";
    let callable = vec![capability.to_string()];
    let tool_name = native_capability_leaf_tool_name(capability);
    let group_map = BTreeMap::from([(tool_name.clone(), BTreeSet::from([capability.to_string()]))]);
    let schemas = BTreeMap::from([(
        capability.to_string(),
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {"type": "string"},
                "top_k": {"type": "integer"},
                "domains_allow": {
                    "type": "array",
                    "items": {"type": "string"}
                }
            },
            "additionalProperties": false
        }),
    )]);
    let actions = actions_from_native_turn_with_schemas(
        &turn(
            vec![ModelToolCall {
                id: "provider-wrapper".to_string(),
                name: tool_name,
                arguments: json!({
                    "query": "runtime",
                    "top_k": "3",
                    "domains_allow": {"item": "example.com"}
                }),
            }],
            "",
        ),
        &callable,
        &group_map,
        &schemas,
        None,
    )
    .expect("schema-proven transport wrappers normalize");

    assert!(matches!(
        actions.as_slice(),
        [AgentAction::CallCapability { capability, args }]
            if capability == "web.search_results"
                && args["top_k"] == json!(3)
                && args["domains_allow"] == json!(["example.com"])
    ));
    assert_eq!(
        normalize_native_argument_to_schema(
            &json!({"type": "array", "items": {"type": "string"}}),
            &json!({"item": "example.com", "extra": "ambiguous"}),
        ),
        json!({"item": "example.com", "extra": "ambiguous"})
    );
}

#[test]
fn native_capability_loader_normalizes_single_item_transport_wrapper() {
    let action = action_from_native_capability_group_load(&ModelToolCall {
        id: "loader-wrapper".to_string(),
        name: "load_capability_groups".to_string(),
        arguments: json!({
            "op": "load_groups",
            "groups": {"item": "web_search_extract"}
        }),
    })
    .expect("single item wrapper normalizes");

    assert!(matches!(
        action,
        AgentAction::CallTool { tool, args }
            if tool == "load_capability_groups"
                && args["groups"] == json!(["web_search_extract"])
    ));
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
        capability_descriptions: BTreeMap::new(),
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
    assert!(request.tools[1]
        .description
        .contains("direct_runtime_capability_arguments_v1"));
    assert_eq!(request.tools[1].input_schema["required"], json!(["path"]));
    assert_eq!(
        request.tools[1].input_schema["additionalProperties"],
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
                arguments: json!({"path": "README.md"}),
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
        AgentAction::CallCapability { capability, args }
            if capability == "doc_parse" && args == &json!({"path": "README.md"})
    ));
}

#[test]
fn native_request_expands_multi_capability_groups_into_direct_leaf_tools() {
    let group = crate::capability_map::PlannerNativeCapabilityGroup {
        skill_name: "fs_basic".to_string(),
        tool_name: "call_fs_basic".to_string(),
        description: "runtime_capability_group_v1; semantic_tags=filesystem".to_string(),
        capability_names: vec![
            "filesystem.list_entries".to_string(),
            "filesystem.read_text_range".to_string(),
        ],
        capability_descriptions: BTreeMap::from([
            (
                "filesystem.list_entries".to_string(),
                "typed direct-child directory inventory".to_string(),
            ),
            (
                "filesystem.read_text_range".to_string(),
                "bounded read for a known path".to_string(),
            ),
        ]),
        capability_argument_schemas: BTreeMap::from([
            (
                "filesystem.list_entries".to_string(),
                json!({
                    "type": "object",
                    "required": ["path"],
                    "properties": {"path": {"type": "string"}},
                    "additionalProperties": false
                }),
            ),
            (
                "filesystem.read_text_range".to_string(),
                json!({
                    "type": "object",
                    "required": ["path"],
                    "properties": {
                        "path": {"type": "string"},
                        "start_line": {"type": "integer"}
                    },
                    "additionalProperties": false
                }),
            ),
        ]),
    };
    let groups = vec![group.clone()];
    let callable = group.capability_names.clone();
    let request = native_planner_request(
        "protocol",
        "current turn",
        None,
        &callable,
        &BTreeMap::new(),
        &groups,
        &groups,
        &[],
    );
    let list_tool_name = native_capability_leaf_tool_name("filesystem.list_entries");
    let read_tool_name = native_capability_leaf_tool_name("filesystem.read_text_range");

    assert_eq!(list_tool_name, "call_filesystem_list_entries");
    assert_eq!(read_tool_name, "call_filesystem_read_text_range");
    assert_ne!(list_tool_name, read_tool_name);
    assert!(list_tool_name.len() <= MAX_NATIVE_TOOL_NAME_BYTES);
    assert!(read_tool_name.len() <= MAX_NATIVE_TOOL_NAME_BYTES);
    assert_eq!(request.tools.len(), 3);
    assert_eq!(request.tools[0].name, list_tool_name);
    assert!(request.tools[0]
        .description
        .contains("typed direct-child directory inventory"));
    assert_eq!(request.tools[0].input_schema["required"], json!(["path"]));
    assert!(request.tools[0].input_schema.get("oneOf").is_none());
    assert_eq!(request.tools[1].name, read_tool_name);
    assert_eq!(request.tools[1].input_schema["required"], json!(["path"]));
    assert!(request.tools[1].input_schema.get("oneOf").is_none());
    assert_eq!(request.tools[2].name, "respond");

    let tool_map = native_capability_tool_map(&groups);
    let actions = actions_from_native_turn_with_groups(
        &turn(
            vec![ModelToolCall {
                id: "read-fixture".to_string(),
                name: read_tool_name,
                arguments: json!({"path": "README.md", "start_line": 1}),
            }],
            "",
        ),
        &callable,
        &tool_map,
        None,
    )
    .expect("direct leaf action");
    assert!(matches!(
        &actions[0],
        AgentAction::CallCapability { capability, args }
            if capability == "filesystem.read_text_range"
                && args == &json!({"path": "README.md", "start_line": 1})
    ));
}

#[test]
fn native_leaf_overlong_name_is_bounded_and_hash_stable() {
    let capability = format!("filesystem.{}", "very_long_capability_segment_".repeat(4));
    let first = native_capability_leaf_tool_name(&capability);
    let second = native_capability_leaf_tool_name(&capability);

    assert_eq!(first, second);
    assert_eq!(first.len(), MAX_NATIVE_TOOL_NAME_BYTES);
    assert!(first.starts_with("call_filesystem_very_long"));
    assert!(first.contains("__"));
}

#[test]
fn native_single_capability_group_accepts_direct_empty_leaf_arguments() {
    let group_map = BTreeMap::from([(
        "call_log_analyze".to_string(),
        BTreeSet::from(["log_analyze".to_string()]),
    )]);
    let actions = actions_from_native_turn_with_groups(
        &turn(
            vec![ModelToolCall {
                id: "analyze-default-log".to_string(),
                name: "call_log_analyze".to_string(),
                arguments: json!({}),
            }],
            "",
        ),
        &["log_analyze".to_string()],
        &group_map,
        None,
    )
    .expect("single capability direct arguments");

    assert!(matches!(
        &actions[0],
        AgentAction::CallCapability { capability, args }
            if capability == "log_analyze" && args == &json!({})
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
        capability_descriptions: BTreeMap::new(),
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
        request.tools[1].input_schema["oneOf"][2]["properties"]["groups"]["items"]["enum"],
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
            if tool == "load_capability_groups"
                && args == &json!({"op": "load_groups", "groups": ["doc_parse"]})
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
fn native_leaf_rejects_missing_direct_required_arguments() {
    let capability = "filesystem.read_text_range";
    let callable = vec![capability.to_string()];
    let group_map = BTreeMap::from([(
        "call_filesystem_read_text_range".to_string(),
        BTreeSet::from([capability.to_string()]),
    )]);
    let schemas = BTreeMap::from([(
        capability.to_string(),
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {"path": {"type": "string"}},
            "additionalProperties": false
        }),
    )]);
    let error = actions_from_native_turn_with_schemas(
        &turn(
            vec![ModelToolCall {
                id: "missing-path".to_string(),
                name: "call_filesystem_read_text_range".to_string(),
                arguments: json!({}),
            }],
            "",
        ),
        &callable,
        &group_map,
        &schemas,
        None,
    )
    .expect_err("missing direct leaf argument rejected");

    assert_eq!(error, "native_plan_required_args_missing");
}

#[test]
fn native_leaf_rejects_empty_direct_required_arguments() {
    let capability = "config.read_fields";
    let callable = vec![capability.to_string()];
    let tool_name = native_capability_leaf_tool_name(capability);
    let group_map = BTreeMap::from([(tool_name.clone(), BTreeSet::from([capability.to_string()]))]);
    let schemas = BTreeMap::from([(
        capability.to_string(),
        json!({
            "type": "object",
            "required": ["path", "field_paths"],
            "properties": {
                "path": {"type": "string", "minLength": 1},
                "field_paths": {
                    "type": "array",
                    "minItems": 1,
                    "items": {"type": "string", "minLength": 1}
                }
            },
            "additionalProperties": false
        }),
    )]);

    for (case, arguments) in [
        ("empty path", json!({"path": " ", "field_paths": ["app"]})),
        (
            "empty field list",
            json!({"path": "config.toml", "field_paths": []}),
        ),
        (
            "blank field item",
            json!({"path": "config.toml", "field_paths": [" "]}),
        ),
    ] {
        let error = actions_from_native_turn_with_schemas(
            &turn(
                vec![ModelToolCall {
                    id: case.to_string(),
                    name: tool_name.clone(),
                    arguments,
                }],
                "",
            ),
            &callable,
            &group_map,
            &schemas,
            None,
        )
        .expect_err("empty required direct leaf argument rejected");

        assert_eq!(error, "native_plan_required_args_missing", "{case}");
    }
}

#[test]
fn native_leaf_accepts_required_argument_when_its_schema_allows_null() {
    let capability = "git.push";
    let callable = vec![capability.to_string()];
    let tool_name = native_capability_leaf_tool_name(capability);
    let group_map = BTreeMap::from([(tool_name.clone(), BTreeSet::from([capability.to_string()]))]);
    let schema = json!({
        "type": "object",
        "required": ["expected_remote_sha"],
        "properties": {
            "expected_remote_sha": {
                "anyOf": [
                    {"type": "string", "pattern": "^[0-9a-f]{40}$"},
                    {"type": "null"}
                ]
            }
        },
        "additionalProperties": false
    });
    let schemas = BTreeMap::from([(capability.to_string(), schema)]);

    let actions = actions_from_native_turn_with_schemas(
        &turn(
            vec![ModelToolCall {
                id: "nullable-required".to_string(),
                name: tool_name.clone(),
                arguments: json!({"expected_remote_sha": null}),
            }],
            "",
        ),
        &callable,
        &group_map,
        &schemas,
        None,
    )
    .expect("required nullable argument is present");

    assert!(matches!(
        actions.as_slice(),
        [AgentAction::CallCapability { capability, args }]
            if capability == "git.push" && args["expected_remote_sha"].is_null()
    ));

    let error = actions_from_native_turn_with_schemas(
        &turn(
            vec![ModelToolCall {
                id: "missing-nullable-required".to_string(),
                name: tool_name,
                arguments: json!({}),
            }],
            "",
        ),
        &callable,
        &group_map,
        &schemas,
        None,
    )
    .expect_err("missing nullable required argument remains invalid");
    assert_eq!(error, "native_plan_required_args_missing");
}

#[path = "planning_native_contract_recovery_tests.rs"]
mod contract_recovery;
